//! The port uses Starling's existing Android IME and folder bridge.
use super::Output;
use std::sync::Mutex;
#[derive(Default)]
struct Mirror {
    active: bool,
    generation: i32,
    text: String,
}
static MIRROR: Mutex<Mirror> = Mutex::new(Mirror {
    active: false,
    generation: 0,
    text: String::new(),
});
pub fn active() -> bool {
    MIRROR.lock().unwrap_or_else(|e| e.into_inner()).active
}
pub fn show(text: &str) {
    let mut m = MIRROR.lock().unwrap_or_else(|e| e.into_inner());
    m.active = true;
    m.generation += 1;
    m.text = text.to_owned();
    let len = text.chars().count() as i32;
    crate::android::text_mirror(true, text, len, len, 256, 6 << 8, m.generation);
}
pub fn hide() {
    let mut m = MIRROR.lock().unwrap_or_else(|e| e.into_inner());
    m.active = false;
    m.generation += 1;
    crate::android::hide_text_mirror();
}
pub fn poll() -> Vec<Output> {
    if !active() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for event in crate::android::take_mirror_events() {
        use crate::android::MirrorEvent as E;
        match event {
            E::Edit {
                text, generation, ..
            } => {
                let mut m = MIRROR.lock().unwrap_or_else(|e| e.into_inner());
                if generation != m.generation {
                    continue;
                }
                let common = m
                    .text
                    .chars()
                    .zip(text.chars())
                    .take_while(|(a, b)| a == b)
                    .count();
                out.extend((common..m.text.chars().count()).map(|_| Output::Backspace));
                let rest: String = text.chars().skip(common).collect();
                if !rest.is_empty() {
                    out.push(Output::Text(rest));
                }
                m.text = text;
            }
            E::Action => out.push(Output::Enter),
            E::Key(111) => out.push(Output::Close),
            E::Key(67) => out.push(Output::Backspace),
            _ => {}
        }
    }
    out
}
pub fn vibrate(p: horizon_client::gamepad::Pulse) {
    crate::android::vibrate(p.millis);
}
pub fn cancel_vibration() {
    crate::android::vibrate(0);
}
pub fn open_documents(folder: &str) -> bool {
    horizon_client::platform::open_folder(&std::path::Path::new(".").join(folder)).unwrap_or(false)
}
pub fn field_at(p: [f32; 2]) -> bool {
    horizon_client::widgets::last_targets().iter().any(|t| {
        let [x, y, w, h] = t.rect;
        matches!(t.kind, horizon_client::widgets::Kind::Field)
            && p[0] >= x
            && p[0] < x + w
            && p[1] >= y
            && p[1] < y + h
    })
}
