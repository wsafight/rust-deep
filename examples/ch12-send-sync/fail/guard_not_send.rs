// ⚠️ 故意编译不过：**`Sync` 和 `Send` 是两个独立的性质**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch12-send-sync/fail/guard_not_send.rs
//
// 预期：
//   error[E0277]: `std::sync::MutexGuard<'_, u64>` cannot be sent between threads safely
//     = help: the trait `Send` is not implemented for `std::sync::MutexGuard<'_, u64>`
//
// ★ 这个反例补上了 12.0 那张表最后两个格子：
//
//   | 类型                  | Send | Sync |
//   |-----------------------|------|------|
//   | Cell<u64>             |  ✅  |  ❌  |   ← fail/not_sync.rs
//   | MutexGuard<'_, u64>   |  ❌  |  ✅  |   ← 本文件
//   | Rc<u64>               |  ❌  |  ❌  |   ← fail/not_send.rs
//   | u64                   |  ✅  |  ✅  |
//
//   四个格子全填满 → **两个 trait 之间没有任何蕴含关系**。
//
// ★ 为什么 `MutexGuard` 是 `!Send`？
//   因为 pthread 只保证"**加锁的那个线程**能解锁"。
//   把 guard 移到别的线程去 unlock 是未定义行为。
//   标准库因此**只**给它实现了 `Sync`，**没有**实现 `Send`。
//
//   （对照：`src/lib.rs` 的 `guard_is_sync` 编译通过 ——
//    `&MutexGuard` 可以跨线程，因为它不解锁。）

use std::sync::Mutex;

fn main() {
    let m = Mutex::new(1u64);
    let g = m.lock().unwrap();
    // 把 guard **按值**送进另一个线程 —— 它会在线程里 drop（= unlock）
    std::thread::spawn(move || {
        let _ = *g;
    });
}
