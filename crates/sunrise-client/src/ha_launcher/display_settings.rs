//! The launcher's controls for the selected game's display settings.
use super::{Action, DisplayOption, Icon, SettingControl};
use crate::ha_launcher::client::settings::Settings;

pub(super) fn values(settings: &Settings) -> [String; DisplayOption::ALL.len()] {
    DisplayOption::ALL.map(|option| {
        let value = option.value(settings);
        if option.margin_preview().is_some() {
            format!("{value}%")
        } else {
            format!("{} ›", option.choice_name(value))
        }
    })
}

pub(super) fn rows<'a>(
    settings: &Settings,
    values: &'a [String; DisplayOption::ALL.len()],
) -> Vec<(&'static str, &'static str, SettingControl<'a>)> {
    use DisplayOption as D;
    let mut rows = Vec::new();
    for (option, label) in D::ALL.into_iter().zip(values) {
        let section = match option {
            D::SafeHorizontal => Some("HUD Safe Zone"),
            D::ButtonsHorizontal => Some("Button Safe Zone"),
            D::InterfaceHorizontal => Some("Interface Safe Zone"),
            _ => None,
        };
        if let Some(title) = section {
            rows.push((
                title,
                "Game margins from the current window edges",
                SettingControl::SectionHeader("Display", Icon::Monitor),
            ));
        }
        let control = if option.margin_preview().is_some() {
            SettingControl::Slider {
                value_label: label,
                fraction: option.value(settings) as f32 / option.max() as f32,
                minus_action: Action::DisplayMarginDelta(option, -1),
                plus_action: Action::DisplayMarginDelta(option, 1),
            }
        } else {
            SettingControl::Button(label, Action::GameDisplay(option), false)
        };
        rows.push((
            option.name(),
            "Shared with the game's Video Settings",
            control,
        ));
    }
    rows
}
