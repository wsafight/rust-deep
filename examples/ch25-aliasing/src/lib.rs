//! 第 25 章：别名规则、`UnsafeCell` 与 `PhantomData`
//!
//! 证据生成：tools/evidence.sh ch25-aliasing
//!
//! ★ 本章的**主证据来自 Miri**（`scripts/verify-miri.sh`），不是 MIR 打印。
//! 原因见本文件下方「为什么 MIR 撑不起本章的证据」。
//!
//! 核心：**别名规则不是"两条指针不能指同一处"，而是"每块内存有一个权限栈"**
//! （Stacked Borrows）。规则管的是**指针的出处**，不是"有没有别名"。
//!
//! 三个可携带的直觉：
//!
//! 1. **`&T` 是只读的** —— 通过任何途径写它，都会把从它派生的指针作废；
//! 2. **`&mut T` 是独占的** —— 从它派生的指针一直有效，直到 `&mut` 本身被用；
//! 3. **`UnsafeCell<T>` 是唯一合法的"通过共享引用修改"的容器** ——
//!    它把"只读"这条性质从 `&UnsafeCell<T>` 上**摘掉**了。

use std::cell::UnsafeCell;
use std::marker::PhantomData;

// ============================================================
// 1) UnsafeCell：唯一合法的"通过共享引用修改"
// ============================================================

/// ★ 一个用 `UnsafeCell` 实现的最小 `Cell<T>`。
///
/// `UnsafeCell` 是标准库里**唯一**一个"声明式地关掉只读性质"的类型。
/// 它的全部作用就是让 `&UnsafeCell<T>` **不再**是一个只读引用 ——
/// 于是通过它写内存是合法的。
///
/// 对比 `fail/write_through_shared_ref.rs`：不用 `UnsafeCell`，
/// 直接把 `&T` 转成 `*mut T` 去写 —— **Miri 直接拒绝编译**：
///
/// ```text
/// error: assigning to `&T` is undefined behavior, consider using an `UnsafeCell`
/// ```
///
/// ⚠️ 这条错误**不是借用检查器报的**，是 **Miri** 报的 ——
/// 而且是在**编译期**（Miri 会做这个特定的静态检查）。
/// 普通 `cargo build` 对这段代码**完全无感**。
pub struct Cell2<T> {
    inner: UnsafeCell<T>,
}

impl<T: Copy> Cell2<T> {
    pub fn new(v: T) -> Self { Self { inner: UnsafeCell::new(v) } }

    pub fn get(&self) -> T {
        // SAFETY: UnsafeCell 保证通过共享引用修改是允许的；
        // 单线程使用，不存在数据竞争。
        unsafe { *self.inner.get() }
    }

    pub fn set(&self, v: T) {
        // SAFETY: 同上
        unsafe { *self.inner.get() = v }
    }
}

/// ★ 在**共享引用**上做的写入 —— Miri 判定合法。
///
/// 关键不是"用了裸指针"，而是**裸指针从 `UnsafeCell` 派生**。
#[unsafe(no_mangle)]
pub fn cell2_roundtrip(v: u64) -> u64 {
    let c = Cell2::new(v);
    c.set(v + 1);
    c.get()
}

/// 对照：`&mut T` 上写 —— 当然是合法的（`&mut` 本来就允许写）。
#[unsafe(no_mangle)]
pub fn mut_roundtrip(v: u64) -> u64 {
    let mut x = v;
    let p: &mut u64 = &mut x;
    *p += 1;
    x
}

// ============================================================
// 2) PhantomData：让裸指针"携带"它该有的权限
// ============================================================

/// ★ `PhantomData<&'a T>`：这个结构体里的裸指针**在语义上是 `&'a T`**。
///
/// `PhantomData` 是零大小的，但它让类型**参与**三件事：
///
/// 1. **生命周期检查**（这个指针借用了 `'a`，`'a` 内不能动那个值）；
/// 2. **auto trait 推导**（第 12 章：`PhantomData<*const T>` 让类型 `!Send`）；
/// 3. **variance**（第 3 章：`PhantomData<&'a T>` 是协变的）。
///
/// ★ 第 3 条在本章最重要：它让 Miri 知道这个指针**是只读的**
/// —— 于是"先从这里读、再从别处写、再回来读"会被判 UB。
pub struct SharedReadOnly<'a, T> {
    ptr: *const T,
    _p: PhantomData<&'a T>,     // ← 零大小，但携带"这是 &'a T"这个事实
}

