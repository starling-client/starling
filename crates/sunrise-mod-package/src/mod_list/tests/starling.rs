//! Starling's tests of mod_list.rs, out of Sunrise's file (tools/starling/move_out.py).
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// A native mod replaced under the same id makes the build out of date; a build from before
/// builds said their request goes by the ids, as it always did.
#[test]
fn a_changed_native_mod_of_the_same_id_needs_a_new_compile() {
    let ids = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
    let voice = ids(&["voicechat"]);
    assert!(native_build_current(
        Some("aaa"),
        &voice,
        &voice,
        Some("aaa")
    ));
    assert!(!native_build_current(
        Some("aaa"),
        &voice,
        &voice,
        Some("bbb")
    ));
    assert!(native_build_current(None, &voice, &voice, Some("bbb")));
    assert!(!native_build_current(
        None,
        &voice,
        &ids(&["voicechat", "wrench"]),
        Some("bbb")
    ));
    assert!(native_build_current(Some("aaa"), &voice, &voice, None));
}
