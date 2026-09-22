//! 第 23 章（实战）：写一个 mini async runtime
//!
//! 证据生成：tools/evidence.sh ch23-project-runtime
//!
//! 第 18–22 章把异步的每一块都拆开讲了。这一章把它们拼起来 ——
//! **从零写一个能跑的 executor**，然后看清 tokio 在上面加了什么。
//!
//! 我们要实现的（按第 18 章那张表逐条兑现）：
//!
//! | 组件 | 章 | 本章的实现 |
//! |---|---|---|
//! | 状态机 + `poll` | 18 | 由编译器生成（我们只调用） |
//! | `Pin<Box<F>>` | 19 | `Task` 里存的就是它 |
//! | `Waker` / `RawWakerVTable` | 18 | ★ 为展示底层契约而手写；也可用安全的 `Wake` |
//! | 任务队列 + 唤醒 | 18 | `Executor` |
//! | `Send` 边界 | 22 | 队列跨线程 → `Task` 必须 `Send` |
//!
//! ★ **executor 的核心循环只有三行**：
//!
//! ```text
//! loop {
//!     let task = queue.pop();      // 取一个就绪任务
//!     task.poll(waker);            // 推进一步
//!     if pending { /* waker 会在将来把它重新入队 */ }
//! }
//! ```
//!
//! 剩下的全部复杂度，都在**"waker 什么时候被调用"**这一件事上。

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

// ============================================================
// 1) ★ 刻意下潜的 unsafe：把 `Arc<Task>` 变成 `Waker`
// ============================================================

/// 一个可执行的任务：一个被 `Pin` 住的 future + 一个回到队列的通道。
///
/// ★ 三个字段各有来历：
///
/// | 字段 | 为什么是这个类型 | 章 |
/// |---|---|---|
/// | `Pin<Box<dyn Future>>` | 状态机不能移动；`dyn` 让它统一 | 19、21 |
/// | `Mutex<Option<...>>` | `poll` 要 `&mut`，而 waker 拿到的是 `&Task` | 1 |
/// | `Weak<Executor>` | ★ 打破引用环（Task 被 Executor 持有，waker 又持有 Task） | — |
///
/// ★ `dyn Future<Output = ()> + Send` 里的 **`+ Send`** 是第 22 章那个约束：
///   任务会被从**任意线程**（waker 可能在别的线程被调用）放回队列，
///   而队列是 `Mutex<VecDeque<Arc<Task>>>` —— 所以 `Task` 必须 `Send`。
pub struct Task {
    /// `Option` 是为了在完成时把 future 丢掉（释放它占的内存）
    future: Mutex<Option<Pin<Box<dyn Future<Output = ()> + Send>>>>,
    /// 回到队列的通道。用 `Weak` 避免 `Executor -> Task -> Executor` 的引用环。
    executor: Weak<Executor>,
}

impl Task {
    /// 推进一步。返回 `true` 表示还没完成。
    ///
    /// ★ 这里就是第 18 章说的"executor 的全部工作就是反复 poll"。
    fn poll(self: &Arc<Self>) -> bool {
        // 先造 waker（它会 clone 一份 Arc，见下面的 vtable）
        let waker = task_waker(self);
        let mut cx = Context::from_waker(&waker);

        // 锁住 future 拿到 `&mut` —— 这是 `Mutex` 存在的唯一理由
        let mut guard = self.future.lock().unwrap();
        let Some(future) = guard.as_mut() else {
            return false; // 已经完成了
        };

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(()) => {
                // 完成：把 future 丢掉（状态机占的内存在这里释放）
                *guard = None;
                false
            }
            Poll::Pending => true,
        }
    }
}

