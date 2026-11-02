use ch02_lifetimes::{borrowed_chars, capture_none, owned_length};

fn require_static<T: 'static>(_: T) {}

#[test]
fn precise_capture_releases_an_unneeded_input_borrow() {
    let marker = ();
    require_static(capture_none(&marker));
    let owned = {
        let input = String::from("temporary");
        owned_length(&input)
    };
    require_static(owned);
}

#[test]
fn precise_capture_can_keep_a_required_lifetime() {
    let input = String::from("Rust");
    assert_eq!(borrowed_chars(&input).collect::<String>(), input);
}
