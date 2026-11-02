use ch12_send_sync::{call_twice, counter, owned_once, owned_reader};

#[test]
fn move_does_not_imply_fnonce_only() {
    let reader = owned_reader(String::from("Rust"));
    assert_eq!(call_twice(&reader), (4, 4));
    assert_eq!(reader(), 4);
}

#[test]
fn fnmut_can_update_owned_state() {
    assert_eq!(call_twice(counter()), (1, 2));
}

#[test]
fn fnonce_can_move_a_captured_value_out() {
    assert_eq!(owned_once(String::from("returned"))(), "returned");
}
