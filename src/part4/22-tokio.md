# 22. tokio 实战：从原理到工程

> 一句话：`tokio::spawn` 要求 `F: Send + 'static` ——
> **两个约束的理由完全不同**：`Send` 因为任务会被搬到别的 worker 线程，
> `'static` 因为任务可能比调用者活得久。
> 这一章把第 12、18、20、21 章的结论落到一个真实的 executor 上。

前三章把异步拆到了状态机、`Pin`、`Send` 传染。
这一章换到**工程视角**：这些机制在 tokio 里长什么样。

★ 本章的骨架是一条因果链：

```text
tokio::spawn 要求 F: Send + 'static
       ↓ 为什么？
任务会被搬到别的 worker 线程上跑，而且可能比调用者活得久
       ↓ 这带来什么？
① 捕获的东西必须 Send（第 12 章）
② 不能借用局部变量（'static）
③ 状态机里的字段不能 !Send（第 20 章）
④ async fn in trait 表达不了 Send（第 21 章）
       ↓ 怎么破？
缩小作用域 / Arc / spawn_blocking / LocalSet
```

## 22.0 一个会让你卡住的例子

你写了一个自认为没问题的并发任务：

```rust
use std::sync::{Arc, Mutex};

pub async fn bad(c: Arc<Mutex<u64>>) {
    let g = c.lock().unwrap();
    tokio::task::yield_now().await;    // ← 让出一下，让别的任务跑
    let _ = *g;
}

pub fn spawn_it(c: Arc<Mutex<u64>>) {
    tokio::spawn(bad(c));
}
```

```text
error: future cannot be sent between threads safely
  = help: within `impl Future<Output = ()>`, the trait `Send` is not
          implemented for `std::sync::MutexGuard<'_, u64>`
note: future is not `Send` as this value is used across an await
note: required by a bound in `spawn`
```

**这是第 20 章那个错误** —— 措辞一字不差。

于是你改用"把数据搬进去"的写法：

```rust
pub fn spawn_it() {
    let v = vec![1u64, 2];
    tokio::spawn(async { v.len() });
}
```

```text
error[E0373]: async block may outlive the current function, but it borrows `v`,
              which is owned by the current function
  = help: use `move` to force the async block to take ownership
```

**这次是另一个约束：`'static`。**

第三个问题更微妙 —— 下面这段**编译得过**：

```rust
pub async fn rc_within_task() -> u64 {
    let r = std::rc::Rc::new(1u64);      // Rc 是 !Send
    tokio::task::yield_now().await;      // r 还跨了 await
    *r
}
```

**`Rc` 是 `!Send`，它甚至跨过了 `await`，但这段代码完全合法。** 为什么？

三个现象指向同一个约束的两个半边。

## 22.1 表层解释（官方书会怎么讲）

官方书会说：

- tokio 是异步运行时，提供 reactor + executor；
- `#[tokio::main]` 把 `async fn main` 变成同步 `main` + `block_on`；
- `tokio::spawn` 要求 `Send + 'static`，因为任务要在多线程 runtime 上跑；
- 阻塞操作要用 `spawn_blocking`。

这些都对，但"要求 `Send + 'static`"这句话**经常被当成一个必须记住的规则**。
本章要把它拆成**两个独立的约束**，各自有各自的理由 ——
拆开之后，22.0 的三个现象就都有了解释。

## 22.2 编译器眼里的样子

### 22.2.1 约束的两半

`tokio::spawn` 的签名（简化）：

```rust
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
```

**注意这是两个不同的约束**：

| 约束 | 为什么需要 |
|---|---|
| `Send` | 任务会被**搬到别的 worker 线程**上执行 |
| `'static` | 任务**可能比调用者活得久**（fire-and-forget） |

★ 对照 `std::thread::spawn`：

```rust
pub fn spawn<F, T>(f: F) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
```

**同样的 `Send + 'static`** —— 因为同样的两个理由（第 12 章）。

> **tokio 没有引入新概念。** 它只是把"线程"换成了"任务"：
> 线程会被 OS 调度到不同 CPU 上 → `Send`；
> 线程可能比创建者活得久 → `'static`。

### 22.2.2 `Send` 那一半：状态机的字段（第 20 章）

`fail/guard_across_await.rs` 的错误和第 20 章**一字不差**：

