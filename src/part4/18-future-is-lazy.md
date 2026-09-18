# 18. `Future` 是惰性的：手写一个最小 executor

> 一句话：`async fn` **不是**"开始执行"，而是"**构造一个还没开始的执行**"。
> 编译器把 `async fn` 的函数体编译成一个**状态机**（coroutine），
> 每个 `await` 是状态机的一个**挂起点**。
> 而"推进状态机"这件事，必须由 executor 通过 `poll` 来做。

## 18.0 一个会让你卡住的例子

你想在异步函数里打印点东西：

```rust
let f = async {
    println!("hello");
};
// 程序结束
```

**什么都不会打印。**

你会想："我创建了一个异步块，为什么它不执行？"

换个写法也让人困惑：

```rust
async fn fetch() -> u64 { 42 }

fn main() {
    let f = fetch();        // ← 这里发生了什么？
    // f 的类型是什么？
}
```

**`f` 的类型不是 `u64`**，是一个编译器生成的匿名类型 ——
而且**调用 `fetch()` 本身不执行任何代码**。

这一章要把这件事拆开：`async fn` 到底被编译成了什么，
以及"执行"这件事为什么必须由外部驱动。

## 18.1 表层解释（官方书会怎么讲）

官方书会说：

- `Future` 是一个 trait，有 `poll` 方法，返回 `Poll::Ready` 或 `Poll::Pending`；
- `.await` 会挂起当前任务，等 future 就绪后继续；
- executor 负责 poll future 并在就绪时唤醒它；
- `async fn` 返回一个实现了 `Future` 的匿名类型。

这些都对，但"返回一个匿名类型"这句话掩盖了全部机制。
**那个匿名类型是什么？它有多大？`await` 在它里面是什么？**

本章要给出可以**逐行读出来**的答案。

## 18.2 编译器眼里的样子

### 18.2.1 `async fn` 的返回类型是"状态机"

```rust
pub async fn two_awaits(a: u64, b: u64) -> u64 {
    let x = Ready(Some(a)).await;
    let y = Ready(Some(b)).await;
    x + y
}
```

MIR 里的函数签名：

```mir
fn two_awaits(_1: u64, _2: u64) -> {async fn body of two_awaits()} {
    let mut _0: {async fn body of two_awaits()};
    ...
    bb0: {
        _0 = {coroutine@examples/ch18-future/src/lib.rs:50:48: 54:2 (#0)} { a: copy _1, b: copy _2 };
    }
}
```

★ **两行读出来的信息**：

1. **返回类型是 `{async fn body of two_awaits()}`** —— 不是 `u64`；
2. **函数体只有一条语句**：`{coroutine@...} { a, b }` ——
   **把参数打包进一个状态机**。没有任何计算。

**这就是"惰性"的机器含义**：调用 `async fn` = **构造状态机**，
**不执行任何用户代码**。

### 18.2.2 状态机的布局：每个 `await` 一个变体

同一个 MIR 文件里，状态机的 `poll` 实现（`two_awaits::{closure#0}`）
开头就有完整的布局：

```mir
fn two_awaits::{closure#0}(_1: Pin<&mut {async fn body of two_awaits()}>, _2: &mut Context<'_>) -> Poll<u64> {
    coroutine layout {
        field _s0: u64;
        field _s1: u64;
        field _s2: Ready<u64>;
        field _s3: Ready<u64>;
        variant_fields = {
            Unresumed(0): [],
            Returned (1): [],
            Panicked (2): [],
            Suspend0 (3): [_s0, _s2],
            Suspend1 (4): [_s1, _s3],
        }
        storage_conflicts = BitMatrix(4x4) { ... }
    }
```

**逐行读**：

| 变体 | 含义 |
|---|---|
| `Unresumed(0)` | 还没开始（刚构造出来） |
| `Returned(1)` | 已完成 |
| `Panicked(2)` | 执行中 panic 了 |
| **`Suspend0(3)`** | **第一个 `await` 挂起在这里**，活着的变量是 `_s0`、`_s2` |
| **`Suspend1(4)`** | **第二个 `await` 挂起在这里**，活着的变量是 `_s1`、`_s3` |

