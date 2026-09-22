//! 第 19 章：`Pin` 与 `Unpin` 为什么存在
//!
//! 证据生成：tools/evidence.sh ch19-pin
//!
//! 第 18 章说：`poll` 的签名是 `Pin<&mut Self>`。本章回答**为什么**。
//!
//! 核心论证链（本章要逐条钉死）：
//!
//! 1. `async fn` 的状态机**可能是自引用的** ——
//!    一个跨 `await` 的局部变量可能借用另一个跨 `await` 的局部变量；
//! 2. 而 Rust 的移动**允许重定位**（第 4 章），地址可能改变；
//! 3. 自引用结构一移动，内部的引用就**悬垂**；
//! 4. 所以必须有一种方式表达"**这个值一旦定下来就不能再移动**"；
//! 5. `Pin<P>` 就是这个表达 —— 而且它**零成本**（纯类型层）。
//!
//! ★ 关键区分（本章最容易搞混的一点）：
//!
//! | 概念 | 是什么 | 谁决定 |
//! |---|---|---|
//! | `Pin<P>` | 一个**包装类型**，表达"被指的东西不能动" | 由 API 作者使用 |
//! | `Unpin` | 一个 **auto trait**，表达"这个类型移动了也没事" | 由编译器自动推导 |

use std::marker::PhantomPinned;
use std::pin::Pin;

// ============================================================
// 1) 自引用结构：为什么它天然困难
// ============================================================

/// ★ 一个**手写的自引用结构**。
///
/// 它内部存着一个指向**自己**的指针（`self_ref` 指向 `data`）。
/// 这在 `unsafe` 下可以构造出来，但**一旦移动就悬垂**。
///
/// 这个类型在标准库里的真实版本就是 `async fn` 的状态机：
/// 一个跨 `await` 的变量可能引用另一个跨 `await` 的变量。
pub struct SelfRef {
    pub data: u64,
    /// 指向 `self.data` 的裸指针 —— 只有钉住了才安全。
    ///
    /// 用 `*mut` 而不是 `*const` 不是巧合：`*const` 对应的
    /// `addr_of!` 会派生 SharedReadOnly 权限，写一次就失效（见 `new`）。
    self_ref: *mut u64,
    _pin: PhantomPinned,        // ← 让 SelfRef 变成 !Unpin
}

impl SelfRef {
    /// 在堆上构造并钉住，然后才建立自引用。
    ///
    /// ★ 构造顺序不能反：先取得稳定地址，再把该地址写进 `self_ref`。
    /// 因此这个安全构造函数直接返回 `Pin<Box<Self>>`，不允许把一个
    /// 指向旧对象的裸指针装进另一个按值返回的新对象。
    ///
    /// ★ 为什么用 `addr_of_mut!(this.data)` 而不是 `&this.data as *const u64`？
    ///
    /// 因为**指针的"出处"决定了它的权限**（Stacked Borrows）。
    /// `addr_of!(place)` 派生出的是一把 **SharedReadOnly** 的钥匙 ——
    /// 之后任何一次对 `data` 的写入都会把它**弹掉**，再读就是 UB。
    /// `addr_of_mut!` 派生的是 **Unique** 权限，写入不会作废它。
    ///
    /// ⚠️ 这条差异**在汇编里完全看不出来**（两者生成同样的指令），
    /// 汇编无法分辨；Miri 可以按当前别名模型检查这条执行路径。这正是
    /// "`unsafe` 的契约不写在机器码里"的典型例子：
    /// 两个逐字节等价的写法，在本章的后续写入序列中一个仍有效、一个已失效。
    ///
    /// 实测（`cargo +nightly miri test -p ch19-pin`）：
    /// - `addr_of!`     → `Undefined Behavior: ... that tag does not exist
    ///                     in the borrow stack`
    /// - `addr_of_mut!` → 通过
    pub fn new(data: u64) -> Pin<Box<Self>> {
        let mut pinned = Box::pin(SelfRef {
            data,
            self_ref: std::ptr::null_mut(),
            _pin: PhantomPinned,
        });
        // SAFETY: `pinned` 已经拥有稳定的堆地址；这里只初始化 self_ref，
        // 不会移动 SelfRef。该指针在 pinned 被销毁前始终指向它自己的 data。
        let this = unsafe { pinned.as_mut().get_unchecked_mut() };
        this.self_ref = std::ptr::addr_of_mut!(this.data);
        pinned
    }

