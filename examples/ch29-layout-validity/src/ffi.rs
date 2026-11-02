#[repr(C)]
pub struct AddResult {
    pub value: i32,
    pub status: u8,
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_deep_checked_add(left: i32, right: i32) -> AddResult {
    match left.checked_add(right) {
        Some(value) => AddResult { value, status: 0 },
        None => AddResult {
            value: 0,
            status: 1,
        },
    }
}
