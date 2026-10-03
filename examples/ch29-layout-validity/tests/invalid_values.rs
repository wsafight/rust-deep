#![cfg(miri)]

use std::mem::MaybeUninit;

#[test]
fn byte_two_is_not_a_valid_bool() {
    let mut slot = MaybeUninit::<bool>::uninit();
    let byte = std::hint::black_box(2u8);

    // Write initialized bytes without claiming that they already form a bool.
    unsafe { slot.as_mut_ptr().cast::<u8>().write(byte) };

    // UB: bool only permits the initialized bit patterns 0 and 1.
    let invalid = unsafe { slot.assume_init() };
    std::hint::black_box(invalid);
}
