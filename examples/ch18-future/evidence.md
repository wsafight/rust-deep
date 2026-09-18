# 第 18 章：`Future` 是惰性的 — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`

## 复现命令

```bash
tools/evidence.sh ch18-future
scripts/verify-all.sh ch18       # 11 条断言（PASS=13）

# ★ 惰性的运行期证据（独立可执行文件）
rustc --edition 2024 examples/ch18-future/lazy_demo.rs -o /tmp/anp && /tmp/anp
```

## 关键结论与断言（11 条，全绿）

| # | 结论 | 断言（`scripts/verify-all.sh`） |
|---|---|---|
| 1 | `async fn` 的返回类型是状态机 | MIR 里 `fn lazy(_1: u64) -> {async fn body of lazy()}` |
| 2 | ★ 函数体被编译成 coroutine | MIR 里 `coroutine@examples/ch18-future/src/lib.rs` |
| 3 | 初始状态 `Unresumed(0)` | MIR 的 `coroutine layout` |
| 4 | 完成状态 `Returned(1)` | 同上 |
| 5 | **第一个 `await` 的挂起点 `Suspend0(3)`** | 同上 |
| 6 | **第二个 `await` 的挂起点 `Suspend1(4)`** | 同上 |
| 7 | `make_future` 只构造（不 poll） | MIR 里返回 `{async fn body of lazy()}` |
| 8 | `run_lazy` 才 poll（经由 `block_on`） | MIR 里 `block_on::<{async fn body of lazy()}>` |
| 9 | 状态机大小可测 | `.O3.s` 的 `^_size_of_two_awaits:` |
| 10 | 无 `await` 的状态机更小 | `.O3.s` 的 `^_size_of_no_await:` |
| 11 | `async fn` 不能递归 | `fail/recursive_async.rs` → **E0733** |

## ★ 核心证据一：`async fn` 的返回类型 + 函数体

```mir
fn two_awaits(_1: u64, _2: u64) -> {async fn body of two_awaits()} {
    let mut _0: {async fn body of two_awaits()};
    ...
    bb0: {
        _0 = {coroutine@examples/ch18-future/src/lib.rs:50:48: 54:2 (#0)} { a: copy _1, b: copy _2 };
    }
}
```

**两行读出来的信息**：

1. **返回类型是 `{async fn body of two_awaits()}`** —— 不是 `u64`；
2. **函数体只有一条语句**：`{coroutine@...} { a, b }` ——
   **把参数打包进状态机，没有任何计算**。

**这就是"惰性"的机器含义：调用 `async fn` = 构造状态机。**

## ★ 核心证据二：状态机的布局

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

| 变体 | 含义 |
|---|---|
| `Unresumed(0)` | 还没开始（刚构造出来） |
| `Returned(1)` | 已完成 |
| `Panicked(2)` | 执行中 panic 了 |
| **`Suspend0(3)`** | **第一个 `await` 挂起在这里**（活着的变量：`_s0`、`_s2`） |
| **`Suspend1(4)`** | **第二个 `await` 挂起在这里**（活着的变量：`_s1`、`_s3`） |

★ `storage_conflicts` 那个位矩阵是**借用检查的产物** ——
它记录"哪些字段不能同时存活"，编译器用它做状态机的布局优化。

### `poll` 就是一个 `match discriminant`

```mir
_30 = discriminant((*_31));
...
discriminant((*_31)) = 3;        // ← 切到 Suspend0
discriminant((*_31)) = 4;        // ← 切到 Suspend1
discriminant((*_31)) = 1;        // ← 切到 Returned
```

**"挂起" = 写一个判别式；"恢复" = 读一个判别式。**
没有任何魔法。

★ 注意 `discriminant((*_31)) = 3` —— **状态写在 `self` 里**，
不是"程序计数器"。这意味着**你可以保存一个"半执行"的 future**
（带着完整的中间状态）。

## ★ 核心证据三：惰性（运行期）

`examples/ch18-future/lazy_demo.rs`：

```text
$ rustc --edition 2024 examples/ch18-future/lazy_demo.rs -o /tmp/anp && /tmp/anp
after construct: N = 0
after drop:      N = 0
after poll:      N = 1
```

- 调用 `lazy(1)` 之后，副作用计数器还是 **0**；
- 把它 **drop 掉**，还是 **0** —— 因为从来没有 poll 过；
- **只有 poll 之后才变成 1**。

★ 这解释了异步里最经典的陷阱：

```rust
let _ = async { println!("hello"); };   // ← 什么都不会打印
```

**`async {}` 块不是"开始执行"，是"构造一个还没开始的执行"。**

## ★ 核心证据四：状态机的大小

| future | 大小 | 为什么 |
|---|---|---|
| `two_awaits(1, 2)` | **56** | 两个 `u64` + 两个 `Ready<u64>` + 判别式/填充 |
| `no_await(1)` | **16** | 捕获 `a: u64`（8）+ 判别式（8，对齐） |

**大小 = 跨 `await` 存活的变量之和**（加上判别式和对齐）。

★ 注意 `no_await` 是 **16** 而不是 1 —— 因为它**捕获了参数**。
一个不捕获任何东西的 `async fn` 状态机会退化成 1 字节。

## 反例：`fail/recursive_async.rs`

```text
error[E0733]: recursion in an async fn requires boxing
  --> examples/ch18-future/fail/recursive_async.rs:16:1
   |
16 | pub async fn rec(n: u64) -> u64 {
   | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
...
20 |         rec(n - 1).await + 1
   |         ---------------- recursive call here
   |
   = note: a recursive `async fn` call must introduce indirection such as
           `Box::pin` to avoid an infinitely sized future
```

★ **编译器说的理由是 "to avoid an infinitely sized future"** ——
它直接说出了原因。**这个错误码本身就证明了"状态机大小"这个概念。**

## 最小 executor

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

**executor 的核心循环就是 `loop { poll }`。**
真实的 executor 不忙等 —— 它注册 waker，`Pending` 时挂起，
被唤醒再 poll。

★ `Box::pin(f)` —— **为什么必须 pin？** 这就是第 19 章的主题。

★ 手写 `noop_waker` 的那几行 `unsafe` 是一个微缩的
"安全抽象 + 不安全实现"样本：`Waker` 的接口完全安全，
但构造它要**承诺**四个函数符合 `RawWakerVTable` 的契约。
（1.98 上标准库提供了 `Waker::noop()`，`lazy_demo.rs` 用的就是它。）

## 交叉验证（可选）

```bash
grep -n 'fn two_awaits' -A 8 .evidence/ch18-future-lib.mir
sed -n '/coroutine layout {/,/storage_conflicts/p' .evidence/ch18-future-lib.mir
grep -n 'discriminant((\*_31))' .evidence/ch18-future-lib.mir
```
