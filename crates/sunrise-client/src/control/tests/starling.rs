//! Starling's tests of control.rs, out of Sunrise's file (tools/starling/move_out.py).
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

#[test]
fn base64_round_trips_and_remote_endpoints_are_marked() {
    for bytes in [
        &b""[..],
        b"f",
        b"fo",
        b"foo",
        b"foobar",
        &[0, 255, 128, 7, 64],
    ] {
        assert_eq!(unbase64(&base64(bytes)).as_deref(), Some(bytes));
    }
    assert_eq!(unbase64("Zm9v!"), None);
    let path = std::env::temp_dir().join(format!("sunrise-ctl-remote-{}.json", std::process::id()));
    std::fs::write(&path, r#"{"port":47800,"token":"t","remote":true}"#).unwrap();
    assert!(is_remote(&path));
    std::fs::write(&path, r#"{"port":47800,"token":"t"}"#).unwrap();
    assert!(!is_remote(&path));
    std::fs::remove_file(&path).unwrap();
}
