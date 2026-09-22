# 20. `async` 中的生命周期与 `Send` 传染

> 一句话：需要跨 `await` 保存的状态会进入 future 的状态机布局。
> 而字段决定两件事：**生命周期**（借用要不要记进状态机）和
> **`Send`**（字段 `!Send`，整个 future 就 `!Send`）。
> 本章最反直觉的一条：**`drop(guard)` 不能替代块作用域。**

## 先把语法认清

异步函数可以像普通函数一样借用参数：`async fn read(x: &Data)`. 返回的
future 会携带相应生命周期；任何活过 `.await` 的局部状态都可能成为 future
字段，并参与 `Send`/`Sync` 自动推导。是否需要 `Send` 取决于调用方是否会把
future 移到其他线程，例如 `tokio::spawn`。

`.await` 像在函数中间按下暂停键：暂停时还攥在手里的局部变量，都得一起
装进状态机。某个 guard 或 `Rc` 只要被装进去，整个 future 就会继承它的性质。

### 放到业务里：持锁后访问远程服务

处理请求时若拿着 `std::sync::MutexGuard` 跨过网络 `.await`，不仅可能让
future 变成 `!Send`，还会把临界区延长到不可控的 IO 时间。正确做法通常是
在小块作用域内复制所需数据并释放锁，再执行异步调用；若确实需要异步锁，
也要重新审视是否应该跨 await 持锁。

```rust
let user_id = { state.lock().unwrap().user_id };
let profile = client.fetch_user(user_id).await;
```

大括号不是审美问题：它让 guard 在 `.await` 前离开状态机，也缩短了临界区。

第 18 章展示了挂起点如何进入状态机布局，
第 19 章说"状态机可能是自引用的，所以要 `Pin`"。
本章把镜头对准**变体里到底装了哪些字段** ——
因为那些字段同时决定了两件在异步里最常卡住人的事。

## 20.0 一个会让你卡住的例子

你在异步函数里加锁：

```rust
use std::sync::Mutex;

pub async fn holds_guard(m: &Mutex<u64>) -> u64 {
    let g = m.lock().unwrap();
    std::future::ready(()).await;   // ← g 活过了这个 await
    *g
}
```

把它的 future 交给一个要求 `Send` 的函数（比如 `tokio::spawn`）：

```text
error: future cannot be sent between threads safely
  |
4 |     assert_send(holds_guard(&Mutex::new(1)));
  |                 ^^^^^^^^^^^^^^^^^^^^^^^^^^^ future returned by `holds_guard` is not `Send`
  |
  = help: within `impl Future<Output = u64>`, the trait `Send` is not implemented
          for `std::sync::MutexGuard<'_, u64>`
note: future is not `Send` as this value is used across an await
  |
4 |     let g = m.lock().unwrap();
  |         - has type `std::sync::MutexGuard<'_, u64>` which is not `Send`
5 |     std::future::ready(()).await;
  |                            ^^^^^ await occurs here, with `g` maybe used later
```

你会去查："`MutexGuard` 为什么不是 `Send`？"
然后学到"因为解锁必须在同一个线程"，于是你想：

> "那我在 `await` 之前把它 `drop` 掉不就行了？"

```rust
let g = m.lock().unwrap();
let v = *g;
drop(g);                        // ← 用完了，显式释放
std::future::ready(()).await;
```

**还是报同一个错。** 编译器说的是
`await occurs here, with g maybe used later` ——
**它说 `g` "可能稍后被用"，可你明明已经 `drop(g)` 了。**

这是本章要拆开的第一个谜。第二个谜紧接着来：

```rust
let v = { let g = m.lock().unwrap(); *g };   // ← 只是换成了块作用域
std::future::ready(()).await;
v
```

**这段编译过了。** 和上一段唯一的区别是：`drop(g)` 换成了块。
为什么块可以，`drop` 不行？

## 20.1 先把常见说法摆上桌

通常会这样概括：

- `async` 块里跨 `await` 的变量会被存进状态机；
- 因此 `!Send` 的值跨 `await` 会让整个 future 变 `!Send`；
- 解法是"缩小作用域，让它在 `await` 之前 drop 掉"。

第三条是对的，但表述不够精确 ——
它没告诉你**"缩小作用域"具体指什么**。
20.0 已经证明：`drop(g)` 也算"drop 掉"，但它**没用**。

## 20.2 编译器眼里的样子

### 20.2.1 跨 `await` 的借用 = 状态机多一个引用字段

`examples/ch20-async-lifetimes/src/lib.rs` 里有两个只差一行位置的函数：

```rust
// A：借用跨过 await
pub async fn borrow_across_await() -> u64 {
    let x = String::from("hi");
    let r = &x;
    std::future::ready(()).await;
    r.len() as u64
}

