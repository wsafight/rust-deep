// ⚠️ 故意编译不过：**`!Send` 会沿着 `.await` 传染到外层**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch20-async-lifetimes/fail/send_contagion.rs
//
// 预期：
//   error: future cannot be sent between threads safely
//     = help: within `impl Future<Output = u64>`, the trait `Send` is not
//             implemented for `std::sync::MutexGuard<'_, u64>`
//   note: future is not `Send` as this value is used across an await
//     --> 指向 **inner** 里的那一行
//
// ★ 关键在**错误信息指向谁**：`outer` 自己一行锁都没写，
//   但编译器指出的是 `inner` 里的 `MutexGuard`。
//   这说明 `Send` 是**沿 `.await` 一路推导上去的** ——
//   `outer` 的 future 里含着一个"半执行的 inner future"，
//   而那个 inner future 里含着一个 `MutexGuard`。
//
// ★ 实践含义：**一个库里有一个 `!Send` 的 async fn，
//   所有 `.await` 它的地方都会变成 `!Send`** ——
//   而且报错位置离真正的原因很远。这是异步代码里最难查的一类问题。

use std::sync::Mutex;

pub async fn inner(m: &Mutex<u64>) -> u64 {
    let g = m.lock().unwrap();
    std::future::ready(()).await;
    *g
}

pub async fn outer(m: &Mutex<u64>) -> u64 { inner(m).await }

pub fn assert_send<F: Send>(_f: F) {}

pub fn check() {
    assert_send(outer(&Mutex::new(1)));   // ← 错误报在这里，原因在 inner 里
}

fn main() {}
