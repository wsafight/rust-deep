//! 看起来像 UB，其实**完全合法** —— 用来证明 Miri 不会误报。
//!
//! 复现：
//!   cargo +nightly miri test -p ch25-aliasing --test sb_legal   # 应通过

/// 两个 `&u64` 别名同一块内存：安全 Rust 允许，LLVM 的 `noalias` 也不禁止。
///
/// 依据 LLVM LangRef：`noalias` 的保证
/// **"only holds for memory locations that are modified"** ——
/// 两个只读指针别名同一处，没有任何内存被修改，不触发任何义务。
#[test]
fn two_shared_refs_may_alias() {
    let v = 21u64;
    let p: *const u64 = &v;
    // SAFETY: v 在作用域内不会被修改，两个共享借用同时存在是合法的
    let (a, b) = unsafe { (&*p, &*p) };
    assert_eq!(a.wrapping_add(*b), 42);
}

/// 先读完，再通过裸指针写 —— 顺序正确就合法。
#[test]
fn read_then_write_is_fine() {
    let mut x = 0u64;
    let p: *mut u64 = &mut x;
    // SAFETY: 共享借用 r 在 *p = 1 之前就已经使用完毕
    let r: &u64 = unsafe { &*p };
    assert_eq!(*r, 0);
    unsafe { *p = 1 };
    assert_eq!(x, 1);
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
