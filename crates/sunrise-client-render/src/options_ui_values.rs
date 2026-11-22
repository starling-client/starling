//! The small option value labels (`generic`, `percent`, `on_off`, `chat_px`,
//! `chat_delay_label`) and `Options.chatDelay`'s slider mapping.
use super::*;

pub(crate) fn generic(caption: &str, value: Text) -> Text {
    Text::translate("options.generic_value", &[tr(caption), value])
}
pub(crate) fn on_off(v: bool) -> Text {
    tr(if v { "options.on" } else { "options.off" })
}
pub(crate) fn chat_px(caption: &str, px: i32) -> Text {
    Text::translate(
        "options.pixel_value",
        &[tr(caption), Text::literal(px.to_string())],
    )
}
pub(crate) fn percent(caption: &str, value: f64) -> Text {
    Text::translate(
        "options.percent_value",
        &[
            tr(caption),
            Text::literal(((value * 100.0) as i32).to_string()),
        ],
    )
}
/// `Options.chatDelay`'s message: `options.chat.delay_none` at zero, else
/// `options.chat.delay` with the delay to one decimal (`%.1f` seconds).
pub(crate) fn chat_delay_label(s: &Settings) -> Text {
    let value = s.chat_delay;
    if value <= 0.0 {
        Text::translate("options.chat.delay_none", &[])
    } else {
        Text::translate_with_fallback(
            "options.chat.delay",
            "Chat Delay: %s second(s)",
            &[Text::literal(sunrise_core::java_number::format_fixed(
                value, 1,
            ))],
        )
    }
}
/// `IntRange(0, 60).toSliderValue` on the stored seconds (`xmap(v / 10.0)`):
/// the tenth-of-a-second step over the slider's 60.
pub(crate) fn kind(value: f64) -> Kind {
    Kind::Slider(((value * 10.0).floor() / 60.0).clamp(0.0, 1.0))
}
/// `IntRange(0, 60).fromSliderValue` then `xmap`'s `/ 10.0`:
/// floor(map(v, 0, 1, 0, 61)), the last step capped at 60, in tenths.
pub(crate) fn from_slider(v: f64) -> f64 {
    (v.clamp(0.0, 1.0) * 61.0).floor().min(60.0) / 10.0
}
