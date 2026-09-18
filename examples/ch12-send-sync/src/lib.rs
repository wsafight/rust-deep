//! 第 12 章：Send 与 Sync 的真相 —— 类型层的检查，零运行时成本
//!
//! 证据生成：tools/evidence.sh ch12-send-sync
//!
//! 本章的核心：**`Send` / `Sync` 是 auto trait，检查发生在类型层**。
//! 汇编里**没有任何"检查 Send"的代码** —— 它完全在单态化时被擦除。

use std::cell::Cell;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;

/// 把值送进另一个线程 —— 编译器要求它 `Send`
#[unsafe(no_mangle)]
pub fn spawn_send(v: Vec<u64>) -> usize {
    let h = std::thread::spawn(move || v.len());
    h.join().unwrap()
}

/// `Arc<T>` 跨线程共享 —— 要求 `T: Send + Sync`
#[unsafe(no_mangle)]
pub fn spawn_share(a: Arc<u64>) -> u64 {
    let b = Arc::clone(&a);
    let h = std::thread::spawn(move || *b);
    let r = h.join().unwrap();
    r + *a
}

/// ★ 反例（类型层，编译不过）—— 见 `fail/` 目录：
/// - `fail/not_send.rs`：`Rc<T>` 不能跨线程（`Rc` 不是 `Send`）
/// - `fail/not_sync.rs`：`Cell<T>` 不能共享引用（`Cell` 不是 `Sync`）
///
/// 这两个反例证明：**检查在编译期，不在运行期**。
/// 汇编里找不到任何痕迹 —— 因为编译失败的程序**根本没有汇编**。

/// 对照：同一个类型，加了 `unsafe impl Send` 就能过 ——
/// 这说明 `Send` 是一个**可以被 unsafe 撒谎的标记**，
/// 而撒谎的代价是 UB（第 24 章）。
pub struct MyBox(pub *mut u64);

// SAFETY: 我们承诺这个裸指针的所有权是独占的（这里只是为了演示，
// 真实代码必须给出完整论证 —— 见第 24 章）。
unsafe impl Send for MyBox {}

/// ★★ 一个非常容易踩的坑（实测，edition 2021+ 的**精确捕获**）：
///
/// 写成 `move || b.0 as usize` 会**编译不过**，即使 `MyBox: Send`——
/// 因为闭包**精确捕获的是 `b.0`（`*mut u64`）这个字段**，而不是整个 `MyBox`，
/// 而 `*mut u64` 不是 `Send`：
/// ```text
/// error[E0277]: `*mut u64` cannot be sent between threads safely
///   = help: within `{closure@...}`, the trait `Send` is not implemented for `*mut u64`
/// ```
/// 加上 `let _ = &b;` 强制捕获整个结构体，就能过 ——
/// **`unsafe impl Send for MyBox` 保护的是 `MyBox`，不是它的字段。**
///
/// 这是"`Send` 是类型层的标记"最生动的反例：
/// **标记加在哪个类型上，就只管哪个类型。**
#[unsafe(no_mangle)]
pub fn spawn_mybox(b: MyBox) -> usize {
    let h = std::thread::spawn(move || {
        let _ = &b;        // ← 强制捕获整个 MyBox（而不是只捕获 b.0）
        b.0 as usize
    });
    h.join().unwrap()
}

/// `Cell` / `Rc` 在**本线程内**完全正常 —— 说明 `!Send` / `!Sync`
/// 不是"这个类型有问题"，而是"这个类型**只在一个线程内**安全"。
#[unsafe(no_mangle)]
pub fn local_only(c: &Cell<u64>, r: &Rc<u64>) -> u64 {
    c.set(c.get() + 1);
    c.get() + **r
}

// ---------- ★ `&T: Send` 需要 `T: Sync`：两个 auto trait 的分工 ----------

/// 实测（`fail/not_sync.rs`）的完整错误信息里有这么一句：
///
/// ```text
/// error[E0277]: `Cell<u64>` cannot be shared between threads safely
///    = help: the trait `Sync` is not implemented for `Cell<u64>`
///    = note: if you want to do aliasing and mutation between multiple threads,
///            use `std::sync::RwLock` or `std::sync::atomic::AtomicU64` instead
///    = note: required for `&Cell<u64>` to implement `Send`
/// ```
///
/// ★ 最后一行是关键：**`&T: Send` 当且仅当 `T: Sync`。**
/// 所以"能不能把一个 `&Cell<u64>` 发到另一个线程"这个问题，
/// 编译器是用 `Sync` 来回答的 —— 而 `Cell` 不是 `Sync`。
///
/// **两个 auto trait 的分工**：
///   - `T: Send`  —— 值本身能搬过去；
///   - `T: Sync`  —— `&T` 能搬过去（也就是能被多个线程同时持有）。
///
/// 注意 `Cell<u64>` **是** `Send`（下面这个函数编译通过），
/// 只是**不是** `Sync`。这个不对称是最容易搞混的地方。
#[unsafe(no_mangle)]
pub fn cell_is_send(c: Cell<u64>) -> u64 {
    // Cell 可以被**移动**到另一个线程 —— 这没问题
    let h = std::thread::spawn(move || c.get());
    h.join().unwrap()
}

