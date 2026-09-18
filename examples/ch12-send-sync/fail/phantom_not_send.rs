// ⚠️ 故意编译不过：**PhantomData 决定 auto trait**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch12-send-sync/fail/phantom_not_send.rs
//
// 预期：
//   error[E0277]: `*const u64` cannot be sent between threads safely
//     = help: within `{closure@...}`, the trait `Send` is not implemented for `*const u64`
//   note: required because it appears within the type `PhantomData<*const u64>`
//
// ★ 最后一行的措辞值得逐字读：编译器指的是 `PhantomData<*const u64>`，
//   **不是 `Handle<u64>`**。它直接把"谁该为 !Send 负责"点了出来。
//
//   对照 src/lib.rs 里的 `SafeHandle<T>`（`PhantomData<fn() -> T>`）——
//   同一个 `T`，只差 PhantomData 的写法，就变成了 `Send + Sync`。
//   因为 `fn() -> T` 是**函数指针**，函数指针总是 `Send + Sync`。
//
//   这条规则的实际用途：手写 `unsafe` 容器时（第 24–25 章），
//   `PhantomData` 是**唯一**能精确控制 auto trait 的工具。

use std::marker::PhantomData;

pub struct Handle<T> {
    id: u64,
    _p: PhantomData<*const T>,
}

pub fn spawn(h: Handle<u64>) {
    // `let _ = &h;` 强制闭包捕获整个 Handle（而不是只捕获 id）
    std::thread::spawn(move || { let _ = &h; h.id }).join().unwrap();
}

fn main() {}
