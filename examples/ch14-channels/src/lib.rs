//! 第 14 章：消息传递 —— 发送即 move，编译期切断别名
//!
//! 证据生成：tools/evidence.sh ch14-channels
//!
//! 本章的核心：**`send` 拿走所有权，所以发送之后本地再也用不了它** ——
//! 这不是运行时约定，是**类型系统**的结果（`send(self, t: T)`）。
//! 对比 `Arc`（共享所有权）与 channel（转移所有权）：两条完全不同的路。

use std::sync::mpsc;
use std::thread;

/// ★ 发送即 move：`s` 之后就不能用了（编译期保证）
/// 汇编里能看到的是：`String` 的三个字（ptr/len/cap）被搬进消息体
#[unsafe(no_mangle)]
pub fn send_string() -> usize {
    let (tx, rx) = mpsc::channel::<String>();
    let s = String::from("hello");
    let len = s.len();
    tx.send(s).unwrap(); // ← s 在这里被 move 走
    // println!("{s}");    // ← 取消注释会报 E0382: use of moved value
    drop(tx);
    let got = rx.recv().unwrap();
    got.len() + len
}

/// 对照：`Arc` 是**共享**所有权 —— 发送 clone，本地仍然可用
#[unsafe(no_mangle)]
pub fn send_arc() -> usize {
    use std::sync::Arc;
    let (tx, rx) = mpsc::channel::<Arc<String>>();
    let s = Arc::new(String::from("hello"));
    tx.send(Arc::clone(&s)).unwrap();
    drop(tx);
    let got = rx.recv().unwrap();
    got.len() + s.len() // ← s 仍然可用（引用计数 +1）
}

/// 跨线程发送：`Send` 是编译期检查，运行时零成本
#[unsafe(no_mangle)]
pub fn send_across_thread() -> usize {
    let (tx, rx) = mpsc::channel::<Vec<u64>>();
    let h = thread::spawn(move || {
        tx.send(vec![1, 2, 3]).unwrap();
    });
    let got = rx.recv().unwrap();
    h.join().unwrap();
    got.len()
}
