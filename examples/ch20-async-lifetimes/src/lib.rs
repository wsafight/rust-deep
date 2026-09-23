//! 第 20 章：`async` 中的生命周期与 `Send` 传染
//!
//! 证据生成：tools/evidence.sh ch20-async-lifetimes
//!
//! 第 18 章：`async fn` 展开成状态机（每个 `await` 一个变体）。
//! 第 19 章：状态机可能自引用，所以必须 `Pin`。
//! 本章：**状态机里"活到 `await` 之后"的那些字段，会决定两件事** ——
//!
//! 1. **生命周期**：借用跨过 `await` 时，状态机必须把它记进字段（MIR 里看得见）；
//! 2. **`Send`**：某个字段 `!Send`，整个 future 就 `!Send` —— 而且会**传染**。
//!
//! ★ 本章最反直觉的一条（`fail/drop_does_not_help.rs`）：
//!   `drop(guard)` **不能**让 future 变回 `Send`。只有**块作用域**可以。
//!   编译器说的是"`g` maybe used later" —— 它数的是**变量的存活**，
//!   不是**值的存活**。

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// 编译期断言：`F: Send`。**没有任何运行时开销** ——
/// 它只在类型层做一次检查，然后消失。
///
/// 这是本章全部 `Send` 证据的来源：把一个 future 交给它，
/// 编译过 = `Send`，编译不过 = `!Send`，错误信息里还会指出**是哪个字段**。
pub fn assert_send<F: Send>(_f: F) {}

/// 对照：断言 `F: Sync`。
pub fn assert_sync<F: Sync>(_f: F) {}

// ============================================================
// 1) 借用跨过 await：状态机必须记住它
// ============================================================

/// ★ `r` 借用 `x`，而 `r` 活过了 `.await` ——
/// 状态机里就会出现 `field _s1: &String`。
#[unsafe(no_mangle)]
pub async fn borrow_across_await() -> u64 {
    let x = String::from("hi");
    let r = &x;
    std::future::ready(()).await;
    r.len() as u64
}

/// 对照：借用**没有**跨过 `await`（`r` 在 await 之前就用完了）——
/// 状态机里**不会**有 `&String` 字段，只有 `usize`。
///
/// ★ 两份代码只差 `r.len()` 的位置，状态机的**字段集合**就不同。
/// 这就是"生命周期在状态机里的可见形态"。
#[unsafe(no_mangle)]
pub async fn borrow_not_across_await() -> u64 {
    let x = String::from("hi");
    let n = {
        let r = &x;
        r.len()
    };
    std::future::ready(()).await;
    n as u64
}

// ============================================================
// 2) async fn 的签名：生命周期省略规则照旧适用
// ============================================================

/// 单输入引用 → 输出引用：省略规则可以推出 `'a`，不必手写。
#[unsafe(no_mangle)]
pub async fn first(v: &[u64]) -> &u64 {
    std::future::ready(()).await;
    &v[0]
}

/// 两个输入引用 → 输出引用：省略规则**推不出来**，必须显式标注。
/// （见 `fail/ambiguous_lifetime.rs`：去掉 `<'a>` 就是 E0106。）
///
/// ★ 注意一个容易搞混的点：**返回的 `&'a u64` 不是状态机的字段** ——
/// 它是 `poll` 返回的 `Poll<&'a u64>` 里那个引用。
/// `'a` 是**类型参数**，不是"状态机里存着一个引用"。
#[unsafe(no_mangle)]
pub async fn longest<'a>(x: &'a [u64], y: &'a [u64]) -> &'a u64 {
    std::future::ready(()).await;
    if x.len() > y.len() { &x[0] } else { &y[0] }
}

/// 借用参数跨过 `await` —— 状态机字段就是那个引用。
/// 这里的 `'a` 真的活在状态机里（对比 `longest`）。
#[unsafe(no_mangle)]
#[allow(clippy::needless_lifetimes)]
pub async fn peek_across_await<'a>(v: &'a [u64]) -> u64 {
    let head = &v[0];
    std::future::ready(()).await;
    *head
}

// ============================================================
// 3) Send 传染：状态机的字段决定 future 的 Send
// ============================================================

