//! 第 4 章：自引用结构与 Pin 的前置知识
//!
//! 证据生成：tools/evidence.sh ch04-pin
//!
//! 本章要回答：**为什么需要 `Pin`？**
//! 因为 Rust 的移动允许值被**重新放到另一个地址**；
//! 而自引用结构里存着"自己的地址"—— 一移就悬垂。
//!
//! ★ 核心证据：`Pin<&mut T>` 与 `&mut T` 生成**完全相同**的代码（LLVM 折叠成 alias）。
//!   `Pin` 是**纯类型层的**约束，零运行时成本。

use std::marker::PhantomPinned;
use std::pin::Pin;

/// ★ 证据 1：`Pin<&mut T>` 是零成本的
#[unsafe(no_mangle)]
pub fn plain(x: &mut u64) -> u64 {
    *x
}

#[unsafe(no_mangle)]
pub fn pinned(p: Pin<&mut u64>) -> u64 {
    *p
}

/// ★ 证据 2：移动允许重定位（本样本在 MIR 里是一条 `move`）
pub struct Big {
    pub a: [u64; 4],
}

#[unsafe(no_mangle)]
pub fn mov_it(b: Big) -> Big {
    b
}

/// ★ 证据 3：`Unpin` 是 auto trait —— 普通类型自动实现
///
/// `Pin::new` 要求 `T: Unpin`，所以下面这个能编译；
/// 去掉 `Unpin`（见 `fail/pin_requires_unpin.rs`）就编译不过。
#[unsafe(no_mangle)]
pub fn pin_a_u64(x: &mut u64) -> Pin<&mut u64> {
    Pin::new(x)
}

/// ★ 证据 4：`!Unpin` 的类型 —— `PhantomPinned` 是零大小的"类型层开关"
///
/// 这个结构体**一个字节都没多**（`PhantomPinned` 是 ZST），
/// 但它不再是 `Unpin` 了 —— 于是 **`Pin::new` 拒绝编译**：
/// ```text
/// error[E0277]: `PhantomPinned` cannot be unpinned
///    = note: consider using the `pin!` macro
///            consider using `Box::pin` if you need to access the pinned value
/// ```
/// 见 `fail/pin_requires_unpin.rs`。
///
/// 要钉住一个 `!Unpin` 的值，必须让它**先落到一个稳定的地址上**：
/// `Box::pin` 或 `pin!`。这正是"堆分配换地址稳定"的由来。
pub struct Pinned {
    pub data: u64,
    _pin: PhantomPinned, // 零大小，但让类型变成 !Unpin
}

impl Pinned {
    pub fn new(data: u64) -> Self {
        Self {
            data,
            _pin: PhantomPinned,
        }
    }
}

/// 用 `Box::pin` 钉住 —— 堆上的地址不会因为栈帧移动而改变
#[unsafe(no_mangle)]
pub fn box_pin(data: u64) -> Pin<Box<Pinned>> {
    Box::pin(Pinned::new(data))
}

/// ★ 证据 5：`Pin` 真的挡得住"移动"—— 只能拿到 `&T`，拿不到 `&mut T`
#[unsafe(no_mangle)]
pub fn read_pinned(p: &Pin<Box<Pinned>>) -> u64 {
    p.data
}

/// 要拿 `&mut T`，必须 `unsafe` 并自己承诺"不会移动它"
#[unsafe(no_mangle)]
pub fn write_pinned(p: &mut Pin<Box<Pinned>>, v: u64) {
    // SAFETY: 我们只改 data，不移动整个 Pinned，也不碰 _pin
    unsafe { p.as_mut().get_unchecked_mut() }.data = v;
}
