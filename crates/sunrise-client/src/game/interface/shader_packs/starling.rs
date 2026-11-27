//! Starling's part of shader_packs.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// A pack the renderer is making on its own thread (`Renderer::load_shader_pack`): its ids,
/// which entity spans use once it draws.
pub(in super::super) struct Making {
    pub(super) ids: PackIds,
}

/// What the Auto profile does to a pack on this device. Packs written for a PC spend most of a
/// phone's frame on their pixels, water and shadow map, so it takes one of the pack's own light
/// profiles (Complementary on a Snapdragon 8 Gen 2 at distance 10: 46 ms of GPU on LOW, 28 ms on
/// VERYLOW), limits the shadow and the resolution, and has terrain draw its depth first. Which
/// profile is lightest is read from what the profiles set (`Values::profiles_by_cost`), not from
/// their names, so any pack that has profiles works.
pub(super) struct AutoProfile {
    /// Place among the pack's profiles, lightest first. A strong phone skips the very lightest
    /// when the pack has three or more (a profile that turns everything off looks like none).
    pub(super) rank: usize,
    pub(super) shadow_resolution: f64,
    pub(super) shadow_distance: f64,
}

impl AutoProfile {
    pub(super) fn here() -> Self {
        use sunrise_client::system_stats::Tier;
        match Tier::here().unwrap_or(Tier::High) {
            Tier::High => Self {
                rank: 1,
                shadow_resolution: 1024.0,
                shadow_distance: 64.0,
            },
            Tier::Mid => Self {
                rank: 0,
                shadow_resolution: 1024.0,
                shadow_distance: 64.0,
            },
            Tier::Low => Self {
                rank: 0,
                shadow_resolution: 512.0,
                shadow_distance: 64.0,
            },
        }
    }
}

/// The pack's option values: the chosen `profile` (`LOW`), then what the player chose on the
/// Shader Options page, which Auto's limits leave alone, then `SUNRISE_SHADER_OPTIONS` lists
/// values over it (`shadowDistance=64.0 SHADOW_QUALITY=0 !BLOOM`).
pub(super) fn picked_values(
    properties: &Properties,
    options: &BTreeMap<String, sunrise_shaders::PackOption>,
    name: &str,
    profile: &str,
    chosen: &BTreeMap<String, String>,
) -> sunrise_shaders::Values {
    let mut values = sunrise_shaders::Values::default();
    if profile == AUTO_PROFILE {
        let auto = AutoProfile::here();
        let ranked = sunrise_shaders::Values::profiles_by_cost(properties, options);
        let at = if ranked.len() > 2 { auto.rank } else { 0 };
        match ranked.get(at) {
            Some((found, cost)) => {
                log::info!(
                    "shader pack {name}: Auto picks profile `{found}` (cost {cost:.1}) of {}",
                    ranked
                        .iter()
                        .map(|(n, c)| format!("{n} {c:.1}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                values = sunrise_shaders::Values::profile(properties, found).unwrap_or_default();
            }
            None => log::info!("shader pack {name}: Auto finds no profile to start from"),
        }
        for (option, limit) in [
            ("shadowMapResolution", auto.shadow_resolution),
            ("shadowDistance", auto.shadow_distance),
        ] {
            if !chosen.contains_key(option) {
                values.cap(option, limit);
            }
        }
    } else if !profile.is_empty() {
        match sunrise_shaders::Values::profile(properties, profile) {
            Some(p) => values = p,
            None => log::warn!("shader pack {name}: no profile `{profile}`"),
        }
    }
    for (option, value) in chosen {
        values.set(option, value);
    }
    if let Ok(list) = std::env::var("SUNRISE_SHADER_OPTIONS") {
        values.parse_list(&list);
    }
    if !values.is_empty() {
        log::info!("shader pack {name}: option values {values:?}");
    }
    values
}

impl Game<'_> {
    /// The world's resolution, of the screen's: the player's, and no more than Auto's while it
    /// runs a pack that isn't a light one.
    pub(in super::super) fn world_scale(&self) -> f32 {
        // Dynamic Resolution's scale where it is on, else the setting's.
        let chosen = super::super::frame_autotune::world_scale(
            &self.settings.sunrise,
            &self.dynamic_resolution,
        ) as f32
            / 100.0;
        match &self.shader_choice {
            Some((name, profile)) if profile == AUTO_PROFILE && !light_shader_pack(name) => {
                chosen.min(sunrise_client::graphics::auto_shader_scale())
            }
            _ => chosen,
        }
    }

    /// The device was lost with a pack picked: most likely its frames were too long for the
    /// driver, so the rebuilt renderer starts without it and it stays off at the next start.
    pub(in super::super) fn shader_pack_lost_device(&mut self) {
        if self.settings.sunrise.shader_pack.is_empty() {
            return;
        }
        let name = std::mem::take(&mut self.settings.sunrise.shader_pack);
        log::error!("shader pack {name}: turned off after the graphics device was lost");
        self.pack_ids = None;
        self.pack_making = None;
        self.settings.save(&Self::options_path());
        self.system_message(Text::colored(
            format!("Shader pack {name} was too heavy for this device and was turned off"),
            0xFF5555,
        ));
    }
}
