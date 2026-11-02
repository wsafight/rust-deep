fn require_fn(callback: impl Fn() -> usize) -> usize {
    callback()
}

pub fn demo() -> usize {
    let mut count = 0;
    require_fn(|| {
        count += 1;
        count
    })
}