    /// 通过自引用指针读 —— 只有在没移动过的前提下才正确。
    ///
    /// ★ 注意这里的裸指针**从哪里来**（见 `make_self_ref`）：
    /// 它来自 `Pin::get_unchecked_mut` 拿到的那个 `&mut SelfRef`。
    /// 在当前 Miri Stacked Borrows 模型下，它是一把从 `&mut` 派生出来、
    /// 与后续访问相容的钥匙。
    ///
    /// 反例：如果裸指针是从 `&self.data`（共享借用）派生的，
    /// 那么一旦有人再写 `data`，那把共享的钥匙就被**弹掉**了，
    /// 之后用它读就是 UB。Miri 会明确报出这一点。
    pub fn read_via_self_ref(&self) -> u64 {
        // SAFETY: 由构造时的约定保证 self_ref 仍然指向 self.data；
        // 且它派生自构造时那个 `&mut SelfRef`，在值被钉住期间一直有效。
        unsafe { *self.self_ref }
    }
}

/// ★ 用 `Box::pin` 钉住一个自引用结构。
///
/// `Box::pin` 做的事：**在堆上分配**，然后构造自引用 ——
/// 堆上的地址不会因为栈帧移动而改变。
///
/// ★ 注意这里**没有 `unsafe` 块**（除了 `get_unchecked_mut` 那一处）：
/// 这段代码在 O3 下会塌缩成极短的几条指令 ——
/// 分配一块 16 字节、把 `data` 和"指向自己的地址"一起写进去：
///
/// ```asm
/// _make_self_ref:
/// 	mov	x19, x0
/// 	mov	w0, #16              ; 16 字节 = u64 + 指针
/// 	mov	w1, #8               ; align 8
/// 	bl	___rust_alloc_zeroed
/// 	cbz	x0, LBB1_2           ; 分配失败 → 走 panic
/// 	stp	x19, x0, [x0]        ; ★ 一次写入：data 和自引用指针
/// 	ret
/// ```
///
/// `stp x19, x0, [x0]` 这一条就是"自引用"的全部运行时形态：
/// 把**分配到的地址 `x0` 本身**写进 `[x0, #8]`。
///
/// ⚠️ 符号名里的 `hash`（如 `CsrtIYgyWToU`）**每次编译都会变**，
/// 引用汇编时不要把它当常量。
#[unsafe(no_mangle)]
pub fn make_self_ref(data: u64) -> Pin<Box<SelfRef>> {
    SelfRef::new(data)
}

#[unsafe(no_mangle)]
pub fn read_self_ref(p: &Pin<Box<SelfRef>>) -> u64 {
    p.read_via_self_ref()
}

// ============================================================
// 2) `Pin` 是零成本的（第 4 章的结论，这里再钉一次）
// ============================================================

#[unsafe(no_mangle)]
pub fn plain(x: &mut u64) -> u64 { *x }

#[unsafe(no_mangle)]
pub fn pinned(p: Pin<&mut u64>) -> u64 { *p }

// ============================================================
// 3) `Unpin` 是 auto trait
// ============================================================

/// `u64: Unpin`（自动推导）→ `Pin::new` 可用。
#[unsafe(no_mangle)]
pub fn pin_a_u64(x: &mut u64) -> Pin<&mut u64> { Pin::new(x) }

/// 一个**普通**结构体：自动是 `Unpin`（因为所有字段都是 `Unpin`）。
pub struct Plain { pub a: u64 }

#[unsafe(no_mangle)]
pub fn pin_a_plain(x: &mut Plain) -> Pin<&mut Plain> { Pin::new(x) }

/// ★ 一个 `!Unpin` 的类型：只因为多了一个 `PhantomPinned` 字段。
///
/// `PhantomPinned` 是**零大小**的 —— `NotUnpin` 的布局和 `Plain` 完全一样。
/// 但它让整个类型不再是 `Unpin`，于是 `Pin::new` 拒绝编译。
pub struct NotUnpin { pub a: u64, _p: PhantomPinned }

#[unsafe(no_mangle)]
pub fn box_pin_not_unpin(a: u64) -> Pin<Box<NotUnpin>> {
    Box::pin(NotUnpin { a, _p: PhantomPinned })
}