/// ★ 错误示范（这里**不编译它**，见 `fail/guard_across_await.rs`）：
///
/// ```ignore
/// pub async fn holds_guard_bad(m: &Mutex<u64>) -> u64 {
///     let g = m.lock().unwrap();
///     std::future::ready(()).await;   // ← g 活过了 await
///     *g
/// }
/// ```
///
/// 报错：`future returned by holds_guard_bad is not Send`，
/// 并指出 `MutexGuard<'_, u64>` 不是 `Send`、以及"await occurs here,
/// with `g` maybe used later"。
///
/// ★ **正确写法：把锁的作用域收进一个块**。
///
/// 于是 `g` 的**存活区间**在 `await` 之前就结束了 ——
/// 状态机的 `Suspend0` 变体里只有 `_s0: u64`，没有 `MutexGuard`。
#[unsafe(no_mangle)]
pub async fn holds_guard_good(m: &Mutex<u64>) -> u64 {
    let v = {
        let g = m.lock().unwrap();
        *g
    };
    std::future::ready(()).await;
    v
}

/// ★ 反直觉：**`drop(g)` 不能替代块作用域**。
///
/// 这段代码（见 `fail/drop_does_not_help.rs`）**仍然报 `!Send`**：
///
/// ```ignore
/// let g = m.lock().unwrap();
/// let v = *g;
/// drop(g);                       // ← 看起来"用完了"
/// std::future::ready(()).await;  // ← 编译器仍然说 "g maybe used later"
/// ```
///
/// 因为编译器数的是**变量 `g` 的存活区间**，而不是**值的存活**。
/// `drop(g)` 是一次使用，但它**没有缩短变量的存活区间**。
/// 只有块作用域能。
///
/// 对照：`&AtomicU64` 是 `Send`（`AtomicU64: Sync`），所以能跨 `await`。
#[unsafe(no_mangle)]
pub async fn holds_atomic_ref(a: &AtomicU64) -> u64 {
    let r = a;
    std::future::ready(()).await;
    r.load(Ordering::Relaxed)
}

/// 对照：`Cell<u64>` 是 `Send` 但**不是 `Sync`** ——
/// 捕获它会让 future 变成 `!Sync`（见 `fail/future_not_sync.rs`）。
#[unsafe(no_mangle)]
pub async fn holds_cell() -> u64 {
    let c = std::cell::Cell::new(1u64);
    std::future::ready(()).await;
    c.get()
}

/// 把上面两个 future 交给 `assert_send`：**编译过就是证据**。
#[unsafe(no_mangle)]
pub fn check_send_good() {
    assert_send(holds_guard_good(&Mutex::new(1)));
    assert_send(holds_atomic_ref(&AtomicU64::new(1)));
    assert_send(borrow_across_await());
    assert_send(peek_across_await(&[1, 2, 3]));
}

// ============================================================
// 4) Send 传染：内层 future 的 Send 决定外层
// ============================================================

/// ★ 一个 `!Send` 的内层 future，会把外层也拖下水。
///
/// `fail/send_contagion.rs` 里的 `outer` 只是 `.await` 了 `inner`，
/// 但错误信息指向的是 **`inner` 里的 `MutexGuard`** ——
/// 说明 `Send` 是沿着 `.await` 一路推导上去的。
#[unsafe(no_mangle)]
pub async fn inner_send_ok() -> u64 {
    let m = Mutex::new(1);
    let v = {
        let g = m.lock().unwrap();
        *g
    };
    std::future::ready(()).await;
    v
}

#[unsafe(no_mangle)]
pub async fn outer_send_ok() -> u64 {
    inner_send_ok().await
}

#[unsafe(no_mangle)]
pub fn check_send_outer() {
    assert_send(outer_send_ok());
}

// ============================================================
// 5) Send 与 Sync 是**两个独立性质**（实测，见 fail/future_not_sync.rs）
// ============================================================

/// ★ 实测表格（rustc 1.98.1）：
///
/// | future | `Send` | `Sync` |
/// |---|---|---|
/// | `async fn plain() -> u64 { 1 }` | ✅ | **✅** |
/// | `async fn with_await()`（有 await，无捕获） | ✅ | **✅** |
/// | `async fn with_cell()`（捕获 `Cell<u64>`） | ✅ | ❌ |
/// | `async fn guard_across(&Mutex<u64>)` | ❌ | **✅** |
///
/// ★ 这**推翻**了"future 天生 `!Sync`，因为 `poll` 要 `&mut self`"这个说法。
///   `poll` 要 `&mut self` 说的是"不能同时 poll 两次"——
///   那是 `&mut` 的独占性，与 `Sync`（"`&Self` 能否跨线程"）是两件事。
///
///   和普通类型完全一样（第 12 章：`Cell<u64>` 是 `Send` 但 `!Sync`）：
///   **`Send` / `Sync` 由状态机里装了哪些字段决定。**
#[unsafe(no_mangle)]
pub fn check_send_not_sync() {
    assert_send(inner_send_ok());
}