/// ★★ 本章为展示底层契约而手写 `RawWakerVTable`。
/// 对 `Arc<T>` 场景也可以实现 `std::task::Wake` 并使用
/// `Waker::from(Arc<T>)`，从而不在业务代码里手写这段 `unsafe`。
///
/// `Waker` 的接口完全安全，但构造它要**承诺**四个函数的行为符合
/// `RawWakerVTable` 的契约（第 18 章提过这个微缩样本）：
///
/// | 函数 | 契约 |
/// |---|---|
/// | `clone` | 必须**增加**引用计数，返回一个新的 `RawWaker` |
/// | `wake` | 必须**消耗**一个引用计数（`from_raw` 的逆操作） |
/// | `wake_by_ref` | 必须**不消耗**引用计数 |
/// | `drop` | 必须**减少**引用计数 |
///
/// ★ **四个函数里的任何一个写错，都是内存泄漏或 use-after-free。**
///   而编译器**完全无法检查** —— 它们都是 `unsafe fn`，
///   类型是裸的 `*const ()`。
///
/// ★ 这就是第 24 章那句话的最纯粹形态：
///   **"`unsafe` 的义务是维持契约的前提"** ——
///   而这里的契约是关于**引用计数配平**的，编译器和 MIR 都看不见。
fn task_waker(task: &Arc<Task>) -> Waker {
    /// 从 `RawWaker` 的 `data` 里拿回 `Arc<Task>`（**不消耗**引用计数）。
    ///
    /// # Safety
    /// `data` 必须来自 `Arc::into_raw(Arc<Task>)`，且该 Arc 仍然有效。
    unsafe fn as_task(data: *const ()) -> Arc<Task> {
        // SAFETY: 由 vtable 的契约保证 data 是 Arc<Task> 的裸指针。
        // 这里用 from_raw + 立刻 into_raw 的写法来"借"一个 Arc
        // （不增加计数）—— 因为 clone/wake_by_ref 的契约是"不消耗"。
        let arc = unsafe { Arc::from_raw(data as *const Task) };
        // 立刻还回去，避免 drop 掉计数
        let _ = Arc::into_raw(arc.clone());
        arc
    }

    unsafe fn clone(data: *const ()) -> RawWaker {
        // 契约：增加引用计数
        let arc = unsafe { Arc::from_raw(data as *const Task) };
        let cloned = Arc::clone(&arc);
        let _ = Arc::into_raw(arc); // 还回原来的那份
        RawWaker::new(Arc::into_raw(cloned) as *const (), &VTABLE)
    }

    unsafe fn wake(data: *const ()) {
        // 契约：消耗一个引用计数
        let arc = unsafe { Arc::from_raw(data as *const Task) };
        wake_task(arc);
    }

    unsafe fn wake_by_ref(data: *const ()) {
        // 契约：不消耗
        let arc = unsafe { as_task(data) };
        wake_task(arc);
    }

    unsafe fn drop_waker(data: *const ()) {
        // 契约：减少引用计数
        drop(unsafe { Arc::from_raw(data as *const Task) });
    }

    static VTABLE: RawWakerVTable =
        RawWakerVTable::new(clone, wake, wake_by_ref, drop_waker);

    let raw = Arc::into_raw(Arc::clone(task)) as *const ();
    // SAFETY: raw 来自 Arc::into_raw，vtable 的四个函数满足上面列出的契约。
    // 这是本章刻意下潜的一处 unsafe，也是"安全抽象 + 不安全实现"的样板。
    unsafe { Waker::from_raw(RawWaker::new(raw, &VTABLE)) }
}

/// waker 被调用时做的事：**把任务放回队列**。
///
/// ★ 这一行就是"唤醒"的全部含义 —— 没有魔法，就是入队。
fn wake_task(task: Arc<Task>) {
    if let Some(exec) = task.executor.upgrade() {
        exec.ready.lock().unwrap().push_back(task);
    }
    // 如果 executor 已经没了（upgrade 失败），什么都不做 ——
    // 任务会被安静地丢掉。
}

// ============================================================
// 2) executor：一个队列 + 一个循环
// ============================================================

/// ★ **executor 的全部状态就是一个队列。**
///
/// 第 18 章说"executor 的核心循环就是 `loop { poll }`"；
/// 加上唤醒机制之后，它变成"`loop { 从队列取一个 → poll }`"。
pub struct Executor {
    ready: Mutex<VecDeque<Arc<Task>>>,
}

impl Executor {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { ready: Mutex::new(VecDeque::new()) })
    }

    /// 把一个 future 变成任务并立刻入队。
    ///
    /// ★ `F: Future<Output = ()> + Send + 'static` —— **与 `tokio::spawn` 同样的约束**
    ///   （第 22 章）。理由也一样：任务会被从别的线程放回队列。
    ///
    /// ★ 对照：如果这个 executor 是**单线程**的（比如 tokio 的 `LocalSet`），
    ///   这里的 `+ Send` 就可以去掉（第 22 章实测过）。
    pub fn spawn<F>(self: &Arc<Self>, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let task = Arc::new(Task {
            future: Mutex::new(Some(Box::pin(future))),
            executor: Arc::downgrade(self),
        });
        self.ready.lock().unwrap().push_back(task);
    }

    /// 运行当前就绪队列，直到队列暂时为空。
    ///
    /// 它不会等待未来的跨线程唤醒，因此不保证返回时所有任务都已完成。
    pub fn run(&self) {
        loop {
            // ① 取一个就绪任务（没有就退出）
            let Some(task) = self.ready.lock().unwrap().pop_front() else {
                return;
            };
            // ② 推进一步。`false` = 已完成，不用管了
            // ③ 如果返回 `true`（Pending），任务**现在不在队列里** ——
            //    它会在 waker 被调用时回来（见 `wake_task`）
            let _pending = task.poll();
        }
    }
}

