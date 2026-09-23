use ch14_channels::{send_across_thread, send_arc, send_string};

#[test]
fn ownership_transfer_examples_return_expected_lengths() {
    assert_eq!(send_string(), 10);
    assert_eq!(send_arc(), 10);
    assert_eq!(send_across_thread(), 3);
}
