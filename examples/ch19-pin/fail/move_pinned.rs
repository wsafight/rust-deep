// ⚠️ 故意编译不过：**`Pin` 用"借用的可变性"挡住了移动**（两种方式）
// 复现：rustc --edition 2024 --crate-type=lib examples/ch19-pin/fail/move_pinned.rs
//
// 预期：
//   error[E0596]: cannot borrow data in dereference of `Pin<Box<NotUnpin>>` as mutable
//     = help: trait `DerefMut` is required to modify through a dereference,
//             but it is not implemented for `Pin<Box<NotUnpin>>`
//   error[E0507]: cannot move out of dereference of `Pin<Box<NotUnpin>>`
//
// ★ 这是 `Pin` 挡住"移动"的**两种具体方式**：
//
//   ① `&mut *p` 失败（E0596）——
//      `Pin<P>` 的 `DerefMut` 只在 `P::Target: Unpin` 时可用。
//      对 `!Unpin` 的类型，你**根本拿不到 `&mut T`**。
//
//   ② `*p` 失败（E0507）——
//      想**移出**被 pin 的值？也不行。
//
//   移动一个值只有两条路：**拿到所有权**，或者**拿到 `&mut`**。
//   `Pin` 把这两条路**同时堵死**。
//
// ★ 想拿 `&mut T` 只有一条路：`unsafe { pin.get_unchecked_mut() }`，
//   并且你必须自己承诺"不移动它"（见 src/lib.rs 的 `write_pinned`）。
//
// ★ 注意错误的措辞：编译器说的是"cannot borrow ... as mutable"，
//   而不是"这个类型不能移动"。**`Pin` 设计的精妙之处就在于：
//   它用"借用的可变性"来表达"能不能移动"。**

use std::marker::PhantomPinned;
use std::pin::Pin;

pub struct NotUnpin {
    pub a: u64,
    _p: PhantomPinned,
}

/// ① 拿 `&mut T` —— E0596
pub fn try_get_mut(p: &mut Pin<Box<NotUnpin>>) {
    let inner: &mut NotUnpin = &mut *p;
    inner.a = 1;
}

/// ② 移出内容 —— E0507
pub fn try_move_out(p: Pin<Box<NotUnpin>>) {
    let inner: NotUnpin = *p;
    let _ = inner.a;
}

fn main() {}
