// ⚠️ 故意编译不过：Rc<T> 不是 Send
// 复现：rustc --edition 2024 --crate-type=lib examples/ch12-send-sync/fail/not_send.rs
// 预期：error[E0277]: `Rc<u64>` cannot be sent between threads safely
use std::rc::Rc;
fn main() {
    let r = Rc::new(1u64);
    std::thread::spawn(move || {
        println!("{}", *r);
    });
}
