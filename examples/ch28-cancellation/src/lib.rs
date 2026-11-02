//! 第 28 章：异步取消安全
//!
//! 证据生成：`tools/evidence.sh ch28-cancellation`
//!
//! Future 的取消没有隐藏协议：executor 停止 poll 并 drop 状态机。析构器会运行，
//! 但已经发生的外部副作用不会自动回滚。

use std::future::{Future, pending, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

pub struct DropSignal(Arc<AtomicUsize>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

// ============================================================
// 1) 取消 = drop future
// ============================================================

/// 进入挂起点前构造 guard。任务被取消时，guard 随状态机一起析构。
#[unsafe(no_mangle)]
pub async fn wait_forever(dropped: Arc<AtomicUsize>, entered: Arc<Notify>) {
    let _guard = DropSignal(dropped);
    entered.notify_one();
    pending::<()>().await;
}

/// `abort` 只是发出取消请求；等待 `JoinHandle` 后，才知道状态机已完成清理。
pub async fn abort_runs_drop() -> usize {
    let dropped = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(Notify::new());
    let task = tokio::spawn(wait_forever(Arc::clone(&dropped), Arc::clone(&entered)));

    entered.notified().await;
    task.abort();
    let error = task
        .await
        .expect_err("aborted task must not complete normally");
    assert!(error.is_cancelled());
    dropped.load(Ordering::SeqCst)
}

// ============================================================
// 2) select! 会 drop 未胜出的分支
// ============================================================

async fn guarded_pending(dropped: Arc<AtomicUsize>) {
    let _guard = DropSignal(dropped);
    pending::<()>().await;
}

pub async fn select_drops_loser() -> usize {
    let dropped = Arc::new(AtomicUsize::new(0));

    tokio::select! {
        biased;
        _ = guarded_pending(Arc::clone(&dropped)) => unreachable!(),
        _ = ready(()) => {}
    }

    dropped.load(Ordering::SeqCst)
}

/// `timeout` 获胜时也会 drop 内层 future。
pub async fn timeout_drops_inner() -> usize {
    let dropped = Arc::new(AtomicUsize::new(0));
    let result = tokio::time::timeout(
        Duration::from_millis(1),
        guarded_pending(Arc::clone(&dropped)),
    )
    .await;
    assert!(result.is_err());
    dropped.load(Ordering::SeqCst)
}

// ============================================================
// 3) 资源安全不等于操作可取消
// ============================================================

/// 反例：先从队列取走元素，再等待。
///
/// future 被 drop 时局部变量 `item` 会正常析构，但队列不会自动恢复；重试同一个
/// 操作会观察到不同状态。因此这个操作不是 cancellation-safe。
pub async fn remove_then_wait(queue: Arc<Mutex<Vec<u64>>>, entered: Arc<Notify>) {
    let item = queue
        .lock()
        .unwrap()
        .pop()
        .expect("queue must not be empty");
    entered.notify_one();
    pending::<()>().await;
    std::hint::black_box(item);
}

/// 安全对照：等待发生在提交之前。
///
/// 取消只能发生在 `.await`，而真正的修改位于最后一个 await 之后；所以一旦修改
/// 开始，该函数会同步完成提交。
pub async fn wait_then_remove(queue: Arc<Mutex<Vec<u64>>>, entered: Arc<Notify>) {
    entered.notify_one();
    pending::<()>().await;
    let item = queue
        .lock()
        .unwrap()
        .pop()
        .expect("queue must not be empty");
    std::hint::black_box(item);
}

async fn abort_after_enter<F>(make_future: impl FnOnce(Arc<Notify>) -> F)
where
    F: Future<Output = ()> + Send + 'static,
{
    let entered = Arc::new(Notify::new());
    let task = tokio::spawn(make_future(Arc::clone(&entered)));
    entered.notified().await;
    task.abort();
    let error = task.await.expect_err("pending task must be cancelled");
    assert!(error.is_cancelled());
}

pub async fn queue_after_unsafe_cancel() -> Vec<u64> {
    let queue = Arc::new(Mutex::new(vec![7]));
    abort_after_enter({
        let queue = Arc::clone(&queue);
        move |entered| remove_then_wait(queue, entered)
    })
    .await;
    Arc::try_unwrap(queue).unwrap().into_inner().unwrap()
}

pub async fn queue_after_safe_cancel() -> Vec<u64> {
    let queue = Arc::new(Mutex::new(vec![7]));
    abort_after_enter({
        let queue = Arc::clone(&queue);
        move |entered| wait_then_remove(queue, entered)
    })
    .await;
    Arc::try_unwrap(queue).unwrap().into_inner().unwrap()
}

async fn read_pair(receiver: &mut tokio::sync::mpsc::UnboundedReceiver<u8>) -> [u8; 2] {
    let first = receiver.recv().await.unwrap();
    let second = receiver.recv().await.unwrap();
    [first, second]
}

pub async fn recreated_read_loses_prefix() -> [u8; 2] {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    for value in [1, 2] {
        sender.send(value).unwrap();
        tokio::select! {
            biased;
            _ = read_pair(&mut receiver) => unreachable!(),
            _ = ready(()) => {}
        }
    }
    sender.send(3).unwrap();
    sender.send(4).unwrap();
    read_pair(&mut receiver).await
}

pub async fn preserved_read_keeps_prefix() -> [u8; 2] {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    sender.send(1).unwrap();
    let read = read_pair(&mut receiver);
    tokio::pin!(read);
    tokio::select! {
        biased;
        _ = &mut read => unreachable!(),
        _ = ready(()) => {}
    }
    sender.send(2).unwrap();
    read.await
}

pub async fn shutdown_and_join() -> usize {
    let dropped = Arc::new(AtomicUsize::new(0));
    let guard = DropSignal(Arc::clone(&dropped));
    let (shutdown, mut stopped) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(async move {
        let _guard = guard;
        stopped.wait_for(|requested| *requested).await.unwrap();
    });
    shutdown.send(true).unwrap();
    task.await.unwrap();
    dropped.load(Ordering::SeqCst)
}