// ---------- ★ PhantomData 决定 auto trait ----------

/// `PhantomData<*const T>` 让整个类型变成 `!Send + !Sync`。
///
/// ★ 注意**错误信息里的措辞**（实测，`fail/phantom_not_send.rs`）：
///
/// ```text
/// error[E0277]: `*const u64` cannot be sent between threads safely
///    = help: within `{closure@...}`, the trait `Send` is not implemented for `*const u64`
/// note: required because it appears within the type `PhantomData<*const u64>`
/// ```
///
/// 最后一行说的是 `PhantomData<*const u64>` —— **不是 `Handle<u64>`**。
/// 编译器直接把"谁该为 `!Send` 负责"指了出来：
/// **`PhantomData` 里的那个类型**。
///
/// 换一个写法就完全相反：
///
/// | `PhantomData<...>` | `Send`? | `Sync`? |
/// |---|---|---|
/// | `PhantomData<*const T>` | ❌ | ❌ |
/// | `PhantomData<fn() -> T>` | ✅ | ✅ |
/// | `PhantomData<T>` | 跟随 `T` | 跟随 `T` |
///
/// **因为 `fn() -> T` 是函数指针，函数指针总是 `Send + Sync`。**
/// 这是"用 `PhantomData` 精确控制 auto trait"的标准手法。
pub struct Handle<T> {
    id: u64,
    _p: PhantomData<*const T>,
}

/// 对照：`fn() -> T` 版本是 `Send` 的 —— 同一个 `T`，只差 PhantomData 的写法。
pub struct SafeHandle<T> {
    id: u64,
    _p: PhantomData<fn() -> T>,
}

#[unsafe(no_mangle)]
pub fn spawn_safe_handle(h: SafeHandle<u64>) -> u64 {
    let h2 = std::thread::spawn(move || h.id);
    h2.join().unwrap()
}

/// ★ 还有一个更细的坑：**闭包只捕获用到的字段**（edition 2021+ 的精确捕获）。
///
/// `Handle<u64>` 整体是 `!Send`，但只要闭包**不捕获** `_p`，
/// 编译就能通过 —— 因为捕获到的是 `id`（`u64`，是 `Send`）。
///
/// 实测：
/// - `move || h.id`               → ✅ 编译通过（只捕获 `id`）
/// - `move || { let _ = &h; h.id }` → ❌ E0277（`&h` 强制捕获整个 `Handle`）
///
/// 这与 `spawn_mybox` 那个坑是**同一个机制的两面**：
/// **捕获粒度决定了哪个类型被拿去检查 auto trait。**
#[unsafe(no_mangle)]
pub fn spawn_handle_field(h: Handle<u64>) -> u64 {
    let h2 = std::thread::spawn(move || h.id);   // ← 只捕获 id
    h2.join().unwrap()
}

// ---------- ★★ `Sync` 和 `Send` 是**两个独立的性质**：MutexGuard ----------

/// `MutexGuard<'_, T>` 是 `Sync`，但**不是** `Send`。
///
/// 这个函数编译通过 —— 它要求 `&MutexGuard: Send`，即 `MutexGuard: Sync`。
#[unsafe(no_mangle)]
pub fn guard_is_sync(g: &std::sync::MutexGuard<'_, u64>) -> u64 {
    **g
}

/// ⚠️ 但把 `MutexGuard` **按值**送进另一个线程就不行（见
/// `fail/guard_not_send.rs`）：
///
/// ```text
/// error[E0277]: `std::sync::MutexGuard<'_, u64>` cannot be sent between threads safely
///    = help: the trait `Send` is not implemented for `std::sync::MutexGuard<'_, u64>`
/// ```
///
/// ★ **为什么 `!Send`？** 因为 `pthread_mutex` 只保证
/// "**加锁的那个线程**能解锁" —— 把它移到别的线程去 unlock 是未定义行为。
/// 标准库因此**只**给 `MutexGuard` 实现了 `Sync`，**没有**实现 `Send`。
///
/// ★ **这就是"`Sync` 和 `Send` 是两个独立性质"最干净的证据**：
///
/// | 类型 | `Send` | `Sync` |
/// |---|---|---|
/// | `Cell<u64>` | ✅ | ❌ |
/// | `MutexGuard<'_, u64>` | ❌ | ✅ |
/// | `Rc<u64>` | ❌ | ❌ |
/// | `u64` | ✅ | ✅ |
///
/// 四个格子全都填满了 —— 两个 trait 之间**没有任何蕴含关系**。
///
/// （这也解释了 `Arc<T>: Send` 为什么要求 `T: Send + Sync`：
/// `Arc` 既让你**移动**它，也让你**共享**它。）
#[unsafe(no_mangle)]
pub fn arc_needs_both(a: std::sync::Arc<u64>) -> u64 {
    let b = std::sync::Arc::clone(&a);
    let h = std::thread::spawn(move || *b);   // 移动 Arc（要 Send）
    h.join().unwrap() + *a                    // 共享 Arc（要 Sync）
}
