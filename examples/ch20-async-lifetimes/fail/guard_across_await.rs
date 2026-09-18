// ⚠️ 故意编译不过：**锁守卫活过了 `await`，于是 future 变成 `!Send`**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch20-async-lifetimes/fail/guard_across_await.rs
//
// 预期：
//   error: future cannot be sent between threads safely
//     = help: within `impl Future<Output = u64>`, the trait `Send` is not
//             implemented for `std::sync::MutexGuard<'_, u64>`
//   note: future is not `Send` as this value is used across an await
//     |
//   5 |     let g = m.lock().unwrap();
//     |         - has type `std::sync::MutexGuard<'_, u64>` which is not `Send`
//   6 |     std::future::ready(()).await;
//     |                            ^^^^^ await occurs here, with `g` maybe used later
//
// ★ 这是异步里最常见的一类 `!Send`。注意编译器**把三个点都标出来了**：
//   ① `g` 的类型；② `await` 的位置；③ 两者的因果关系（"maybe used later"）。
//
// ★ 但真正的教训在 `drop_does_not_help.rs`：**"看起来用完了"不算数。**

use std::sync::Mutex;

pub async fn holds_guard_bad(m: &Mutex<u64>) -> u64 {
    let g = m.lock().unwrap();
    std::future::ready(()).await;   // ← g 活过了这个 await
    *g
}

pub fn assert_send<F: Send>(_f: F) {}

pub fn check() {
    assert_send(holds_guard_bad(&Mutex::new(1)));   // ← 错误在这里报出
}

fn main() {}
