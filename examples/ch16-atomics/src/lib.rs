//! 第 16 章：无锁与 atomics —— 内存序如何落到具体指令
//!
//! 证据生成：tools/evidence.sh ch16-atomics
//!
//! 本章的核心：**内存序不是抽象概念，它决定生成哪条指令**。
//! AArch64 有独立的内存序指令（`ldar` / `ldapr` / `stlr` / `ldaddal`），
//! 所以不同 Ordering 生成**不同的指令**；而 x86 的强内存模型下，
//! 大部分 Ordering 会退化成同一条 `mov` —— 这是两个架构最漂亮的对照。

use std::sync::atomic::{AtomicU64, Ordering};

/// `Relaxed` 读 → 普通 `ldr`（**没有任何内存序语义**）
#[unsafe(no_mangle)]
pub fn load_relaxed(a: &AtomicU64) -> u64 {
    a.load(Ordering::Relaxed)
}

/// `Acquire` 读 → `ldapr`（AArch64 的 acquire-load 专用指令）
#[unsafe(no_mangle)]
pub fn load_acquire(a: &AtomicU64) -> u64 {
    a.load(Ordering::Acquire)
}

/// `SeqCst` 读 → `ldar`
#[unsafe(no_mangle)]
pub fn load_seqcst(a: &AtomicU64) -> u64 {
    a.load(Ordering::SeqCst)
}

/// `Release` 写 → `stlr`（store-release 专用指令）
#[unsafe(no_mangle)]
pub fn store_release(a: &AtomicU64, v: u64) {
    a.store(v, Ordering::Release);
}

/// `Relaxed` 写 → 普通 `str`
#[unsafe(no_mangle)]
pub fn store_relaxed(a: &AtomicU64, v: u64) {
    a.store(v, Ordering::Relaxed);
}

/// `fetch_add(SeqCst)` → `ldaddal`（原子加 + acquire-release 语义）
#[unsafe(no_mangle)]
pub fn fetch_add_seqcst(a: &AtomicU64) -> u64 {
    a.fetch_add(1, Ordering::SeqCst)
}

/// `fetch_add(Relaxed)` → `ldadd`（同样的原子性，但没有内存序）
#[unsafe(no_mangle)]
pub fn fetch_add_relaxed(a: &AtomicU64) -> u64 {
    a.fetch_add(1, Ordering::Relaxed)
}

/// CAS → `casal` + `cmp`/`cset`（比较结果由旧值决定）
#[unsafe(no_mangle)]
pub fn compare_exchange(a: &AtomicU64, expected: u64, new: u64) -> bool {
    a.compare_exchange(expected, new, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}
