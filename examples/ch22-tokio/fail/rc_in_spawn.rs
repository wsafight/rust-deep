// ⚠️ 故意编译不过：**`Rc` 不能跨线程**（第 12 章的知识，tokio 上的形态）
// 复现：rustc --edition 2024 --crate-type=lib --extern tokio=... examples/ch22-tokio/fail/rc_in_spawn.rs
//
// 预期：
//   error[E0277]: `Rc<u64>` cannot be sent between threads safely
//     = help: within `{async block@...}`, the trait `Send` is not implemented
//             for `Rc<u64>`
//   note: required by a bound in `spawn`
//
// ★ 注意最后一行：**"required by a bound in `spawn`"** ——
//   编译器告诉你要求来自哪里。
//
// ★ 修法：`Arc`（第 13 章）。
//
// ★ 但有个重要的对照（见 `src/lib.rs` 的 `rc_within_task`）：
//   **如果不用 `spawn`，`Rc` 在异步函数里完全可以用** ——
//   因为 `Send` 的要求来自 `spawn`，不是来自 `async`。
//   单线程 runtime（`LocalSet` / `spawn_local`）就是为这类需求准备的。

use std::rc::Rc;

pub fn spawn_it() {
    let r = Rc::new(1u64);
    tokio::spawn(async move {
        let _ = *r;                   // ← Rc 进了任务
    });
}

fn main() {}
