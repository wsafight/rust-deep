//! 第 18 章：Future 是惰性的 —— 手写一个最小 executor
//!
//! 证据生成：tools/evidence.sh ch18-future

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

/// 最小 executor：只 poll 一次，不支持真正的唤醒。
pub fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = Box::pin(f);
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    loop {
        match f.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::hint::spin_loop(),
        }
    }
}

/// 一个什么都不做的 waker。
fn noop_waker() -> Waker {
    const VTABLE: RawWakerVTable = RawWakerVTable::new(
        |_| RawWaker::new(std::ptr::null(), &VTABLE),
        |_| {},
        |_| {},
        |_| {},
    );
    // SAFETY: 四个函数都不解引用 data，data 为 null 是合法的
    unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
}

/// 手写一个 Future：立即返回。
pub struct Ready<T>(pub Option<T>);

impl<T: Unpin> Future for Ready<T> {
    type Output = T;
    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<T> {
        Poll::Ready(self.0.take().unwrap())
    }
}

#[unsafe(no_mangle)]
pub fn run_ready() -> u64 {
    block_on(Ready(Some(42u64)))
}

/// ★ 关键证据：`async fn` 展开成的状态机
pub async fn two_awaits(a: u64, b: u64) -> u64 {
    let x = Ready(Some(a)).await;
    let y = Ready(Some(b)).await;
    x + y
}

#[unsafe(no_mangle)]
pub fn run_two_awaits(a: u64, b: u64) -> u64 {
    block_on(two_awaits(a, b))
}

// ============================================================
// 惰性：造一个 future **什么都不做**
// ============================================================

/// ★ `async fn` 的调用**不执行任何代码** —— 它只是**构造**状态机。
///
/// 这个函数返回一个 future，但**没有**调用 `block_on`。
/// 实测：`make_future()` 的 MIR 里只有一条
/// `{coroutine@...} { a: copy _1 }` —— 就是"打包"，
/// **函数体里的 `side_effect` 一次都不会被调用**。
pub async fn lazy(a: u64) -> u64 {
    // 这一行只有在 **poll** 之后才会执行
    side_effect();
    a + 1
}

/// 一个能被观测到的副作用。
pub static CALL_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn side_effect() {
    CALL_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// 只构造，不 poll —— 返回一个**未被推进**的状态机。
#[unsafe(no_mangle)]
pub fn make_future(a: u64) -> impl Future<Output = u64> {
    lazy(a)
}

/// 构造 + poll —— 副作用发生一次。
#[unsafe(no_mangle)]
pub fn run_lazy(a: u64) -> u64 {
    block_on(lazy(a))
}

// ============================================================
// 状态机的大小
// ============================================================

/// ★ 状态机要容纳最占空间的挂起状态、判别式与对齐；
/// 不同状态中的字段可能复用存储，具体布局是编译器实现细节。
///
/// 实测（`size_of_val`，`-O`，1.98.1）：
///
/// | future | 大小 | 为什么 |
/// |---|---|---|
/// | `two_awaits(1, 2)` | **56** 字节 | 两个 `u64` + 两个 `Ready<u64>` + 判别式/填充 |
/// | `async fn no_await(a: u64) -> u64 { a }` | **16** 字节 | 捕获了 `a: u64`，状态机只有一个状态 |
///
/// ★ 注意 `no_await` 是 **16** 而不是 1 ——
/// 因为它**捕获了参数 `a`**（8 字节）+ 判别式（8 字节对齐）。
/// 如果是一个不捕获任何东西的 `async fn`，状态机会退化成 1 字节。
///
/// ★ 但关键结论不变：**大小由"跨 await 存活的东西"决定。**
/// 这解释了"为什么 `async fn` 不能递归"：
/// **递归调用会让大小无穷大**（每个栈帧都要装进状态机里）。
/// 需要递归时必须 `Box::pin`（把大小变成指针）。
#[unsafe(no_mangle)]
pub fn size_of_two_awaits() -> usize {
    std::mem::size_of_val(&two_awaits(1, 2))
}

/// 只有一个状态（没有 `await`）的 `async fn`。
pub async fn no_await(a: u64) -> u64 { a }

#[unsafe(no_mangle)]
pub fn size_of_no_await() -> usize {
    std::mem::size_of_val(&no_await(1))
}
