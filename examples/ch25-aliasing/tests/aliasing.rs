//! 别名规则：**合法**用例 —— Miri 必须通过（不误报）。
//!
//! 复现：
//!   cargo +nightly miri test -p ch25-aliasing --test aliasing      # 必须通过
//!   cargo +nightly miri test -p ch25-aliasing --test aliasing_ub   # 必须失败
//!
//! 与 `sb_legal.rs` 的分工：
//! - `sb_legal.rs` 演示的是**手工裸指针**下的合法边界；
//! - 本文件演示的是**本章的主角**：`UnsafeCell` 与 `PhantomData`。
//!
//! `#![cfg(miri)]` 不是因为这些代码不合法，而是因为
//! "通过 Stacked Borrows 检查"这件事只有 Miri 能证明。

#![cfg(miri)]

use ch25_aliasing::{Cell2, SharedReadOnly, SharedReadWrite};
use std::cell::UnsafeCell;

/// ★ 核心：`UnsafeCell` 让"通过共享引用修改"变得合法。
#[test]
fn unsafe_cell_allows_shared_write() {
    let c = Cell2::new(1u64);
    c.set(2); // &self 上写 —— 没有 UnsafeCell 就是 UB
    assert_eq!(c.get(), 2);
}

/// `&mut T` 上的写：当然合法。
#[test]
fn mut_ref_write_is_fine() {
    let mut x = 1u64;
    let p = &mut x;
    *p = 2;
    assert_eq!(x, 2);
}

/// 从裸指针造引用、用完就丢 —— 合法。
#[test]
fn raw_roundtrip() {
    let mut x = 7u64;
    let p: *mut u64 = &mut x;
    // SAFETY: 指针来自有效引用，且在借用期间没有其他访问
    let r = unsafe { &mut *p };
    *r = 8;
    assert_eq!(x, 8);
}

/// ★ `SharedReadOnly`（`PhantomData<&'a T>`）：只读访问合法。
#[test]
fn shared_read_only_is_fine() {
    let x = 5u64;
    let a = SharedReadOnly::new(&x);
    assert_eq!(a.get(), 5);
}

/// ★ `SharedReadWrite`（`PhantomData<&'a UnsafeCell<T>>`）：读写都合法。
///
/// 两个结构体的字段布局完全一样，区别只在 `PhantomData` 的类型参数 ——
/// 而这个区别正是 Miri 用来判断"这个指针允许被写吗"的依据。
#[test]
fn shared_read_write_is_fine() {
    let c = UnsafeCell::new(5u64);
    let a = SharedReadWrite::new(&c);
    a.set(6);
    assert_eq!(a.get(), 6);
}

/// ★★ 本章最微妙的一条：`UnsafeCell` **不会救活一个已经存在的 `&T`**。
///
/// 顺序决定一切（实测 Miri）：
///
/// | 顺序 | 结果 |
/// |---|---|
/// | 先建 `&T`，再通过 `UnsafeCell` 写，再用 `&T` | ❌ **UB**（见 `aliasing_ub.rs`） |
/// | 先通过 `UnsafeCell` 写，再建 `&T` | ✅ 合法（本用例） |
/// | 全程只用 `UnsafeCell` 指针 | ✅ 合法 |
///
/// **实践规则**：不要在写入之前建立 `&T` 并持有到写入之后。
#[test]
fn write_then_read_is_fine() {
    let c = UnsafeCell::new(1u64);
    let w = SharedReadWrite::new(&c);
    w.set(2); // ← 先写
    // SAFETY: 写入已经完成，此后不再通过 w 写
    let r: &u64 = unsafe { &*c.get() }; // ← 后建 &T
    assert_eq!(*r, 2);
}

/// 全程只用 `UnsafeCell` 指针，从不建 `&T` —— 合法。
#[test]
fn only_unsafe_cell_pointers() {
    let c = UnsafeCell::new(1u64);
    let p = c.get();
    // SAFETY: 单线程，且没有别的引用同时存在
    unsafe { *p = 2 };
    assert_eq!(unsafe { *p }, 2);
}

/// 两个 `&u64` 别名同一块内存：安全 Rust 允许，`noalias` 也不禁止。
///
/// 依据 LLVM LangRef：`noalias` 的保证
/// **"only holds for memory locations that are modified"** ——
/// 两个只读指针别名同一处，没有任何内存被修改，不触发任何义务。
#[test]
fn two_shared_refs_may_alias() {
    let v = 21u64;
    let p: *const u64 = &v;
    // SAFETY: v 在作用域内不会被修改
    let (a, b) = unsafe { (&*p, &*p) };
    assert_eq!(a.wrapping_add(*b), 42);
}