```text
error: future cannot be sent between threads safely
  = help: within `impl Future<Output = ()>`, the trait `Send` is not
          implemented for `std::sync::MutexGuard<'_, u64>`
note: future is not `Send` as this value is used across an await
  |
  |     let g = c.lock().unwrap();
  |         - has type `std::sync::MutexGuard<'_, u64>` which is not `Send`
  |     tokio::task::yield_now().await;
  |                                ^^^^^ await occurs here, with `g` maybe used later
```

★ tokio 的报错**多了一行**：

```text
note: required by a bound in `spawn`
```

**它告诉你要求来自哪里。** 这是"约束从哪里来"的直接证据 ——
来自 `spawn` 的 bound，不是来自 `async`。

**修法**（`src/lib.rs` 的 `shared_counter`）：把锁的作用域收进一个块。

```rust
tokio::spawn(async move {
    let cur = { let g = c.lock().unwrap(); *g };   // ← guard 在 await 之前 drop
    tokio::task::yield_now().await;
    let mut g = c.lock().unwrap();
    *g = cur + 1;
});
```

⚠️ **`drop(g)` 没有用** —— 第 20 章实测过，编译器数的是**变量**的存活区间。

### 22.2.3 `'static` 那一半：不能借用局部变量

`fail/borrow_local.rs`：

```text
error[E0373]: async block may outlive the current function, but it borrows `v`,
              which is owned by the current function
  = help: use `move` to force the async block to take ownership
```

★ **编译器直接给出了修法**：`use move`。

两种破法：

| 破法 | 例子 | 代价 |
|---|---|---|
| **把所有权搬进去** | `async move { s.len() }` | 值被移走，调用者不能再用了 |
| **共享所有权** | `Arc<str>` / `Arc<[T]>` | 一次原子计数（第 13 章） |

```rust
pub fn spawn_owned(s: String) -> JoinHandle<usize> {
    tokio::spawn(async move { s.len() })
}

pub fn spawn_shared(s: Arc<str>) -> JoinHandle<usize> {
    tokio::spawn(async move { s.len() })
}
```

### 22.2.4 那个"编译得过"的例子：`Rc` 在 async 里完全可以用

回到 22.0 的第三个现象：

```rust
pub async fn rc_within_task() -> u64 {
    let r = std::rc::Rc::new(1u64);      // Rc 是 !Send
    tokio::task::yield_now().await;      // r 跨了 await
    *r
}
```

**它编译得过。** 因为**它没有被 `spawn`**。

★ 这是本章最重要的一条对照：

> **`Send` 的要求来自 `spawn`，不是来自 `async`。**

汇编证据（`tools/evidence.sh ch22-tokio`）：

```asm
_rc_within_task:
	strb	wzr, [x8, #24]      ; 只写判别式
	ret
```

**它就是一个普通的 `async fn`**，和 `Rc` 在同步代码里一样自由。

★ 推论：**`Rc` 在异步代码里不是禁忌** ——
只要那个 future 不被 `spawn` 到多线程 runtime 上。
而如果确实需要 `!Send` + 并发，tokio 提供了逃生门（22.2.5）。

### 22.2.5 逃生门：`LocalSet` / `spawn_local`

```rust
pub async fn local_task() -> u64 {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let r = std::rc::Rc::new(7u64);
            tokio::task::spawn_local(async move { *r }).await.unwrap()
        })
        .await
}
```

```asm
_local_task:
	strb	wzr, [x8, #72]
	ret
```

**`spawn_local` 不要求 `Send`** —— 因为 `LocalSet` 保证这些任务
**在同一个线程**上跑。

★ 这是"`Send` 的要求来自 spawn"的**直接证明**：
**换一个 spawn 方式，约束就没了。**

### 22.2.6 阻塞操作：`spawn_blocking`

工程上最常踩的坑：**在异步任务里做阻塞操作，会卡住整个 worker 线程。**

tokio 的 worker 线程数默认 = CPU 核数（多线程 runtime）。
一个任务阻塞住，那个 worker 上的**其他所有任务都停摆**。

```rust
pub async fn blocking_work(data: Vec<u64>) -> u64 {
    tokio::task::spawn_blocking(move || {
        data.iter().fold(0u64, |a, b| a.wrapping_add(*b))
    })
    .await
    .expect("blocking task panicked")
}
```

★ 注意它的约束**不是** `Future + Send + 'static`，而是
`FnOnce() -> R + Send + 'static`：

