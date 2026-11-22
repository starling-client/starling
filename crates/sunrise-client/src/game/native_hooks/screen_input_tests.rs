#![cfg(test)]

/// The characters a `ScreenEvent.CharacterTyped.Pre` sees are the
/// printable ones of a text: a cancelled one leaves that text and a
/// control character (a key, not text) is never offered a hook.
#[test]
fn typed_text_keeps_control_characters_and_drops_cancelled_ones() {
    let mut offered = Vec::new();
    let left = super::screen_text_hooked("a\u{7f}b\u{1b}c", |c| {
        offered.push(c);
        c == 'b'
    });
    assert_eq!(offered, ['a', 'b', 'c']);
    assert_eq!(left.as_deref(), Some("a\u{7f}\u{1b}c"));
    // Every printable character cancelled: no text is left to type.
    assert_eq!(super::screen_text_hooked("ab", |_| true), None);
    // Nothing to offer: an empty text stays empty and keys alone stay whole.
    assert_eq!(super::screen_text_hooked("", |_| true).as_deref(), Some(""));
    assert_eq!(
        super::screen_text_hooked("\u{7f}\t", |_| true).as_deref(),
        Some("\u{7f}\t")
    );
    // What no hook cancels is handed back as it came in.
    assert_eq!(
        super::screen_text_hooked("hello", |_| false).as_deref(),
        Some("hello")
    );
}

/// The window's key path: only a key's printable characters go on to the
/// hook and the text box.
#[test]
fn a_key_offers_only_its_printable_characters() {
    // Backspace, Escape and the like are keys, not text.
    assert_eq!(super::chars_to_type("a\u{7f}\u{1b}b", true), "ab");
    // Nothing typed: nothing to hand over.
    assert_eq!(super::chars_to_type("", true), "");
    // Ctrl+A is a shortcut: its `a` reaches no hook and no box.
    assert_eq!(super::chars_to_type("a", false), "");
}
