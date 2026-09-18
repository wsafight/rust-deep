// ⚠️ 故意编译不过：`Cell<T>` 不是 `Sync`
// 复现：rustc --edition 2024 --crate-type=lib examples/ch12-send-sync/fail/not_sync.rs
//
// 预期：
//   error[E0277]: `Cell<u64>` cannot be shared between threads safely
//     = help: the trait `Sync` is not implemented for `Cell<u64>`
//     = note: if you want to do aliasing and mutation between multiple threads,
//             use `std::sync::RwLock` or `std::sync::atomic::AtomicU64` instead
//     = note: required for `&Cell<u64>` to implement `Send`
//
// ★ 最后一行 note 是 `Sync` 的**定义**：
//   编译器要检查的其实是 `&Cell<u64>: Send`，
//   而标准库里的实际 impl 是
//       impl<T: ?Sized + Sync> Send for &T {}
//   —— **`Sync` 就是"`&T` 是 `Send`"这件事的名字。**
//
//   ★ 注意 `Cell<u64>` **是** `Send`（见 src/lib.rs 的 `cell_is_send`）——
//   值可以整个搬到另一个线程。不行的是**共享引用** `&Cell<u64>`。
//   这个不对称是本章最容易搞混的地方。
//
//   （同一个道理也会在 `Arc<Cell<u64>>` 上出现：`Arc<T>: Send` 要求
//    `T: Send + Sync`，卡在同一个 `Sync` 上，note 会写成
//    `` required for `Arc<Cell<u64>>` to implement `Send` ``。）

use std::cell::Cell;

fn send_ref(c: &Cell<u64>) {
    // 把**共享引用**送进另一个线程 —— 这要求 `&Cell<u64>: Send`
    std::thread::spawn(move || c.set(1));
}

fn main() {}
