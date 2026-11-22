//! Android display choices shared by the options UI and the activity.
use crate::options_file::V;
use crate::settings::Settings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayOption {
    Cutout,
    Orientation,
    Rotation,
    Aspect,
    SafeHorizontal,
    SafeVertical,
    InterfaceHorizontal,
    InterfaceVertical,
    ButtonsHorizontal,
    ButtonsVertical,
}

impl DisplayOption {
    pub const ALL: [Self; 10] = [
        Self::Cutout,
        Self::Orientation,
        Self::Rotation,
        Self::Aspect,
        Self::SafeHorizontal,
        Self::SafeVertical,
        Self::ButtonsHorizontal,
        Self::ButtonsVertical,
        Self::InterfaceHorizontal,
        Self::InterfaceVertical,
    ];

    pub const ASPECTS: [(i32, i32, &'static str); 11] = [
        (0, 0, "Native"),
        (1, 1, "1:1"),
        (5, 4, "5:4"),
        (4, 3, "4:3"),
        (3, 2, "3:2"),
        (16, 10, "16:10"),
        (16, 9, "16:9"),
        (18, 9, "18:9"),
        (39, 18, "19.5:9"),
        (20, 9, "20:9"),
        (21, 9, "21:9"),
    ];

    pub const fn key(self) -> &'static str {
        match self {
            Self::Cutout => "starlingDisplayCutout",
            Self::Orientation => "starlingDisplayOrientation",
            Self::Rotation => "starlingDisplayRotation",
            Self::Aspect => "starlingDisplayAspect",
            Self::SafeHorizontal => "starlingSafeHorizontal",
            Self::SafeVertical => "starlingSafeVertical",
            Self::InterfaceHorizontal => "starlingInterfaceHorizontal",
            Self::InterfaceVertical => "starlingInterfaceVertical",
            Self::ButtonsHorizontal => "starlingButtonsHorizontal",
            Self::ButtonsVertical => "starlingButtonsVertical",
        }
    }

    pub const fn max(self) -> i32 {
        match self {
            Self::Cutout => 1,
            Self::Orientation => 2,
            Self::Rotation => 3,
            Self::Aspect => Self::ASPECTS.len() as i32 - 1,
            _ => 20,
        }
    }

    pub fn value(self, settings: &Settings) -> i32 {
        let value = settings.file.int(self.key());
        if self == Self::Cutout {
            // The old third choice used the whole window too.
            i32::from(value == 1)
        } else {
            value.clamp(0, self.max())
        }
    }

    pub fn set(self, settings: &mut Settings, value: i32) {
        settings
            .file
            .set(self.key(), V::Int(value.clamp(0, self.max())));
    }

    pub fn choices(self) -> &'static [&'static str] {
        match self {
            Self::Cutout => &["Full display", "Avoid cutout"],
            Self::Orientation => &["Landscape", "Portrait", "Both"],
            Self::Rotation => &["Automatic", "Inverted automatic", "Fixed", "Fixed reversed"],
            _ => &[],
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Cutout => "Camera Cutout",
            Self::Orientation => "Orientation",
            Self::Rotation => "Rotation",
            Self::Aspect => "Aspect Ratio",
            Self::SafeHorizontal => "Horizontal Safe Margin",
            Self::SafeVertical => "Vertical Safe Margin",
            Self::ButtonsHorizontal => "Horizontal Button Margin",
            Self::ButtonsVertical => "Vertical Button Margin",
            Self::InterfaceHorizontal => "Horizontal Menu Margin",
            Self::InterfaceVertical => "Vertical Menu Margin",
        }
    }

    pub fn choice_name(self, value: i32) -> &'static str {
        if self == Self::Aspect {
            Self::ASPECTS[value as usize].2
        } else {
            self.choices()[value as usize]
        }
    }

    pub const fn margin_preview(self) -> Option<SafeZone> {
        match self {
            Self::SafeHorizontal | Self::SafeVertical => Some(SafeZone::Hud),
            Self::InterfaceHorizontal | Self::InterfaceVertical => Some(SafeZone::Interface),
            Self::ButtonsHorizontal | Self::ButtonsVertical => Some(SafeZone::Buttons),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
pub enum SafeZone {
    Hud,
    Interface,
    Buttons,
}

fn margin(size: f32, percentage: i32) -> f32 {
    (size * percentage.clamp(0, 20) as f32 / 100.0).round()
}

/// Insets from the current window's edges in GUI pixels.
pub fn safe_zone(size: [f32; 2], percentages: [i32; 2]) -> [f32; 4] {
    let [x, y] = std::array::from_fn(|axis| margin(size[axis], percentages[axis]));
    [x, y, x, y]
}

/// Preserve space for fixed-size controls and menus at the selected GUI scale.
pub fn interface_safe_zone(size: [f32; 2], percentages: [i32; 2]) -> [f32; 4] {
    let [x, y] = std::array::from_fn(|axis| {
        let limit = ((size[axis] - [320.0, 240.0][axis]) / 2.0).floor().max(0.0);
        margin(size[axis], percentages[axis]).min(limit)
    });
    [x, y, x, y]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn margins_follow_the_current_window() {
        assert_eq!(safe_zone([602.0, 270.0], [0, 0]), [0.0; 4]);
        assert_eq!(safe_zone([602.0, 270.0], [10, 5]), [60.0, 14.0, 60.0, 14.0]);
        assert_eq!(safe_zone([583.0, 270.0], [10, 5]), [58.0, 14.0, 58.0, 14.0]);
    }

    #[test]
    fn menu_margins_keep_room_for_buttons() {
        assert_eq!(
            interface_safe_zone([602.0, 270.0], [10, 20]),
            [60.0, 15.0, 60.0, 15.0]
        );
        assert_eq!(interface_safe_zone([310.0, 230.0], [20, 20]), [0.0; 4]);
    }
}