★ **`Suspend0` / `Suspend1` 就是两个 `.await` 的落点。**
"挂起"在实现上就是"记住当前在哪个变体 + 哪些变量还活着"。

`storage_conflicts` 那个位矩阵是借用检查的产物 ——
它记录"哪些字段不能同时存活"（编译器用来做状态机的布局优化）。

### 18.2.3 `poll` 就是一个 `match discriminant`

`poll` 的主体是一个基于判别式的跳转：

```mir
_30 = discriminant((*_31));
...
discriminant((*_31)) = 3;        // ← 切到 Suspend0
...
discriminant((*_31)) = 4;        // ← 切到 Suspend1
...
discriminant((*_31)) = 1;        // ← 切到 Returned
```

**"挂起" = 写一个判别式；"恢复" = 读一个判别式。**
这就是 `async` 的全部运行时机制 —— 没有任何魔法。

### 18.2.4 惰性的运行期证据

MIR 证明了"构造不执行"，但更有说服力的是**跑一遍**。
`examples/ch18-future/lazy_demo.rs`：

```bash
rustc --edition 2024 examples/ch18-future/lazy_demo.rs -o /tmp/anp && /tmp/anp
```

```text
after construct: N = 0
after drop:      N = 0
after poll:      N = 1
```

- 调用 `lazy(1)` 之后，副作用计数器还是 **0**；
- 把它 **drop 掉**，还是 **0** —— 因为从来没有 poll 过；
- **只有 poll 之后才变成 1**。

★ 这也解释了异步里最经典的陷阱：

```rust
let _ = async { println!("hello"); };   // ← 什么都不会打印
```

**`async {}` 块不是"开始执行"，是"构造一个还没开始的执行"。**
要让它跑，必须 `.await` 或者交给 executor。

### 18.2.5 状态机的大小

```rust
size_of_val(&two_awaits(1, 2))              // → 56
size_of_val(&no_await(1))                   // → 16
```

| future | 大小 | 为什么 |
|---|---|---|
| `two_awaits(1, 2)` | **56** | 两个 `u64` + 两个 `Ready<u64>` + 判别式/填充 |
| `no_await(1)` | **16** | 捕获了 `a: u64`（8）+ 判别式（8，对齐） |

**大小 = 所有跨 `await` 存活的变量之和**（加上判别式和对齐）。

★ 这解释了一个很多人踩过的坑：**`async fn` 不能递归**。

```rust
async fn rec(n: u64) -> u64 {
    if n == 0 { 0 } else { rec(n - 1).await + 1 }   // ❌ 编译不过
}
```

因为**递归调用会让大小无穷大** —— 每一层调用都要装进同一个状态机里。
需要递归时必须 `Box::pin`（把内层的大小变成一个指针）。

### 18.2.6 最小 executor

```rust
pub fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = Box::pin(f);
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    loop {
        match f.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::hint::spin_loop(),   // ← 忙等（简化）
        }
    }
}
```

**executor 的全部工作就是"反复 poll 直到 Ready"。**

真实的 executor 会做得更好：不忙等，而是**注册 waker**，
在 `Poll::Pending` 时挂起，等被唤醒再 poll。
但**核心循环就是这个 `loop { poll }`**。

★ 注意 `Box::pin(f)` —— **为什么必须 pin？** 这就是第 19 章的主题。

## 18.3 为什么必须这样设计

### 为什么 `async fn` 要编译成状态机

因为**"挂起"这件事无法用普通函数调用表达**。

普通函数的栈帧是**硬件栈**上的一块内存，函数返回就销毁。
而 `await` 要求"在这里暂停，**保留所有局部变量**，之后从这一点继续" ——
这需要把栈帧**搬到一个可以长期保存的地方**。

**状态机就是这个"可长期保存的栈帧"**：

| 普通函数 | async fn |
|---|---|
| 栈帧在硬件栈上 | 状态机在堆/栈上的一个结构体里 |
| 局部变量在栈帧里 | 跨 `await` 的变量是状态机的字段 |
| 返回 = 销毁栈帧 | `Poll::Pending` = 保留状态机 |
| 调用 = 执行 | **调用 = 构造状态机** |

**"调用 = 构造"正是惰性的来源。**

