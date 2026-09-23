//! 第 26 章：何时**不该**用 `unsafe`
//!
//! 证据生成：tools/evidence.sh ch26-when-not-to
//!
//! 本章不给"永远别用 unsafe"这种结论，而是给一条**可执行的判据**：
//!
//! > **先看安全版本编译成了什么。**
//! > 只有在你确实验证了"这里编译器证不出来"、
//! > 并且这个位置在热路径上时，`unsafe` 才值得考虑。
//!
//! ★ 本章的核心实测：**"安全版本"和"`unsafe` 版本"经常生成同一个函数** ——
//!   汇编里是 `_sum_unchecked = _sum_safe` 这样的 alias。
//!   这种情况下，`unsafe` 换来的**恰好是零**。
//!
//! 三组对照（`-O` / `aarch64-apple-darwin` / rustc 1.98.1）：
//!
//! | 情形 | 安全版本 | `unsafe` 版本 | 差值 |
//! |---|---|---|---|
//! | 循环内索引（编译器证得出不越界） | 39 条指令 + 向量化 | **同一个函数** | 0 |
//! | 掩码索引 `i & 3`（编译器证得出） | **3 条**指令 | **同一个函数** | 0 |
//! | 外部传入的索引（证不出） | 11 条 + panic 路径 | **2 条**指令 | 9 条 |

// ============================================================
// 1) 循环内索引：编译器证得出不越界 → 两者是**同一个函数**
// ============================================================

/// 安全版本：`v[i]` 带边界检查（但检查被 LLVM 消除了，见下）。
#[unsafe(no_mangle)]
#[allow(clippy::needless_range_loop)]
pub fn sum_safe(v: &[u64]) -> u64 {
    let mut s = 0u64;
    for i in 0..v.len() {
        s = s.wrapping_add(v[i]);
    }
    s
}

/// ★ `unsafe` 版本：手写 `get_unchecked`。
///
/// **实测：它与 `sum_safe` 生成完全相同的代码** ——
/// LLVM 把 `sum_unchecked` 折叠成了 `sum_safe` 的 alias：
///
/// ```asm
/// _sum_unchecked = _sum_safe
/// ```
///
/// 为什么？因为循环条件是 `0..v.len()`，编译器**证得出** `i < v.len()`，
/// 于是安全版本里的检查**已经被消除了** —— 两个函数逐位等价。
///
/// ★ 这就是本章第一条判据：**`unsafe` 想换的东西，编译器可能已经给你了。**
///
/// # Safety
/// 循环上界就是 `v.len()`，`i` 恒小于 `len`
#[unsafe(no_mangle)]
pub fn sum_unchecked(v: &[u64]) -> u64 {
    let mut s = 0u64;
    for i in 0..v.len() {
        // SAFETY: i < v.len() 由循环条件保证
        s = s.wrapping_add(unsafe { *v.get_unchecked(i) });
    }
    s
}

/// 第三种写法：迭代器。**与 `sum_safe` 逐字节相同**（都是 39 条指令、都向量化）。
#[unsafe(no_mangle)]
pub fn sum_iter(v: &[u64]) -> u64 {
    v.iter().fold(0u64, |a, b| a.wrapping_add(*b))
}

// ============================================================
// 2) 掩码索引：编译器证得出 → 同样是**同一个函数**
// ============================================================

/// 安全版本：`i & 3` 一定落在 `0..4`，所以检查被**消除**。
///
/// ```asm
/// _masked_safe:
///     and    x8, x1, #0x3          ; ← 只剩掩码本身
///     ldr    x0, [x0, x8, lsl #3]
///     ret                        ; 3 条指令，没有任何检查
/// ```
#[unsafe(no_mangle)]
pub fn masked_safe(v: &[u64; 4], i: usize) -> u64 {
    v[i & 3]
}

/// ★ `unsafe` 版本：**与 `masked_safe` 是同一个函数**（`_masked_unchecked = _masked_safe`）。
///
/// LLVM 从 `i & 3` 直接推出了 `i & 3 < 4`。
/// **这里 `unsafe` 换来的东西是零。**
///
/// # Safety
/// `i & 3` 恒小于 4，即数组长度
#[unsafe(no_mangle)]
pub fn masked_unchecked(v: &[u64; 4], i: usize) -> u64 {
    // SAFETY: i & 3 < 4
    unsafe { *v.get_unchecked(i & 3) }
}

// ============================================================
// 3) 唯一有差值的情形：索引**来自外部**，编译器证不出来
// ============================================================