// B：借用不跨 await
pub async fn borrow_not_across_await() -> u64 {
    let x = String::from("hi");
    let n = { let r = &x; r.len() };   // ← r 在 await 之前就用完了
    std::future::ready(()).await;
    n as u64
}
```

MIR 里的状态机布局（`tools/evidence.sh ch20-async-lifetimes`）：

```mir
// A: borrow_across_await
coroutine layout {
    field _s0: String;                  // 被借用的值
    field _s1: &String;                 // ★ 借用 —— 活过了 await
    field _s2: std::future::Ready<()>;
    variant_fields = { ..., Suspend0 (3): [_s0, _s1, _s2] }
}

// B: borrow_not_across_await
coroutine layout {
    field _s0: String;
    field _s1: usize;                   // ★ 只有算出来的长度，没有引用
    field _s2: std::future::Ready<()>;
    variant_fields = { ..., Suspend0 (3): [_s0, _s1, _s2] }
}
```

★ **两份代码只差 `r.len()` 的位置，状态机的字段类型就从 `&String` 变成了 `usize`。**

这就是"生命周期在异步里的可见形态"：

> **一个借用"活过 `await`"，在实现上就等于"它成了状态机的一个字段"。**

这也顺带解释了第 19 章：`_s1: &String` 指向 `_s0: String` ——
**状态机里的字段互相指涉**，这就是自引用，就是 `Pin` 存在的理由。

> ⚠️ 别把两件事搞混：`first(v: &[u64]) -> &u64` 里返回的那个 `&u64`
> **不是**状态机字段 —— 它是 `poll` 返回的 `Poll<&'a u64>` 里那个引用。
> 参数 `v` 才是字段（`field _s0: &[u64]`）。
> `longest<'a>(x, y) -> &'a u64` 里 `_s0` 和 `_s1` 都是字段，
> 但那个返回的 `&'a u64` 不是。

### 20.2.2 生命周期省略规则照旧适用

`async fn` 的参数/返回生命周期规则和第 2 章**完全一样**：

```rust
pub async fn first(v: &[u64]) -> &u64 { ... }              // ✅ 单输入，省略规则可推
pub async fn longest<'a>(x: &'a [u64], y: &'a [u64]) -> &'a u64 { ... }   // 必须显式
```

去掉 `<'a>`（`fail/ambiguous_lifetime.rs`）：

```text
error[E0106]: missing lifetime specifier
  = help: this function's return type contains a borrowed value, but the
          signature does not say whether it is borrowed from `x` or `y`
```

★ 这条规则**没有**因为 `async` 而改变 —— 但它更容易踩：
因为异步里"引用"看起来更像"状态机里的东西"，人们会误以为可以省略。

### 20.2.3 `Send` 是状态机**字段**的性质

判断一个 future 是不是 `Send`，最干净的办法是写一个**编译期断言**：

```rust
pub fn assert_send<F: Send>(_f: F) {}
```

**它没有任何运行时开销** —— 只在类型层检查一次，然后消失。
（`verify-all.sh` 里有一条断言直接检查：`.O3.s` 里**没有** `_assert_send:` 符号。）

把各种 future 交给它，就是一张实测表：

| future | 状态机里的字段 | `Send` |
|---|---|---|
| `borrow_across_await()` | `String` + `&String` | ✅ |
| `holds_guard_good(&Mutex<u64>)` | `u64` | ✅ |
| `holds_atomic_ref(&AtomicU64)` | `&Atomic<u64>` | ✅ |
| `holds_guard_bad(&Mutex<u64>)` | **`MutexGuard<'_, u64>`** | ❌ |
| `holds_cell()` | `Cell<u64>` | ✅（但 `!Sync`，见 20.4） |

★ **`Send` 完全由字段决定**，和字段的类型在普通结构体里决定
`Send` 是同一条规则（第 12 章）。