| | `spawn` | `spawn_blocking` |
|---|---|---|
| 接受 | `Future + Send + 'static` | `FnOnce() -> R + Send + 'static` |
| 跑在 | async worker | **阻塞线程池**（默认上限 512） |
| 用于 | 异步 IO | CPU 密集 / 同步 IO / 调 C 库 |

**共同点是 `Send + 'static`** —— 因为两者都会**跨线程、跨时间**。

### 22.2.7 运行期演示

`examples/ch22-tokio/src/main.rs`：

```bash
cargo run -p ch22-tokio
# total = 4
```

4 个任务并发，每个 +1。`#[tokio::main]` 做的事是
**构造 runtime 并 `block_on`** —— 与第 18 章手写的 `block_on` 是同一个东西，
只是 tokio 的版本带线程池、IO 驱动、定时器驱动。

## 22.3 为什么必须这样设计

### 为什么 `Send` 是必需的

因为**任务会被搬到别的线程**。

tokio 的多线程 runtime 有一组 worker 线程，任务被放进一个队列，
**任意一个空闲的 worker 都可以取走它**。这意味着：

> 一个任务可能**在 A 线程开始执行、在 B 线程继续执行**（跨过 `await` 之后）。

而"能安全地在线程间移动"就是 `Send` 的定义（第 12 章）。
状态机里的每个字段都要 `Send` —— 因为整个状态机要跨线程移动。

### 为什么 `'static` 是必需的

因为**任务可能比调用者活得久**。

```rust
fn handler() {
    tokio::spawn(async { /* 长时间的任务 */ });
    // ← 函数返回了，但任务还在跑
}
```

`spawn` 是 **fire-and-forget** —— 它不保证任务在函数返回前跑完。
所以任务里不能借用**任何**局部变量（那些变量随栈帧销毁）。

★ 这与 `std::thread::spawn` **完全一样**。
"把 future 交给 spawn"和"把闭包交给线程"是同一类操作：
**交出所有权，换一个可能在别处、可能在以后执行的承诺。**

### 为什么 `async` 本身不要求 `Send`

因为**单线程执行不需要 `Send`**。

`Send` 是关于"跨线程移动"的（第 12 章）。
一个 future 如果从头到尾在同一个线程上跑，它完全不需要 `Send` ——
`Rc`、`RefCell`、`*mut T` 都可以用（22.2.4 实测过）。

★ **这是异步 Rust 里最容易被误解的一点**：
人们看到"`async` + 多线程"就以为 `async` 自带 `Send` 要求。
实际上：

```text
async fn            →  只要求"是个 Future"
交给 tokio::spawn   →  才要求 Send + 'static
交给 spawn_local    →  只要求 'static（不要求 Send）
```

**约束来自"交给谁"，不来自 `async`。**

## 22.4 反直觉的点

### 反直觉之一：`Send` 的要求来自 `spawn`，不是来自 `async`

22.2.4 已经展开。这条值得单独记住，因为它**推翻了一整类"经验之谈"**：

- "`Rc` 不能用在异步代码里" —— **错**，不 `spawn` 就行；
- "异步函数必须处处 `Send`" —— **错**，看你把它交给谁；
- "`!Send` 的 future 没法并发" —— **错**，`LocalSet` 就是为它准备的。

★ 判断方法很简单：**看错误信息里那句 `required by a bound in ...`**。
它直接告诉你约束来自哪里。

### 反直觉之二：`async` 块不会自动 `move`

```rust
let v = vec![1, 2];
tokio::spawn(async { v.len() });      // ← E0373
```

`async { }` 和 `async move { }` 的区别，和闭包一样（第 12 章）：
**默认按引用捕获，`move` 才是按值。**

★ 但这里有个容易混的地方：**`async` 块的捕获规则是"按使用方式"**，
和闭包一致 —— 只读就按 `&` 捕获，要移动就按值捕获。
所以一个 `async` 块里如果**没有**用到某个变量，它就不捕获它。

实践上：**给 `spawn` 的 `async` 块一律写 `move`** —— 反正 `'static` 要求它。

### 反直觉之三：`spawn_blocking` 的闭包**不是** future

```rust
tokio::task::spawn_blocking(move || { /* 同步代码 */ })
```

注意闭包里**不能 `.await`** —— 它是 `FnOnce`，不是 `Future`。

