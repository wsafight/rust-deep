use ch11_project_abstract::errors::{BoxError, ConfigError, load_limit, parse_limit};
use std::error::Error;

#[test]
fn parsing_keeps_a_typed_error_and_its_source() {
    assert_eq!(parse_limit(&b" 42 \n"[..]).unwrap(), 42);
    let error = parse_limit(&b"no number"[..]).unwrap_err();
    assert!(matches!(error, ConfigError::Parse(_)));
    assert!(error.source().is_some());
    assert!(error.to_string().starts_with("parsing limit:"));
}

#[test]
fn io_errors_use_a_different_variant() {
    let error = parse_limit(&[0xff][..]).unwrap_err();
    assert!(matches!(error, ConfigError::Io(_)));
}

#[test]
fn erased_errors_keep_the_thread_safe_contract() {
    fn require_thread_safe<T: Send + Sync + 'static>(_: &T) {}
    let error: BoxError = load_limit(&b"invalid"[..]).unwrap_err();
    require_thread_safe(&error);
    assert!(error.downcast_ref::<ConfigError>().is_some());
}
