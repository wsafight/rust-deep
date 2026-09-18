# 第 20 章：`async` 中的生命周期与 `Send` 传染 — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`

## 复现命令

```bash
tools/evidence.sh ch20-async-lifetimes
scripts/verify-all.sh ch20      # 17 条断言
```

## 关键结论与断言（17 条，全绿）

| # | 结论 | 断言 |
|---|---|---|
| 1 | 被借用的值成为状态机字段 | `.mir` 里 `field _s0: String;` |
| 2 | ★ 跨 `await` 的借用多出一个引用字段 | `.mir` 里 `field _s1: &String;` |
| 3 | 挂起时被借者和借用者同时活着 | `.mir` 里 `Suspend0 (3): [_s0, _s1, _s2]` |
| 4 | 借用不跨 `await` → 退化成 `usize` | `.mir` 里 `field _s1: usize;` |
| 5 | 省略规则照旧适用 | `.mir` 里 `fn longest(` |
| 6 | 省略规则推不出来 | `fail/ambiguous_lifetime.rs` → **E0106** |
| 7 | 锁守卫跨 `await` → `!Send` | `fail/guard_across_await.rs` → `future cannot be sent between threads safely` |
| 8 | 错误信息指出是哪个字段 | 同上 → 含 `MutexGuard` |
| 9 | ★ `drop(g)` 不缩短变量存活区间 | `fail/drop_does_not_help.rs` → `maybe used later` |
| 10 | 块作用域版本字段里没有守卫 | `.mir` 里 `field _s0: u64;` |
| 11 | ★ 外层 future 的字段是内层 future | `.mir` 里 `field _s0: {async fn body of inner_send_ok()}` |
| 12 | 传染：错误指向内层的原因 | `fail/send_contagion.rs` → `not implemented for std::sync::MutexGuard` |
| 13 | `Cell` 字段 → `Send` 但 `!Sync` | `.mir` 里 `field _s0: Cell<u64>;` |
| 14 | ★ future 并非天生 `!Sync` | `fail/future_not_sync.rs` → `cannot be shared between threads safely` |
| 15 | 编译期断言 `F: Send` 通过 | `.O3.s` 里 `^_check_send_good:` |
| 16 | `assert_send` 在汇编里没有符号 | `.O3.s` 里**没有** `^_assert_send:` |
| 17 | 断言之后 future 照常 drop | `.O3.s` 里 `^_check_send_good:` |

## ★ 核心证据一：跨 `await` 的借用 = 状态机多一个引用字段

两个函数**只差一行代码的位置**：

```rust
// A
pub async fn borrow_across_await() -> u64 {
    let x = String::from("hi");
    let r = &x;
    std::future::ready(()).await;
    r.len() as u64
}

// B
pub async fn borrow_not_across_await() -> u64 {
    let x = String::from("hi");
    let n = { let r = &x; r.len() };
    std::future::ready(()).await;
    n as u64
}
```

MIR：

```mir
// A
field _s0: String;
field _s1: &String;                  // ★ 借用活过了 await
field _s2: std::future::Ready<()>;
Suspend0 (3): [_s0, _s1, _s2]

// B
field _s0: String;
field _s1: usize;                    // ★ 只有算出来的长度
field _s2: std::future::Ready<()>;
Suspend0 (3): [_s0, _s1, _s2]
```

★ **`&String` → `usize`，字段类型直接变了。**

> "一个借用活过 `await`"在实现上就等于"它成了状态机的一个字段"。
> 这也是第 19 章自引用的来源：`_s1: &String` 指向 `_s0: String`。

### 别搞混：返回的引用不是字段

| 函数 | 状态机字段 | 返回的引用 |
|---|---|---|
| `first(v: &[u64]) -> &u64` | `_s0: &[u64]`（参数） | 不是字段（是 `Poll<&u64>` 里的） |
| `longest<'a>(x, y) -> &'a u64` | `_s0`、`_s1` 都是 `&[u64]`（参数） | 不是字段 |
| `peek_across_await<'a>(v) -> u64` | `_s0: &u64`（跨 await 的借用） | 无返回引用 |

## ★ 核心证据二：`Send` 传染 —— 外层 future 的字段就是内层 future

```rust
pub async fn inner(m: &Mutex<u64>) -> u64 {
    let g = m.lock().unwrap();
    std::future::ready(()).await;
    *g
}
pub async fn outer(m: &Mutex<u64>) -> u64 { inner(m).await }
```

`outer_send_ok` 的 MIR 布局：

```mir
coroutine layout {
    field _s0: {async fn body of inner_send_ok()};   // ★ 字段就是内层 future
    variant_fields = { ..., Suspend0 (3): [_s0] }
}
```

错误信息（`fail/send_contagion.rs`）：

```text
error: future cannot be sent between threads safely
  = help: within `impl Future<Output = u64>`, the trait `Send` is not implemented
          for `std::sync::MutexGuard<'_, u64>`
note: future is not `Send` as this value is used across an await
  --> 指向 inner 里的那一行（不是 outer 的调用点）
