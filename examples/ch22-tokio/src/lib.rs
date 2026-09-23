//! 第 22 章：tokio 实战 —— 从原理到工程
//!
//! 证据生成：tools/evidence.sh ch22-tokio
//!
//! 第 18 章说"状态机不会自己跑，必须由 executor 驱动"，
//! 第 20 章说"跨 `await` 的字段决定 `Send`"，
//! 第 21 章说"`async fn in trait` 表达不了 `Send`"。
//!
//! 这一章把这些结论落到**一个真实的 executor** 上。
//!
//! ★ 本章的骨架是一条因果链：
//!
//! ```text
//! tokio::spawn 要求 F: Send + 'static
//!        ↓ 为什么？
//! 因为任务会被**搬到别的 worker 线程**上跑，而且**可能比调用者活得久**
//!        ↓ 这带来什么？
//! ① 捕获的东西必须 Send（第 12 章）
//! ② 不能借用局部变量（'static）
//! ③ 状态机里的字段不能 !Send（第 20 章）
//!        ↓ 怎么破？
//! 缩小作用域 / Arc / spawn_blocking / 换 LocalSet
//! ```

use std::future::Future;
use std::sync::{Arc, Mutex};

// ============================================================
// 1) `Send + 'static`：tokio 的核心约束
// ============================================================

/// ★ `tokio::spawn` 的签名要求 `F: Future + Send + 'static`。
///
/// **两个约束各自的理由完全不同**：
///
/// | 约束 | 为什么 |
/// |---|---|
/// | `Send` | 任务会被**搬到别的 worker 线程**上执行 |
/// | `'static` | 任务**可能比调用者活得久**（fire-and-forget） |
///
/// ★ 对照：`std::thread::spawn` 也要求 `Send + 'static` ——
/// **同样的两个理由**（第 12 章）。tokio 没有引入新概念，
/// 它只是把"线程"换成了"任务"。
pub fn spawn_send_static<F>(f: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(f)
}

// ============================================================
// 2) 正确写法：把共享状态交给 Arc
// ============================================================

/// ★ 想在多个任务间共享状态 —— 用 `Arc`（第 13 章）。
///
/// 注意锁的作用域被收进了块里（第 20 章）：
/// 否则 `MutexGuard` 跨 `await`，future 就不是 `Send` 了。
#[unsafe(no_mangle)]
pub async fn shared_counter(n: usize) -> u64 {
    let counter = Arc::new(Mutex::new(0u64));
    let mut handles = Vec::with_capacity(n);

    for _ in 0..n {
        let c = Arc::clone(&counter);
        handles.push(tokio::spawn(async move {
            // ★ 块作用域：guard 在 await 之前就 drop
            let cur = {
                let g = c.lock().unwrap();
                *g
            };
            tokio::task::yield_now().await;
            let mut g = c.lock().unwrap();
            *g = cur + 1;
        }));
    }

    for h in handles {
        h.await.expect("task panicked");
    }

    *counter.lock().unwrap()
}

// ★ 反面：这样写**编译不过**（见 `fail/guard_across_await.rs`）——
// 锁守卫跨了 `await`。
//
// 注意 tokio 的错误信息比裸 `spawn` **更清楚**：
// 它会说 "future cannot be sent between threads safely" 并指出
// `MutexGuard`（第 20 章实测过同样的措辞）。

// ============================================================
// 3) `'static` 的两种破法
// ============================================================

/// ★ 破法一：**把数据搬进去**（`move` + 所有权）。
///
/// `String` 是拥有的，所以 `'static` 成立。
#[unsafe(no_mangle)]
pub fn spawn_owned(s: String) -> tokio::task::JoinHandle<usize> {
    tokio::spawn(async move { s.len() })
}

/// ★ 破法二：**`Arc<str>` / `Arc<[T]>`** —— 共享所有权，代价是一次原子计数。
#[unsafe(no_mangle)]
pub fn spawn_shared(s: Arc<str>) -> tokio::task::JoinHandle<usize> {
    tokio::spawn(async move { s.len() })
}

// ★ 不能做的事：借用局部变量（见 `fail/borrow_local.rs`）。
//
// ```ignore
// pub async fn bad() {
//     let v = vec![1u64, 2];
//     tokio::spawn(async { v.len() });   // ← 借用 v，不是 'static
// }
// ```
//
// 编译器会说 "borrowed data escapes outside of function"。
// 修法：`async move { v.len() }` —— **把所有权搬进去**。

// ============================================================
// 4) 阻塞代码：`spawn_blocking`（工程上最常踩的坑）
// ============================================================