impl Default for Executor {
    fn default() -> Self { Self { ready: Mutex::new(VecDeque::new()) } }
}

// ============================================================
// 3) 最简单的 executor：block_on（第 18 章的版本）
// ============================================================

/// ★ 没有队列、没有 waker 交换 —— 只有一个忙等循环。
///
/// 它对应第 18 章那个 `block_on`。**注意它不需要 `Send`** ——
/// 因为它不跨线程（对比 `Executor::spawn`）。
pub fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = Box::pin(f);
    // `Waker::noop()` 是 1.98 标准库提供的（第 18 章）
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    loop {
        match f.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::hint::spin_loop(), // 忙等（简化）
        }
    }
}

// ============================================================
// 4) 一个会自我唤醒的 future：`YieldNow`
// ============================================================

/// ★ 一个"让出一次"的 future —— 最小的教学用 `Future` 实现之一。
///
/// 它的 `poll` 第一次返回 `Pending`，同时**调用 `waker.wake_by_ref()`**
/// 把自己重新入队；第二次返回 `Ready`。
///
/// ★ 它表达的是："这次还没完成，但状态已经变化，请再 poll 一次。"
/// 真实场景里，唤醒通常来自 IO 就绪或定时器；这里立即唤醒。
pub struct YieldNow {
    yielded: bool,
}

impl YieldNow {
    pub fn new() -> Self { Self { yielded: false } }
}

impl Default for YieldNow {
    fn default() -> Self { Self::new() }
}

impl Future for YieldNow {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.yielded {
            Poll::Ready(())
        } else {
            self.yielded = true;
            // ★ 关键一行：**告诉 executor "我还在队列里，别忘了我"**。
            //   如果漏掉这一行，任务会永远卡在 Pending 上（"任务丢失"）。
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

/// 一个真的会数数的 future：数到 `n` 就返回。
///
/// ★ 它用 `YieldNow` 来"让出"——所以每次 `await` 都会经过一次队列往返。
pub async fn count_to(n: u64) -> u64 {
    let mut i = 0u64;
    while i < n {
        YieldNow::new().await;
        i += 1;
    }
    i
}

// ============================================================
// 5) 一个"跨 await 持有 !Send" 的例子（第 20、22 章的对照）
// ============================================================

/// ★ 这个 future **不是 `Send`**（因为 `Rc`）—— 于是它**不能** `Executor::spawn`。
///
/// 但它完全可以 `block_on`（第 22 章实测过同一件事：
/// **`Send` 的要求来自 spawn，不是来自 async**）。
///
/// ★ 这里我们干脆用 `block_on` 跑它，并把它导出成一个普通函数，
///   好让本章的证据里有一份"`!Send` 的 future 也能跑"的汇编。
#[unsafe(no_mangle)]
pub fn run_local() -> u64 {
    block_on(async {
        let r = std::rc::Rc::new(21u64);
        YieldNow::new().await;
        *r * 2
    })
}

// ============================================================
// 6) 运行期演示（见 `src/main.rs`）
// ============================================================

/// 用 mini executor 跑三个并发任务。
///
/// ★ 返回三个任务的完成顺序 —— 这是**调度行为**的可断言证据。
///
/// 预期：`[0, 1, 2]`（先入队的先被 poll 第一次；每个任务各 yield 一次）
#[unsafe(no_mangle)]
pub fn run_three() -> Vec<u64> {
    let order = Arc::new(Mutex::new(Vec::new()));
    let exec = Executor::new();

    for id in 0..3u64 {
        let order = Arc::clone(&order);
        exec.spawn(async move {
            YieldNow::new().await;
            order.lock().unwrap().push(id);
        });
    }

    exec.run();
    let out = order.lock().unwrap().clone();
    out
}
