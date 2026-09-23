//! 第 5 章：边界检查 —— 安全检查什么时候是免费的
//!
//! 证据生成：tools/evidence.sh ch05-bounds
//!
//! 本章要破除的误解：**"`get_unchecked` 比 `[]` 快"**。
//! 热路径上它们生成**完全相同的代码**；差别只在**失败路径**。

/// 安全版本：越界进 `panic_bounds_check`
#[unsafe(no_mangle)]
pub fn safe(v: &[u8], i: usize) -> u8 {
    v[i]
}

/// 手动 assert + `get_unchecked`：热路径与安全版**逐条相同**，
/// 但失败路径是 `panic`（带 assertion 消息），不是 `panic_bounds_check`
#[unsafe(no_mangle)]
pub fn checked_unchecked(v: &[u8], i: usize) -> u8 {
    assert!(i < v.len());
    // SAFETY: 上一行的 assert 保证了 i < v.len()
    unsafe { *v.get_unchecked(i) }
}

/// 真正危险的写法：删掉 assert 却仍然假设成立 —— 这才是 UB
#[unsafe(no_mangle)]
pub fn raw_unchecked(v: &[u8], i: usize) -> u8 {
    // SAFETY: 调用者必须保证 i < v.len()（这是 unsafe fn 该做的事，
    // 但这里故意写成 safe fn —— 本书用它来演示"漏掉的检查"）
    unsafe { *v.get_unchecked(i) }
}

/// 检查被**消除**的情形：索引由 `%` 约束在范围内时，LLVM 能证明它不越界
#[unsafe(no_mangle)]
pub fn provably_in_bounds(v: &[u8; 4], i: usize) -> u8 {
    v[i % 4]
}

/// 对照：`Vec` 增长时 LLVM 无法消除检查
#[unsafe(no_mangle)]
#[allow(clippy::needless_range_loop)]
pub fn sum_all(v: &[u64]) -> u64 {
    let mut s = 0u64;
    for i in 0..v.len() {
        s = s.wrapping_add(v[i]);
    }
    s
}
