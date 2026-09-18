// ⚠️ 故意编译不过：**不能借用局部变量进 `spawn`**（`'static` 约束）
// 复现：rustc --edition 2024 --crate-type=lib --extern tokio=... examples/ch22-tokio/fail/borrow_local.rs
//
// 预期：
//   error[E0373]: async block may outlive the current function, but it borrows `v`,
//                 which is owned by the current function
//     = help: use `move` to force the async block to take ownership
//
// ★ 这是 `Send + 'static` 里 **`'static`** 那一半的形态。
//   为什么需要 `'static`？因为任务**可能比调用者活得久** ——
//   `spawn` 是 fire-and-forget，它不保证任务在函数返回前跑完。
//
// ★ 对照 `std::thread::spawn`：同样的约束、同样的理由（第 12 章）。
//   tokio 没有引入新概念，它只是把"线程"换成了"任务"。
//
// ★ 修法（见 `src/lib.rs`）：
//   - `async move { v.len() }` —— **把所有权搬进去**；
//   - 或者 `Arc<[T]>` —— 共享所有权，代价是一次原子计数。

pub fn spawn_it() {
    let v = vec![1u64, 2];
    tokio::spawn(async {
        let _ = v.len();              // ← 借用 v，不是 'static
    });
}

fn main() {}
