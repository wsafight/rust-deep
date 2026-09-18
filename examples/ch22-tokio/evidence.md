# 第 22 章：tokio 实战 — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`
> tokio **1.53.1**

## 复现命令

```bash
# ★ 运行期演示
cargo run -p ch22-tokio            # 预期输出：total = 4

# ★ 证据（本章是全书唯一依赖第三方 crate 的 example）
tools/evidence.sh ch22-tokio
scripts/verify-all.sh ch22         # 7 条断言
```

### ⚠️ 本章的 example 依赖 tokio —— 工具链为此加了一个机制

本书其余 example 都是**零依赖**的，`rustc x.rs` 就能编。
ch22 需要真的链接 tokio，于是：

1. `examples/ch22-tokio/externs` 里逐行写需要哪些 crate（这里是 `tokio`）；
2. `tools/lib.sh` 的 `rd_extern_args` 用
   `cargo build --message-format=json` 找出 `.rlib` 路径，
   拼出 `--extern=tokio=...` **以及 `-L dependency=<deps 目录>`**；
3. `tools/{mir,llvm,asm,objdump}.sh` 都调用它；
4. `scripts/verify-all.sh` 的 `assert_fails` 同样带上。

★ **`-L dependency=` 是必需的**：只给 `--extern=tokio=...` 不够 ——
rustc 会从 tokio 的 metadata 里读到它依赖 `pin_project_lite` 等 crate，
然后按 `-L` 的搜索路径去找。缺了会报：

```text
error[E0463]: can't find crate for `pin_project_lite` which `tokio` depends on
```

（cargo 自动做这件事，裸 `rustc` 不会。）

## 关键结论与断言（7 条，全绿）

| # | 结论 | 断言 |
|---|---|---|
| 1 | `spawn` 的任务可编译（锁作用域正确） | `.O3.s` 里 `^_shared_counter:` |
| 2 | `spawn_blocking` 可编译 | `.O3.s` 里 `^_blocking_work:` |
| 3 | `LocalSet` 可以 spawn `!Send` | `.O3.s` 里 `^_local_task:` |
| 4 | ★ 不 spawn 时 `Rc` 完全可用 | `.O3.s` 里 `^_rc_within_task:` |
| 5 | ★ 锁守卫跨 `await` → 不是 `Send` | `fail/guard_across_await.rs` |
| 6 | ★ `Rc` 进 `spawn` → `!Send` | `fail/rc_in_spawn.rs` |
| 7 | ★ 借用局部变量 → `'static` 不满足 | `fail/borrow_local.rs` → **E0373** |

## ★ 核心：`Send + 'static` 是**两个**约束

```rust
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
```

| 约束 | 为什么需要 |
|---|---|
| `Send` | 任务会被**搬到别的 worker 线程**上执行 |
| `'static` | 任务**可能比调用者活得久**（fire-and-forget） |

★ 对照 `std::thread::spawn`：

```rust
pub fn spawn<F, T>(f: F) -> JoinHandle<T>
where F: FnOnce() -> T + Send + 'static, T: Send + 'static,
```

**同样的 `Send + 'static`，同样的两个理由**（第 12 章）。
tokio 没有引入新概念，它只是把"线程"换成了"任务"。

## ★ 核心证据一：`Rc` 在异步函数里完全可以用（不 spawn 就行）

```rust
pub async fn rc_within_task() -> u64 {
    let r = std::rc::Rc::new(1u64);      // Rc 是 !Send
    tokio::task::yield_now().await;      // r 跨了 await
    *r
}
```

**编译得过。** 汇编：

```asm
_rc_within_task:
	strb	wzr, [x8, #24]      ; 只写判别式
	ret
```

★ 这是本章最重要的一条对照：

> **`Send` 的要求来自 `spawn`，不是来自 `async`。**

推翻的一整类"经验之谈"：

| 说法 | 实际 |
|---|---|
| "`Rc` 不能用在异步代码里" | ❌ 不 `spawn` 就行 |
| "异步函数必须处处 `Send`" | ❌ 看你把它交给谁 |
| "`!Send` 的 future 没法并发" | ❌ `LocalSet` 就是为它准备的 |

★ 判断方法：**看错误信息里那句 `required by a bound in ...`** ——
它直接告诉你约束来自哪里。

## ★ 核心证据二：`Send` 那一半的错误形态（与第 20 章一字不差）

```text
error: future cannot be sent between threads safely
  = help: within `impl Future<Output = ()>`, the trait `Send` is not
          implemented for `std::sync::MutexGuard<'_, u64>`
note: future is not `Send` as this value is used across an await
  |     let g = c.lock().unwrap();
  |         - has type `std::sync::MutexGuard<'_, u64>` which is not `Send`
  |     tokio::task::yield_now().await;
  |                                ^^^^^ await occurs here, with `g` maybe used later
note: required by a bound in `spawn`          ← ★ tokio 多的一行
```

★ `required by a bound in spawn` —— **它告诉你要求来自哪里**。

**修法**（`src/lib.rs::shared_counter`）：把锁的作用域收进块。

```rust
tokio::spawn(async move {
    let cur = { let g = c.lock().unwrap(); *g };   // ← guard 在 await 之前 drop
    tokio::task::yield_now().await;
    let mut g = c.lock().unwrap();
    *g = cur + 1;
});
```

⚠️ **`drop(g)` 没有用**（第 20 章实测）。

## ★ 核心证据三：`'static` 那一半（E0373）

```text
error[E0373]: async block may outlive the current function, but it borrows `v`,
              which is owned by the current function
  = help: use `move` to force the async block to take ownership
```

★ **编译器直接给出了修法**：`use move`。

