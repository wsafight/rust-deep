// ⚠️ 故意编译不过：**发送即 move**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch14-channels/fail/use_after_send.rs
//
// 预期：
//   error[E0382]: borrow of moved value: `s`
//     value moved here
//     move occurs because `s` has type `String`, which does not implement the `Copy` trait
//
// ★ 这就是"发送即 move"的全部含义：
//   `Sender::send(self, t: T)` 的签名里 `t: T` 是**按值**，
//   所以调用 `send(s)` 就把 `s` 搬走了。搬走之后 `s` 处于未初始化状态，
//   **本地再也用不了它**。
//
//   `src/lib.rs` 里的 `send_string` 已经把这一行注释掉了
//   （`// println!("{s}");`）—— 本文件就是取消注释之后的样子。
//
//   MIR 里对应的是（`.evidence/ch14-channels-lib.mir`）：
//       _10 = move _4;
//       _8 = std::sync::mpsc::Sender::<String>::send(move _9, move _10)
//   —— `move _4` 是**字面意义**上的搬移。

use std::sync::mpsc;

fn main() {
    let (tx, _rx) = mpsc::channel::<String>();
    let s = String::from("hello");
    tx.send(s).unwrap();
    println!("{s}");        // ← E0382
}
