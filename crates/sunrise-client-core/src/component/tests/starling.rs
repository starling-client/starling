//! Starling's tests of component.rs, out of Sunrise's file (tools/starling/move_out.py).
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

#[test]
fn the_shadow_colour_reads_as_an_int_or_four_floats_and_is_inherited() {
    let v = serde_json::json!({"text": "a", "shadow_color": 0, "extra": ["b"]});
    let t = to_text(&v, &mut |_| Text::empty());
    let shadows: Vec<_> = t.chars().map(|(_, s)| s.shadow_color).collect();
    assert_eq!(shadows, [Some(0), Some(0)]);
    let v = serde_json::json!({"text": "c", "shadow_color": [1.0, 0.0, 0.0, 0.5]});
    let t = to_text(&v, &mut |_| Text::empty());
    assert_eq!(t.spans[0].style.shadow_color, Some(0x7F_FF_00_00));
    let again = to_text(&t.component(), &mut |_| Text::empty());
    assert_eq!(again.spans[0].style.shadow_color, Some(0x7F_FF_00_00));
}
