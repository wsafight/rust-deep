#![deny(invalid_value)]

pub fn fake_reference() -> &'static u64 {
    // A reference must be non-null and aligned even if it is never dereferenced.
    unsafe { std::mem::zeroed() }
}
