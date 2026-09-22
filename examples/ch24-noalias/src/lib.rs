//! 第 24 章：unsafe 的边界哲学 —— rustc 到底向 LLVM 承诺了什么
//!
//! 证据生成：tools/evidence.sh ch24-noalias
//!
//! 本章的核心：**`&T` / `&mut T` 不只是类型，它们是写进 LLVM IR 的契约**。
//! 这一组函数把契约的四种形态钉死在可断言的位置上。

/// `&[u64]` 只读 → `noalias + readonly`
#[unsafe(no_mangle)]
pub fn sum_shared(v: &[u64]) -> u64 {
    v.iter().fold(0u64, |a, b| a.wrapping_add(*b))
}

/// `&mut [u64]` **也只读** → 与上面**完全相同**的 `noalias + readonly`
#[unsafe(no_mangle)]
pub fn sum_mut_ro(v: &mut [u64]) -> u64 {
    v.iter().fold(0u64, |a, b| a.wrapping_add(*b))
}

/// `&mut [u64]` 真的写了 → `readonly` 消失，但 `noalias` 还在
#[unsafe(no_mangle)]
pub fn sum_mut_rw(v: &mut [u64]) -> u64 {
    v[0] = 7;
    v.iter().fold(0u64, |a, b| a.wrapping_add(*b))
}

/// ★ 两个共享引用**可能指向同一块内存**，但两个参数**都**被标 `noalias`。
/// 这**不是** bug：LLVM LangRef 明确说 `noalias` 只约束"**被修改**"的内存
/// （"This guarantee only holds for memory locations that are modified"）。
/// 两个只读指针别名同一处 —— 没有任何内存被修改 —— 不触发 noalias 的任何义务。
#[unsafe(no_mangle)]
pub fn two_shared(a: &u64, b: &u64) -> u64 {
    a.wrapping_add(*b)
}

/// ★ `noalias` 的威力只有在**跨参数**时才真正显现：
/// LLVM 敢同时读 `a`、读 `b`、写 `dst` 而不做运行时别名检查，
/// 三个参数的 `noalias` 是重要依据；是否向量化还取决于其他优化条件。
#[unsafe(no_mangle)]
pub fn add_all(dst: &mut [u64], a: &[u64], b: &[u64]) {
    for i in 0..dst.len() {
        dst[i] = a[i].wrapping_add(b[i]);
    }
}

// ============================================================
// ★ 6) `noalias` 换来了什么：同一段逻辑，安全引用 vs 裸指针
// ============================================================

/// 用**安全引用**写：`*a += *b` 写两次。
///
/// 这两句能不能合并成 `*a += *b * 2`，取决于"第二次读 `*b` 时它有没有变"——
/// 而 `*a` 的写入会不会改到 `*b`，正是别名问题。
/// `noalias` 告诉 LLVM"不会"，于是它**敢合并**。
#[unsafe(no_mangle)]
pub fn safe_double_add(a: &mut i32, b: &i32) {
    let x = *b;
    *a += x;
    *a += *b;
}

/// 同一段逻辑，用**裸指针**写。
///
/// 裸指针**不带 `noalias`** —— 于是 LLVM 必须假设 `a` 和 `b` 可能指向同一处，
/// 第二次 `*b` 必须**重新读**。多出一次 load。
///
/// ★ 这说明 `unsafe` 本身不是性能开关：这里真正影响优化的是
/// "引用参数换成裸指针"导致 `noalias` 信息丢失，而不是 `unsafe` 关键字。
///
/// # Safety
/// `a` 必须非空、正确对齐、可写，并在调用期间指向一个已初始化的 `i32`；
/// `b` 必须非空、正确对齐、可读，并在调用期间指向一个已初始化的 `i32`；
/// 两个指针指向的内存不得重叠。
#[unsafe(no_mangle)]
pub unsafe fn raw_double_add(a: *mut i32, b: *const i32) {
    // SAFETY: 上述调用契约保证两次裸指针解引用有效，且写 a 不会影响 b。
    unsafe {
        let x = *b;
        *a += x;
        *a += *b;
    }
}