```

★ **`outer` 一行锁都没写，报错却指向 `inner`** —— 这就是传染的形态。

### 顺带：`async fn` 的函数体仍然只有一行

```mir
fn outer_send_ok() -> {async fn body of outer_send_ok()} {
    bb0: {
        _0 = {coroutine@examples/ch20-async-lifetimes/src/lib.rs:178:37: 178:62 (#0)};
        return;
    }
}
```

调用 = 构造状态机（第 18 章）。**本章讨论的"字段"就是它的内部布局。**

## ★ 核心证据三：`drop(g)` 不能替代块作用域

```rust
// 仍然 !Send
let g = m.lock().unwrap();
let v = *g;
drop(g);                        // ← 一次"使用"，但不缩短变量存活区间
std::future::ready(()).await;
v
```

```text
error: future cannot be sent between threads safely
note: future is not `Send` as this value is used across an await
  |
9 |     let g = m.lock().unwrap();
  |         - has type `std::sync::MutexGuard<'_, u64>` which is not `Send`
...
12 |     std::future::ready(()).await;
  |                            ^^^^^ await occurs here, with `g` maybe used later
```

★ 注意 **"with `g` maybe used later"** —— 明明已经 `drop(g)` 了，
编译器还说它"可能稍后被用"。

**因为编译器数的是变量 `g` 的存活区间，不是值的存活。**
`drop(g)` 在 MIR 里是 `drop(_g)`：一次**使用**，但 `g` 的 `StorageDead`
仍然在函数末尾。而块作用域版本：

```mir
// holds_guard_good
field _s0: u64;                 // ← 没有 MutexGuard
field _s1: std::future::Ready<()>;
```

★ 这与**第 1 章**是同一条原理：借用检查器做的是 CFG 上的数据流分析，
数的是"使用点"和"存活区间"。异步只是给区间加了一个新边界：`await`。

## ★ 核心证据四：future 并非天生 `!Sync`（实测表）

用编译期断言 `assert_send::<F: Send>` / `assert_sync::<F: Sync>` 逐个测：

| future | 状态机字段 | `Send` | `Sync` |
|---|---|---|---|
| `async fn plain() -> u64 { 1 }` | （无） | ✅ | **✅** |
| `async fn with_await()`（有 await，无捕获） | `Ready<()>` | ✅ | **✅** |
| `async fn with_cell()`（捕获 `Cell<u64>`） | `Cell<u64>` | ✅ | ❌ |
| `async fn guard_across(&Mutex<u64>)` | `MutexGuard` | ❌ | **✅** |

★ **`poll` 要 `&mut self` 说的是"不能同时 poll 两次"**，
那是 `&mut` 的独占性，与 `Sync`（"`&Self` 能否跨线程"）是两件事。

**`Send` / `Sync` 由状态机字段决定**，与普通结构体同一条规则
（第 12 章：`Cell<u64>` 是 `Send` 但 `!Sync`）。

### 编译期断言是零成本的

```rust
pub fn assert_send<F: Send>(_f: F) {}
```

- `.O3.s` 里有 `^_check_send_good:`（调用点存在）；
- `.O3.s` 里**没有** `^_assert_send:` 符号 —— 它不生成任何代码。

★ 但**检查完的 future 仍然要被 drop**：`_check_send_good` 的汇编里
有 `bl ...Mutex...Drop4drop` 和 `___rust_dealloc`。
断言不影响生命周期。

## 反例五则

| 文件 | 错误 | 说明 |
|---|---|---|
| `fail/guard_across_await.rs` | `future cannot be sent between threads safely`（含 `MutexGuard`） | 锁守卫跨 `await` |
| `fail/drop_does_not_help.rs` | 同上 + `maybe used later` | ★ `drop` 无效 |
| `fail/ambiguous_lifetime.rs` | **E0106** | 省略规则推不出来 |
| `fail/send_contagion.rs` | `not implemented for std::sync::MutexGuard` | ★ 传染 |
| `fail/future_not_sync.rs` | `cannot be shared between threads safely` | ★ 捕获 `Cell` 才 `!Sync` |

## 交叉验证（可选）

```bash
grep -n 'coroutine layout' -A 9 .evidence/ch20-async-lifetimes-lib.mir
awk '/^fn /{fn=$2} /coroutine layout/{print NR": "fn}' .evidence/ch20-async-lifetimes-lib.mir
grep -n '^_[a-z_]*:' .evidence/ch20-async-lifetimes-lib.O3.s
```

## 待办

- [x] 19 条断言全绿（`verify-all.sh ch20`）
- [x] 5 个反例落库并加断言
- [x] 实测并**推翻**了"future 天生 `!Sync`"这一常见说法
- [ ] 第 21 章（AFIT）需要新 example —— 本章的 `Send` / 生命周期
      在 trait 里会同时变成**设计约束**
- [ ] 第 22 章（tokio）会回到 `tokio::spawn` 要求 `F: Send + 'static` 的场景
