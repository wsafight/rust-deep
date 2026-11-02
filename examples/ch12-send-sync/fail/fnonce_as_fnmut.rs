fn call_twice(mut callback: impl FnMut() -> String) {
    drop(callback());
    drop(callback());
}

pub fn demo() {
    let text = String::from("owned");
    call_twice(move || text);
}
