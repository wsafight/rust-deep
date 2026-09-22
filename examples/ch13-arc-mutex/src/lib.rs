//! 第 13 章：Arc 与 Mutex 的指令级实现
//!
//! 证据生成：tools/evidence.sh ch13-arc-mutex

use std::sync::Arc;

/// 应生成 ldadd（原子 fetch-add）+ 溢出分支
pub fn clone_arc(a: &Arc<u64>) -> u64 {
    let b = Arc::clone(a);
    *b
}

use std::sync::Mutex;

/// ★ 平台差异：macOS 上 `Mutex` 走 **`pthread_mutex`**，不是 futex。
/// Linux、Android、FreeBSD 等目标在当前 std 中走 futex 实现。实测调用链：
///   `Mutex::lock` -> `...pal::unix::sync::mutex::Mutex::lock` -> `pthread_mutex_lock`
/// 锁类型是 `PTHREAD_MUTEX_NORMAL`（std 源码里显式设置，见
/// `library/std/src/sys/pal/unix/sync/mutex.rs`）。
///
/// ⚠️ 平台分派属于标准库实现细节，升级工具链后应重新核对。
/// **"Mutex = futex" 是一个常见的过度简化。**
#[unsafe(no_mangle)]
pub fn lock_mutex(m: &Mutex<u64>) -> u64 {
    *m.lock().unwrap()
}

/// `try_lock` 是**无阻塞**路径：失败立即返回 `None`，不进 pthread 等待
#[unsafe(no_mangle)]
pub fn try_lock_mutex(m: &Mutex<u64>) -> Option<u64> {
    m.try_lock().ok().map(|g| *g)
}
