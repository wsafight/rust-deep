// ⚠️ 故意编译不过：**锁守卫跨了 `await`，任务就不是 `Send` 了**
// 复现：cargo build -p ch22-tokio --features fail-guard  （或见本文件头部的 rustc 命令）
//
// 预期：
//   error: future cannot be sent between threads safely
//     = help: within `impl Future<Output = ()>`, the trait `Send` is not
//             implemented for `std::sync::MutexGuard<'_, u64>`
//   note: future is not `Send` as this value is used across an await
//
// ★ 这是第 20 章的结论在 tokio 上的形态 —— 措辞完全一样。
//   区别只是"谁在要求 Send"：
//
//   - 第 20 章：我们手写的 `assert_send`；
//   - 这里：**`tokio::spawn`**。
//
// ★ tokio 的报错**更啰嗦但更有用**：它会把 `tokio::spawn` 的 bound
//   (`F: Future + Send + 'static`) 也标出来，让你知道**要求来自哪里**。
//
// ★ 修法（见 `src/lib.rs` 的 `shared_counter`）：
//   把锁的作用域收进一个块，让 guard 在 `await` 之前 drop。
//
//   ⚠️ **`drop(g)` 没有用**（第 20 章实测）—— 编译器数的是**变量**的存活区间。

use std::sync::{Arc, Mutex};

pub async fn bad(c: Arc<Mutex<u64>>) {
    let g = c.lock().unwrap();
    tokio::task::yield_now().await;   // ← g 活过了这个 await
    let _ = *g;
}

pub fn spawn_it(c: Arc<Mutex<u64>>) {
    tokio::spawn(bad(c));             // ← 错误在这里报出
}

fn main() {}