/// 安全版本：索引是参数，编译器**无法**证明 `i < len` ——
/// 检查必须保留，并带上 panic 路径。**11 条指令**。
///
/// ```asm
/// _get_safe:
///     cmp    x2, x1                 ; ← 边界检查
///     b.hs   LBB0_2                 ; ← 越界则跳去 panic
///     ldr    x0, [x0, x2, lsl #3]
///     ret
/// LBB0_2:                          ; cold 路径
///     ...  bl  panic_bounds_check
/// ```
#[unsafe(no_mangle)]
pub fn get_safe(v: &[u64], i: usize) -> u64 {
    v[i]
}

/// `unsafe` 版本：省掉检查，**2 条指令**。
///
/// ```asm
/// _get_unchecked:
///     ldr    x0, [x0, x2, lsl #3]
///     ret
/// ```
///
/// ★ **这是本章唯一一个 `unsafe` 真的省下东西的情形** ——
/// 省下 `cmp` + `b.hs` 两条指令，加上一段 cold 的 panic 代码。
///
/// ★ 所以正确的判断流程是：
///
/// 1. **这个索引在热路径上吗？** 不是 → 不需要 `unsafe`；
/// 2. **编译器证得出不越界吗？** 证得出 → 不需要 `unsafe`（检查已经没了）；
/// 3. **你能量化"省下两条指令"对整体吞吐的影响吗？** 不能 → 先别写。
///
/// 三条都过不了，`unsafe` 就是**纯粹的风险**。
///
/// # Safety
/// 调用者必须保证 `i < v.len()`
#[unsafe(no_mangle)]
pub unsafe fn get_unchecked(v: &[u64], i: usize) -> u64 {
    // SAFETY: 由调用者保证 i < v.len()
    unsafe { *v.get_unchecked(i) }
}

// ============================================================
// 4) 反例：`unsafe` 反而**更慢**（第 24 章的结论，这里再钉一次）
// ============================================================

/// 安全引用版本：参数带 `noalias`，LLVM 敢把两次加法合并。**5 条指令**。
#[unsafe(no_mangle)]
pub fn safe_double_add(a: &mut i32, b: &i32) {
    let x = *b;
    *a += x;
    *a += *b;
}

/// 裸指针版本：**没有 `noalias`**，必须重新读 `*b` —— **8 条指令**。
///
/// ★ 这是"用 `unsafe` 换性能"这个前提**本身就是错的**最直接的证据：
/// 你不是写了一个更快的版本，你是**主动放弃了编译器的一项优化**。
///
/// # Safety
/// `a` 必须有效、正确对齐、可读写并指向已初始化的 `i32`；`b` 必须有效、
/// 正确对齐、可读并指向已初始化的 `i32`；两者在调用期间存活且不重叠。
#[unsafe(no_mangle)]
pub unsafe fn raw_double_add(a: *mut i32, b: *const i32) {
    // SAFETY: 调用者保证 a 可写、b 可读、两者不重叠
    unsafe {
        let x = *b;
        *a += x;
        *a += *b;
    }
}

// ============================================================
// 5) 安全版本的"正确替代"：把 `unsafe` 交给标准库
// ============================================================

/// ★ 想"把一个切片劈成两半" —— 这是手写 `unsafe` 的经典场景
/// （裸指针 + 偏移）。但标准库已经提供了**安全**的版本。
#[unsafe(no_mangle)]
pub fn split_and_sum(v: &mut [u64]) -> u64 {
    let mid = v.len() / 2;
    let (a, b) = v.split_at_mut(mid); // ← 安全，且不需要任何 unsafe
    a.iter()
        .chain(b.iter())
        .fold(0u64, |x, y| x.wrapping_add(*y))
}

/// ★ 想"按索引交换两个元素" —— 手写需要 `unsafe`
/// （两个 `&mut` 指向同一数组的不同位置，借用检查器看不出来）。
/// `split_at_mut` 把这件事**变成安全的**：它给出两个不重叠的 `&mut`。
///
/// ★ 注意 `swap_two` 的汇编里有 `panic_bounds_check` ——
/// 它**保留了检查**（因为 `i`、`j` 是外部输入）。
/// **这正是应该的样子**：安全代码在有风险的地方检查，在没风险的地方不检查。
#[unsafe(no_mangle)]
pub fn swap_two(v: &mut [u64], i: usize, j: usize) -> u64 {
    if i != j && i < v.len() && j < v.len() {
        let (lo, hi) = if i < j { (i, j) } else { (j, i) };
        let (left, rest) = v.split_at_mut(hi);
        std::mem::swap(&mut left[lo], &mut rest[0]);
    }
    v.first().copied().unwrap_or(0)
}
