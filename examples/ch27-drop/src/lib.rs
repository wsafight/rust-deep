//! 第 27 章：`Drop`、drop flags 与 panic safety
//!
//! 证据生成：`tools/evidence.sh ch27-drop`
//!
//! 本章区分三件事：
//!
//! 1. **何时析构**：作用域、声明顺序和字段顺序决定；
//! 2. **析构哪个值**：所有权移动后，MIR 的 drop flag 防止重复析构；
//! 3. **异常时如何析构**：可展开的 panic 沿 cleanup edge 释放已初始化值。

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

// ============================================================
// 1) 局部变量与字段的析构顺序不同
// ============================================================

struct Logged<'a> {
    id: u8,
    log: &'a RefCell<Vec<u8>>,
}

impl Drop for Logged<'_> {
    fn drop(&mut self) {
        self.log.borrow_mut().push(self.id);
    }
}

struct Fields<'a> {
    first: Logged<'a>,
    second: Logged<'a>,
}

/// 局部变量按声明的**逆序**析构。
// ANCHOR: local_drop_order
pub fn local_drop_order() -> Vec<u8> {
    let log = RefCell::new(Vec::new());
    {
        let _first = Logged { id: 1, log: &log };
        let _second = Logged { id: 2, log: &log };
    }
    log.into_inner()
}
// ANCHOR_END: local_drop_order

/// 结构体字段按**声明顺序**析构，不是逆序。
pub fn field_drop_order() -> Vec<u8> {
    let log = RefCell::new(Vec::new());
    {
        let fields = Fields {
            first: Logged { id: 1, log: &log },
            second: Logged { id: 2, log: &log },
        };
        std::hint::black_box((&fields.first, &fields.second));
    }
    log.into_inner()
}

// ============================================================
// 2) 条件移动与 drop flag
// ============================================================

pub struct Counted {
    value: u64,
    drops: Arc<AtomicUsize>,
}

impl Counted {
    fn new(value: u64, drops: &Arc<AtomicUsize>) -> Self {
        Self {
            value,
            drops: Arc::clone(drops),
        }
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
}

#[inline(never)]
fn consume(value: Counted) -> u64 {
    value.value
}

/// `value` 只在 `take == true` 时被移动。
///
/// MIR 会保存一位布尔状态：已移动时，函数尾不能再次 drop；未移动时，
/// 函数尾必须 drop。这个状态就是 drop flag 的具体形态。
#[unsafe(no_mangle)]
pub fn conditional_move(take: bool) -> usize {
    let drops = Arc::new(AtomicUsize::new(0));
    {
        let value = Counted::new(7, &drops);
        if take {
            let _ = consume(value);
        }
    }
    drops.load(Ordering::Relaxed)
}

// ============================================================
// 3) panic 展开也必须执行析构
// ============================================================

/// 无论正常返回还是 panic 展开，`guard` 都恰好析构一次。
///
/// MIR 中正常路径和 cleanup 路径都会走 `drop(_guard)`；cleanup 路径随后
/// `resume`，把 panic 继续向上传播。
#[unsafe(no_mangle)]
pub fn unwind_cleanup(should_panic: bool, drops: Arc<AtomicUsize>) -> u64 {
    let _guard = Counted::new(9, &drops);
    if should_panic {
        panic!("requested panic");
    }
    9
}

pub fn catch_and_count() -> usize {
    let drops = Arc::new(AtomicUsize::new(0));
    let result = std::panic::catch_unwind({
        let drops = Arc::clone(&drops);
        move || unwind_cleanup(true, drops)
    });
    assert!(result.is_err());
    drops.load(Ordering::Relaxed)
}

/// `mem::forget` 是安全函数，因为 Rust 从不承诺析构器一定运行。
/// 代价是资源泄漏，而不是内存不安全。
pub fn forget_skips_drop() -> usize {
    let drops = Arc::new(AtomicUsize::new(0));
    std::mem::forget(Counted::new(1, &drops));
    drops.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locals_are_dropped_in_reverse_order() {
        assert_eq!(local_drop_order(), vec![2, 1]);
    }

    #[test]
    fn fields_are_dropped_in_declaration_order() {
        assert_eq!(field_drop_order(), vec![1, 2]);
    }

    #[test]
    fn a_moved_value_is_not_dropped_twice() {
        assert_eq!(conditional_move(true), 1);
        assert_eq!(conditional_move(false), 1);
    }

    #[test]
    fn cleanup_runs_during_unwind() {
        assert_eq!(catch_and_count(), 1);
    }

    #[test]
    fn forgetting_is_a_leak_not_ub() {
        assert_eq!(forget_skips_drop(), 0);
    }
}
