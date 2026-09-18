// ⚠️ 故意编译不过：**`Rc<RefCell<T>>` 不能跨线程**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch15-shared-state/fail/rc_refcell_thread.rs
//
// 预期：
//   error[E0277]: `Rc<RefCell<u64>>` cannot be sent between threads safely
//     = help: the trait `Send` is not implemented for `Rc<RefCell<u64>>`
//     = note: required because it appears within the type `Rc<RefCell<u64>>`
//
// ★ 这不是"`Rc<RefCell>` 不好"，而是"它的设计目标就是单线程"：
//   - `Rc` 的引用计数是**非原子**的（一条普通 `add`，不是 `ldadd`）；
//   - `RefCell` 的借用检查是**运行时**的（违反时 panic，不是编译错误）。
//
//   搬到多线程的等价物是 `Arc<Mutex<T>>` —— 但代价完全不同：
//
//   | | `Rc<RefCell<T>>` | `Arc<Mutex<T>>` |
//   |---|---|---|
//   | 引用计数 | 普通 `add`（1 条） | 原子 `ldadd` |
//   | 借用检查 | 运行时计数 + panic 路径 | 阻塞等待 + poison 检查 |
//   | 能跨线程 | ❌ | ✅ |
//
//   ★ 注意 `src/lib.rs` 里的 `local_refcell` **编译通过** ——
//   `Rc<RefCell<T>>` 在单线程里完全正常，而且比 `Arc<Mutex<T>>` 便宜得多。
//   **选对工具的前提是选对"共享范围"。**

use std::cell::RefCell;
use std::rc::Rc;

fn main() {
    let r = Rc::new(RefCell::new(0u64));
    let r2 = Rc::clone(&r);
    // 把 Rc<RefCell<u64>> 送进另一个线程
    std::thread::spawn(move || {
        *r2.borrow_mut() += 1;
    })
    .join()
    .unwrap();
}
