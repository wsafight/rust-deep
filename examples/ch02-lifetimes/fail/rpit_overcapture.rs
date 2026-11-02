fn capture_all(_: &()) -> impl Sized {}

pub fn require_static(input: &()) -> impl Sized + 'static {
    capture_all(input)
}
