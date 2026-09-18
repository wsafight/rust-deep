//! 调度行为：三个可断言的事实。
//!
//! 复现：cargo test -p ch23-project-runtime

use ch23_project_runtime::{block_on, count_to, run_local, run_three, Executor, YieldNow};
use std::sync::{Arc, Mutex};

/// ★ 三个任务按入队顺序各被 poll 一次，然后按入队顺序完成。
#[test]
fn three_tasks_run_in_order() {
    assert_eq!(run_three(), vec![0, 1, 2]);
}

/// `count_to(n)` 每数一次就 yield 一次，所以会经过 n 次队列往返。
#[test]
fn count_to_n() {
    assert_eq!(block_on(count_to(5)), 5);
    assert_eq!(block_on(count_to(0)), 0);
}

/// ★ `!Send` 的 future（含 `Rc`）**可以** `block_on` ——
/// 因为 `block_on` 不跨线程（对照第 22 章：`Send` 的要求来自 `spawn`）。
#[test]
fn non_send_future_can_block_on() {
    assert_eq!(run_local(), 42);
}

/// ★★ 一个**不调用 waker** 的 future 会被安静地丢掉。
///
/// 这是 executor 实现里最经典的 bug：`poll` 返回 `Pending` 却忘了
/// `wake()`，任务就永远躺在队列外面 —— **不报错、不 panic、就是不动**。
///
/// 本用例把这个行为**固化下来**：它证明"唤醒"这件事完全由 future
/// 自己负责，executor 不会替你检查。
#[test]
fn pending_without_wake_is_lost() {
    struct NeverWakes;
    impl std::future::Future for NeverWakes {
        type Output = ();
        fn poll(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<()> {
            // ← 故意不调用 _cx.waker()
            std::task::Poll::Pending
        }
    }

    let done = Arc::new(Mutex::new(false));
    let d = Arc::clone(&done);
    let exec = Executor::new();
    exec.spawn(async move {
        NeverWakes.await;
        *d.lock().unwrap() = true;
    });

    // `run` 会**立刻返回** —— 队列空了，但任务其实没完成
    exec.run();
    assert!(!*done.lock().unwrap(), "任务被丢掉了（这正是本用例要证明的）");
}

/// 对照：`YieldNow` 调用了 waker，所以任务会回到队列并最终完成。
#[test]
fn yield_now_wakes_and_finishes() {
    let done = Arc::new(Mutex::new(0u64));
    let d = Arc::clone(&done);
    let exec = Executor::new();
    exec.spawn(async move {
        YieldNow::new().await;
        YieldNow::new().await;
        *d.lock().unwrap() = 1;
    });
    exec.run();
    assert_eq!(*done.lock().unwrap(), 1);
}

/// ★★ 跨线程唤醒：`Task: Send` 的**唯一理由**。
///
/// 真实的 executor 里，waker 常常在**别的线程**被调用
/// （IO 线程收到数据、定时器线程到点）。
/// 所以 `Task` 必须 `Send` —— 它会随着 waker 跨线程移动。
///
/// 本用例把 waker 拿到另一个线程去调用，验证：
/// 1. 任务确实被重新入队并完成；
/// 2. Miri 不报数据竞争（`cargo +nightly miri test`）。
#[test]
fn wake_from_another_thread() {
    use std::sync::mpsc;
    use std::task::{Context, Poll};

    /// 一个 future，它把 waker 交给另一个线程去调用。
    struct WakeFromOtherThread {
        tx: Option<mpsc::Sender<std::task::Waker>>,
    }

    impl std::future::Future for WakeFromOtherThread {
        type Output = ();
        fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            match self.tx.take() {
                Some(tx) => {
                    // 把 waker 发到另一个线程（waker 必须是 Send 的）
                    tx.send(cx.waker().clone()).unwrap();
                    Poll::Pending
                }
                None => Poll::Ready(()),
            }
        }
    }

    let (tx, rx) = mpsc::channel();
    let done = Arc::new(Mutex::new(false));
    let d = Arc::clone(&done);
    let exec = Executor::new();

    exec.spawn(async move {
        WakeFromOtherThread { tx: Some(tx) }.await;
        *d.lock().unwrap() = true;
    });

    // ★ 注意顺序：**先 `run()` 一次** —— 那次 poll 会把 waker 发出来，
    //   然后队列空了，`run()` 返回（这正是"简化版 executor"的特征：
    //   它不 park/等待，只是把当前就绪的都跑完）。
    exec.run();

    // 现在从通道里拿到 waker（第一次 poll 时发出来的）
    let waker = rx.recv().unwrap();

    // ★ 在另一个线程里调用 waker —— 这就是"跨线程唤醒"。
    //   `Waker: Send + Sync`，而它内部持有的 `Arc<Task>` 必须 `Send` ——
    //   这就是 `Task: Send` 的唯一理由。
    let h = std::thread::spawn(move || {
        waker.wake();
    });
    h.join().unwrap();

    // waker 已经把任务放回队列了，再跑一次就完成
    exec.run();
    assert!(*done.lock().unwrap(), "跨线程唤醒后任务应该完成");
}
