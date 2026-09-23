//! 违反 Stacked Borrows 的**最小例子** —— 只给 Miri 跑。
//!
//! `#![cfg(miri)]` 让整个文件在普通 `cargo test` 下**不存在**
//! （UB 在普通运行下不一定当场崩，混进常规测试会让 CI 不稳定）。
//!
//! 复现：
//!   cargo +nightly miri test -p ch25-aliasing --test sb_ub   # 必须失败

#![cfg(miri)]

/// 用失效的共享引用 —— Miri 报：
/// `trying to retag from <N> for SharedReadOnly permission ...,
///  but that tag does not exist in the borrow stack`
#[test]
fn invalidated_shared_ref() {
    let mut x = 0u64;
    let p: *mut u64 = &mut x;
    // SAFETY（伪）：这里其实不 sound —— 正是本节要展示的 UB
    let r: &u64 = unsafe { &*p };
    unsafe { *p = 1 }; // 通过裸指针写，把 r 从借用栈里弹掉
    assert_eq!(*r, 1); // ← 用已失效的 r：UB
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