// ============================================================
// 4) `Pin` 挡住了什么：只能拿 `&T`，拿不到 `&mut T`
// ============================================================

/// ★ `Pin<&mut T>` **不提供** `&mut T`。
///
/// 这个函数能编译：它只要 `&T`（`Deref`）。
#[unsafe(no_mangle)]
pub fn read_pinned(p: &Pin<Box<NotUnpin>>) -> u64 { p.a }

/// 要拿 `&mut T`，必须 `unsafe` + 自己承诺"不会移动它"。
///
/// ★ 这个 `unsafe` 的**义务**是：**不移动 `*p`**。
/// 改字段是允许的（只要那个字段不是"被自引用指着的那个"）。
#[unsafe(no_mangle)]
pub fn write_pinned(p: &mut Pin<Box<NotUnpin>>, v: u64) {
    // SAFETY: 我们只改 a，不移动整个 NotUnpin，也不碰 _p
    unsafe { p.as_mut().get_unchecked_mut() }.a = v;
}

// ============================================================
// 5) `pin!` 宏：在栈上钉住（不需要堆分配）
// ============================================================

/// ★ `std::pin::pin!` 可以在**栈上**钉住一个值 ——
/// 代价是"这个值活不过当前作用域"（它被一个隐藏的局部变量拥有）。
///
/// 对比 `Box::pin`：
///
/// | | `Box::pin` | `pin!` |
/// |---|---|---|
/// | 分配 | 堆 | **栈** |
/// | 生命周期 | 任意（跟 `Box` 走） | 当前作用域 |
/// | 能否返回 | ✅ | ❌ |
#[unsafe(no_mangle)]
pub fn pin_on_stack(data: u64) -> u64 {
    let pinned = std::pin::pin!(NotUnpin { a: data, _p: PhantomPinned });
    // `pinned` 是 `Pin<&mut NotUnpin>` —— 借用的是隐藏的局部变量
    pinned.a
}

// ============================================================
// 6) ★ 链接第 18 章：`async fn` 的状态机是 `!Unpin` 的
// ============================================================

/// 一个**真的会**借用另一个跨 `await` 变量的 `async fn`。
///
/// `r` 借用 `x`，而 `r` 的存活跨过了 `.await` ——
/// 这就是自引用：状态机里一个字段（`&String`）指向另一个字段（`String`）。
///
/// MIR 里的 `coroutine layout` 会把它摆出来：
///
/// ```text
/// coroutine layout {
///     field _s0: String;          // ← 被借用的那个
///     field _s1: &String;         // ← 借用的那个（自引用！）
///     field _s2: std::future::Ready<()>;
///     variant_fields = { ..., Suspend0 (3): [_s0, _s1, _s2] }
/// }
/// ```
///
/// ★ 这是第 18 章"状态机"与本章"`Pin`"之间的**接缝**：
/// 第 18 章证明了 `await` 会被编译成状态机；
/// 本章说明了**为什么那个状态机必须 `!Unpin`** —— 因为它可能长这样。
pub async fn borrow_across_await() -> u64 {
    let x = String::from("hi");
    let r = &x;
    std::future::ready(()).await;   // ← r 跨过这个 await 还活着
    r.len() as u64
}

/// ★ 实测结论（1.98.1，见 `fail/future_not_unpin.rs`）：
///
/// **所有** `async fn` / `async` 块产生的 future 都是 `!Unpin` ——
/// 哪怕它的函数体里**一个借用都没有**、**一次 `await` 都没有**。
///
/// ```text
/// error[E0277]: `{async fn body of zero_await()}` cannot be unpinned
///   = note: consider using the `pin!` macro
///           consider using `Box::pin` if you need to access the pinned value
/// ```
///
/// 这是当前编译器对匿名 async 状态机采取的保守语义。不要从这里进一步推导
/// 编译器内部 pass 的时序原因；稳定、可依赖的是这些 future 不自动实现 `Unpin`，
/// 需要时由调用者用 `Box::pin` / `pin!` 钉住。
///
/// 这个"宁可保守"的代价，就是你在异步代码里到处见到 `Box::pin` 的原因。
#[unsafe(no_mangle)]
pub async fn zero_await(a: u64) -> u64 { a }
