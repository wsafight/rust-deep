// 运行期演示：**不 poll 就没有副作用**（这个文件能编译，是正例）
//
// 复现：rustc --edition 2024 examples/ch18-future/lazy_demo.rs -o /tmp/anp && /tmp/anp
// 预期输出：
//   after construct: N = 0
//   after drop:      N = 0
//
// ★ 这是"`Future` 是惰性的"最直接的运行期证据：
//   调用 `lazy(1)` **只构造了状态机**，函数体里的 `N.fetch_add` 一次都没执行。
//   甚至把它 drop 掉也不执行 —— 因为**从来没有 poll 过**。
//
//   ★ 这也解释了 Rust 异步的一个经典陷阱：
//   ```rust
//   let _ = async { println!("hello"); };   // ← 什么都不会打印
//   ```
//   `async {}` 块**不是**"开始执行"，而是"构造一个还没开始的执行"。
//   要让它跑，必须 `.await` 或交给 executor。

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};

pub static N: AtomicU64 = AtomicU64::new(0);

pub struct Ready<T>(pub Option<T>);

impl<T: Unpin> Future for Ready<T> {
    type Output = T;
    fn poll(mut self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>)
        -> std::task::Poll<T>
    {
        std::task::Poll::Ready(self.0.take().unwrap())
    }
}

pub async fn lazy(a: u64) -> u64 {
    N.fetch_add(1, Ordering::Relaxed);   // ← 只有 poll 之后才执行
    a + 1
}

fn main() {
    let f = lazy(1);                       // 只构造
    println!("after construct: N = {}", N.load(Ordering::Relaxed));
    drop(f);
    println!("after drop:      N = {}", N.load(Ordering::Relaxed));

    // 对照：poll 之后
    let mut f = Box::pin(lazy(1));
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    let _ = f.as_mut().poll(&mut cx);
    println!("after poll:      N = {}", N.load(Ordering::Relaxed));
}
