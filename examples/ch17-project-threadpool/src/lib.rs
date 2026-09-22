//! 第 17 章（实战）：构造一个并发任务池
//!
//! 证据生成：tools/evidence.sh ch17-project-threadpool
//!
//! 本章把第三部分（12–16）的概念用在一个**真实组件**上：
//! 一个最小的线程池 —— `ThreadPool::new(n)` + `execute(f)`。
//!
//! 它的形状和标准库的 `threadpool` crate 几乎一样，
//! 但每一行都能对应到前面某一章的结论：
//!
//! | 代码 | 用到的概念 | 来自 |
//! |---|---|---|
//! | `Box<dyn FnOnce() + Send + 'static>` | trait object + `Send` | 第 7、12 章 |
//! | `mpsc::channel::<Job>()` | 发送即 move | 第 14 章 |
//! | `Arc<Mutex<Receiver<Job>>>` | 共享可变状态 | 第 13、15 章 |
//! | worker 用 `move` 捕获 receiver | 所有权转移 | 第 1、14 章 |
//!
//! ★ 结论先行：**这个线程池的"共享可变"部分只有一处** ——
//! 接收端。而 `Arc<Mutex<Receiver>>` 正是第 15 章判据表里的层次 3
//! （真的需要共享可变）。**其余全部是"不共享"。**

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// 任务类型：一个**只能调用一次**的闭包。
///
/// ★ 三个约束缺一不可，每一个都对应一章的结论：
///
/// - `FnOnce()` —— 任务只执行一次，调用后可以消耗捕获值；
/// - `Send` —— 任务要跨线程（第 12 章）；
/// - `'static` —— 任务在线程里存活，不能借用栈上的东西
///   （否则线程可能比借用者活得久）。
///
/// ★ 而且这里**必须**是 `Box<dyn ...>`（第 7 章）：
/// 不同的任务类型不同，`Vec<Job>` 需要一个统一大小 ——
/// 于是装箱成 trait object。
pub type Job = Box<dyn FnOnce() + Send + 'static>;

/// 最小线程池。
pub struct ThreadPool {
    workers: Vec<Worker>,
    sender: Option<Sender<Job>>,
}

struct Worker {
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ThreadPool {
    /// 创建 `n` 个 worker 线程。
    ///
    /// ★ 注意 `receiver` 的处理：**只有一个 `Receiver`**（第 14 章实测：
    /// `Receiver` 是 `Send` 但 `!Sync`），
    /// 而每个 worker 都需要访问它 —— 于是必须 `Arc<Mutex<...>>`。
    ///
    /// **这是本章唯一一处"共享可变"**，也是判据表里唯一用得上
    /// 层次 3 的地方。
    pub fn new(n: usize) -> ThreadPool {
        assert!(n > 0, "线程池至少需要一个 worker");

        let (sender, receiver) = mpsc::channel::<Job>();
        let receiver = Arc::new(Mutex::new(receiver));

        let workers = (0..n)
            .map(|i| Worker::new(i, Arc::clone(&receiver)))
            .collect();

        ThreadPool { workers, sender: Some(sender) }
    }

    /// 提交一个任务。
    ///
    /// ★ `send` 拿走所有权（第 14 章）：任务被 move 进 channel，
    /// 调用方之后**再也拿不到它**。
    pub fn execute<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let job: Job = Box::new(f);
        // unwrap：如果所有 worker 都挂了，send 会失败
        self.sender.as_ref().expect("线程池已关闭").send(job).unwrap();
    }
}

impl Worker {
    fn new(id: usize, receiver: Arc<Mutex<Receiver<Job>>>) -> Worker {
        let handle = std::thread::spawn(move || {
            loop {
                // ★ 关键：**锁的作用域只覆盖 recv，不覆盖任务的执行**。
                // 否则所有任务会串行化 —— 这是线程池最常见的一个 bug。
                let msg = {
                    let guard = receiver.lock().unwrap();
                    guard.recv()
                };
                match msg {
                    Ok(job) => {
                        let _ = id;
                        job();
                    }
                    // 所有 sender 都被 drop 了 → 退出
                    Err(_) => break,
                }
            }
        });
        Worker { handle: Some(handle) }
    }
}

impl Drop for ThreadPool {
    /// ★ `Drop` 的顺序是本章最微妙的部分：
    ///
    /// 1. 先 drop `sender`（`Option::take`）——
    ///    否则 worker 的 `recv` 永远不返回，`join` 会**永久阻塞**；
    /// 2. 再 `join` 每个 worker。
    ///
    /// **如果顺序反了，程序会挂住。** 这不是理论问题 ——
    /// 它是"channel 的关闭语义"（所有 `Sender` 被 drop → `recv` 返回 `Err`）
    /// 和"`join` 会阻塞"这两件事的组合后果。
    fn drop(&mut self) {
        // ① 关闭 channel：drop 掉唯一的 sender
        drop(self.sender.take());
        // ② 现在 worker 的 recv 会返回 Err，它们会退出
        for w in &mut self.workers {
            if let Some(h) = w.handle.take() {
                let _ = h.join();
            }
        }
    }
}

// ============================================================
// 用法示例（也用来做证据）
// ============================================================

/// 用线程池算一批数的和。
#[unsafe(no_mangle)]
pub fn pool_sum(n_workers: usize, data: Vec<u64>) -> u64 {
    let pool = ThreadPool::new(n_workers);
    let (tx, rx) = mpsc::channel::<u64>();

    for chunk in data.chunks(4) {
        let tx = tx.clone();
        let chunk = chunk.to_vec();
        pool.execute(move || {
            let s = chunk.iter().fold(0u64, |a, b| a.wrapping_add(*b));
            tx.send(s).unwrap();
        });
    }
    drop(tx);       // ★ 必须 drop 掉主线程这份，否则 rx 永远不结束

    let total = rx.iter().fold(0u64, |a, b| a.wrapping_add(b));
    drop(pool);     // ← 这里会走 Drop：先关 channel，再 join
    total
}

// ============================================================
// 对照：不用线程池，每个任务开一个线程
// ============================================================

/// ★ 这个对照不是"谁更快"（本书不写没有数据的性能断言），
/// 而是**结构上的差异**：
///
/// | | 每任务一线程 | 线程池 |
/// |---|---|---|
/// | 线程创建次数 | N（任务数） | n（worker 数） |
/// | 队列 | 无（由 OS 调度） | channel |
/// | 共享可变状态 | 无 | **一处**（`Arc<Mutex<Receiver>>`） |
/// | 背压 | 无 | **无**（当前是无界 `mpsc::channel`） |
///
/// **线程池把"线程创建"的成本从 N 次降到 n 次**，
/// 代价是引入了一处共享可变状态和一次 move。
/// 若需要背压，应改用 `sync_channel` 或其他有界队列。
#[unsafe(no_mangle)]
pub fn spawn_per_task(data: Vec<u64>) -> u64 {
    let handles: Vec<_> = data
        .chunks(4)
        .map(|c| {
            let c = c.to_vec();
            std::thread::spawn(move || c.iter().fold(0u64, |a, b| a.wrapping_add(*b)))
        })
        .collect();
    handles.into_iter().map(|h| h.join().unwrap()).fold(0u64, u64::wrapping_add)
}
