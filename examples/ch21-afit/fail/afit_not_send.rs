// ⚠️ 故意编译不过：**`async fn` in trait 表达不了 `Send`**（本章的核心问题）
// 复现：rustc --edition 2024 --crate-type=lib examples/ch21-afit/fail/afit_not_send.rs
//
// 预期：
//   error: future cannot be sent between threads safely
//     = help: within `impl Future<Output = u64>`, the trait `Send` is not
//             implemented for `impl Future<Output = u64>`
//   note: the trait bound `impl Future<Output = u64>: Send` is not satisfied
//
// ★ 为什么？因为 `async fn` 的返回类型是**不透明的** ——
//   trait 只承诺"它是一个 `Future`"，**没有承诺它是 `Send`**。
//
// ★ 这条错误的实际后果：**`tokio::spawn` 直接撞墙**。
//   `tokio::spawn` 要求 `F: Send + 'static`（第 22 章），
//   而用 `async fn` 写的 trait 方法返回的 future **保证不了 `Send`**。
//
// ★ 这就是那条 lint 的由来：
//   ```text
//   warning: use of `async fn` in public traits is discouraged as auto trait
//            bounds cannot be specified
//     = note: `#[warn(async_fn_in_trait)]` on by default
//   ```
//   **"auto trait bounds cannot be specified"** —— 编译器直接把原因说了。
//
// ★ 解法（见 `src/lib.rs`）：用 RPITIT 手写 `-> impl Future<Output = u64> + Send`。
//   代价是**每个实现都得手写 `impl Future` 包装**，不能再简写 `async fn`。

#![allow(async_fn_in_trait)]

pub trait Store {
    async fn get(&self, k: u64) -> u64;
}

pub struct Mem;
impl Store for Mem {
    async fn get(&self, k: u64) -> u64 { k }
}

pub fn assert_send<F: Send>(_f: F) {}

/// 泛型调用点：要求返回的 future 是 `Send` —— trait 没承诺过，拒绝。
pub fn spawn_it<S: Store>(s: S) {
    assert_send(async move { s.get(1).await });   // ← 错误在这里
}

fn main() {}
