//! 别名规则：**违反**用例 —— 只给 Miri 跑（必须报 UB）。
//!
//! 复现：
//!   cargo +nightly miri test -p ch25-aliasing --test aliasing_ub    # 必须失败
//!
//! `#![cfg(miri)]` 让整个文件在普通 `cargo test` 下**不存在** ——
//! 因为 UB 在普通运行下不一定当场崩，混进常规测试会让 CI 不稳定。
//! （实测：不包 `cfg(miri)` 时，下面的 `shared_ref_invalidated_by_mut_write`
//! 在 `-O` 下也照样输出正确数字。）

#![cfg(miri)]

use std::cell::UnsafeCell;
use std::hint::black_box;

/// ★ 违反 Stacked Borrows 的**教科书形态**：
/// 从 `&T` 派生的指针，被一次写入**弹掉**，之后再用它。
///
/// Miri 报：
/// `Undefined Behavior: attempting a read access using <N> at alloc<N>[0x0],
///  but that tag does not exist in the borrow stack for this location`
///
/// 并指出：`<N> was created by a SharedReadOnly retag`、
/// `<N> was later invalidated ... by a write access`。
#[test]
fn shared_ref_invalidated_by_mut_write() {
    let mut x = 0u64;
    let p: *mut u64 = &mut x;
    // SAFETY（伪）：下面 *p = 1 之后 r 就失效了
    let r: &u64 = unsafe { &*p };
    unsafe { *p = 1 }; // 写：把 r 从借用栈里弹掉
    assert_eq!(*r, 1); // ← 用已失效的 r：UB
}

/// ★ 同一类错误，但**借用检查器和 lint 都看不见**（跨函数 + `black_box`）。
///
/// 这是 `fail/write_through_shared_ref.rs`（rustc 的 `invalid_reference_casting`
/// lint 能抓到的那个版本）的**绕开版本**：
/// 把铸型点藏进一个函数、再用 `black_box` 挡住数据流，
/// rustc 的局部 lint 就抓不到了；Miri 可以在这条执行路径上发现它。
///
/// ★ 这正是"`unsafe` 的义务不写在代码里"的代价：
/// **同一段逻辑，写在一个函数里会被拒，拆成两个函数就编译通过。**
#[test]
fn shared_write_hidden_behind_black_box() {
    let c = UnsafeCell::new(1u64);
    // SAFETY（伪）：r 从 &T 派生，而下面要通过裸指针写
    let r: &u64 = unsafe { &*c.get() };
    let p = black_box(r as *const u64 as *mut u64);
    unsafe { *p = 2 }; // ← 通过从 &T 派生的裸指针写：UB
    assert_eq!(unsafe { *c.get() }, 2);
}

/// ★★ `UnsafeCell` **不会救活一个已经存在的 `&T`** —— 顺序决定一切。
///
/// 这段代码里每一次写入都经由 `UnsafeCell`（看起来"合法"），
/// 但 `r` 是在写入**之前**建立的 —— 那次写把它弹掉了。
///
/// Miri 报：`<N> was created by a SharedReadOnly retag ... was later
/// invalidated ... by a write access`。
///
/// ★ 对照 `tests/aliasing.rs` 的 `write_then_read_is_fine`：
/// **只差两行的顺序**，一个合法、一个 UB。
#[test]
fn shared_ref_invalidated_by_unsafe_cell_write() {
    let c = UnsafeCell::new(1u64);
    // SAFETY（伪）：r 建立之后还会有人写
    let r: &u64 = unsafe { &*c.get() };
    let w = ch25_aliasing::SharedReadWrite::new(&c);
    w.set(2); // ← 经由 UnsafeCell 的写：本身合法，但弹掉了 r
    assert_eq!(*r, 2); // ← 用已失效的 r：UB
}

/// 悬垂指针 —— Miri 报：
/// `memory access failed: alloc<N> has been freed, so this pointer is dangling`
#[test]
fn dangling_pointer() {
    let mut v = vec![1u64, 2, 3];
    let p: *const u64 = &v[0];
    v.push(4); // 可能 realloc，v 的缓冲区被释放
    // SAFETY（伪）：v 已重新分配，p 悬垂
    let x = unsafe { *p };
    assert!(x == 1 || x == 4); // 结果不可预测
}
