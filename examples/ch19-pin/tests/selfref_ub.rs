//! 自引用结构：**错误版本** —— 只给 Miri 跑。
//!
//! 复现：
//!   cargo +nightly miri test -p ch19-pin --test selfref_ub    # 必须失败
//!
//! 这里演示的是 `Pin` 文档里那句话的**具体形态**：
//! "自引用指针必须从 `&mut` 派生，不能从 `&` 派生。"

#![cfg(miri)]

use std::marker::PhantomPinned;
use std::pin::Pin;

/// 本地复刻一个自引用结构（字段公开，方便直接构造出错误状态）。
///
/// 它和 `ch19_pin::SelfRef` 布局完全一样 —— 区别只在：
/// 这里我们**故意**用错误的方式给它填 `self_ref`。
pub struct RawSelfRef {
    pub data: u64,
    pub self_ref: *mut u64,
    pub _pin: PhantomPinned,
}

impl RawSelfRef {
    pub fn read(&self) -> u64 {
        // SAFETY（伪）：在下面的用例里，这个指针已经失效
        unsafe { *self.self_ref }
    }
}

/// 把**值本身**搬到另一个地址 —— 自引用指针随即悬垂。
///
/// 注意这里必须让**旧的分配真的被释放**（`drop(a)`），
/// 否则 Miri 只会把新地址当成同一块内存的另一个 tag，**不报 UB** ——
/// 这也是一条值得知道的实测结论：
/// **"移动 = memcpy"不必然当场暴露，只有旧地址不再有效时才暴露。**
/// 这正是自引用结构的危险之处：它可能"碰巧能跑"。
///
/// Miri 报：
/// `Undefined Behavior: memory access failed: alloc<N> has been freed,
///  so this pointer is dangling`
#[test]
fn value_moved_between_allocations() {
    // 先在堆上构造：self_ref 指向第一块分配
    let mut a = Box::new(RawSelfRef {
        data: 5,
        self_ref: std::ptr::null_mut(),
        _pin: PhantomPinned,
    });
    a.self_ref = std::ptr::addr_of_mut!(a.data);

    // 把**值**搬进新的 Box，并释放旧的
    let b = Box::new(RawSelfRef { data: a.data, self_ref: a.self_ref, _pin: PhantomPinned });
    drop(a); // ← 旧地址被释放；此时 b.self_ref 是悬垂指针

    // SAFETY（伪）：b.self_ref 指向已释放的内存
    let v = b.read();
    assert_eq!(v, 5); // ← 读悬垂指针：UB
}

/// 自引用指针从**共享借用**派生（`&x` 而不是 `&mut x`）——
/// 之后任何一次写入都会把它作废。
///
/// 这与 `make_self_ref` 里用 `get_unchecked_mut` + `addr_of_mut!` 的做法
/// 形成对照：那里指针从 `&mut` 派生，权限足够强。
///
/// ★ 这是本章最值得记住的一条：
/// **两个逐字节等价的写法（`addr_of!` / `addr_of_mut!`）生成同样的汇编，
/// 但一个是 sound 的，一个是 UB。** 汇编看不出来，只有 Miri 能分辨。
///
/// Miri 报：
/// `Undefined Behavior: attempting a read access using <N> at alloc<N>[0x0],
///  but that tag does not exist in the borrow stack for this location`
#[test]
fn pointer_derived_from_shared_ref() {
    let mut x = 1u64;
    // 从共享借用派生裸指针（&x 产生的 SharedReadOnly tag）
    let p: *const u64 = &x;
    // 通过另一条路径写 —— 共享借用被弹掉
    let m: *mut u64 = &mut x;
    unsafe { *m = 2 };
    // SAFETY（伪）：p 已经不在借用栈里了
    let v = unsafe { *p };
    assert_eq!(v, 2); // ← 用失效的指针：UB
}