### 为什么必须由 executor 驱动

因为状态机**不会自己跑**。它只是一块数据，
需要一个循环来"读判别式 → 执行对应分支 → 再写判别式"。

**这就是 `poll`。** 而 `poll` 必须由外部调用 ——
状态机自己没有"我要继续"的能力。

> 这是 Rust 异步与 Go / JavaScript 异步的**根本差异**：
>
> | | Go / JS | Rust |
> |---|---|---|
> | 谁调度 | 运行时（goroutine / event loop） | **executor（库）** |
> | 栈 | 运行时管理（可增长） | **状态机（编译期固定大小）** |
> | 挂起点 | 任何函数调用 | **只能 `.await`** |
>
> Rust 的选择换来了**零运行时**：没有 GC、没有调度器、
> 状态机大小在编译期已知（可以放在栈上）。
> 代价是**你必须自己选一个 executor**（tokio / async-std / smol / 手写）。

### 为什么 `poll` 需要 `Pin`

`poll` 的签名是 `fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output>`。

**为什么不是 `&mut self`？** 因为状态机可能是**自引用**的：
一个跨 `await` 的局部变量可能借用另一个跨 `await` 的局部变量。
如果状态机在内存里**移动**了，那个借用就悬垂了。

`Pin` 就是在类型层面表达"**这个值移动之后会坏，所以不能移动**"。
（第 19 章展开。）

## 18.4 反直觉的点

### 反直觉之一：调用 `async fn` **不执行任何代码**

```rust
let f = fetch();     // 什么都没发生
```

MIR 里 `two_awaits` 的函数体**只有一条**构造状态机的语句。

**这不是"延迟执行"的优化，这是语义**：
`async fn` 是一个**构造器**，不是"启动器"。

★ 后果：

```rust
let _ = async { do_important_thing(); };   // ← 永远不会执行
```

**编译不报错、运行不报错、什么都不发生。**
这是 Rust 异步里最容易踩的坑之一（`#[must_use]` 只能部分缓解）。

### 反直觉之二：状态机的判别式**写在 `self` 里**

```mir
discriminant((*_31)) = 3;
```

**状态不是"程序计数器"，是 `self` 的一个字段。**

这意味着一件重要的事：**你可以保存一个"半执行"的 future** ——
它带着完整的中间状态。这也是为什么 `Pin` 是必须的
（那个字段可能被其他字段的引用指着）。

### 反直觉之三：`async fn` 不能递归

```rust
async fn rec(n: u64) -> u64 {
    if n == 0 { 0 } else { rec(n - 1).await + 1 }   // ❌
}
```

```text
error[E0733]: recursion in an async fn requires boxing
  |
2 | pub async fn rec(n: u64) -> u64 {
  | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
3 |     if n == 0 { 0 } else { rec(n - 1).await + 1 }
  |                            ---------------- recursive call here
  |
  = note: a recursive `async fn` call must introduce indirection such as
          `Box::pin` to avoid an infinitely sized future
```

**因为状态机的大小必须是一个**有限**的数。**
递归调用要求"状态机里装一个同类型的状态机" —— 无穷大。

`Box::pin` 把内层的大小变成一个指针（8 字节），问题消失。

★ 注意编译器的措辞：**"to avoid an infinitely sized future"** ——
它直接说出了原因。**这个错误码 E0733 本身就证明了"状态机大小"这个概念。**

### 反直觉之四：没有 `await` 的 `async fn` 也有开销

```rust
async fn no_await(a: u64) -> u64 { a }
```

实测大小是 **16 字节**（捕获 `a` + 判别式），而不是 0。

**`async fn` 的"异步"是编译期的类型变化，不是零成本的。**
一个不需要挂起的异步函数，其状态机仍然要占空间、仍然要经过 `poll`。

> 所以"把同步函数标成 `async`"不是免费的 ——
> 它会引入状态机、`Pin`、以及调用方的 `.await`。

## 18.5 亲手验证

