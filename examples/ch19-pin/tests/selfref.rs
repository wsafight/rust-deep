//! 自引用结构：**合法版本**。
//!
//! 普通 `cargo test` 与 `cargo +nightly miri test` 都必须通过。普通测试用于
//! 回归功能与地址稳定性；Miri 额外检查当前模型能够发现的 UB。
//!
//! 复现：
//!   cargo +nightly miri test -p ch19-pin --test selfref      # 必须通过
//!   cargo +nightly miri test -p ch19-pin --test selfref_ub   # 必须失败

use ch19_pin::{make_self_ref, read_self_ref, SelfRef};

/// ★ 核心：自引用结构**不移动就正确**。
///
/// `make_self_ref` 里那条自引用指针来自 `Pin::get_unchecked_mut()`
/// 拿到的 `&mut SelfRef` —— 权限足够强，之后再写 `data` 也不会作废它。
#[test]
fn self_ref_is_correct() {
    let p = make_self_ref(7);
    assert_eq!(read_self_ref(&p), 7);
}

/// 公共构造器必须让自引用指向返回对象本身，而不是某个临时对象。
#[test]
fn public_constructor_points_into_returned_value() {
    let p = SelfRef::new(11);
    assert_eq!(p.read_via_self_ref(), 11);

    // 移动的是 Pin<Box<_>> 这个句柄，堆上的 SelfRef 地址保持不变。
    let q = p;
    assert_eq!(q.read_via_self_ref(), 11);
}

/// 钉住之后**改字段**是允许的（改的不是被自引用指着的那块，或者
/// 即便改的是 `data`，只要指针派生自 `&mut` 就仍然有效）。
#[test]
fn mutate_pinned_field_is_ok() {
    let mut p = make_self_ref(1);
    unsafe { p.as_mut().get_unchecked_mut() }.data = 42;
    // 指针派生自 `&mut`，重写 data 不会作废它
    assert_eq!(read_self_ref(&p), 42);
}

/// `Box::pin` 钉住之后，**移动 `Pin<Box<_>>` 本身**是安全的 ——
/// 被移动的只是那个指针（8 字节），堆上的值一动没动。
#[test]
fn moving_the_pin_is_fine() {
    let p = make_self_ref(99);
    let q = p; // 只搬走 Box 指针，堆上的 SelfRef 没动
    assert_eq!(read_self_ref(&q), 99);
}

/// 钉住之后地址确实**没有**变（这是 `Box::pin` 能成立的全部依据）。
#[test]
fn address_is_stable() {
    let p = make_self_ref(3);
    let addr_before = &*p as *const SelfRef as usize;
    let q = p; // 移动 Pin<Box<_>>
    let addr_after = &*q as *const SelfRef as usize;
    assert_eq!(addr_before, addr_after, "堆上的值不该被搬动");
}
