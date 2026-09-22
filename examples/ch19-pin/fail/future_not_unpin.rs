// ⚠️ 故意编译不过：**当前 async fn 产生的 future 不自动实现 `Unpin`**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch19-pin/fail/future_not_unpin.rs
//
// 预期：
//   error[E0277]: `{async fn body of zero_await()}` cannot be unpinned
//     = note: consider using the `pin!` macro
//             consider using `Box::pin` if you need to access the pinned value
//                       outside of the current scope
//
// ★ 这个例子的重点是"**保守**"：
//
//   `zero_await` 的函数体里**一个借用都没有**、**一次 `await` 都没有** ——
//   它根本不可能是自引用的。但编译器照样不给它 `Unpin`。
//
//   这是当前编译器对匿名 async 状态机采取的保守语义：它们不会自动实现
//   `Unpin`。这个反例只证明可观察的 API 行为，不推断内部 pass 的时序原因。
//
//   代价就是你到处要写 `Box::pin` / `pin!`；
//   收益是"自引用 future 一定是安全的"这件事**无需任何额外规则**。
//
// ★ 对照：`std::future::ready(5)` 是 `Unpin` 的（它不是编译器生成的状态机），
//   所以 `Pin::new(&mut ready)` 可以编译。见 src/lib.rs 的 `pin_a_u64`。

use std::pin::Pin;

/// 一个再普通不过的 `async fn` —— 没有借用，没有 await。
pub async fn zero_await(a: u64) -> u64 { a }

/// 把它交给一个要求 `Unpin` 的函数 —— 拒绝。
pub fn try_pin_future() {
    let mut f = zero_await(1);
    let _p: Pin<&mut _> = Pin::new(&mut f);   // ← E0277
}

fn main() {}