```bash
tools/evidence.sh ch18-future
scripts/verify-all.sh ch18

# ★ async fn 的返回类型 + 函数体（只有构造）
grep -n 'fn two_awaits' -A 8 .evidence/ch18-future-lib.mir

# ★ 状态机的布局（每个 await 一个变体）
sed -n '/coroutine layout {/,/storage_conflicts/p' .evidence/ch18-future-lib.mir

# ★ 惰性（运行期）
rustc --edition 2024 examples/ch18-future/lazy_demo.rs -o /tmp/anp && /tmp/anp

# 状态机大小
cargo expand -p ch18-future        # 看展开后的 async fn（仍是 async 关键字）
```

**怎么算验证成功**：

1. `two_awaits` 的 MIR 签名是 `-> {async fn body of two_awaits()}`，
   且函数体只有 `{coroutine@...} { a, b }` —— **没有计算**；
2. `coroutine layout` 里有 `Unresumed(0)` / `Returned(1)` /
   **`Suspend0(3)` / `Suspend1(4)`** —— 两个 `await` 两个挂起点；
3. `lazy_demo` 输出 `after construct: N = 0` / `after drop: N = 0` /
   `after poll: N = 1` —— **惰性的运行期证据**；
4. `size_of_two_awaits()` = 56，`size_of_no_await()` = 16。

```bash
scripts/verify-all.sh ch18      # 11 条断言
```

## 18.6 与 unsafe 的关系

这一章**几乎没有 `unsafe`**（除了手写 `noop_waker` 那几行）。

但 `async` 与 `unsafe` 有一个**深刻的联系**：**`Pin` 是
"自引用结构"这个 `unsafe` 话题在标准库里的唯一官方解法**（第 19 章）。

★ `noop_waker` 里那段 `unsafe` 值得看一眼：

```rust
const VTABLE: RawWakerVTable = RawWakerVTable::new(
    |_| RawWaker::new(std::ptr::null(), &VTABLE),
    |_| {}, |_| {}, |_| {},
);
// SAFETY: 四个函数都不解引用 data，data 为 null 是合法的
unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
```

**这是一个"安全抽象 + 不安全实现"的微缩样本**：
`Waker` 的接口完全安全，但构造它需要 `unsafe` ——
因为你要**承诺**那四个函数的行为符合 `RawWakerVTable` 的契约。

★ 在 1.98 上，标准库提供了 **`Waker::noop()`**，
不需要手写这个 vtable（上面 `lazy_demo.rs` 用的就是它）。
手写版本仍然值得看，因为它把"`unsafe` 承诺的是什么"摆在明面上。

> 更深一层：**executor 是 `unsafe` 的高发区**。
> 一个真实的 executor 要处理：
> - `Waker` 的生命周期（`RawWaker` 的 `clone` / `drop` 必须成对）；
> - 跨线程唤醒（`Send + Sync` 的 waker）；
> - `Pin` 的正确使用（不能把 `!Unpin` 的 future 移来移去）。
>
> 这也是为什么**推荐直接用 tokio**，而不是自己写 executor。

## 18.7 小结

- **`async fn` 的返回类型是一个状态机**（MIR 里写作
  `{async fn body of ...}`），**不是**它的声明返回类型。
- **调用 `async fn` = 构造状态机**：MIR 里函数体只有一条
  `{coroutine@...} { a, b }` —— **不执行任何用户代码**。
  这就是"惰性"的机器含义。
- **每个 `.await` 是状态机的一个变体**：
  `Suspend0(3)` / `Suspend1(4)`（加上 `Unresumed(0)` / `Returned(1)` / `Panicked(2)`）。
  "挂起" = 写一个判别式；"恢复" = 读一个判别式。
- **惰性的运行期证据**：`lazy_demo` 输出
  `after construct: N = 0` / `after drop: N = 0` / `after poll: N = 1`。
- **状态机大小 = 跨 `await` 存活的变量之和**：
  `two_awaits` = 56 字节，`no_await` = 16 字节。
  这解释了**为什么 `async fn` 不能递归**（大小会无穷大）。
- **executor 的核心循环就是 `loop { poll }`** ——
  状态机不会自己跑，必须由外部驱动。
- **`poll` 要 `Pin<&mut Self>`**，因为状态机可能是自引用的（第 19 章）。

下一章专门讲 `Pin`：为什么它必须存在，
以及"不能移动"这件事怎么在类型层面表达。
