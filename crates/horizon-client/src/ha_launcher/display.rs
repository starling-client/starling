//! Whole-frame resolution, independent of the world's graphics settings.
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Older launchers stored these values outside the selected game's options.
/// Import them only where the profile has no corresponding saved value.
pub fn migrate_preferences(config: &super::catalog::Config, home: &Path) {
    use super::{catalog::Orientation, client::android_display::DisplayOption as D};

    let (orientation, rotation) = match config.orientation {
        Orientation::Landscape => (0, 0),
        Orientation::Portrait => (1, 0),
        Orientation::ReverseLandscape => (0, 3),
        Orientation::ReversePortrait => (1, 3),
        Orientation::Auto => (2, 0),
    };
    let [w, h] = config.screen.aspect.ratio();
    let aspect = D::ASPECTS
        .iter()
        .position(|&(width, height, _)| [width, height] == [w as i32, h as i32])
        .unwrap_or(0) as i32;
    let game = config.game_home(home);
    let raw = std::fs::read_to_string(game.join("options.txt")).unwrap_or_default();
    let has_option = |file: &str, key| {
        file.lines()
            .any(|line| line.split_once(':').is_some_and(|(name, _)| name == key))
    };
    let pending: Vec<_> = [
        (D::Cutout, i32::from(config.screen.avoid_cutout)),
        (D::Orientation, orientation),
        (D::Rotation, rotation),
        (D::Aspect, aspect),
    ]
    .into_iter()
    .filter(|&(option, value)| value != 0 && !has_option(&raw, option.key()))
    .collect();
    let scale = config.render_scale.filter(|scale| {
        scale.percent() != 100
            && !std::fs::read_to_string(game.join("horizon-graphics.txt"))
                .is_ok_and(|file| has_option(&file, "resolutionScale"))
    });
    if !pending.is_empty() || scale.is_some() {
        config.update_game_settings(home, |settings| {
            for (option, value) in pending {
                option.set(settings, value);
            }
            if let Some(scale) = scale {
                settings.horizon.resolution_scale = scale.percent();
            }
        });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RenderScale(u32);

impl Default for RenderScale {
    fn default() -> Self {
        Self(100)
    }
}

impl RenderScale {
    pub const PRESETS: [u32; 10] = [200, 175, 150, 125, 100, 85, 75, 67, 50, 25];

    pub fn new(percent: u32) -> Self {
        Self(percent.clamp(25, 200))
    }

    pub fn parse(text: &str) -> Result<Self, &'static str> {
        let percent = text
            .trim()
            .trim_end_matches('%')
            .trim()
            .parse::<u32>()
            .map_err(|_| "Enter a whole percentage from 25 to 200")?;
        if !(25..=200).contains(&percent) {
            return Err("Enter a percentage from 25 to 200");
        }
        Ok(Self(percent))
    }

    pub fn percent(self) -> u32 {
        self.0.clamp(25, 200)
    }

    pub fn buffer_size(self, view: [u32; 2]) -> [u32; 2] {
        let [w, h] = view.map(|v| v.max(1));
        // Keep standard menus usable even on a narrow keypad screen. Never
        // enlarge a surface that is already smaller than the GUI minimum.
        let minimum = (320.0 / w as f64).max(240.0 / h as f64).min(1.0);
        let scale = (self.percent() as f64 / 100.0).clamp(minimum, 1.0);
        [w, h].map(|v| (v as f64 * scale).round().max(1.0) as u32)
    }
}

#[cfg(target_os = "android")]
static REQUESTED_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(100);

#[cfg(target_os = "android")]
pub fn set_scale(scale: RenderScale) {
    REQUESTED_SCALE.store(scale.percent(), std::sync::atomic::Ordering::Relaxed);
}

/// The launcher draws at the window's own size. Horizon Android lowered its own resolution with
/// the game's (its winit drew a smaller surface scaled up); Starling's game scales its own frame,
/// and its winit has no such surface.
#[cfg(target_os = "android")]
pub fn prepare(_window: &winit::window::Window) -> Result<Option<[u32; 2]>, String> {
    Ok(None)
}

/// Some Android firmware updates the native window without delivering a matching Resized event:
/// the renderer follows the window's size.
#[cfg(target_os = "android")]
pub fn apply(
    window: &winit::window::Window,
    renderer: &mut crate::ha_launcher::game::Renderer,
) -> Result<(), String> {
    if renderer.suspended {
        return Ok(());
    }
    let size = window.inner_size();
    if size.width > 0
        && size.height > 0
        && [size.width, size.height] != [renderer.config.width, renderer.config.height]
    {
        renderer.resize(size.width, size.height);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_surface_area_dimensions_without_changing_aspect() {
        assert_eq!(RenderScale::new(50).buffer_size([2772, 1280]), [1386, 640]);
        assert_eq!(
            RenderScale::default().buffer_size([2772, 1280]),
            [2772, 1280]
        );
    }

    #[test]
    fn keypad_screen_keeps_usable_menu_width() {
        assert_eq!(RenderScale::new(25).buffer_size([480, 854]), [320, 569]);
        assert_eq!(RenderScale::new(25).buffer_size([240, 320]), [240, 320]);
        assert_eq!(RenderScale::new(25).buffer_size([0, 0]), [1, 1]);
    }

    #[test]
    fn edited_and_old_settings_have_bounded_values() {
        assert_eq!(RenderScale::parse(" 67% ").unwrap().percent(), 67);
        assert!(RenderScale::parse("0").is_err());
        assert_eq!(RenderScale::parse("150").unwrap().percent(), 150);
        assert!(RenderScale::parse("201").is_err());
        assert_eq!(
            serde_json::from_str::<RenderScale>("0").unwrap().percent(),
            25
        );
    }
}

/// Fixed presentation ratios; logical rendering and input share the same bounds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Aspect {
    #[default]
    Native,
    Wide,
    Classic,
    Square,
}
impl Aspect {
    pub fn label(self) -> &'static str {
        match self {
            Self::Native => "Native",
            Self::Wide => "16:9",
            Self::Classic => "4:3",
            Self::Square => "1:1",
        }
    }
    pub fn ratio(self) -> [u32; 2] {
        match self {
            Self::Native => [0, 0],
            Self::Wide => [16, 9],
            Self::Classic => [4, 3],
            Self::Square => [1, 1],
        }
    }
}

/// Client-wide screen choices. Missing preferences start in navigation-free mode.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenPreferences {
    pub immersive: bool,
    pub avoid_cutout: bool,
    pub aspect: Aspect,
}
impl Default for ScreenPreferences {
    fn default() -> Self {
        Self {
            // Toggling immersive system-bar hiding has freeze reports on some
            // devices; keep navigation bars visible until that is understood.
            immersive: false,
            avoid_cutout: false,
            aspect: Aspect::Native,
        }
    }
}
#[cfg(test)]
mod screen_tests {
    use super::*;
    #[test]
    fn missing_screen_preferences_keep_navigation_bars_visible() {
        let defaults: ScreenPreferences = serde_json::from_str("{}").unwrap();
        assert!(!defaults.immersive);
        let saved: ScreenPreferences =
            serde_json::from_str(r#"{"immersive":false,"aspect":"classic","avoid_cutout":true}"#)
                .unwrap();
        assert!(!saved.immersive);
        assert!(saved.avoid_cutout);
        assert_eq!(saved.aspect, Aspect::Classic);
    }
}
