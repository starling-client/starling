//! Starling's part of shader_ui.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// The resolution a pack runs at: Auto holds a heavy pack below what the player chose.
pub(super) fn resolution_value(s: &Settings) -> String {
    let scale = s.sunrise.resolution_scale;
    let limit = (crate::graphics::auto_shader_scale() * 100.0).round() as u32;
    let limited = s.sunrise.shader_choice().is_some_and(|(pack, profile)| {
        profile == crate::graphics::AUTO_PROFILE && !crate::graphics::light_shader_pack(&pack)
    }) && scale > limit;
    if limited {
        format!("{limit}% (Auto)")
    } else {
        format!("{scale}%")
    }
}

/// The profile of the picked pack that runs, as the row shows it.
pub(super) fn pack_profile_value(s: &Settings) -> String {
    let profile = s.sunrise.shader_choice().map(|c| c.1).unwrap_or_default();
    match profile.as_str() {
        "" => lang::get("Pack Default"),
        crate::graphics::AUTO_PROFILE => lang::get("Auto"),
        profile => profile.to_owned(),
    }
}

impl ShaderScreen {
    /// The profiles the picked pack lists, after Auto (on a phone) and the pack's defaults:
    /// read again when a pack is picked or the options open.
    pub(crate) fn refresh_profiles(&mut self, s: &Settings) {
        let dir = std::env::current_dir()
            .unwrap_or_default()
            .join(crate::shader_pack_list::DIR);
        self.profiles = vec![String::new()];
        if cfg!(target_os = "android") {
            self.profiles
                .insert(0, crate::graphics::AUTO_PROFILE.to_owned());
        }
        if let Some((pack, _)) = s.sunrise.shader_choice()
            && let Ok(pack) = sunrise_shaders::Pack::open(&dir.join(pack))
        {
            let text = pack.read_text("shaders.properties").unwrap_or_default();
            let macros = sunrise_shaders::Macros::standard();
            let properties = sunrise_shaders::Properties::parse(&text, &macros);
            // Only the profiles that set a switch or a number: some packs list labels or
            // single choices (a tonemap) under `profile.` and those pick nothing to run.
            let ranked = sunrise_shaders::Values::profiles_by_cost(
                &properties,
                &sunrise_shaders::options(&pack),
            );
            self.profiles.extend(
                properties
                    .entries
                    .iter()
                    .filter_map(|e| e.key.strip_prefix("profile."))
                    .filter(|name| ranked.iter().any(|(n, _)| n == name))
                    .map(str::to_owned),
            );
        }
    }
}
