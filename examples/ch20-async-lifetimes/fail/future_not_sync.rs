// ⚠️ 故意编译不过：**future 是不是 `Sync`，取决于它捕获了什么**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch20-async-lifetimes/fail/future_not_sync.rs
//
// 预期：
//   error: future cannot be shared between threads safely
//     = help: within `impl Future<Output = u64>`, the trait `Sync` is not
//             implemented for `Cell<u64>`
//
// ★ 这条错误**推翻了**一个很常见的说法："future 天生不是 `Sync`，
//   因为 `poll` 要 `&mut self`。"
//
//   实测（rustc 1.98.1）：
//
//   | future | `Send` | `Sync` |
//   |---|---|---|
//   | `async fn plain() -> u64 { 1 }` | ✅ | **✅** |
//   | `async fn with_await()`（有 await，无捕获） | ✅ | **✅** |
//   | `async fn with_cell()`（捕获 `Cell<u64>`） | ✅ | ❌ |
//   | `async fn guard_across(&Mutex<u64>)` | ❌ | **✅** |
//
//   **`Send` 与 `Sync` 是状态机字段的两个独立性质**，
//   和普通类型完全一样（第 12 章：`Cell<u64>` 是 `Send` 但 `!Sync`）。
//   `Future` 不是"天生 `!Sync`"——它只是**经常**因为捕获了 `!Sync` 的东西而 `!Sync`。
//
//   （`poll` 要 `&mut self` 说的是"你不能同时 poll 两次"，
//     那是 `&mut` 的独占性，与 `Sync` 无关 ——
//     `Sync` 说的是"`&Self` 能不能跨线程"。）
//
// ★ 这条也顺带说明：**本章的两条主线（生命周期 / Send）都来自同一个东西** ——
//   状态机里到底装了哪些字段。

use std::cell::Cell;

pub async fn with_cell() -> u64 {
    let c = Cell::new(1u64);   // ← Cell<u64>: Send + !Sync
    std::future::ready(()).await;
    c.get()
}

pub fn assert_sync<F: Sync>(_f: F) {}

pub fn check() {
    assert_sync(with_cell());   // ← 错误报在这里
}

fn main() {}
