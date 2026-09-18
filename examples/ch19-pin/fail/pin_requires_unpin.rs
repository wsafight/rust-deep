// ⚠️ 故意编译不过：**`Pin::new` 要求 `T: Unpin`**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch19-pin/fail/pin_requires_unpin.rs
//
// 预期：
//   error[E0277]: `PhantomPinned` cannot be unpinned
//     = note: consider using the `pin!` macro
//             consider using `Box::pin` if you need to access the pinned value
//
// ★ 这条错误的**理由**值得读清楚：编译器说的不是"`NotUnpin` 不是 Unpin"，
//   而是 "`PhantomPinned` cannot be unpinned" —— 它指的是**那个字段**。
//   这与第 12 章 `PhantomData<*const u64>` 让类型 !Send 的措辞是同一类：
//   **编译器会指出"谁该负责"。**
//
// ★ 为什么 `Pin::new` 要这个约束？
//   因为 `Pin::new(x: &mut T)` 会**把 x 移动到别的地方**（它是按值传的引用，
//   而 `Pin<&mut T>` 只承诺"被指的东西不动"）。
//   如果 `T: Unpin`（移动了也没事），这个承诺没有代价；
//   如果 `T: !Unpin`，那么"能不能保证不动"就成问题了 —— 编译器拒绝。
//
//   要钉住 `!Unpin` 的值，必须让它**先落到一个稳定的地址上**：
//   `Box::pin`（堆）或 `pin!`（栈上的隐藏变量）。
//   这就是"堆分配换地址稳定"的由来。

use std::marker::PhantomPinned;
use std::pin::Pin;

pub struct NotUnpin {
    pub a: u64,
    _p: PhantomPinned,
}

pub fn try_pin(x: &mut NotUnpin) -> Pin<&mut NotUnpin> {
    Pin::new(x)          // ← E0277
}

fn main() {}
