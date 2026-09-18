// ⚠️ 故意编译不过：`Pin::new` 要求 `T: Unpin`
// 复现：rustc --edition 2024 --crate-type=lib examples/ch04-pin/fail/pin_requires_unpin.rs
//
// 预期：
//   error[E0277]: `PhantomPinned` cannot be unpinned
//    = note: consider using the `pin!` macro
//            consider using `Box::pin` if you need to access the pinned value
//            outside of the current scope
//
// ★ 这一条错误信息就是 `Pin` 的全部意义：
//   "这个类型不能保证移动后还有效，所以我不让你从 &mut T 造 Pin<&mut T>"。
//   要钉住它，必须先给它一个**稳定地址**（`Box::pin` / `pin!`）。

use std::marker::PhantomPinned;
use std::pin::Pin;

pub struct Pinned {
    pub data: u64,
    _pin: PhantomPinned,
}

pub fn pin_it(x: &mut Pinned) -> Pin<&mut Pinned> {
    Pin::new(x)          // ← E0277
}

fn main() {}