| 破法 | 例子 | 代价 |
|---|---|---|
| 把所有权搬进去 | `async move { s.len() }` | 值被移走 |
| 共享所有权 | `Arc<str>` | 一次原子计数 |

## ★ 逃生门：`LocalSet` / `spawn_local`

```rust
let local = tokio::task::LocalSet::new();
local.run_until(async {
    let r = std::rc::Rc::new(7u64);
    tokio::task::spawn_local(async move { *r }).await.unwrap()
}).await
```

```asm
_local_task:
	strb	wzr, [x8, #72]
	ret
```

**`spawn_local` 不要求 `Send`** —— `LocalSet` 保证这些任务在**同一个线程**上跑。

★ 这是"约束来自 spawn"的**直接证明**：换个 spawn 方式，约束就没了。

## ★ `spawn_blocking`：接受 `FnOnce`，不是 `Future`

```rust
pub async fn blocking_work(data: Vec<u64>) -> u64 {
    tokio::task::spawn_blocking(move || {
        data.iter().fold(0u64, |a, b| a.wrapping_add(*b))
    }).await.expect("blocking task panicked")
}
```

```asm
_blocking_work:
	ldr	q0, [x0]
	str	q0, [x8]
	ldr	x9, [x0, #16]
	str	x9, [x8, #16]
	strb	wzr, [x8, #32]
	ret
```

| | `spawn` | `spawn_blocking` |
|---|---|---|
| 接受 | `Future + Send + 'static` | `FnOnce() -> R + Send + 'static` |
| 跑在 | async worker | 阻塞线程池（默认上限 512） |
| 用于 | 异步 IO | CPU 密集 / 同步 IO / 调 C 库 |

**共同点：`Send + 'static`** —— 两者都会跨线程、跨时间。

### ⚠️ 为什么这个坑很危险

tokio 的 worker 数默认 = **CPU 核数**。

| 场景 | 后果 |
|---|---|
| 4 核，1 个任务阻塞 1 秒 | 那个 worker 上的**其他所有任务**延迟 1 秒 |
| 4 核，4 个任务都阻塞 | **整个 runtime 停摆** |

**这是生产事故的常见来源，而且小规模测试里完全看不出来。**

## 运行期演示

```bash
$ cargo run -p ch22-tokio
total = 4
```

`src/main.rs` 里 4 个任务并发，每个 +1，用 `Arc<Mutex<u64>>` 共享计数。

★ `#[tokio::main]` 做的事：**构造 runtime 并 `block_on`**：

```rust
fn main() {
    tokio::runtime::Runtime::new().unwrap().block_on(async { ... })
}
```

与第 18 章手写的 `block_on` 是同一个东西，
只是 tokio 的版本带线程池、IO 驱动、定时器驱动。

## 反例三则

| 文件 | 错误 | 说明 |
|---|---|---|
| `fail/guard_across_await.rs` | `future cannot be sent between threads safely` | 第 20 章的结论，tokio 上的形态 |
| `fail/rc_in_spawn.rs` | 同上（`Rc<u64>`） | 第 12 章：`Rc` 引用计数不是原子的 |
| `fail/borrow_local.rs` | **E0373** `async block may outlive the current function` | `'static` 那一半 |

## ★ 与第 21 章的交叉：`Send` 问题在这里集中爆发

```rust
pub trait Store { async fn get(&self, k: u64) -> u64; }   // 返回的 future 保证不了 Send

tokio::spawn(async move { store.get(1).await });          // ← 所有调用点全部失败
```

★ **报错位置离真正的原因很远**：错误说"这个 future 不是 `Send`"，
但原因在 **trait 的定义**里。

**这就是 `async_fn_in_trait` 那条 lint 存在的理由** ——
编译器知道这个坑，但没法替你决定（你可能根本不用 `spawn`）。

## 本章所有问题都不需要 `unsafe`

| 问题 | 正确做法 |
|---|---|
| 锁守卫跨 `await` | 缩小作用域（块） |
| 需要共享所有权 | `Arc`（不是 `Rc`） |
| 借用局部变量 | `async move` / `Arc` |
| 真的需要 `!Send` | `LocalSet` / `spawn_local` |
| 阻塞操作 | `spawn_blocking` |

★ 对照第 20 章那条警告：

```rust
unsafe impl Send for MyTask {}    // ❌ 替编译器撒谎
```

代价是**真实的后果**：`MutexGuard: !Send` 是因为解锁必须在加锁线程
（macOS 上 `Mutex` 走 pthread，跨线程解锁是 UB）；
`Rc: !Send` 是因为引用计数不是原子的。

**`unsafe impl Send` 不是"关掉一个检查"，是"让一个真实的保证失效"。**

## 交叉验证（可选）

```bash
cargo run -p ch22-tokio
for f in shared_counter blocking_work local_task rc_within_task; do
  echo "--- $f ---"
  awk -v s="^_$f:" '$0~s{on=1} on{print} on&&/cfi_endproc/{exit}' \
    .evidence/ch22-tokio-lib.O3.s | grep -vE 'cfi|Ltmp|Lloh'
done
cargo expand -p ch22-tokio | head -40
```

## 待办

- [x] 7 条断言全绿（`verify-all.sh ch22`）
- [x] 运行期演示 `total = 4`
- [x] `tools/lib.sh` 增加 `rd_extern_args`（第三方依赖的 `--extern` + `-L`）
- [x] 3 个反例落库并加断言
- [ ] 若将来接 criterion，可以对 `spawn_blocking` vs 直接阻塞做真实测量
- [ ] 第 23 章（实战：mini async runtime）会把本章的结论收进一个可运行的项目