而且编译器的错误信息**直接指出是哪个字段**：

```text
= help: within `impl Future<Output = u64>`, the trait `Send` is not implemented
        for `std::sync::MutexGuard<'_, u64>`
note: future is not `Send` as this value is used across an await
  |
4 |     let g = m.lock().unwrap();
  |         - has type `std::sync::MutexGuard<'_, u64>` which is not `Send`
5 |     std::future::ready(()).await;
  |                            ^^^^^ await occurs here, with `g` maybe used later
```

**三个点都标出来了**：① 是哪个值；② 它的类型；③ 它跨过了哪个 `await`。

### 20.2.4 `Send` 会沿 `.await` 传染

`fail/send_contagion.rs`：

```rust
pub async fn inner(m: &Mutex<u64>) -> u64 {
    let g = m.lock().unwrap();
    std::future::ready(()).await;
    *g
}

pub async fn outer(m: &Mutex<u64>) -> u64 { inner(m).await }   // ← 一行锁都没写

pub fn check() { assert_send(outer(&Mutex::new(1))); }         // ← 报错
```

```text
error: future cannot be sent between threads safely
  = help: within `impl Future<Output = u64>`, the trait `Send` is not implemented
          for `std::sync::MutexGuard<'_, u64>`
note: future is not `Send` as this value is used across an await
  --> 指向 **inner** 里的那一行