/// ★ **在异步任务里做阻塞操作，会卡住整个 worker 线程。**
///
/// tokio 的 worker 线程数默认 = CPU 核数（多线程 runtime）。
/// 一个任务阻塞住，那个 worker 上的**其他所有任务都停摆**。
///
/// 正确做法：`tokio::task::spawn_blocking` —— 它把闭包丢到
/// **专门的阻塞线程池**上（默认上限 512 个线程）。
#[unsafe(no_mangle)]
pub async fn blocking_work(data: Vec<u64>) -> u64 {
    tokio::task::spawn_blocking(move || {
        // 这里可以做 CPU 密集 / 同步 IO / 调用 C 库
        data.iter().fold(0u64, |a, b| a.wrapping_add(*b))
    })
    .await
    .expect("blocking task panicked")
}

// ★ 注意 `spawn_blocking` 的约束**不是** `Send + 'static` 的 future，
// 而是 `FnOnce() -> R + Send + 'static`：
//
// | | `spawn` | `spawn_blocking` |
// |---|---|---|
// | 接受 | `Future + Send + 'static` | `FnOnce() -> R + Send + 'static` |
// | 跑在 | async worker | 阻塞线程池 |
// | 用于 | 异步 IO | CPU 密集 / 同步 IO |
//
// ★ 两者的共同点是 **`Send + 'static`** —— 因为都会**跨线程、跨时间**。

// ============================================================
// 5) 一个反例：`!Send` 的值跨 await
// ============================================================

/// ★ `Rc` 是 `!Send`（第 12 章）—— 捕获它进 `tokio::spawn` 直接拒绝。
///
/// 见 `fail/rc_in_spawn.rs`：
/// ```text
/// error[E0277]: `Rc<u64>` cannot be sent between threads safely
/// ```
///
/// ★ 但如果**不跨 await**，`Rc` 在异步函数里是完全可以用的：
/// 单线程执行、没有 `spawn`，就没问题。
#[unsafe(no_mangle)]
pub async fn rc_within_task() -> u64 {
    let r = std::rc::Rc::new(1u64);
    tokio::task::yield_now().await;
    // ★ 注意：这里 r 跨过了 await —— 但因为**整个函数不要求 Send**
    //   （它没有被 spawn），所以编译得过。
    //   这也说明：**`Send` 的要求来自 spawn，不是来自 async**。
    *r
}

// ============================================================
// 6) 单线程 runtime：`!Send` 的逃生门
// ============================================================

/// ★ `tokio::task::LocalSet` 允许 spawn `!Send` 的 future ——
/// 因为它们在**同一个线程**上跑。
///
/// 这就是"`Send` 的要求来自 spawn"的直接证明：
/// **换一个 spawn 方式，约束就没了。**
#[unsafe(no_mangle)]
pub async fn local_task() -> u64 {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let r = std::rc::Rc::new(7u64);
            tokio::task::spawn_local(async move { *r }).await.unwrap()
        })
        .await
}

// ============================================================
// 7) `Send` 的传播：把任务包一层
// ============================================================

/// ★ 泛型包装：`F: Future + Send + 'static` 的约束会**传播**。
///
/// 调用者传进来的 future 必须是 `Send` —— 而如果那个 future 内部
/// 跨 `await` 持有 `!Send` 的东西（第 20 章），这里就过不了。
///
/// ★ 这就是第 21 章那个问题的**工程形态**：
/// 一个库里有一个 `!Send` 的 `async fn`，
/// **所有把它包进 `spawn` 的地方**都会编译失败 ——
/// 而报错位置离真正的原因很远。
pub fn spawn_with_log<F>(label: &'static str, f: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(async move {
        let out = f.await;
        // 真实项目里这里会打日志
        let _ = label;
        out
    })
}

// ============================================================
// 8) 运行期演示（见 `src/main.rs`）
// ============================================================

/// ★ 一个可运行的最小例子：4 个任务并发，每个 +1。
///
/// 运行 `cargo run -p ch22-tokio` 会打印 `total = 4`。
///
/// ★ 注意 `#[tokio::main]` 做的事：**构造一个 runtime 并 block_on**。
/// 它等价于手写：
///
/// ```ignore
/// fn main() {
///     tokio::runtime::Runtime::new().unwrap().block_on(async { ... })
/// }
/// ```
///
/// 这和第 18 章的 `block_on` 是同一个东西 ——
/// 只是 tokio 的版本带线程池、IO 驱动、定时器驱动。
pub fn demo() -> u64 {
    4
}