★ 这个设计是对的：`spawn_blocking` 的目的就是"跑一段**同步**代码"，
所以它不该是 `async`。如果你在里面 `.await`，那说明你根本不需要它。

### 反直觉之四：在异步任务里阻塞，比想象中更糟

"我就 sleep 一下" —— `std::thread::sleep` 会**卡住整个 worker 线程**。

tokio 的 worker 数默认 = CPU 核数。所以：

| 场景 | 后果 |
|---|---|
| 4 核机器，1 个任务阻塞 1 秒 | **那个 worker 上的其他任务全部延迟 1 秒** |
| 4 核机器，4 个任务都阻塞 | **整个 runtime 停摆** |

★ 正确做法：`tokio::time::sleep(...).await`（异步的），
或者 `spawn_blocking`（真的需要阻塞时）。

**这是生产事故的常见来源**，而且它在小规模测试里**完全看不出来**。

### 反直觉之五：第 21 章的 `Send` 问题在这里集中爆发

第 21 章说"`async fn in trait` 表达不了 `Send`"。现在能看清它的**工程形态**了：

```rust
// 一个库里这样定义
pub trait Store {
    async fn get(&self, k: u64) -> u64;      // ← 返回的 future 保证不了 Send
}

// 所有想把它的调用 spawn 起来的地方，全部编译失败
tokio::spawn(async move { store.get(1).await });   // ← 错误
```

★ 而且报错位置**离真正的原因很远**：
错误说"这个 future 不是 `Send`"，但原因在 **trait 的定义**里。

**这就是为什么 `async fn in trait` 有一条 `warn` 级别的 lint** ——
编译器知道这个坑，但没法替你决定（你可能根本不用 `spawn`）。

### 反直觉之六：`#[tokio::main]` 就是一个宏

```rust
#[tokio::main]
async fn main() { ... }
```

展开后大致是：

```rust
fn main() {
    tokio::runtime::Runtime::new().unwrap().block_on(async { ... })
}
```

★ 没有魔法 —— 就是第 18 章那个 `block_on`（带线程池和 IO 驱动）。

**实测的展开结果**（`cargo expand -p ch22-tokio --bin ch22-tokio`，只看尾部）：

```rust
fn main() {
    let body = async { /* 你写的 main 体 */ };
    {
        use tokio::runtime::Builder;
        return Builder::new_multi_thread()      // ★ 多线程 runtime
            .enable_all()                        // ★ 打开 IO + 定时器驱动
            .build()
            .expect("Failed building the Runtime")
            .block_on(body);                     // ★ 第 18 章那个 block_on
    }
}
```

⚠️ **注意 `--bin ch22-tokio`**：这个 example 同时有 `lib.rs` 和 `main.rs`，
`cargo expand -p ch22-tokio` 会报
`extra arguments to rustc can only be passed to one target` —— 必须指定目标。

**实践含义**：`#[tokio::main]` 的默认 runtime 是**多线程**的，
可以换成单线程：

```rust
#[tokio::main(flavor = "current_thread")]
async fn main() { ... }
```

（这时候 `spawn` 仍然要求 `Send` —— 因为 tokio 的类型签名没变。
要真正 `!Send`，得用 `LocalSet`。）

## 22.5 亲手验证

```bash
# ★ 运行期演示
cargo run -p ch22-tokio
# total = 4

# ★ 证据（ch22 依赖 tokio，tools/ 会通过 examples/ch22-tokio/externs 自动加 --extern）
tools/evidence.sh ch22-tokio
scripts/verify-all.sh ch22      # 7 条断言

# ★ 四个函数的汇编
for f in shared_counter blocking_work local_task rc_within_task; do
  echo "--- $f ---"
  awk -v s="^_$f:" '$0~s{on=1} on{print} on&&/cfi_endproc/{exit}' \
    .evidence/ch22-tokio-lib.O3.s | grep -vE 'cfi|Ltmp|Lloh'
done

# ★ 三个反例
for f in guard_across_await rc_in_spawn borrow_local; do
  echo "--- $f ---"
  eval rustc --edition 2024 --crate-type=lib \
    "\$(bash -c 'source tools/lib.sh; rd_root >/dev/null; rd_extern_args ch22-tokio')" \
    examples/ch22-tokio/fail/$f.rs 2>&1 | head -3
done

# ★ 看 #[tokio::main] 展开成了什么
cargo expand -p ch22-tokio | head -40
```