```

★ **关键在于错误指向谁**：`outer` 自己一行锁都没写，
但编译器指出的是 `inner` 里的 `MutexGuard`。

原因在 MIR 里看得一清二楚 —— `outer_send_ok` 的状态机布局：

```mir
coroutine layout {
    field _s0: {async fn body of inner_send_ok()};   // ★ 字段就是内层 future
    variant_fields = { ..., Suspend0 (3): [_s0] }
}
```

**外层的 future 里装着一个"半执行的内层 future"**，
而那个内层 future 里装着一个 `MutexGuard`。于是 `!Send` 一路传上来。

> **实践含义**：一个库里有一个 `!Send` 的 `async fn`，
> **所有 `.await` 它的地方都会变成 `!Send`** —— 而且报错位置离真正的原因很远。
> 这是异步代码里最难查的一类问题之一。

### 20.2.5 `async fn` 的"函数体"只有一条语句

顺便复习第 18 章那条：`outer_send_ok` 的 MIR 函数体**只有一行**：

```mir
fn outer_send_ok() -> {async fn body of outer_send_ok()} {
    bb0: {
        _0 = {coroutine@examples/ch20-async-lifetimes/src/lib.rs:178:37: 178:62 (#0)};
        return;
    }
}
```

调用 `async fn` = 构造状态机。**这一章讨论的"字段"，
就是那个被构造出来的东西的内部布局。**

## 20.3 为什么必须这样设计

### 为什么跨 `await` 的变量必须进状态机

因为 `await` 的语义就是"**在这里暂停，保留所有局部变量，之后从这里继续**"
（第 18 章）。普通函数的栈帧在 `await` 处无法保留 ——
硬件栈会被后续调用覆盖。所以必须把"活着的东西"搬到状态机里。

**"活着"的判据，就是第 1 章的借用检查器那一套**：
从这个 `await` 出发，**存在一条路径**还能用到这个变量，它就必须被装进去。

这也解释了 20.0 那个谜的一半：**编译器数的是"变量存活"，不是"值存活"。**

### 为什么 `drop(g)` 没有用

看这段（`fail/drop_does_not_help.rs`）：

```rust
let g = m.lock().unwrap();
let v = *g;
drop(g);                        // ← 一次"使用"
std::future::ready(()).await;
v
```

`drop(g)` 在 MIR 里是 `drop(_g)` —— **一次对 `g` 的使用**。
但它**没有缩短 `g` 的存活区间**：`g` 这个局部变量仍然声明在函数作用域里，
它的 `StorageDead` 仍然在函数末尾。

于是借用检查器问的是：

> 从 `await` 这个点出发，有没有路径还能到达 `g` 的使用点？
> —— **有**（`g` 的 `StorageDead` / drop flag 检查都还在后面）。

于是 `g` 被判定"活过 `await`"，进了状态机字段，future 就 `!Send` 了。

而块作用域版本：

```rust
let v = { let g = m.lock().unwrap(); *g };
```

`g` 的**作用域**结束在块尾 —— 它的 `StorageDead` 在 `await` **之前**。
借用检查器数不到后面的使用点，于是 `g` 不进状态机。
MIR 里验证得明明白白：`holds_guard_good` 的字段只有 `_s0: u64`。

> **一句话**：`drop(g)` 释放的是**值**，块作用域结束的是**变量**。
> 借用检查器数的是后者。

这和**第 1 章**是同一条原理的两个面：
借用检查器做的是**控制流图上的数据流分析**，它数的是"使用点"和"存活区间"。
异步只是让这个区间有了一个新的边界：**`await`**。

## 20.4 反直觉的点

### 反直觉之一：`drop(g)` 不缩短变量存活区间

20.0 和 20.3 已经展开。值得补一句**实践上的推论**：

> **要跨 `await`，就别让 `!Send` 的值进作用域。**
> 写 `drop()` 是**无效**的，必须写块 ——
> 或者更彻底：把锁的操作抽成一个**同步函数**，
> 让 `await` 的那一层根本看不到守卫。

```rust
fn read_lock(m: &Mutex<u64>) -> u64 { *m.lock().unwrap() }   // 同步函数，没有 await

pub async fn ok(m: &Mutex<u64>) -> u64 {
    let v = read_lock(m);
    std::future::ready(()).await;
    v
}
```

### 反直觉之二：future **不是**天生 `!Sync`

你可能听过这个说法：

> "future 天生不是 `Sync`，因为 `poll` 要 `&mut self`。"

**实测推翻它**（`fail/future_not_sync.rs`）：

| future | `Send` | `Sync` |
|---|---|---|
| `async fn plain() -> u64 { 1 }` | ✅ | **✅** |
| `async fn with_await()`（有 `await`，无捕获） | ✅ | **✅** |
| `async fn with_cell()`（捕获 `Cell<u64>`） | ✅ | ❌ |
| `async fn guard_across(&Mutex<u64>)` | ❌ | **✅** |

`poll` 要 `&mut self` 说的是"**不能同时 poll 两次**"——
那是 `&mut` 的独占性，和 `Sync`（"`&Self` 能不能跨线程"）是**两件事**。

★ 所以正确的说法是：**`Send` / `Sync` 由状态机的字段决定**，
和普通结构体完全一样（第 12 章：`Cell<u64>` 是 `Send` 但 `!Sync`）。
future 只是**经常**因为捕获了 `!Sync` 的东西而 `!Sync`。

> 这也回答了"为什么 `Send` 与 `Sync` 会不一致"：
> 看表格里 `guard_across` 那一行 —— **`!Send` 但 `Sync`**。
> 两个性质互相独立，没有任何蕴含关系。

### 反直觉之三：`Send` 传染的报错位置离原因很远

20.2.4 里 `outer` 一行锁都没写，报错却指向 `inner`。
在真实项目里，这个链条可能有**五层** `async fn` 深。

最有效的查法，是从错误信息里那句
`within impl Future<Output = ...>, the trait Send is not implemented for X`
读出 `X` —— 那才是真正的原因。

（`tokio::spawn` 要求 `F: Send + 'static`，所以这个问题在 tokio 项目里
几乎必然遇到。第 22 章会回到这个场景。）

### 反直觉之四：状态机的"字段"和你的局部变量不是一一对应

MIR 里 `Suspend0 (3): [_s0, _s1, _s2]` 列出的是**这个挂起点上活着的字段**。
编译器还会做布局优化（`storage_conflicts` 位矩阵就是给这个用的，
第 18 章提过）—— 两个不冲突的变量可以**复用同一块空间**。

所以状态机大小 **不等于** "所有局部变量之和"，
而是"**同一个挂起点上同时活着的那些**"。

## 20.5 亲手验证

```bash
tools/evidence.sh ch20-async-lifetimes
scripts/verify-all.sh ch20      # 17 条断言

# ★ 借用跨 await → 状态机里多一个引用字段
grep -n 'coroutine layout' -A 9 .evidence/ch20-async-lifetimes-lib.mir | head -20

# ★ Send 传染：外层 future 的字段是内层 future
awk '/fn outer_send_ok/{f=1} f&&/coroutine layout/{p=1} p{print} p&&/storage_conflicts/{exit}' \
  .evidence/ch20-async-lifetimes-lib.mir

# ★ 五个反例（都必须编译不过）
for f in guard_across_await drop_does_not_help ambiguous_lifetime send_contagion future_not_sync; do
  rustc --edition 2024 --crate-type=lib examples/ch20-async-lifetimes/fail/$f.rs 2>&1 | head -2
done
```

**怎么算验证成功**：

1. `borrow_across_await` 的布局里有 `field _s1: &String`，
   而 `borrow_not_across_await` 的对应字段是 `field _s1: usize`
   —— **只差一行代码的位置**；
2. `outer_send_ok` 的布局里 `field _s0: {async fn body of inner_send_ok()}`
   —— 内层 future 是外层的一个字段，这就是传染的机器形态；
3. `holds_guard_good` 的布局里只有 `field _s0: u64`（没有 `MutexGuard`）；
4. 五个反例分别报：
   `future cannot be sent between threads safely`（×2，含 `MutexGuard` 字样）、
   **E0106**、`cannot be shared between threads safely`。

## 20.6 与 unsafe 的关系

本章**一个 `unsafe` 都没有** —— 这本身是本章的结论：

> **"跨 `await` 的变量进状态机"是编译器自动做的事，
> 而 `Send` 是编译器自动推导的 auto trait。两者都不需要你写任何东西。**

但你会在别处遇到它。`unsafe impl Send for MyFuture` 是一个常见的
（且经常是**错误的**）"修复"。它的含义是：

> **我承诺这个类型跨线程移动是安全的。**

对 future 来说，这个承诺等价于：

> **我承诺状态机里那些字段跨线程移动是安全的。**

★ 而这正是 20.0 那个错误的根源：`MutexGuard` 之所以 `!Send`，
是因为**解锁必须在加锁的线程**（第 12 章实测过这条：
`fail/guard_not_send.rs` 报 "cannot be sent between threads safely"）。

```rust
unsafe impl Send for HoldsGuard {}   // ← 这是 UB：解锁会发生在别的线程
```

**编译器拒绝这个 future，是在保护你**。
强行 `unsafe impl Send` 绕过去，换来的是"锁在错误的线程上被释放"——
在 macOS 上 `pthread_mutex_unlock` 有**未定义行为**（第 13 章实测过
`Mutex` 在 macOS 走的是 pthread 路径）。

> **`Send` 不是一个"要不要"的开关，它是一个"是不是"的事实。**
> 编译器说 `!Send` 的时候，它是在陈述事实，不是在刁难你。
> 正确方向是**改代码结构**（缩小作用域 / 抽成同步函数 / 选择本地执行器），
> 而不是用 `unsafe impl` 掩盖不满足的约束。

## 20.7 小结

- **需要跨过 `await` 保存的状态会进入状态机布局。**
  借用一个值跨过 `await`，状态机里就多一个引用字段
  （`field _s1: &String`）；借用不跨 `await`，就退化成 `usize`。
- **`async fn` 的生命周期省略规则和第 2 章完全一样**：
  两个输入引用 + 输出引用必须显式写 `<'a>`，否则 E0106。
- **`Send` 是状态机字段的性质**，和普通结构体的规则是同一条。
  编译器报错时会指出**是哪个字段**（`has type ... which is not Send`）。
- **`Send` 会沿 `.await` 传染**：外层 future 的字段就是内层 future
  （`field _s0: {async fn body of inner()}`）。
  报错位置可能离原因很远，要从错误信息里读出那个类型。
- **★ `drop(g)` 不能替代块作用域。** `drop` 释放的是**值**，
  块作用域结束的是**变量**；借用检查器数的是**变量的存活区间**。
  这条和第 1 章是同一个原理。
- **future 不是天生 `!Sync`**：`Send` / `Sync` 是两个独立性质，
  由字段决定。实测有 `!Send` 但 `Sync` 的 future。
- **`unsafe impl Send for MyFuture` 几乎总是错的** ——
  它是在替编译器撒谎，而谎言会以"锁在错误的线程上释放"的形式兑现。

下一章换一个方向：**`async fn` 出现在 trait 里**会怎样。
这是异步 Rust 里演进最快、也最容易踩坑的一块 ——
本章讲的 `Send` 与生命周期，在那里会同时变成**设计约束**。