impl<'a, T: Copy> SharedReadOnly<'a, T> {
    pub fn new(r: &'a T) -> Self { Self { ptr: r, _p: PhantomData } }

    pub fn get(&self) -> T {
        // SAFETY: ptr 来自 &'a T，且在 'a 内有效
        unsafe { *self.ptr }
    }
}

/// 对照：`PhantomData<&'a UnsafeCell<T>>` —— 这个指针**允许被写**。
///
/// 两个结构体的字段布局**完全一样**（都是 8 字节指针 + ZST），
/// 区别只在 `PhantomData` 的**类型参数**。但 Miri 对它们的判定不同：
/// `SharedReadOnly` 派生的指针会被写入作废，`SharedReadWrite` 的不会。
///
/// ★ 这就是 `PhantomData` 的用途：**把"这个裸指针的权限"写进类型里**，
/// 让编译器（和 Miri）知道该按哪套规则检查。
pub struct SharedReadWrite<'a, T> {
    ptr: *mut T,
    _p: PhantomData<&'a UnsafeCell<T>>,
}

impl<'a, T: Copy> SharedReadWrite<'a, T> {
    pub fn new(r: &'a UnsafeCell<T>) -> Self {
        Self { ptr: r.get(), _p: PhantomData }
    }

    pub fn get(&self) -> T {
        // SAFETY: ptr 来自 UnsafeCell，通过共享引用读写都是允许的
        unsafe { *self.ptr }
    }

    pub fn set(&self, v: T) {
        // SAFETY: 同上
        unsafe { *self.ptr = v }
    }
}

/// ★★ 本章最微妙的一条：**`UnsafeCell` 不会"救活"一个已经存在的 `&T`**。
///
/// 直觉上会以为："既然是 `UnsafeCell`，那通过共享引用写就是允许的，
/// 所以从 `&T` 派生的指针应该一直有效。"
///
/// **实测（Miri）不是这样。** 顺序很关键：
///
/// | 顺序 | 结果 |
/// |---|---|
/// | 先建 `&T`，再通过 `UnsafeCell` 写，再用 `&T` | ❌ **UB** |
/// | 先通过 `UnsafeCell` 写，再建 `&T` | ✅ 合法 |
/// | 全程只用 `UnsafeCell` 指针，从不建 `&T` | ✅ 合法 |
///
/// 也就是说：`UnsafeCell` 让**那次写入本身**是合法的
/// （不会像 `fail/write_through_shared_ref.rs` 那样直接报错），
/// 但这次写入**仍然会弹掉在此之前建立的 `&T`**。
///
/// **实践规则**：**不要在写入之前建立 `&T` 并持有到写入之后。**
/// 需要共享读就**在最后一次写之后**再建引用。
///
/// 这个函数演示**合法**的顺序（先写、后建引用）。
#[unsafe(no_mangle)]
pub fn write_then_read(v: u64) -> u64 {
    let c = UnsafeCell::new(v);
    let w = SharedReadWrite::new(&c);
    w.set(v + 1);                       // ← 先写
    let r: &u64 = unsafe { &*c.get() }; // ← 后建 &T
    *r
}

// ============================================================
// 3) MIR 里能看到的：`no_retag`
// ============================================================

/// ★ `--emit=mir` 里出现的 `no_retag` **不是** retag 的证据，恰恰相反 ——
/// 它是"这里**不**做 retag"的标记。
///
/// 机制（读 rustc 1.98 源码确认）：
/// - `Rvalue::Use` 带一个 `WithRetag::Yes/No` 标志
///   （`rustc_middle/src/mir/syntax.rs`）；
/// - pretty printer **只在 `No` 时**打印 `no_retag`
///   （`rustc_middle/src/mir/pretty.rs`，源码注释原文：
///   *"With retag is more common so we only print when it's without."*）。
///
/// 所以：**做了 retag 的地方是静默的**。stable 上 `no_retag` 只有两个来源 ——
/// `EraseDerefTemps`（把 `CopyForDeref` 改写成 `Use(.., WithRetag::No)`，
/// 源码注释：*"We do **NOT** want a retag here!"*）和 `DerefSeparator`。
///
/// ★ 更关键：**`-Zdump-mir`（逐 pass dump）里也看不到 retag** ——
/// 1.98 的 pass 列表里根本没有 `AddRetag`；retag 是 **codegen 阶段**的事。
///
/// 这个函数的作用就是**让读者亲眼看到 `no_retag` 长什么样、出现在哪**，
/// 然后据此理解"为什么不能拿它当 retag 的证据"。
pub fn through_tuple(a: &&u64) -> u64 {
    let p = (a,);
    let q = std::hint::black_box(p);
    **q.0
}

/// 顶层 `&mut` 参数：**任何 MIR pass 里都不出现 retag**。
pub fn reborrow(v: &mut Vec<u64>) -> u64 {
    v.push(1);
    v.len() as u64
}

pub fn sum(v: &[u64]) -> u64 {
    v.iter().fold(0u64, |a, b| a.wrapping_add(*b))
}
