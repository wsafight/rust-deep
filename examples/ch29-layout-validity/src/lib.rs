//! 第 29 章：内存布局、有效值与 `MaybeUninit`
//!
//! 证据生成：`tools/evidence.sh ch29-layout-validity`
//!
//! `size_of::<T>()` 只回答占多少字节；soundness 还要求地址满足对齐、字节已经
//! 初始化，并且 bit pattern 是 `T` 的有效值。

use std::mem::{MaybeUninit, align_of, offset_of, size_of};
use std::num::NonZeroU64;

pub mod ffi;

// ============================================================
// 1) repr(Rust) 与 repr(C)
// ============================================================

/// 默认布局只承诺字段正确对齐、字段不重叠、类型整体对齐正确。
/// 字段的实际偏移是当前 rustc 的观察值，不是稳定 ABI。
pub struct RustHeader {
    pub tag: u8,
    pub count: u32,
    pub kind: u16,
}

/// `repr(C)` 按目标平台的 C ABI 决定字段顺序、padding 和对齐。
#[repr(C)]
pub struct CHeader {
    pub tag: u8,
    pub count: u32,
    pub kind: u16,
}

/// 把 size / align / 三个字段偏移编码进一个 u64，便于测试当前工具链。
const fn encode_layout(size: usize, align: usize, a: usize, b: usize, c: usize) -> u64 {
    (size as u64)
        | ((align as u64) << 8)
        | ((a as u64) << 16)
        | ((b as u64) << 24)
        | ((c as u64) << 32)
}

#[unsafe(no_mangle)]
pub const fn rust_layout() -> u64 {
    encode_layout(
        size_of::<RustHeader>(),
        align_of::<RustHeader>(),
        offset_of!(RustHeader, tag),
        offset_of!(RustHeader, count),
        offset_of!(RustHeader, kind),
    )
}

#[unsafe(no_mangle)]
pub const fn c_layout() -> u64 {
    encode_layout(
        size_of::<CHeader>(),
        align_of::<CHeader>(),
        offset_of!(CHeader, tag),
        offset_of!(CHeader, count),
        offset_of!(CHeader, kind),
    )
}

// ============================================================
// 2) niche：无效值也可以成为布局资源
// ============================================================

#[repr(transparent)]
pub struct UserId(pub NonZeroU64);

/// 每个字节依次编码一个类型的大小。
///
/// 当前目标上：
/// - `Option<u64>` = 16：`u64` 的每个 bit pattern 都有效，没有 niche；
/// - `Option<NonZeroU64>` = 8：0 可以表示 `None`；
/// - `Option<UserId>` = 8：transparent wrapper 保留内部有效值约束；
/// - `Option<bool>` = 1：bool 只有 0/1 有效；
/// - `Option<MaybeUninit<bool>>` = 2：MaybeUninit 允许任意字节，niche 消失。
#[unsafe(no_mangle)]
pub const fn niche_sizes() -> u64 {
    (size_of::<Option<u64>>() as u64)
        | ((size_of::<Option<NonZeroU64>>() as u64) << 8)
        | ((size_of::<Option<UserId>>() as u64) << 16)
        | ((size_of::<Option<bool>>() as u64) << 24)
        | ((size_of::<Option<MaybeUninit<bool>>>() as u64) << 32)
}

// ============================================================
// 3) MaybeUninit：允许未初始化，不等于已经得到 T
// ============================================================

/// 正确顺序：先 `write` 建立一个有效 `u64`，再 `assume_init`。
#[unsafe(no_mangle)]
pub fn write_then_assume(value: u64) -> u64 {
    let mut slot = MaybeUninit::<u64>::uninit();
    slot.write(value);
    // SAFETY: 上一行已经用一个有效 u64 初始化了 slot，之后没有移动或覆盖它。
    unsafe { slot.assume_init() }
}

/// `MaybeUninit<T>` 与 `T` 具有相同大小和对齐，但放弃了 `T` 的有效值约束。
#[unsafe(no_mangle)]
pub const fn maybe_uninit_layout() -> u64 {
    (size_of::<MaybeUninit<CHeader>>() as u64)
        | ((align_of::<MaybeUninit<CHeader>>() as u64) << 16)
        | ((size_of::<CHeader>() as u64) << 32)
        | ((align_of::<CHeader>() as u64) << 48)
}

// ============================================================
// 4) packed：布局更紧，但字段可能未对齐
// ============================================================

#[repr(C, packed)]
pub struct PackedHeader {
    pub tag: u8,
    pub count: u32,
}

/// 不能创建 `&packed.count`，因为引用必须对齐。先用 `addr_of!` 得到裸指针，
/// 再执行显式的未对齐读取。
#[unsafe(no_mangle)]
pub fn read_packed_count(header: &PackedHeader) -> u32 {
    // SAFETY: 指针来自有效 header，字段的 4 个字节可读；read_unaligned 不要求对齐。
    unsafe { std::ptr::addr_of!(header.count).read_unaligned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_layout_has_abi_padding() {
        assert_eq!(size_of::<CHeader>(), 12);
        assert_eq!(align_of::<CHeader>(), 4);
        assert_eq!(offset_of!(CHeader, tag), 0);
        assert_eq!(offset_of!(CHeader, count), 4);
        assert_eq!(offset_of!(CHeader, kind), 8);
    }

    #[test]
    fn rust_layout_is_an_observation_not_a_contract() {
        assert_eq!(size_of::<RustHeader>(), 8);
        assert_eq!(rust_layout(), encode_layout(8, 4, 6, 0, 4));
    }

    #[test]
    fn niches_change_option_size() {
        assert_eq!(size_of::<Option<u64>>(), 16);
        assert_eq!(size_of::<Option<NonZeroU64>>(), 8);
        assert_eq!(size_of::<Option<UserId>>(), 8);
        assert_eq!(size_of::<Option<bool>>(), 1);
        assert_eq!(size_of::<Option<MaybeUninit<bool>>>(), 2);
    }

    #[test]
    fn maybe_uninit_preserves_size_and_alignment() {
        assert_eq!(size_of::<MaybeUninit<CHeader>>(), size_of::<CHeader>());
        assert_eq!(align_of::<MaybeUninit<CHeader>>(), align_of::<CHeader>());
        assert_eq!(write_then_assume(42), 42);
    }

    #[test]
    fn packed_fields_need_unaligned_access() {
        let header = PackedHeader {
            tag: 1,
            count: 0x1122_3344,
        };
        assert_eq!(read_packed_count(&header), 0x1122_3344);
    }
}