**怎么算验证成功**：

1. `cargo run -p ch22-tokio` 输出 **`total = 4`** —— 4 个任务并发计数；
2. `_rc_within_task` 的汇编**只有 `strb` + `ret`**
   —— `Rc` 在异步函数里毫无阻碍；
3. 三个反例分别报：
   `future cannot be sent between threads safely`（×2）、
   **E0373** `async block may outlive the current function`。

★ **注意反例的编译命令**：ch22 是全书唯一依赖第三方 crate 的 example。
`tools/lib.sh` 里的 `rd_extern_args` 会从 `examples/ch22-tokio/externs`
读出需要哪些 crate，再用 `cargo build --message-format=json` 找出
`.rlib` 路径，拼出 `--extern` 和 `-L dependency=` 参数。

## 22.6 与 unsafe 的关系

本章**没有 `unsafe`**，但它是第 20 章那条警告的**工程验证**：

> `unsafe impl Send for MyFuture` 几乎总是错的 ——
> 它是在替编译器撒谎，而谎言会以"锁在错误的线程上释放"的形式兑现。

tokio 把这句话变成了**生产事故**：

```rust
// ❌ 不要这样"修复"编译错误
unsafe impl Send for MyTask {}
```

理由（第 12、13 章实测过）：

- `MutexGuard: !Send` 是因为**解锁必须在加锁的线程**；
- 在 macOS 上 `Mutex` 走 **pthread** 路径，跨线程解锁是**未定义行为**；
- `Rc: !Send` 是因为引用计数**不是原子的** —— 跨线程会数据竞争。

★ **`unsafe impl Send` 不是"关掉一个检查"，是"让一个真实的保证失效"。**

正确的做法（本章全部内容）：

| 问题 | 正确做法 |
|---|---|
| 锁守卫跨 `await` | **缩小作用域**（块） |
| 需要共享所有权 | `Arc`（不是 `Rc`） |
| 借用局部变量 | `async move` 搬进去，或 `Arc` |
| 真的需要 `!Send` | `LocalSet` / `spawn_local` |
| 阻塞操作 | `spawn_blocking` |

**没有一条需要 `unsafe`。**

> 这与第 21 章的结论一致：
> **有些限制不是"检查太严"，而是"这件事本身不安全"。**
> 而"想用 `unsafe` 绕过"往往意味着**找错了工具**。

## 22.7 小结

- **`tokio::spawn` 的两个约束理由不同**：
  `Send` 因为任务会被搬到别的 worker 线程；
  `'static` 因为任务可能比调用者活得久。
  与 `std::thread::spawn` **完全一样** —— tokio 没有引入新概念。
- **`Send` 的要求来自 `spawn`，不是来自 `async`**：
  实测 `rc_within_task`（`Rc` 跨 `await`，但不 spawn）**编译得过**。
  错误信息里那句 `required by a bound in spawn` 就是证据。
- **`'static` 的两种破法**：`async move`（把所有权搬进去）、
  `Arc`（共享所有权，一次原子计数）。
- **`LocalSet` / `spawn_local` 是 `!Send` 的逃生门** ——
  这是"约束来自 spawn"的**直接证明**：换个 spawn 方式，约束就没了。
- **`spawn_blocking` 接受的是 `FnOnce`，不是 `Future`**；
  在异步任务里阻塞会卡住整个 worker（worker 数默认 = CPU 核数），
  **这是生产事故的常见来源，且小规模测试里看不出来**。
- **第 21 章的 `Send` 问题在这里集中爆发**：
  一个库里有一个用 `async fn` 写的 trait 方法，
  **所有想 `spawn` 它的地方都会失败** —— 而报错位置离原因很远。
- **`#[tokio::main]` 就是一个宏**，展开成
  `Runtime::new().unwrap().block_on(...)` —— 与第 18 章手写的 `block_on` 同理。
- **本章所有问题都不需要 `unsafe`**：
  换设计（缩作用域 / `Arc` / `move` / `LocalSet` / `spawn_blocking`）。
  `unsafe impl Send` 是替编译器撒谎，代价是真实的数据竞争或跨线程解锁。

下一章是第四部分的实战：**把这些拼起来，写一个 mini async runtime**。
你会亲手实现第 18 章那个 `block_on`，并看到 tokio 在它上面加了什么。
