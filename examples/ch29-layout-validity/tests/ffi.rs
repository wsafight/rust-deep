use ch29_layout_validity::ffi::{AddResult, rust_deep_checked_add};
use std::mem::{align_of, offset_of, size_of};

#[test]
fn the_c_boundary_reports_errors_without_unwinding() {
    let sum = rust_deep_checked_add(20, 22);
    assert_eq!((sum.value, sum.status), (42, 0));
    let overflow = rust_deep_checked_add(i32::MAX, 1);
    assert_eq!((overflow.value, overflow.status), (0, 1));
    assert_eq!(offset_of!(AddResult, value), 0);
    assert_eq!(offset_of!(AddResult, status), 4);
    assert_eq!(align_of::<AddResult>(), align_of::<i32>());
    assert_eq!(size_of::<AddResult>(), 8);
}
