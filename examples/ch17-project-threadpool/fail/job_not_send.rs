// ⚠️ 故意编译不过：**任务必须是 `Send`**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch17-project-threadpool/fail/job_not_send.rs
//
// 预期：
//   error[E0277]: `Rc<u64>` cannot be sent between threads safely
//     = help: within `...`, the trait `Send` is not implemented for `Rc<u64>`
//
// ★ 线程池的 `Job` 类型是 `Box<dyn FnOnce() + Send + 'static>`。
//   三个约束缺一不可，每一个都对应前面某一章的结论：
//
//   - `FnOnce()`  —— 任务只执行一次，调用后可以消耗捕获值；
//   - `Send`      —— 任务要跨线程（第 12 章）；
//   - `'static`   —— 任务在线程里存活，不能借用栈上的东西。
//
//   这里违反的是第二条：闭包捕获了 `Rc<u64>`（不是 `Send`）。
//
//   ★ 对照 `src/lib.rs` 的 `pool_sum` —— 它捕获的是
//   `Vec<u64>`（`Send`）和 `Sender<u64>`（`Send`），所以能过。

use std::rc::Rc;
use std::sync::mpsc;

fn main() {
    let (tx, _rx) = mpsc::channel::<u64>();
    let r = Rc::new(1u64);

    // 模拟 ThreadPool::execute 的约束
    let job: Box<dyn FnOnce() + Send + 'static> = Box::new(move || {
        tx.send(*r).unwrap();       // ← 捕获了 Rc<u64>
    });
    job();
}
