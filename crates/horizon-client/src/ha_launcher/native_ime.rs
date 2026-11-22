//! Horizon Android's keyboard, pointer and screen bridge, over Starling's: StarlingActivity already
//! owns the keyboard's editor (TextMirror), the system bars, the camera cutout's insets, the
//! pointer's capture and the file picker, and two editors in one activity would fight over the
//! keyboard. Horizon Android's own (an EditText it added through JNI) is not used.
use std::sync::Mutex;

/// The keyboard's editor flags (TextMirror.java): the Done action, and a password's.
const DONE: i32 = 6 << 8;
const PASSWORD: i32 = 4;

struct Editor {
    active: bool,
    generation: i32,
    text: String,
    /// The keyboard's Enter (its action key) or Back since the last poll.
    keys: Vec<Key>,
}
#[derive(Clone, Copy)]
pub enum Key {
    Enter,
    Back,
    Backspace,
}
static EDITOR: Mutex<Editor> = Mutex::new(Editor {
    active: false,
    generation: 0,
    text: String::new(),
    keys: Vec::new(),
});

fn editor() -> std::sync::MutexGuard<'static, Editor> {
    EDITOR.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn show(text: &str) -> bool {
    show_mode(text, false)
}
pub fn show_secret(text: &str) -> bool {
    show_mode(text, true)
}
fn show_mode(text: &str, secret: bool) -> bool {
    let mut e = editor();
    e.active = true;
    e.generation += 1;
    e.text = text.to_owned();
    e.keys.clear();
    let len = text.chars().count() as i32;
    crate::android::text_mirror(
        true,
        text,
        len,
        len,
        0,
        DONE | if secret { PASSWORD } else { 0 },
        e.generation,
    );
    true
}
pub fn hide() {
    let mut e = editor();
    if e.active {
        e.active = false;
        e.generation += 1;
        crate::android::hide_text_mirror();
    }
}
/// The editor's text now, None while it is hidden.
pub fn text() -> Option<String> {
    let mut e = editor();
    if !e.active {
        return None;
    }
    for event in crate::android::take_mirror_events() {
        use crate::android::MirrorEvent as E;
        match event {
            E::Edit {
                text, generation, ..
            } if generation == e.generation => e.text = text,
            E::Edit { .. } => {}
            E::Action => e.keys.push(Key::Enter),
            // KEYCODE_BACK and KEYCODE_DEL with nothing left to delete.
            E::Key(4 | 111) => e.keys.push(Key::Back),
            E::Key(67) => e.keys.push(Key::Backspace),
            E::Key(_) => {}
        }
    }
    Some(e.text.clone())
}
/// The keys the keyboard sent since the last call, after [`text`].
pub fn take_keys() -> Vec<Key> {
    std::mem::take(&mut editor().keys)
}

/// StarlingActivity captures the pointer on its UI thread.
pub fn capture_mouse(capture: bool) -> bool {
    crate::android::capture_pointer(capture);
    true
}

/// Where the screen's visible part ends above the keyboard, in window pixels; None without one.
pub fn viewport_bottom() -> Option<u32> {
    let keyboard = crate::android::ime_height();
    if keyboard == 0 {
        return None;
    }
    let height = crate::android::android_app()
        .native_window()?
        .height()
        .max(0) as u32;
    Some(height.saturating_sub(keyboard))
}

/// The system's file picker, whose answer comes back to `documents::document_result`
/// (StarlingActivity.pickDocument, request 4101).
pub fn pick_document() -> bool {
    crate::android::with_activity(|env, activity| {
        env.call_method(activity, "pickDocument", "()V", &[])
            .map(drop)
    })
    .is_some()
}

/// StarlingActivity keeps the system bars hidden and the cutout's insets for the game.
pub fn configure_screen(_immersive: bool, _avoid_cutout: bool) {}
pub fn refresh_bounds() {}
