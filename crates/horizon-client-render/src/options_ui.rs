//! The options screens: OptionsScreen, Video Settings, Music & Sounds,
//! Controls, Key Binds, Mouse Settings, Chat Settings, Accessibility,
//! Language and Resource Packs, laid out like vanilla's HeaderAndFooter /
//! OptionsList screens and drawn with the shared widgets.
use std::time::Instant;

pub use crate::settings::Opt;

use crate::{
    gamepad::{self, GlyphStyle},
    hdr_calibration::Picture,
    hud::{HudAtlas, HudBatch, rgba},
    keybinds::{self, CATEGORIES, KeyBinds, MAPPINGS},
    lang::{self, LanguageInfo},
    own_buttons,
    settings::{SOUND_SOURCES, Settings},
    settings_shell as shell,
    text::{self, Style, Text},
    ui,
};

mod shell_pages;

fn tr(key: &str) -> Text {
    Text::translate(key, &[])
}
/// `text` cut to `chars` characters, `...` marking the cut.
fn clipped(text: &str, chars: usize) -> String {
    if text.chars().count() <= chars {
        return text.to_owned();
    }
    let cut: String = text.chars().take(chars - 3).collect();
    format!("{}...", cut.trim_end())
}
/// Where the Shader Packs page's folder is, as a player finds it: on a phone, from the storage's
/// root (`Starling/shaderpacks`), or in the app's own files, which Open Pack Folder sets up a
/// reachable folder for.
pub(crate) fn shader_folder_note() -> String {
    let dir = std::env::current_dir()
        .unwrap_or_default()
        .join("shaderpacks");
    if cfg!(target_os = "android") {
        if crate::logging::game_home() == crate::logging::private_home() {
            return "In the app's own files: Open Pack Folder to reach them".into();
        }
        if let Ok(shared) = dir.strip_prefix("/storage/emulated/0") {
            return format!("Folder: {}", shared.display());
        }
    }
    clipped(&format!("Folder: {}", dir.display()), 52)
}

/// False only in a game built without the `horizon-extras` feature (docs/forking.md), which says
/// so once at startup: the Options screen then has vanilla's Telemetry Data slot instead of
/// Horizon Settings and Mods, and Credits & Attribution names the official build.
pub static HORIZON_EXTRAS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
fn horizon_extras() -> bool {
    HORIZON_EXTRAS.load(std::sync::atomic::Ordering::Relaxed)
}
fn generic(caption: &str, value: Text) -> Text {
    Text::translate("options.generic_value", &[tr(caption), value])
}
fn percent(caption: &str, value: f32) -> Text {
    Text::translate(
        "options.percent_value",
        &[
            tr(caption),
            Text::literal(((f64::from(value) * 100.0) as i32).to_string()),
        ],
    )
}
/// `text` cut into lines of at most `width` characters, at spaces where it has them.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let len = line.chars().count();
        if len > 0 && len + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
        while line.chars().count() > width {
            let rest: String = line.chars().skip(width).collect();
            line = line.chars().take(width).collect();
            lines.push(std::mem::replace(&mut line, rest));
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}
/// Setting `j` of the native mod in row `i` of Options > Mods.
fn mod_setting(s: &Settings, i: u16, j: u16) -> Option<&crate::mod_list::NativeSetting> {
    s.mods
        .installed
        .get(usize::from(i))?
        .native
        .as_ref()?
        .settings
        .get(usize::from(j))
}
/// A native mod's setting as its row says it: the label, then the value.
fn mod_setting_label(setting: &crate::mod_list::NativeSetting) -> Text {
    use crate::mod_list::NativeSettingValue as V;
    let value = match &setting.value {
        V::Toggle(on) => on_off(*on),
        V::Choice { options, index } => options
            .get(*index)
            .map_or_else(|| Text::literal(""), |o| tr(o)),
        V::Range { value, .. } => {
            let shown = format!("{value:.2}");
            Text::literal(shown.trim_end_matches('0').trim_end_matches('.'))
        }
        V::Color(rgb) => Text::literal(format!("#{rgb:06X}")).with_color(*rgb),
    };
    Text::option_name_value(tr(&setting.label), value)
}
fn on_off(v: bool) -> Text {
    tr(if v { "options.on" } else { "options.off" })
}
/// A music track's name as its row shows it: the file, without its folder
/// (`minecraft:music/game/calm1` -> `calm1`; the group names the situation).
fn track_name(id: &str) -> &str {
    id.rsplit('/').next().unwrap_or(id)
}
fn chat_px(caption: &str, px: f32) -> Text {
    Text::translate(
        "options.pixel_value",
        &[tr(caption), Text::literal((px as i32).to_string())],
    )
}
pub(crate) enum Kind {
    /// Slider position 0..1.
    Slider(f32),
    Cycle,
}
/// `PlayerModelPart` keys, in vanilla's bit order (`modelPart_*` in options.txt).
const MODEL_PART_KEYS: [&str; 7] = [
    "modelPart_cape",
    "modelPart_jacket",
    "modelPart_left_sleeve",
    "modelPart_right_sleeve",
    "modelPart_left_pants_leg",
    "modelPart_right_pants_leg",
    "modelPart_hat",
];
/// The Accessibility Settings page:
/// `AccessibilityOptionsScreen.options`, with the Controls link inserted separately.
pub(crate) const ACCESSIBILITY_OPTIONS: &[Opt] = &[
    Opt::Narrator,
    Opt::ShowSubtitles,
    Opt::HighContrast,
    Opt::MenuBlur,
    Opt::TextBackgroundOpacity,
    Opt::TextBackground,
    Opt::ChatOpacity,
    Opt::ChatLineSpacing,
    Opt::ChatDelay,
    Opt::NotificationDisplayTime,
    Opt::ViewBobbing,
    Opt::DistortionEffects,
    Opt::FovEffects,
    Opt::DarknessPulsing,
    Opt::DamageTilt,
    Opt::GlintSpeed,
    Opt::GlintStrength,
    Opt::HideLightningFlashes,
    Opt::MonochromeLogo,
    Opt::PanoramaSpeed,
    Opt::HideSplashTexts,
    Opt::NarratorHotkey,
    Opt::RotateWithMinecarts,
    Opt::HighContrastBlockOutlines,
];

/// `ControlsScreen.options`: the controls added by Java Edition 26.3, in
/// their vanilla order. The macOS-only Control-click switch is omitted on
/// other platforms just as `InputQuirks.EMULATE_RIGHT_CLICK_WITH_CTRL_KEY`
/// does in Java.
pub(crate) fn vanilla_control_options() -> Vec<Opt> {
    let mut options = vec![
        Opt::ToggleCrouch,
        Opt::ToggleSprint,
        Opt::ToggleAttack,
        Opt::ToggleUse,
        Opt::AutoJump,
        Opt::SprintWindow,
        Opt::QuitShortcuts,
    ];
    if cfg!(target_os = "macos") {
        options.push(Opt::CtrlClickEmulatesRightClick);
    }
    options.push(Opt::OperatorItemsTab);
    options
}

/// `MouseSettingsScreen.options` in Java Edition 26.3, in vanilla order.
pub(crate) const MOUSE_OPTIONS: &[Opt] = &[
    Opt::Sensitivity,
    Opt::WheelSensitivity,
    Opt::DiscreteScroll,
    Opt::InvertX,
    Opt::InvertY,
    Opt::AllowCursorChanges,
];

impl Opt {
    /// Keep controls visible while their underlying systems are still missing.
    pub(crate) fn available(self) -> bool {
        !matches!(
            self,
            // Stored, but there are no blocked players or signed chat yet.
            Opt::HideMatchedNames
                | Opt::OnlySecureChat
                // Saved, but no Horizon system reads them yet: no Realms, no
                // friends list, no directional audio.
                | Opt::RealmsNotifications
                | Opt::InGameNotification
                | Opt::SharePresence
                | Opt::DirectionalAudio
                // Saved, but nothing in rendering reads it yet.
                | Opt::ChunkBuilder
        )
    }
    fn file_key(self) -> Option<&'static str> {
        Some(match self {
            Opt::Narrator => "narrator",
            Opt::TextBackground => "backgroundForChatOnly",
            Opt::ChatDelay => "chatDelay",
            Opt::HideMatchedNames => "hideMatchedNames",
            Opt::OnlySecureChat => "onlyShowSecureChat",
            Opt::SaveChatDrafts => "saveChatDrafts",
            Opt::DistortionEffects => "screenEffectScale",
            Opt::FovEffects => "fovEffectScale",
            Opt::DarknessPulsing => "darknessEffectScale",
            Opt::DamageTilt => "damageTiltStrength",
            Opt::GlintSpeed => "glintSpeed",
            Opt::GlintStrength => "glintStrength",
            Opt::HideLightningFlashes => "hideLightningFlashes",
            Opt::MonochromeLogo => "darkMojangStudiosBackground",
            Opt::PanoramaSpeed => "panoramaScrollSpeed",
            Opt::HideSplashTexts => "hideSplashTexts",
            Opt::NarratorHotkey => "narratorHotkey",
            Opt::RotateWithMinecarts => "rotateWithMinecart",
            Opt::HighContrastBlockOutlines => "highContrastBlockOutline",
            Opt::MusicFrequency => "musicFrequency",
            Opt::SoundDevice => "soundDevice",
            Opt::OperatorItemsTab => "operatorItemsTab",
            Opt::QuitShortcuts => "quitShortcuts",
            Opt::CtrlClickEmulatesRightClick => "ctrlClickEmulatesRightClick",
            Opt::ModelPart(i) => MODEL_PART_KEYS[i],
            Opt::AllowServerListing => "allowServerListing",
            Opt::RealmsNotifications => "realmsNotifications",
            Opt::InGameNotification => "inGameNotification",
            Opt::SharePresence => "sharePresence",
            Opt::DirectionalAudio => "directionalAudio",
            Opt::MusicToast => "musicToast",
            _ => return None,
        })
    }

    pub(crate) fn caption(self) -> &'static str {
        match self {
            Opt::AndroidInputSelection => "Input device",
            Opt::TouchMode => "Control scheme",
            Opt::TouchCrosshairGestures => "Touch gestures",
            Opt::TouchControlHub => "Control hub button",
            Opt::TouchUiVibration => "UI vibration",
            Opt::TouchGameVibration => "Gameplay vibration",
            Opt::TouchAttackButtonLook => "Look while interacting",
            Opt::TouchMouseSensitivity => "Virtual mouse sensitivity",
            Opt::TouchAnalog => "360-degree movement",
            Opt::TouchCircleSize => "Touch circle size",
            Opt::TouchAttackSide => "Attack indicator side",
            Opt::TouchHorizontalMargin => "HUD margins: left / right",
            Opt::TouchVerticalMargin => "HUD margins: top / bottom",
            Opt::TouchSensitivity => "Touch sensitivity",
            Opt::TouchSensitivityHorizontal => "Horizontal sensitivity",
            Opt::TouchSensitivityVertical => "Vertical sensitivity",
            Opt::TouchMouseButton => "Virtual Mouse Button",
            Opt::TouchKeyboardButton => "Keyboard Button",
            Opt::TouchOffhand => "Swap hands button",
            Opt::TouchShield => "Raise shield with",
            Opt::TouchButtonScale => "Control button size",
            Opt::TouchInteract => "Interact button",
            Opt::TouchLeftHand => "Left handed",
            Opt::TouchSprintButton => "Sprint button",
            Opt::TouchSprintToggle => "Toggle sprint button",
            Opt::TouchInventoryControls => "Controls in inventory",
            Opt::TouchDynamicJoystick => "Dynamic joystick",
            Opt::TouchDynamicAreaX => "Dynamic joystick width",
            Opt::TouchDynamicAreaY => "Dynamic joystick height",
            Opt::AndroidGyroscope => "Gyroscope aiming",
            Opt::AndroidGyroSensitivity => "Gyroscope sensitivity",
            Opt::TouchSwapJumpSneak => "Swap jump / sneak",
            Opt::TouchAutoSprint => "Auto Sprint",
            Opt::TouchAvoidCutout => "Avoid camera cutout",

            Opt::Fov => "options.fov",
            Opt::RenderDistance => "options.renderDistance",
            Opt::SimulationDistance => "options.simulationDistance",
            Opt::Sensitivity => "options.sensitivity",
            Opt::WheelSensitivity => "options.mouseWheelSensitivity",
            Opt::InvertX => "options.invertMouseX",
            Opt::InvertY => "options.invertMouseY",
            Opt::DiscreteScroll => "options.discrete_mouse_scroll",
            Opt::AllowCursorChanges => "options.allowCursorChanges",
            Opt::Volume(_) => "soundCategory.master",
            Opt::ModSetting(..) => "horizon.mods.settings",
            Opt::SoundDevice => "options.audioDevice",
            Opt::GuiScale => "options.guiScale",
            Opt::ViewBobbing => "options.viewBobbing",
            Opt::Gamma => "options.gamma",
            Opt::MaxFps => "options.framerateLimit",
            Opt::Vsync => "options.vsync",
            Opt::Fullscreen => "options.fullscreen",
            Opt::ExclusiveFullscreen => "options.exclusiveFullscreen",
            Opt::ExclusiveMode => "options.fullscreen.exclusive.mode",
            Opt::GraphicsBackend => "options.graphicsApi",
            Opt::TextureFiltering => "options.textureFiltering",
            Opt::MaxAnisotropy => "options.maxAnisotropy",
            Opt::SmoothLighting => "options.ao",
            Opt::Clouds => "options.renderClouds",
            Opt::CloudRange => "options.renderCloudsDistance",
            Opt::Particles => "options.particles",
            Opt::EntityDistance => "options.entityDistanceScaling",
            Opt::EntityShadows => "options.entityShadows",
            Opt::BiomeBlend => "options.biomeBlendRadius",
            Opt::Preset => "options.graphics.preset",
            Opt::Mipmaps => "options.mipmapLevels",
            Opt::WeatherRadius => "options.weatherRadius",
            Opt::ChunkFade => "options.chunkFade",
            Opt::Vignette => "options.vignette",
            Opt::Autosave => "options.autosaveIndicator",
            Opt::CutoutLeaves => "options.cutoutLeaves",
            Opt::ImprovedTransparency => "options.improvedTransparency",
            Opt::AttackIndicator => "options.attackIndicator",
            Opt::InactivityFps => "options.inactivityFpsLimit",
            Opt::ChunkBuilder => "options.prioritizeChunkUpdates",
            Opt::MenuBlur => "options.accessibility.menu_background_blurriness",
            Opt::ToggleCrouch => "key.sneak",
            Opt::ToggleSprint => "key.sprint",
            Opt::ToggleAttack => "key.attack",
            Opt::ToggleUse => "key.use",
            Opt::AutoJump => "options.autoJump",
            Opt::SprintWindow => "options.sprintWindow",
            Opt::QuitShortcuts => "options.quitShortcuts",
            Opt::CtrlClickEmulatesRightClick => "options.ctrlClickEmulatesRightClick",
            Opt::OperatorItemsTab => "options.operatorItemsTab",
            Opt::ChatVisibility => "options.chat.visibility",
            Opt::ChatColors => "options.chat.color",
            Opt::ChatLinks => "options.chat.links",
            Opt::ChatLinksPrompt => "options.chat.links.prompt",
            Opt::ChatOpacity => "options.chat.opacity",
            Opt::TextBackgroundOpacity => "options.accessibility.text_background_opacity",
            Opt::ChatScale => "options.chat.scale",
            Opt::ChatWidth => "options.chat.width",
            Opt::ChatHeightFocused => "options.chat.height.focused",
            Opt::ChatHeightUnfocused => "options.chat.height.unfocused",
            Opt::ChatLineSpacing => "options.chat.line_spacing",
            Opt::AutoSuggestions => "options.autoSuggestCommands",
            Opt::HideMatchedNames => "options.hideMatchedNames",
            Opt::OnlySecureChat => "options.onlyShowSecureChat",
            Opt::SaveChatDrafts => "options.chat.drafts",
            Opt::ReducedDebugInfo => "options.reducedDebugInfo",
            Opt::HighContrast => "options.accessibility.high_contrast",
            Opt::ShowSubtitles => "options.showSubtitles",
            Opt::MusicFrequency => "options.music_frequency",
            Opt::NotificationDisplayTime => "options.notifications.display_time",
            Opt::Narrator => "options.narrator",
            Opt::TextBackground => "options.accessibility.text_background",
            Opt::ChatDelay => "horizon.options.chat_delay",
            Opt::DistortionEffects => "options.screenEffectScale",
            Opt::FovEffects => "options.fovEffectScale",
            Opt::DarknessPulsing => "options.darknessEffectScale",
            Opt::DamageTilt => "options.damageTiltStrength",
            Opt::GlintSpeed => "options.glintSpeed",
            Opt::GlintStrength => "options.glintStrength",
            Opt::HideLightningFlashes => "options.hideLightningFlashes",
            Opt::MonochromeLogo => "options.darkMojangStudiosBackgroundColor",
            Opt::PanoramaSpeed => "options.accessibility.panorama_speed",
            Opt::HideSplashTexts => "options.hideSplashTexts",
            Opt::NarratorHotkey => "options.accessibility.narrator_hotkey",
            Opt::RotateWithMinecarts => "options.rotateWithMinecart",
            Opt::HighContrastBlockOutlines => "options.accessibility.high_contrast_block_outline",
            Opt::ForceUnicode => "options.forceUnicodeFont",
            Opt::JapaneseGlyphs => "options.japaneseGlyphVariants",
            // Horizon's own options: vanilla has no keys for them, so they read
            // the namespace the client bundles (assets/horizon/lang).
            Opt::PadEnabled => "horizon.options.controller",
            Opt::PadVibration => "horizon.options.vibration",
            Opt::PadPlayInBackground => "horizon.options.play_in_background",
            Opt::PadAdaptiveTriggers => "horizon.options.adaptive_triggers",
            Opt::PadLightbar => "horizon.options.lightbar",
            Opt::PadLookSensitivity => "horizon.options.look_sensitivity",
            Opt::PadPointerSpeed => "horizon.options.pointer_speed",
            Opt::PadScrollSpeed => "horizon.options.scroll_speed",
            Opt::PadRepeatDelay => "horizon.options.repeat_delay",
            Opt::PadRepeatInterval => "horizon.options.repeat_interval",
            Opt::PadInvertX => "horizon.options.invert_x",
            Opt::PadInvertY => "horizon.options.invert_y",
            Opt::PadCraftOnServers => "horizon.options.craft_on_servers",
            Opt::PadToggleSneak => "horizon.options.sneak_button",
            Opt::PadServerStick => "horizon.options.server_stick",
            Opt::PadStickPrompt => "horizon.options.stick_prompt",
            Opt::PadMoveDeadzone => "horizon.options.move_deadzone",
            Opt::PadLookDeadzone => "horizon.options.look_deadzone",
            Opt::PadTriggerThreshold => "horizon.options.trigger_threshold",
            Opt::PadGlyphs => "horizon.options.button_glyphs",
            Opt::PadNintendoButtons => "horizon.options.nintendo_buttons",
            Opt::PadKeyboard => "horizon.options.on_screen_keyboard",
            Opt::PadGameplayGuide => "horizon.options.gameplay_guide",
            Opt::PadRadialMode => "horizon.options.radial_mode",
            Opt::PadRadialStick => "horizon.options.radial_stick",
            Opt::ModelPart(i) => [
                "options.modelPart.cape",
                "options.modelPart.jacket",
                "options.modelPart.left_sleeve",
                "options.modelPart.right_sleeve",
                "options.modelPart.left_pants_leg",
                "options.modelPart.right_pants_leg",
                "options.modelPart.hat",
            ][i],
            Opt::SkinLayers3d => "horizon.options.three_d_skin_layers",
            Opt::MainHand => "options.mainHand",
            Opt::AllowServerListing => "options.allowServerListing",
            Opt::RealmsNotifications => "options.realmsNotifications.button",
            Opt::InGameNotification => "options.inGameNotification",
            Opt::SharePresence => "options.sharePresence",
            Opt::DirectionalAudio => "options.directionalAudio",
            Opt::MusicToast => "options.musicToast",
            Opt::Page(page, row) => pages::slider_caption(page, row),
        }
    }
    pub(crate) fn kind(self, s: &Settings) -> Kind {
        let unit = |v: f32, lo: f32, hi: f32| Kind::Slider(((v - lo) / (hi - lo)).clamp(0.0, 1.0));
        match self {
            Opt::TouchCircleSize => unit(s.touch.circle_size, 16.0, 64.0),
            Opt::TouchDynamicAreaX => unit(s.touch.dynamic_area_x, 0.0, 1.0),
            Opt::TouchDynamicAreaY => unit(s.touch.dynamic_area_y, 0.0, 1.0),
            Opt::TouchHorizontalMargin => unit(s.touch.horizontal_margin, 0.0, 0.12),
            Opt::TouchVerticalMargin => unit(s.touch.vertical_margin, 0.0, 0.12),
            Opt::AndroidGyroSensitivity => unit(s.touch.gyro_sensitivity, 0.1, 3.0),
            Opt::TouchSensitivity => unit(s.touch.sensitivity, 0.1, 3.0),
            Opt::TouchSensitivityHorizontal => unit(s.touch.sensitivity_horizontal, 0.1, 3.0),
            Opt::TouchSensitivityVertical => unit(s.touch.sensitivity_vertical, 0.1, 3.0),
            Opt::TouchMouseSensitivity => unit(s.touch.virtual_mouse_sensitivity, 0.25, 3.0),
            Opt::TouchButtonScale => unit(s.touch.button_scale, 0.5, 2.0),

            Opt::ModSetting(i, j) => {
                Kind::Slider(mod_setting(s, i, j).map_or(0.0, |m| m.value.fraction()))
            }
            Opt::FovEffects | Opt::DarknessPulsing => {
                Kind::Slider(s.file.double(self.file_key().unwrap()).sqrt() as f32)
            }
            Opt::DistortionEffects
            | Opt::DamageTilt
            | Opt::GlintSpeed
            | Opt::GlintStrength
            | Opt::PanoramaSpeed => Kind::Slider(s.file.double(self.file_key().unwrap()) as f32),
            Opt::Fov => unit(s.fov, 30.0, 110.0),
            Opt::RenderDistance => unit(
                s.render_distance as f32,
                2.0,
                crate::options_file::MAX_RENDER_DISTANCE as f32,
            ),
            Opt::SimulationDistance => unit(s.simulation_distance as f32, 5.0, 32.0),
            Opt::Sensitivity => Kind::Slider(s.sensitivity),
            Opt::WheelSensitivity => unit(s.mouse_wheel_sensitivity, 0.01, 10.0),
            Opt::Volume(i) => Kind::Slider(if i == 0 {
                s.master_volume
            } else {
                s.volumes[i]
            }),
            Opt::Gamma => Kind::Slider(s.gamma),
            Opt::MaxFps => unit(s.max_fps as f32, 10.0, 260.0),
            Opt::CloudRange => unit(s.cloud_range as f32, 2.0, 128.0),
            Opt::EntityDistance => unit(s.entity_distance, 0.5, 5.0),
            Opt::BiomeBlend => unit(s.biome_blend as f32, 0.0, 7.0),
            Opt::Preset => unit(preset_index(&s.graphics_preset) as f32, 0.0, 3.0),
            Opt::Mipmaps => unit(s.mipmap_levels as f32, 0.0, 4.0),
            Opt::MaxAnisotropy => unit(s.file.int("maxAnisotropyBit") as f32, 1.0, 3.0),
            Opt::ExclusiveMode => {
                let last = s.video_modes.len() as f32 - 1.0;
                if last < 0.0 {
                    Kind::Slider(0.0)
                } else {
                    unit(s.video_mode_index() as f32, -1.0, last)
                }
            }
            Opt::WeatherRadius => unit(s.weather_radius as f32, 3.0, 10.0),
            Opt::ChunkFade => unit(s.chunk_fade, 0.0, 2.0),
            Opt::MenuBlur => unit(s.menu_blur as f32, 0.0, 10.0),
            Opt::NotificationDisplayTime => unit(s.notification_display_time, 0.5, 10.0),
            Opt::ChatOpacity => Kind::Slider(s.chat_opacity),
            Opt::TextBackgroundOpacity => Kind::Slider(s.text_background_opacity),
            Opt::ChatScale => Kind::Slider(s.chat_scale),
            Opt::ChatWidth => Kind::Slider(s.chat_width),
            Opt::ChatHeightFocused => Kind::Slider(s.chat_height_focused),
            Opt::ChatHeightUnfocused => Kind::Slider(s.chat_height_unfocused),
            Opt::ChatLineSpacing => Kind::Slider(s.chat_line_spacing),
            Opt::PadLookSensitivity => Kind::Slider(s.controller.look_sensitivity),
            Opt::PadPointerSpeed => unit(s.controller.pointer_speed, 0.25, 3.0),
            Opt::PadScrollSpeed => unit(s.controller.scroll_speed, 1.0, 36.0),
            Opt::PadRepeatDelay => unit(s.controller.repeat_delay, 0.1, 1.0),
            Opt::PadRepeatInterval => unit(s.controller.repeat_interval, 0.03, 0.3),
            Opt::PadMoveDeadzone => unit(s.controller.move_deadzone, 0.0, 0.5),
            Opt::PadLookDeadzone => unit(s.controller.look_deadzone, 0.0, 0.5),
            Opt::PadTriggerThreshold => unit(s.controller.trigger_threshold, 0.1, 0.9),
            Opt::SprintWindow => unit(s.sprint_window as f32, 0.0, 10.0),
            Opt::Page(page, row) => pages::slider_kind(page, row),
            _ => Kind::Cycle,
        }
    }
    /// OptionInstance's static or value-dependent tooltip translation.
    fn tooltip_key(self, s: &Settings) -> String {
        if let Opt::ModSetting(i, j) = self {
            return mod_setting(s, i, j)
                .and_then(|m| m.tooltip.clone())
                .unwrap_or_default();
        }
        let key = match self {
            Opt::InactivityFps => {
                if s.inactivity_fps_limit == "minimized" {
                    "options.inactivityFpsLimit.minimized"
                } else {
                    "options.inactivityFpsLimit.afk"
                }
            }
            Opt::ChunkBuilder => [
                "options.prioritizeChunkUpdates.none",
                "options.prioritizeChunkUpdates.byPlayer",
                "options.prioritizeChunkUpdates.nearby",
            ][usize::from(s.prioritize_chunk_updates.min(2))],
            // Options.exclusiveFullscreen: a tooltip per value.
            Opt::ExclusiveFullscreen => {
                if s.file.bool("exclusiveFullscreen") {
                    "options.exclusiveFullscreen.on"
                } else {
                    "options.exclusiveFullscreen.off"
                }
            }
            Opt::TextureFiltering => [
                "options.textureFiltering.none",
                "options.textureFiltering.rgss",
                "options.textureFiltering.anisotropic",
            ][s.file.int("textureFiltering").clamp(0, 2) as usize],
            // InputQuirks.REPLACE_CTRL_KEY_WITH_CMD_KEY: Cmd+B on macOS.
            Opt::NarratorHotkey if cfg!(target_os = "macos") => {
                "options.accessibility.narrator_hotkey.mac"
            }
            _ => self.caption(),
        };
        format!("{key}.tooltip")
    }
    /// A slider's position (0..1), or `None` for a cycle button.
    pub fn slider_position(self, s: &Settings) -> Option<f32> {
        match self.kind(s) {
            Kind::Slider(v) => Some(v),
            Kind::Cycle => None,
        }
    }
    /// The discrete step count for stepped sliders, or `None` if continuous.
    pub fn slider_steps(self) -> Option<u32> {
        match self {
            Opt::Fov => Some(80),
            Opt::RenderDistance => Some((crate::options_file::MAX_RENDER_DISTANCE - 2) as u32),
            Opt::SimulationDistance => Some(27),
            Opt::MaxFps => Some(25),
            Opt::CloudRange => Some(126),
            Opt::EntityDistance => Some(18),
            Opt::BiomeBlend => Some(7),
            Opt::Preset => Some(3),
            Opt::Mipmaps => Some(4),
            Opt::WeatherRadius => Some(7),
            Opt::ChunkFade => Some(40),
            Opt::MenuBlur => Some(10),
            Opt::NotificationDisplayTime => Some(95),
            Opt::PadMoveDeadzone | Opt::PadLookDeadzone => Some(50),
            Opt::PadTriggerThreshold => Some(80),
            Opt::SprintWindow => Some(10),
            _ => None,
        }
    }
    /// Step a slider forward or backward, derived from its own slider definition and step count.
    pub fn step_slider(self, s: &mut Settings, forward: bool, width: f32) {
        if !self.available() {
            return;
        }
        let Some(pos) = self.slider_position(s) else {
            return;
        };
        // A mod's range steps by its registered step.
        let steps = match self {
            Opt::ModSetting(i, j) => match mod_setting(s, i, j).map(|m| &m.value) {
                Some(crate::mod_list::NativeSettingValue::Range { min, max, step, .. }) => {
                    Some(((max - min) / step).round().max(1.0) as u32)
                }
                _ => None,
            },
            _ => self.slider_steps(),
        };
        let next_pos = if let Some(steps) = steps {
            let steps = steps as f32;
            let current = (pos * steps).round();
            let next = if forward {
                current + 1.0
            } else {
                current - 1.0
            };
            (next / steps).clamp(0.0, 1.0)
        } else {
            let delta = 1.0 / (width - 8.0).max(1.0);
            if forward {
                (pos + delta).min(1.0)
            } else {
                (pos - delta).max(0.0)
            }
        };
        self.set_slider(s, next_pos);
    }
    /// The widget message (OptionInstance.toString / CycleButton full name).
    pub fn label(self, s: &Settings) -> Text {
        if let Opt::ModSetting(i, j) = self {
            return mod_setting(s, i, j).map_or_else(|| Text::literal(""), mod_setting_label);
        }
        let c = self.caption();
        let touch_value = match self {
            Opt::TouchMode => Some(
                crate::android_controls::Mode::SCHEMES
                    .iter()
                    .find(|(mode, split, _)| {
                        *mode == s.touch.mode
                            && (!matches!(
                                mode,
                                crate::android_controls::Mode::Dpad
                                    | crate::android_controls::Mode::Tap
                            ) || *split == s.touch.split_controls)
                    })
                    .map_or("Joystick & aim crosshair", |(_, _, label)| *label)
                    .to_owned(),
            ),
            Opt::TouchCircleSize => Some(format!("{} px", s.touch.circle_size.round())),
            Opt::TouchAttackSide => Some(
                if s.touch.attack_indicator_left {
                    "Left"
                } else {
                    "Right"
                }
                .into(),
            ),
            Opt::TouchHorizontalMargin => {
                Some(format!("{:.1}%", s.touch.horizontal_margin * 100.0))
            }
            Opt::TouchVerticalMargin => Some(format!("{:.1}%", s.touch.vertical_margin * 100.0)),
            Opt::AndroidGyroscope => Some(if s.touch.gyroscope { "ON" } else { "OFF" }.into()),
            Opt::AndroidGyroSensitivity => {
                Some(format!("{:.0}%", s.touch.gyro_sensitivity * 100.0))
            }
            Opt::TouchSensitivity => Some(format!("{:.0}%", s.touch.sensitivity * 100.0)),
            Opt::TouchSensitivityHorizontal => {
                Some(format!("{:.0}%", s.touch.sensitivity_horizontal * 100.0))
            }
            Opt::TouchSensitivityVertical => {
                Some(format!("{:.0}%", s.touch.sensitivity_vertical * 100.0))
            }
            Opt::TouchMouseButton => Some(
                if s.touch.virtual_mouse_enabled {
                    "On"
                } else {
                    "Off"
                }
                .into(),
            ),
            Opt::TouchKeyboardButton => {
                Some(if s.touch.keyboard_button { "On" } else { "Off" }.into())
            }
            Opt::TouchOffhand => Some(if s.touch.offhand_button { "On" } else { "Off" }.into()),
            Opt::TouchShield => Some(
                if s.touch.shield_button {
                    "Button"
                } else {
                    "Sneak"
                }
                .into(),
            ),
            Opt::TouchMouseSensitivity => {
                Some(format!("{:.0}%", s.touch.virtual_mouse_sensitivity * 100.0))
            }
            Opt::TouchAttackButtonLook => Some(
                if s.touch.attack_button_look {
                    "ON"
                } else {
                    "OFF"
                }
                .into(),
            ),
            Opt::TouchControlHub => Some(
                if s.touch.show_control_hub {
                    "ON"
                } else {
                    "OFF"
                }
                .into(),
            ),
            Opt::TouchUiVibration => Some(if s.touch.ui_vibrate { "ON" } else { "OFF" }.into()),
            Opt::TouchGameVibration => Some(if s.touch.vibrate { "ON" } else { "OFF" }.into()),
            Opt::TouchCrosshairGestures => Some(
                if s.touch.crosshair_gestures {
                    "ON"
                } else {
                    "OFF"
                }
                .into(),
            ),
            Opt::TouchButtonScale => Some(format!("{:.0}%", s.touch.button_scale * 100.0)),
            Opt::TouchAnalog => Some(if s.touch.analog_movement { "ON" } else { "OFF" }.into()),
            Opt::TouchInteract => Some(
                if s.touch.contextual_interact {
                    "ON"
                } else {
                    "OFF"
                }
                .into(),
            ),
            Opt::TouchLeftHand => Some(if s.touch.left_handed { "ON" } else { "OFF" }.into()),
            Opt::TouchSprintButton => Some(if s.touch.sprint_button { "ON" } else { "OFF" }.into()),
            Opt::TouchSprintToggle => Some(if s.touch.sprint_toggle { "ON" } else { "OFF" }.into()),
            Opt::TouchInventoryControls => Some(
                if s.touch.inventory_controls {
                    "ON"
                } else {
                    "OFF"
                }
                .into(),
            ),
            Opt::TouchDynamicJoystick => Some(
                if s.touch.dynamic_joystick {
                    "ON"
                } else {
                    "OFF"
                }
                .into(),
            ),
            Opt::TouchDynamicAreaX => Some(format!("{:.0}%", s.touch.dynamic_area_x * 100.0)),
            Opt::TouchDynamicAreaY => Some(format!("{:.0}%", s.touch.dynamic_area_y * 100.0)),
            Opt::TouchSwapJumpSneak => {
                Some(if s.touch.swap_jump_sneak { "ON" } else { "OFF" }.into())
            }
            Opt::AndroidInputSelection => Some(s.touch.input_selection.label().into()),
            Opt::TouchAutoSprint => Some(
                if s.touch.sprint_with_joystick {
                    "ON"
                } else {
                    "OFF"
                }
                .into(),
            ),
            Opt::TouchAvoidCutout => Some(if s.touch.avoid_cutout { "ON" } else { "OFF" }.into()),
            _ => None,
        };
        if let Some(value) = touch_value {
            return Text::literal(format!("{c}: {value}"));
        }

        match self {
            Opt::ChatDelay => tr("options.chat.delay_none"),
            Opt::FovEffects
            | Opt::DarknessPulsing
            | Opt::DistortionEffects
            | Opt::DamageTilt
            | Opt::GlintSpeed
            | Opt::GlintStrength => {
                let value = s.file.double(self.file_key().unwrap()) as f32;
                if value == 0.0 {
                    generic(c, tr("options.off"))
                } else {
                    percent(c, value)
                }
            }
            Opt::PanoramaSpeed => percent(c, s.file.double("panoramaScrollSpeed") as f32),
            Opt::SprintWindow => {
                let value = if s.sprint_window == 0 {
                    tr("options.off")
                } else {
                    Text::tr("options.value", &[&s.sprint_window])
                };
                generic(c, value)
            }
            Opt::Fov => match s.fov as i32 {
                70 => generic(c, tr("options.fov.min")),
                110 => generic(c, tr("options.fov.max")),
                v => generic(c, Text::literal(v.to_string())),
            },
            Opt::RenderDistance => generic(c, Text::tr("options.chunks", &[&s.render_distance])),
            Opt::SimulationDistance => {
                generic(c, Text::tr("options.chunks", &[&s.simulation_distance]))
            }
            Opt::Sensitivity => {
                if s.sensitivity == 0.0 {
                    generic(c, tr("options.sensitivity.min"))
                } else if s.sensitivity == 1.0 {
                    generic(c, tr("options.sensitivity.max"))
                } else {
                    percent(c, 2.0 * s.sensitivity)
                }
            }
            Opt::WheelSensitivity => generic(
                c,
                Text::literal(format!("{:.2}", s.mouse_wheel_sensitivity)),
            ),
            Opt::Volume(i) => {
                let v = if i == 0 {
                    s.master_volume
                } else {
                    s.volumes[i]
                };
                let caption = format!("soundCategory.{}", SOUND_SOURCES[i]);
                if v == 0.0 {
                    generic(&caption, tr("options.off"))
                } else {
                    percent(&caption, v)
                }
            }
            Opt::GuiScale => {
                let value = if s.gui_scale == 0 {
                    tr("options.guiScale.auto")
                } else {
                    Text::literal(s.gui_scale.to_string())
                };
                generic(c, value)
            }
            Opt::Gamma => match (f64::from(s.gamma) * 100.0) as i32 {
                0 => generic(c, tr("options.gamma.min")),
                50 => generic(c, tr("options.gamma.default")),
                100 => generic(c, tr("options.gamma.max")),
                v => generic(c, Text::literal(v.to_string())),
            },
            Opt::MaxFps => {
                if s.max_fps >= 260 {
                    generic(c, tr("options.framerateLimit.max"))
                } else {
                    generic(c, Text::tr("options.framerate", &[&s.max_fps]))
                }
            }
            Opt::CloudRange => generic(c, Text::tr("options.chunks", &[&s.cloud_range])),
            Opt::EntityDistance => percent(c, s.entity_distance),
            Opt::BiomeBlend => generic(
                c,
                tr(&format!(
                    "options.biomeBlendRadius.{}",
                    s.biome_blend * 2 + 1
                )),
            ),
            Opt::Preset => generic(c, tr(&format!("options.graphics.{}", s.graphics_preset))),
            Opt::MaxAnisotropy => match s.file.int("maxAnisotropyBit") {
                0 => generic(c, tr("options.off")),
                bit => generic(c, Text::tr("options.multiplier", &[&(1 << bit)])),
            },
            Opt::ExclusiveMode => match s.video_modes.get(s.video_mode_index().max(0) as usize) {
                None if s.video_modes.is_empty() => tr("options.fullscreen.unavailable"),
                Some(mode) if s.video_mode_index() >= 0 => generic(
                    c,
                    Text::tr(
                        "options.fullscreen.entry",
                        &[
                            &mode.width,
                            &mode.height,
                            &mode.refresh_rate_label(),
                            &mode.bits(),
                        ],
                    ),
                ),
                _ => generic(c, tr("options.fullscreen.current")),
            },
            Opt::Mipmaps => {
                if s.mipmap_levels == 0 {
                    generic(c, tr("options.off"))
                } else {
                    generic(c, Text::literal(s.mipmap_levels.to_string()))
                }
            }
            Opt::WeatherRadius => generic(c, Text::tr("options.blocks", &[&s.weather_radius])),
            Opt::ChunkFade => {
                if s.chunk_fade <= 0.0 {
                    tr("options.chunkFade.none")
                } else {
                    Text::tr(
                        "options.chunkFade.seconds",
                        &[&format!("{:.2}", s.chunk_fade)],
                    )
                }
            }
            Opt::MenuBlur => {
                if s.menu_blur == 0 {
                    generic(c, tr("options.off"))
                } else {
                    generic(c, Text::literal(s.menu_blur.to_string()))
                }
            }
            Opt::ChatOpacity => percent(c, s.chat_opacity * 0.9 + 0.1),
            Opt::TextBackgroundOpacity => percent(c, s.text_background_opacity),
            Opt::ChatScale => {
                if s.chat_scale == 0.0 {
                    generic(c, tr("options.off"))
                } else {
                    percent(c, s.chat_scale)
                }
            }
            Opt::ChatWidth => chat_px(c, (s.chat_width * 280.0 + 40.0).floor()),
            Opt::ChatHeightFocused => chat_px(c, (s.chat_height_focused * 160.0 + 20.0).floor()),
            Opt::ChatHeightUnfocused => {
                chat_px(c, (s.chat_height_unfocused * 160.0 + 20.0).floor())
            }
            Opt::ChatLineSpacing => percent(c, s.chat_line_spacing),
            Opt::NotificationDisplayTime => generic(
                c,
                Text::tr(
                    "options.multiplier",
                    &[&format!("{:.1}", s.notification_display_time)],
                ),
            ),
            Opt::PadLookSensitivity => percent(c, 2.0 * s.controller.look_sensitivity),
            Opt::PadPointerSpeed => percent(c, s.controller.pointer_speed),
            Opt::PadScrollSpeed => percent(c, s.controller.scroll_speed / 12.0),
            Opt::PadRepeatDelay | Opt::PadRepeatInterval => generic(
                c,
                Text::tr(
                    "horizon.options.milliseconds",
                    &[&format!(
                        "{:.0}",
                        1000.0
                            * if self == Opt::PadRepeatDelay {
                                s.controller.repeat_delay
                            } else {
                                s.controller.repeat_interval
                            }
                    )],
                ),
            ),
            Opt::PadMoveDeadzone | Opt::PadLookDeadzone | Opt::PadTriggerThreshold => {
                let v = match self {
                    Opt::PadMoveDeadzone => s.controller.move_deadzone,
                    Opt::PadLookDeadzone => s.controller.look_deadzone,
                    _ => s.controller.trigger_threshold,
                };
                Text::translate(
                    "options.percent_value",
                    &[
                        tr(c),
                        Text::literal(((v * 100.0).round() as i32).to_string()),
                    ],
                )
            }
            _ => generic(c, self.value_text(s)),
        }
    }
    fn value_text(self, s: &Settings) -> Text {
        match self {
            Opt::TextBackground => tr(if s.file.bool("backgroundForChatOnly") {
                "options.accessibility.text_background.chat"
            } else {
                "options.accessibility.text_background.everywhere"
            }),
            // Options.narrator's caption: the status while the narrator is
            // there, "Not Available" otherwise.
            Opt::Narrator => tr(if horizon_narrator::is_active() {
                s.narrator().key()
            } else {
                "options.narrator.notavailable"
            }),
            Opt::ChatDelay => tr("options.chat.delay_none"),
            Opt::HideLightningFlashes => on_off(s.file.bool("hideLightningFlashes")),
            Opt::MonochromeLogo => on_off(s.file.bool("darkMojangStudiosBackground")),
            Opt::HideSplashTexts => on_off(s.file.bool("hideSplashTexts")),
            Opt::HideMatchedNames
            | Opt::OnlySecureChat
            | Opt::SaveChatDrafts
            | Opt::ModelPart(_)
            | Opt::AllowServerListing
            | Opt::RealmsNotifications
            | Opt::InGameNotification
            | Opt::DirectionalAudio => on_off(s.file.bool(self.file_key().unwrap())),
            Opt::MainHand => tr(if s.main_hand_left {
                "options.mainHand.left"
            } else {
                "options.mainHand.right"
            }),
            Opt::SharePresence => tr([
                "options.sharePresence.none",
                "options.sharePresence.limited",
                "options.sharePresence.all",
            ][s.file.int("sharePresence").clamp(0, 2) as usize]),
            Opt::MusicToast => tr([
                "options.musicToast.never",
                "options.musicToast.pauseMenu",
                "options.musicToast.pauseMenuAndToast",
            ][s.file.int("musicToast").clamp(0, 2) as usize]),
            Opt::NarratorHotkey => on_off(s.file.bool("narratorHotkey")),
            Opt::RotateWithMinecarts => on_off(s.file.bool("rotateWithMinecart")),
            Opt::HighContrastBlockOutlines => on_off(s.file.bool("highContrastBlockOutline")),
            Opt::InvertX => on_off(s.invert_x),
            Opt::InvertY => on_off(s.invert_y),
            Opt::DiscreteScroll => on_off(s.discrete_scroll),
            Opt::AllowCursorChanges => on_off(s.allow_cursor_changes),
            Opt::ViewBobbing => on_off(s.view_bobbing),
            Opt::Vsync => on_off(s.vsync),
            Opt::Fullscreen => on_off(s.fullscreen),
            Opt::ExclusiveFullscreen => on_off(s.file.bool("exclusiveFullscreen")),
            Opt::TextureFiltering => tr([
                "options.textureFiltering.none",
                "options.textureFiltering.rgss",
                "options.textureFiltering.anisotropic",
            ][s.file.int("textureFiltering").clamp(0, 2) as usize]),
            Opt::GraphicsBackend => tr(match s.horizon.api {
                crate::graphics::GraphicsApi::OpenGl => "options.graphicsApi.opengl",
                crate::graphics::GraphicsApi::Vulkan => "options.graphicsApi.vulkan",
                _ => "options.graphicsApi.default",
            }),
            Opt::SmoothLighting => on_off(s.smooth_lighting),
            Opt::Clouds => tr(match s.clouds.as_str() {
                "false" => "options.off",
                "fast" => "options.clouds.fast",
                _ => "options.clouds.fancy",
            }),
            Opt::Particles => tr([
                "options.particles.all",
                "options.particles.decreased",
                "options.particles.minimal",
            ][usize::from(s.particles.min(2))]),
            Opt::EntityShadows => on_off(s.entity_shadows),
            Opt::Vignette => on_off(s.vignette),
            Opt::Autosave => on_off(s.autosave_indicator),
            Opt::CutoutLeaves => on_off(s.cutout_leaves),
            Opt::ImprovedTransparency => on_off(s.improved_transparency),
            Opt::AttackIndicator => tr([
                "options.off",
                "options.attack.crosshair",
                "options.attack.hotbar",
            ][usize::from(s.attack_indicator.min(2))]),
            Opt::InactivityFps => tr(if s.inactivity_fps_limit == "minimized" {
                "options.inactivityFpsLimit.minimized"
            } else {
                "options.inactivityFpsLimit.afk"
            }),
            Opt::ChunkBuilder => tr([
                "options.prioritizeChunkUpdates.none",
                "options.prioritizeChunkUpdates.byPlayer",
                "options.prioritizeChunkUpdates.nearby",
            ][usize::from(s.prioritize_chunk_updates.min(2))]),
            Opt::ToggleCrouch => tr(if s.toggle_crouch {
                "options.key.toggle"
            } else {
                "options.key.hold"
            }),
            Opt::ToggleSprint => tr(if s.toggle_sprint {
                "options.key.toggle"
            } else {
                "options.key.hold"
            }),
            Opt::SkinLayers3d => tr(if s.extras.three_d_skin_layers {
                "options.on"
            } else {
                "options.off"
            }),
            Opt::ToggleAttack => tr(if s.toggle_attack {
                "options.key.toggle"
            } else {
                "options.key.hold"
            }),
            Opt::ToggleUse => tr(if s.toggle_use {
                "options.key.toggle"
            } else {
                "options.key.hold"
            }),
            Opt::AutoJump => on_off(s.auto_jump),
            Opt::QuitShortcuts => on_off(s.quit_shortcuts),
            Opt::CtrlClickEmulatesRightClick => on_off(s.ctrl_click_emulates_right_click),
            Opt::OperatorItemsTab => on_off(s.operator_items_tab),
            Opt::ChatVisibility => tr([
                "options.chat.visibility.full",
                "options.chat.visibility.system",
                "options.chat.visibility.hidden",
            ][usize::from(s.chat_visibility.min(2))]),
            Opt::ChatColors => on_off(s.chat_colors),
            Opt::ChatLinks => on_off(s.chat_links),
            Opt::ChatLinksPrompt => on_off(s.chat_links_prompt),
            Opt::AutoSuggestions => on_off(s.auto_suggestions),
            Opt::ReducedDebugInfo => on_off(s.reduced_debug_info),
            Opt::HighContrast => on_off(s.high_contrast),
            Opt::ShowSubtitles => on_off(s.show_subtitles),
            Opt::SoundDevice => {
                if s.sound_device.is_empty() {
                    tr("options.audioDevice.default")
                } else {
                    Text::literal(&s.sound_device)
                }
            }
            Opt::MusicFrequency => tr([
                "options.music_frequency.default",
                "options.music_frequency.frequent",
                "options.music_frequency.constant",
            ][s.file.int("musicFrequency").clamp(0, 2) as usize]),
            Opt::ForceUnicode => on_off(s.force_unicode),
            Opt::JapaneseGlyphs => on_off(s.japanese_glyphs),
            Opt::PadEnabled => on_off(s.controller.enabled),
            Opt::PadVibration => on_off(s.controller.vibration),
            Opt::PadPlayInBackground => on_off(s.controller.play_in_background),
            Opt::PadAdaptiveTriggers => on_off(s.controller.adaptive_triggers),
            Opt::PadGameplayGuide => tr(match s.controller.guide_detail {
                gamepad::GuideDetail::Full => "horizon.options.gameplay_guide.full",
                gamepad::GuideDetail::Basic => "horizon.options.gameplay_guide.basic",
                gamepad::GuideDetail::Off => "horizon.options.gameplay_guide.off",
            }),
            Opt::PadInvertX => on_off(s.controller.invert_x),
            Opt::PadInvertY => on_off(s.controller.invert_y),
            Opt::PadServerStick => tr(&s.controller.server_stick.lang_key()),
            Opt::PadStickPrompt => on_off(s.controller.stick_prompt),
            Opt::PadCraftOnServers => on_off(s.controller.craft_on_servers),
            Opt::PadToggleSneak => tr(if s.controller.toggle_sneak {
                "options.key.toggle"
            } else {
                "options.key.hold"
            }),
            Opt::PadLightbar => tr(match s.controller.lightbar {
                gamepad::Lightbar::Health => "horizon.options.lightbar.health",
                gamepad::Lightbar::Dimension => "horizon.options.lightbar.dimension",
                gamepad::Lightbar::Off => "options.off",
            }),
            Opt::PadGlyphs => tr(match s.controller.glyphs {
                GlyphStyle::Auto => "horizon.options.glyphs.auto",
                GlyphStyle::Xbox => "horizon.options.glyphs.xbox",
                GlyphStyle::PlayStation => "horizon.options.glyphs.playstation",
                GlyphStyle::Nintendo => "horizon.options.glyphs.nintendo",
            }),
            Opt::PadNintendoButtons => tr(if s.controller.nintendo_letters {
                "horizon.options.nintendo_buttons.letter"
            } else {
                "horizon.options.nintendo_buttons.position"
            }),
            Opt::PadKeyboard => tr(match s.controller.keyboard {
                gamepad::TextKeyboard::Auto => "horizon.options.keyboard.auto",
                gamepad::TextKeyboard::Horizon => "horizon.options.keyboard.horizon",
                gamepad::TextKeyboard::Steam => "horizon.options.keyboard.steam",
            }),
            Opt::PadRadialMode | Opt::PadRadialStick => self.radial_text(s),
            Opt::Page(page, row) => pages::slider_text(page, row),
            _ => Text::empty(),
        }
    }
    /// Set a slider from its 0..1 position (OptionInstance value sets).
    pub fn set_slider(self, s: &mut Settings, v: f32) {
        if let Opt::ModSetting(i, j) = self {
            let setting = s
                .mods
                .installed
                .get_mut(usize::from(i))
                .and_then(|m| m.native.as_mut())
                .and_then(|page| page.settings.get_mut(usize::from(j)));
            if let Some(setting) = setting {
                setting.value.slide(v);
            }
            return;
        }
        if !self.available() {
            return;
        }
        let v = v.clamp(0.0, 1.0);
        let step = |lo: f32, hi: f32| (lo + v * (hi - lo)).round();
        match self {
            Opt::TouchCircleSize => s.touch.circle_size = step(16.0, 64.0),
            Opt::TouchDynamicAreaX => s.touch.dynamic_area_x = (v * 100.0).round() / 100.0,
            Opt::TouchDynamicAreaY => s.touch.dynamic_area_y = (v * 100.0).round() / 100.0,
            Opt::TouchHorizontalMargin => s.touch.horizontal_margin = (v * 120.0).round() / 1000.0,
            Opt::TouchVerticalMargin => s.touch.vertical_margin = (v * 120.0).round() / 1000.0,
            Opt::AndroidGyroSensitivity => s.touch.gyro_sensitivity = 0.1 + v * 2.9,
            Opt::TouchSensitivity => s.touch.sensitivity = 0.1 + v * 2.9,
            Opt::TouchSensitivityHorizontal => s.touch.sensitivity_horizontal = 0.1 + v * 2.9,
            Opt::TouchSensitivityVertical => s.touch.sensitivity_vertical = 0.1 + v * 2.9,
            Opt::TouchMouseSensitivity => s.touch.virtual_mouse_sensitivity = 0.25 + v * 2.75,
            Opt::TouchButtonScale => s.touch.button_scale = 0.5 + v * 1.5,

            Opt::FovEffects | Opt::DarknessPulsing => s.file.set(
                self.file_key().unwrap(),
                crate::options_file::V::Double(f64::from(v).powi(2)),
            ),
            Opt::DistortionEffects
            | Opt::DamageTilt
            | Opt::PanoramaSpeed
            | Opt::GlintSpeed
            | Opt::GlintStrength => s.file.set(
                self.file_key().unwrap(),
                crate::options_file::V::Double(f64::from(v)),
            ),
            Opt::Fov => s.fov = step(30.0, 110.0),
            Opt::RenderDistance => {
                s.render_distance =
                    step(2.0, crate::options_file::MAX_RENDER_DISTANCE as f32) as i32
            }
            Opt::SimulationDistance => s.simulation_distance = step(5.0, 32.0) as i32,
            Opt::Sensitivity => s.sensitivity = v,
            Opt::WheelSensitivity => s.mouse_wheel_sensitivity = 0.01 + v * 9.99,
            Opt::Volume(0) => s.master_volume = v,
            Opt::Volume(i) => s.volumes[i] = v,
            Opt::Gamma => s.gamma = v,
            Opt::MaxFps => s.max_fps = (step(1.0, 26.0) as u32) * 10,
            Opt::CloudRange => s.cloud_range = step(2.0, 128.0) as i32,
            Opt::EntityDistance => s.entity_distance = step(2.0, 20.0) / 4.0,
            Opt::BiomeBlend => s.biome_blend = step(0.0, 7.0) as i32,
            Opt::Preset => s.graphics_preset = PRESETS[step(0.0, 3.0) as usize].into(),
            Opt::Mipmaps => s.mipmap_levels = step(0.0, 4.0) as i32,
            Opt::MaxAnisotropy => s.file.set(
                "maxAnisotropyBit",
                crate::options_file::V::Int(step(1.0, 3.0) as i32),
            ),
            Opt::ExclusiveMode => {
                // Window.setPreferredFullscreenVideoMode: -1 is the desktop's own mode.
                let Some(last) = s.video_modes.len().checked_sub(1) else {
                    return;
                };
                let index = step(-1.0, last as f32) as i32;
                s.file.fullscreen_resolution = usize::try_from(index)
                    .ok()
                    .map(|i| s.video_modes[i].write());
            }
            Opt::WeatherRadius => s.weather_radius = step(3.0, 10.0) as i32,
            Opt::ChunkFade => s.chunk_fade = step(0.0, 40.0) / 20.0,
            Opt::MenuBlur => s.menu_blur = step(0.0, 10.0) as i32,
            Opt::NotificationDisplayTime => s.notification_display_time = step(5.0, 100.0) / 10.0,
            Opt::ChatOpacity => s.chat_opacity = v,
            Opt::TextBackgroundOpacity => s.text_background_opacity = v,
            Opt::ChatScale => s.chat_scale = v,
            Opt::ChatWidth => s.chat_width = v,
            Opt::ChatHeightFocused => s.chat_height_focused = v,
            Opt::ChatHeightUnfocused => s.chat_height_unfocused = v,
            Opt::ChatLineSpacing => s.chat_line_spacing = v,
            Opt::PadLookSensitivity => s.controller.look_sensitivity = v,
            Opt::PadPointerSpeed => s.controller.pointer_speed = step(25.0, 300.0) / 100.0,
            Opt::PadScrollSpeed => s.controller.scroll_speed = step(1.0, 36.0),
            Opt::PadRepeatDelay => s.controller.repeat_delay = step(100.0, 1000.0) / 1000.0,
            Opt::PadRepeatInterval => s.controller.repeat_interval = step(30.0, 300.0) / 1000.0,
            Opt::PadMoveDeadzone => s.controller.move_deadzone = step(0.0, 50.0) / 100.0,
            Opt::PadLookDeadzone => s.controller.look_deadzone = step(0.0, 50.0) / 100.0,
            Opt::PadTriggerThreshold => s.controller.trigger_threshold = step(10.0, 90.0) / 100.0,
            Opt::SprintWindow => s.sprint_window = step(0.0, 10.0) as i32,
            Opt::Page(page, row) => pages::slide(page, row, v),
            _ => {}
        }
        if self.affects_preset() {
            s.graphics_preset = "custom".into();
        }
    }
    fn affects_preset(self) -> bool {
        matches!(
            self,
            Opt::RenderDistance
                | Opt::SimulationDistance
                | Opt::SmoothLighting
                | Opt::Clouds
                | Opt::CloudRange
                | Opt::Particles
                | Opt::EntityDistance
                | Opt::EntityShadows
                | Opt::BiomeBlend
                | Opt::Mipmaps
                | Opt::WeatherRadius
                | Opt::CutoutLeaves
                | Opt::ImprovedTransparency
                | Opt::MenuBlur
                | Opt::ChunkBuilder
                | Opt::TextureFiltering
                | Opt::MaxAnisotropy
        )
    }
    /// Advance a cycle button (CycleButton.cycleValue).
    pub fn cycle(self, s: &mut Settings, back: bool, framebuffer: [i32; 2]) {
        if !self.available() {
            return;
        }
        let next = |n: u8, count: u8| {
            if back {
                (n + count - 1) % count
            } else {
                (n + 1) % count
            }
        };
        match self {
            Opt::TouchMode => {
                let i = crate::android_controls::Mode::SCHEMES
                    .iter()
                    .position(|(mode, split, _)| {
                        *mode == s.touch.mode
                            && (!matches!(
                                mode,
                                crate::android_controls::Mode::Dpad
                                    | crate::android_controls::Mode::Tap
                            ) || *split == s.touch.split_controls)
                    })
                    .unwrap_or(0);
                let (mode, split, _) = crate::android_controls::Mode::SCHEMES[usize::from(next(
                    i as u8,
                    crate::android_controls::Mode::SCHEMES.len() as u8,
                ))];
                s.touch.mode = mode;
                s.touch.split_controls = split;
            }
            Opt::AndroidGyroscope => s.touch.gyroscope ^= true,
            Opt::TouchAnalog => s.touch.analog_movement ^= true,
            Opt::TouchInteract => s.touch.contextual_interact ^= true,
            Opt::TouchMouseButton => s.touch.virtual_mouse_enabled ^= true,
            Opt::TouchKeyboardButton => s.touch.keyboard_button ^= true,
            Opt::TouchOffhand => s.touch.offhand_button ^= true,
            Opt::TouchShield => s.touch.shield_button ^= true,
            Opt::TouchLeftHand => s.touch.left_handed ^= true,
            Opt::TouchSprintButton => s.touch.sprint_button ^= true,
            Opt::TouchSprintToggle => s.touch.sprint_toggle ^= true,
            Opt::TouchInventoryControls => s.touch.inventory_controls ^= true,
            Opt::TouchDynamicJoystick => s.touch.dynamic_joystick ^= true,
            Opt::TouchCrosshairGestures => s.touch.crosshair_gestures ^= true,
            Opt::TouchControlHub => s.touch.show_control_hub ^= true,
            Opt::TouchUiVibration => s.touch.ui_vibrate ^= true,
            Opt::TouchGameVibration => s.touch.vibrate ^= true,
            Opt::TouchAttackButtonLook => s.touch.attack_button_look ^= true,
            Opt::TouchAttackSide => s.touch.attack_indicator_left ^= true,
            Opt::TouchSwapJumpSneak => s.touch.swap_jump_sneak ^= true,
            Opt::TouchAutoSprint => s.touch.sprint_with_joystick ^= true,
            Opt::TouchAvoidCutout => s.touch.avoid_cutout ^= true,
            Opt::AndroidInputSelection => s.touch.input_selection.cycle(back),

            Opt::GuiScale => {
                let max = crate::window::calculate_scale(framebuffer, 0, s.force_unicode) as u32;
                let value = s.gui_scale.min(max);
                s.gui_scale = if back {
                    if value == 0 { max } else { value - 1 }
                } else if value == max {
                    0
                } else {
                    value + 1
                };
            }
            Opt::TextBackground
            | Opt::HideLightningFlashes
            | Opt::HideSplashTexts
            | Opt::HighContrastBlockOutlines
            | Opt::MonochromeLogo
            | Opt::HideMatchedNames
            | Opt::OnlySecureChat
            | Opt::SaveChatDrafts
            | Opt::ModelPart(_)
            | Opt::AllowServerListing
            | Opt::RealmsNotifications
            | Opt::InGameNotification
            | Opt::DirectionalAudio
            | Opt::RotateWithMinecarts
            | Opt::NarratorHotkey => {
                let key = self.file_key().unwrap();
                s.file
                    .set(key, crate::options_file::V::Bool(!s.file.bool(key)));
            }
            // OptionInstance.Enum over NarratorStatus.values().
            Opt::Narrator => {
                let n = next(s.narrator().id() as u8, 4);
                s.set_narrator(horizon_narrator::Status::by_id(i32::from(n)));
            }
            Opt::InvertX => s.invert_x = !s.invert_x,
            Opt::InvertY => s.invert_y = !s.invert_y,
            Opt::DiscreteScroll => s.discrete_scroll = !s.discrete_scroll,
            Opt::AllowCursorChanges => s.allow_cursor_changes = !s.allow_cursor_changes,
            Opt::ViewBobbing => s.view_bobbing = !s.view_bobbing,
            Opt::Vsync => s.vsync = !s.vsync,
            Opt::Fullscreen => s.fullscreen = !s.fullscreen,
            Opt::ExclusiveFullscreen => {
                let on = s.file.bool("exclusiveFullscreen");
                s.file
                    .set("exclusiveFullscreen", crate::options_file::V::Bool(!on));
            }
            Opt::TextureFiltering => {
                let n = next(s.file.int("textureFiltering").clamp(0, 2) as u8, 3);
                s.file.set(
                    "textureFiltering",
                    crate::options_file::V::Int(i32::from(n)),
                );
            }
            Opt::GraphicsBackend => {
                // PreferredGraphicsApi: Default, OpenGL, Vulkan; a platform lacking one skips it.
                use crate::graphics::GraphicsApi::{Auto, OpenGl, Vulkan};
                let offered: Vec<_> = [Auto, OpenGl, Vulkan]
                    .into_iter()
                    .filter(|a| crate::graphics::GraphicsApi::choices().contains(a))
                    .collect();
                let at = offered
                    .iter()
                    .position(|a| *a == s.horizon.api)
                    .unwrap_or(0);
                let count = offered.len();
                let api = offered[if back {
                    (at + count - 1) % count
                } else {
                    (at + 1) % count
                }];
                s.horizon.api = api;
                s.file.set_name(
                    "preferredGraphicsBackend",
                    match api {
                        OpenGl => "opengl",
                        Vulkan => "vulkan",
                        _ => "default",
                    },
                );
            }
            Opt::SmoothLighting => s.smooth_lighting = !s.smooth_lighting,
            Opt::Clouds => {
                let order = ["false", "fast", "true"];
                let i = order.iter().position(|c| *c == s.clouds).unwrap_or(2) as u8;
                s.clouds = order[usize::from(next(i, 3))].into();
            }
            Opt::Particles => s.particles = next(s.particles, 3),
            Opt::EntityShadows => s.entity_shadows = !s.entity_shadows,
            Opt::Vignette => s.vignette = !s.vignette,
            Opt::Autosave => s.autosave_indicator = !s.autosave_indicator,
            Opt::CutoutLeaves => s.cutout_leaves = !s.cutout_leaves,
            Opt::ImprovedTransparency => s.improved_transparency = !s.improved_transparency,
            Opt::AttackIndicator => s.attack_indicator = next(s.attack_indicator, 3),
            Opt::InactivityFps => {
                s.inactivity_fps_limit = if s.inactivity_fps_limit == "afk" {
                    "minimized".into()
                } else {
                    "afk".into()
                }
            }
            Opt::ChunkBuilder => s.prioritize_chunk_updates = next(s.prioritize_chunk_updates, 3),
            Opt::ToggleCrouch => s.toggle_crouch = !s.toggle_crouch,
            Opt::ToggleSprint => s.toggle_sprint = !s.toggle_sprint,
            Opt::SkinLayers3d => s.extras.three_d_skin_layers = !s.extras.three_d_skin_layers,
            Opt::ToggleAttack => s.toggle_attack = !s.toggle_attack,
            Opt::ToggleUse => s.toggle_use = !s.toggle_use,
            Opt::AutoJump => s.auto_jump = !s.auto_jump,
            Opt::QuitShortcuts => s.quit_shortcuts = !s.quit_shortcuts,
            Opt::CtrlClickEmulatesRightClick => {
                s.ctrl_click_emulates_right_click = !s.ctrl_click_emulates_right_click
            }
            Opt::OperatorItemsTab => s.operator_items_tab = !s.operator_items_tab,
            Opt::ChatVisibility => s.chat_visibility = next(s.chat_visibility, 3),
            Opt::ChatColors => s.chat_colors = !s.chat_colors,
            Opt::ChatLinks => s.chat_links = !s.chat_links,
            Opt::ChatLinksPrompt => s.chat_links_prompt = !s.chat_links_prompt,
            Opt::AutoSuggestions => s.auto_suggestions = !s.auto_suggestions,
            Opt::ReducedDebugInfo => s.reduced_debug_info = !s.reduced_debug_info,
            Opt::HighContrast => s.high_contrast = !s.high_contrast,
            Opt::ShowSubtitles => s.show_subtitles = !s.show_subtitles,
            Opt::SoundDevice => {
                let devices = crate::audio::output_devices();
                let mut all = vec![String::new()];
                for d in devices {
                    if !all.contains(&d) {
                        all.push(d);
                    }
                }
                let current_idx = all.iter().position(|d| d == &s.sound_device).unwrap_or(0);
                let count = all.len();
                let next_idx = if back {
                    (current_idx + count - 1) % count
                } else {
                    (current_idx + 1) % count
                };
                s.sound_device = all[next_idx].clone();
            }
            Opt::MusicFrequency => {
                let n = next(s.file.int("musicFrequency").clamp(0, 2) as u8, 3);
                s.file
                    .set("musicFrequency", crate::options_file::V::Int(i32::from(n)));
            }
            Opt::MainHand => s.main_hand_left = !s.main_hand_left,
            Opt::SharePresence => {
                let n = next(s.file.int("sharePresence").clamp(0, 2) as u8, 3);
                s.file
                    .set("sharePresence", crate::options_file::V::Int(i32::from(n)));
            }
            Opt::MusicToast => {
                let n = next(s.file.int("musicToast").clamp(0, 2) as u8, 3);
                s.file
                    .set("musicToast", crate::options_file::V::Int(i32::from(n)));
            }
            Opt::ForceUnicode => s.force_unicode = !s.force_unicode,
            Opt::JapaneseGlyphs => s.japanese_glyphs = !s.japanese_glyphs,
            Opt::PadEnabled => s.controller.enabled = !s.controller.enabled,
            Opt::PadVibration => s.controller.vibration = !s.controller.vibration,
            Opt::PadPlayInBackground => {
                s.controller.play_in_background = !s.controller.play_in_background
            }
            Opt::PadAdaptiveTriggers => {
                s.controller.adaptive_triggers = !s.controller.adaptive_triggers
            }
            Opt::PadInvertX => s.controller.invert_x = !s.controller.invert_x,
            Opt::PadInvertY => s.controller.invert_y = !s.controller.invert_y,
            Opt::PadCraftOnServers => {
                s.controller.craft_on_servers = !s.controller.craft_on_servers
            }
            Opt::PadToggleSneak => s.controller.toggle_sneak = !s.controller.toggle_sneak,
            Opt::PadServerStick => s.controller.server_stick = s.controller.server_stick.next(),
            Opt::PadStickPrompt => s.controller.stick_prompt = !s.controller.stick_prompt,
            Opt::PadNintendoButtons => {
                s.controller.nintendo_letters = !s.controller.nintendo_letters
            }
            Opt::PadLightbar => {
                let all = gamepad::Lightbar::ALL;
                let i = all
                    .iter()
                    .position(|l| *l == s.controller.lightbar)
                    .unwrap_or(0) as u8;
                s.controller.lightbar = all[usize::from(next(i, 3))];
            }
            Opt::PadGlyphs => {
                let i = GlyphStyle::ALL
                    .iter()
                    .position(|g| *g == s.controller.glyphs)
                    .unwrap_or(0) as u8;
                s.controller.glyphs = GlyphStyle::ALL[usize::from(next(i, 4))];
            }
            Opt::PadGameplayGuide => {
                let all = gamepad::GuideDetail::ALL;
                let i = all
                    .iter()
                    .position(|d| *d == s.controller.guide_detail)
                    .unwrap_or(0) as u8;
                s.controller.guide_detail = all[usize::from(next(i, 3))];
            }
            Opt::PadKeyboard => {
                let all = gamepad::TextKeyboard::ALL;
                let i = all
                    .iter()
                    .position(|k| *k == s.controller.keyboard)
                    .unwrap_or(0) as u8;
                s.controller.keyboard = all[usize::from(next(i, 3))];
            }
            Opt::PadRadialMode | Opt::PadRadialStick => self.radial_step(s, back),
            _ => {}
        }
        if self.affects_preset() {
            s.graphics_preset = "custom".into();
        }
    }
}
const PRESETS: [&str; 4] = ["fast", "fancy", "fabulous", "custom"];
fn preset_index(p: &str) -> usize {
    PRESETS.iter().position(|x| *x == p).unwrap_or(1)
}
/// A slider moved to `v` (OptionInstance.set): the graphics preset applies
/// its options when its value changes (`Options.applyGraphicsPreset`).
pub fn set_slider(opt: Opt, s: &mut Settings, v: f32) {
    let preset = preset_index(&s.graphics_preset);
    opt.set_slider(s, v);
    if opt == Opt::Preset && preset_index(&s.graphics_preset) != preset {
        apply_preset(s);
    }
}
/// A slider pressed or dragged to `v`: apply it live, and tell the game when
/// a category volume changed (`Options.createSoundSliderOptionInstance`'s
/// callback, which `OptionInstance.set` runs only for a new value).
fn drag_slider(opt: Opt, s: &mut Settings, v: f32, events: &mut Vec<Event>) {
    let volume = |s: &Settings| match opt {
        Opt::Volume(0) => Some(s.master_volume),
        Opt::Volume(i) => Some(s.volumes[i]),
        _ => None,
    };
    let before = volume(s);
    set_slider(opt, s, v);
    events.push(Event::SliderDrag);
    if let Opt::ModSetting(i, j) = opt {
        events.push(Event::NativeSetting(usize::from(i), usize::from(j)));
    }
    if let (Opt::Volume(source), Some(after)) = (opt, volume(s))
        && before != Some(after)
    {
        events.push(Event::Volume(source, after));
    }
}
/// GraphicsPreset.apply: the options Fast, Fancy and Fabulous set.
pub fn apply_preset(s: &mut Settings) {
    let fabulous = s.graphics_preset == "fabulous";
    match s.graphics_preset.as_str() {
        "fast" => {
            s.biome_blend = 1;
            s.render_distance = 8;
            s.prioritize_chunk_updates = 0;
            s.simulation_distance = 6;
            s.smooth_lighting = false;
            s.clouds = "fast".into();
            s.particles = 1;
            s.mipmap_levels = 2;
            s.entity_shadows = false;
            s.entity_distance = 0.75;
            s.menu_blur = 2;
            s.cloud_range = 32;
            s.cutout_leaves = false;
            s.improved_transparency = false;
            s.weather_radius = 5;
            set_filtering(s, 1, 0);
        }
        "fancy" | "fabulous" => {
            s.biome_blend = 2;
            s.render_distance = if fabulous { 32 } else { 16 };
            s.prioritize_chunk_updates = 1;
            s.simulation_distance = 12;
            s.smooth_lighting = true;
            s.clouds = "true".into();
            s.particles = 0;
            s.mipmap_levels = 4;
            s.entity_shadows = true;
            s.entity_distance = if fabulous { 1.25 } else { 1.0 };
            s.menu_blur = 5;
            s.cloud_range = if fabulous { 128 } else { 64 };
            s.cutout_leaves = true;
            s.improved_transparency = fabulous;
            s.weather_radius = 10;
            // Fabulous takes anisotropic filtering (RGSS on a device with known issues, which
            // wgpu does not report).
            set_filtering(
                s,
                if fabulous { 2 } else { 1 },
                if fabulous { 2 } else { 1 },
            );
        }
        _ => {}
    }
}
/// The preset's `maxAnisotropyBit` and `textureFiltering` (0 none, 1 RGSS, 2 anisotropic).
fn set_filtering(s: &mut Settings, anisotropy_bit: i32, filtering: i32) {
    use crate::options_file::V;
    s.file.set("maxAnisotropyBit", V::Int(anisotropy_bit));
    s.file.set("textureFiltering", V::Int(filtering));
}

/// `Options.preferredGraphicsBackend`'s tooltip: the restart note once the choice differs from
/// the startup one, the description, and Vulkan's warning.
/// Android's own rows. A constant: horizon-mixins only reads `format!`, `matches!` and `vec!` in
/// a function it hooks.
const ON_A_PHONE: bool = cfg!(target_os = "android");

/// The Options page's entries, with Diagnostics on a phone. Apart from `OptionsUi::layout`:
/// horizon-mixins refuses to hook a function with a `#[cfg]` in it (E_UNSUPPORTED_CFG).
fn with_diagnostics(
    entries: impl Into<Vec<(&'static str, Option<Page>)>>,
) -> Vec<(&'static str, Option<Page>)> {
    let mut all = entries.into();
    if ON_A_PHONE {
        all.push(("starling.diagnostics.title", Some(Page::Diagnostics)));
    }
    all
}

/// The catalog's shelf for a page with a footer that has a Catalog button. The Resource Packs page
/// draws its own (`pack_screen`).
fn catalog_shelf(page: Page) -> Option<CatalogShelf> {
    match page {
        Page::Mods | Page::NativeGame => Some(CatalogShelf::Mods),
        Page::ShaderPacks => Some(CatalogShelf::ShaderPacks),
        _ => None,
    }
}

fn graphics_api_tooltip(s: &Settings) -> Text {
    let line = |key: &str| lang::get_or(key, key);
    let mut lines = Vec::new();
    if s.horizon.api != s.startup_api {
        lines.push(line("options.needsRestart"));
        lines.push(String::new());
    }
    lines.push(line("options.graphicsApi.tooltip"));
    if s.horizon.api == crate::graphics::GraphicsApi::Vulkan {
        lines.push(String::new());
        lines.push(line("options.graphicsApi.tooltip.vulkan"));
    }
    Text::literal(lines.join("\n"))
}

/// `selected_key` while a key a native mod registered waits (`selected_mod_key`).
const MOD_KEY: &str = "horizon.modKey";

/// The id of mod `i`'s registered key `k` in `mods.installed`, with the mod's id.
fn mod_mapping(s: &Settings, i: usize, k: usize) -> Option<(String, String)> {
    let m = s.mods.installed.get(i)?;
    let mapping = m.native.as_ref()?.mappings.get(k)?;
    Some((m.id.clone(), mapping.id.clone()))
}

fn key_conflicts(
    s: &Settings,
    owner: &str,
    id: &str,
    key: &str,
    modifiers: u32,
    context: crate::mod_list::KeyConflictContext,
    at_default: bool,
) -> Vec<String> {
    use crate::mod_list::{KeyConflictContext, bindings_conflict};
    let binding = (key, modifiers, context, at_default);
    let mut conflicts = Vec::new();
    for &(name, _, _) in keybinds::MAPPINGS {
        if !(owner.is_empty() && name == id)
            && bindings_conflict(
                binding,
                (
                    s.keys.get(name),
                    0,
                    KeyConflictContext::Universal,
                    s.keys.is_default(name),
                ),
            )
        {
            conflicts.push(lang::get(name));
        }
    }
    for installed in &s.mods.installed {
        for mapping in installed.native.iter().flat_map(|page| &page.mappings) {
            if installed.id == owner && mapping.id == id {
                continue;
            }
            let (other_key, other_modifiers) = s.mods.binding(
                &installed.id,
                &mapping.id,
                (&mapping.key, mapping.modifiers),
            );
            if bindings_conflict(
                binding,
                (
                    &other_key,
                    other_modifiers,
                    mapping.context,
                    other_key == mapping.key && other_modifiers == mapping.modifiers,
                ),
            ) {
                conflicts.push(lang::get(&mapping.label));
            }
        }
    }
    conflicts
}

/// A native mod's title or key category on Controls: bold and underlined, centred on the row at `y`.
fn section_title(text: Text, y: f32, w: f32) -> Widget {
    Widget {
        x: 0.0,
        y: y + 7.0,
        w,
        h: 9.0,
        kind: WKind::Text {
            text: text.with_style(&Style {
                bold: Some(true),
                underlined: Some(true),
                ..Style::default()
            }),
            color: u32::MAX,
            centered: true,
        },
        list: false,
        row: 0,
    }
}

/// A registered key's binding as Controls shows it: its modifiers' key names, then its key's.
fn binding_text(key: &str, modifiers: u32) -> Text {
    let mut text = Text::empty();
    for (bit, name) in [
        (crate::mod_list::SHIFT, "key.keyboard.left.shift"),
        (crate::mod_list::CONTROL, "key.keyboard.left.control"),
        (crate::mod_list::ALT, "key.keyboard.left.alt"),
    ] {
        if modifiers & bit != 0 {
            text = text.append(keybinds::display_name(name)).append(" + ");
        }
    }
    text.append(keybinds::display_name(key))
}

/// Which long list a custom button's page opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustomPick {
    Control,
    Keyboard,
    Icon,
}

/// What a finger paints with, on a custom button's canvas; the last clears.
const PALETTE: [[u8; 4]; 16] = [
    [255, 255, 255, 255],
    [190, 190, 190, 255],
    [120, 120, 120, 255],
    [38, 38, 38, 255],
    [214, 60, 60, 255],
    [240, 140, 40, 255],
    [245, 215, 70, 255],
    [90, 190, 70, 255],
    [40, 120, 60, 255],
    [70, 200, 210, 255],
    [60, 110, 220, 255],
    [130, 70, 200, 255],
    [230, 110, 190, 255],
    [130, 85, 50, 255],
    [245, 210, 170, 255],
    [0, 0, 0, 0],
];

/// The canvas of a custom button's own image.
#[derive(Default)]
struct Painter {
    pixels: Vec<u8>,
    /// Its place on the page: x, y and how many GUI pixels a pixel is.
    at: [f32; 3],
    color: usize,
    /// The pixel the finger was last on, to fill in between when it moves fast.
    last: Option<(i32, i32)>,
    /// It changed since it opened: Done keeps it.
    dirty: bool,
    /// A kept image the atlas's slot has yet to get.
    upload: Option<(u32, Vec<u8>)>,
}

impl Painter {
    /// A touch sprite as the atlas has it (Bedrock's look where the phone fetched it), a button's
    /// size; else the game's own.
    fn sprite(atlas: &HudAtlas, name: &str) -> Option<Vec<u8>> {
        let rect = atlas
            .sprites
            .get(&format!("starling:gui/sprites/touch/{name}"));
        let cut = rect.and_then(|r| {
            let (x0, y0) = (
                (r.u0 * atlas.width as f32).round() as u32,
                (r.v0 * atlas.height as f32).round() as u32,
            );
            let mut pixels = Vec::with_capacity((r.w * r.h * 4) as usize);
            let all = atlas.pixels();
            for y in y0..y0 + r.h {
                let at = ((y * atlas.width + x0) * 4) as usize;
                pixels.extend_from_slice(all.get(at..at + (r.w * 4) as usize)?);
            }
            Some(own_buttons::fit(r.w, r.h, &pixels))
        });
        cut.or_else(|| crate::hud::touch_sprite(name))
    }

    /// A button's own image, or the icon it wears, else a blank button, to draw on.
    fn open(&mut self, atlas: &HudAtlas, button: Option<&crate::settings::CustomButton>) {
        self.dirty = false;
        self.last = None;
        let side = own_buttons::SIZE as usize;
        let own = button
            .filter(|b| b.image)
            .and_then(|b| own_buttons::load(b.id));
        let icon = button
            .and_then(|b| b.icon.as_deref())
            .and_then(|icon| Self::sprite(atlas, &format!("custom_{icon}")));
        self.pixels = own
            .or(icon)
            .or_else(|| Self::sprite(atlas, "custom_blank"))
            .filter(|p| p.len() == side * side * 4)
            .unwrap_or_else(|| vec![0; side * side * 4]);
    }

    /// The image a button wears while pressed, else a pressed blank button, to draw on.
    fn open_pressed(&mut self, atlas: &HudAtlas, slot: u32) {
        self.dirty = false;
        self.last = None;
        let side = own_buttons::SIZE as usize;
        self.pixels = own_buttons::load(slot)
            .or_else(|| Self::sprite(atlas, "custom_blank_pressed"))
            .or_else(|| Self::sprite(atlas, "custom_blank"))
            .filter(|p| p.len() == side * side * 4)
            .unwrap_or_else(|| vec![0; side * side * 4]);
    }

    fn clear(&mut self) {
        self.pixels.fill(0);
        self.dirty = true;
    }

    /// Back to the plate of the game's own buttons.
    fn plate(&mut self, atlas: &HudAtlas) {
        if let Some(plate) = Self::sprite(atlas, "custom_blank") {
            self.pixels = plate;
            self.dirty = true;
        }
    }

    /// A picture the system handed over.
    fn import(&mut self, width: u32, height: u32, rgba: &[u8]) {
        self.pixels = own_buttons::fit(width, height, rgba);
        self.dirty = true;
    }

    /// The swatch at `i`: x and y on the page, beside the canvas.
    fn swatch(&self, i: usize) -> (f32, f32) {
        let [x, y, cell] = self.at;
        let right = x + own_buttons::SIZE as f32 * cell + 8.0;
        (right + (i % 2) as f32 * 16.0, y + (i / 2) as f32 * 16.0)
    }

    fn paint(&mut self, x: i32, y: i32) {
        let side = own_buttons::SIZE as i32;
        if (0..side).contains(&x) && (0..side).contains(&y) {
            let at = ((y * side + x) * 4) as usize;
            if let Some(pixel) = self.pixels.get_mut(at..at + 4) {
                pixel.copy_from_slice(&PALETTE[self.color]);
                self.dirty = true;
            }
        }
    }

    /// A press or a drag on the page; true when it landed on the canvas or a swatch.
    fn press(&mut self, x: f32, y: f32, drag: bool) -> bool {
        let [left, top, cell] = self.at;
        let side = own_buttons::SIZE as f32;
        if !drag {
            self.last = None;
            for i in 0..PALETTE.len() {
                let (sx, sy) = self.swatch(i);
                if ui::hovered((x, y), sx, sy, 14.0, 14.0) {
                    self.color = i;
                    return true;
                }
            }
        }
        if !ui::hovered((x, y), left, top, side * cell, side * cell) {
            // A finger sliding off lifts the stroke, and comes back without a line across.
            self.last = None;
            return drag;
        }
        let now = (((x - left) / cell) as i32, ((y - top) / cell) as i32);
        // Every pixel between the finger's last place and this.
        let (x0, y0) = self.last.unwrap_or(now);
        let steps = (now.0 - x0).abs().max((now.1 - y0).abs()).max(1);
        for t in 0..=steps {
            self.paint(x0 + (now.0 - x0) * t / steps, y0 + (now.1 - y0) * t / steps);
        }
        self.last = Some(now);
        true
    }

    fn draw(&self, batch: &mut HudBatch, atlas: &HudAtlas) {
        let [left, top, cell] = self.at;
        let side = own_buttons::SIZE as usize;
        let span = side as f32 * cell;
        batch.fill(
            atlas,
            left - 1.0,
            top - 1.0,
            span + 2.0,
            span + 2.0,
            rgba(16, 16, 16, 255),
        );
        for y in 0..side {
            for x in 0..side {
                let (px, py) = (left + x as f32 * cell, top + y as f32 * cell);
                // Squares under what is clear, so it shows as clear.
                let g = if (x + y) % 2 == 0 { 96 } else { 72 };
                batch.fill(atlas, px, py, cell, cell, rgba(g, g, g, 255));
                let p = self.pixels.get((y * side + x) * 4..(y * side + x) * 4 + 4);
                if let Some(&[r, g, b, a]) = p
                    && a > 0
                {
                    batch.fill(atlas, px, py, cell, cell, rgba(r, g, b, a));
                }
            }
        }
        for (i, [r, g, b, a]) in PALETTE.iter().copied().enumerate() {
            let (x, y) = self.swatch(i);
            let edge = if i == self.color {
                u32::MAX
            } else {
                rgba(16, 16, 16, 255)
            };
            batch.fill(atlas, x - 1.0, y - 1.0, 16.0, 16.0, edge);
            let face = if a == 0 {
                rgba(84, 84, 84, 255)
            } else {
                rgba(r, g, b, a)
            };
            batch.fill(atlas, x, y, 14.0, 14.0, face);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Diagnostics,
    Main,
    Video,
    Sound,
    Controls,
    Touch,
    KeyBinds,
    Mouse,
    Chat,
    Accessibility,
    Language,
    Font,
    Packs,
    Mods,
    /// Browse mods: the registry's content mods, searched (`mod_browser`).
    ModBrowser,
    /// One registry project's page: `OptionsUi::mod_project`.
    ModProject,
    /// A native mod's settings page: its index in `mods.installed`.
    NativeMod(usize),
    /// Android: the game with native mods and the mods chosen for its next build.
    NativeGame,
    /// A page a crate registered (`horizon_hooks::pages`): its index in `OptionsUi::pages`.
    Added(usize),
    /// Horizon's shader packs, either kind (`shader_pack_list`): one on or none, and a
    /// `shaders/` pack's profile.
    ShaderPacks,
    /// The picked pack's options: the shader screen (`shader_ui`) at the pack's main screen.
    ShaderOptions,
    /// The player's own touch buttons (`Settings::custom_buttons`), and what one of them does.
    CustomButtons,
    CustomButton(u32),
    CustomPick(u32, CustomPick),
    /// Drawing a custom button's own image.
    CustomDraw(u32),
    /// The selected resource packs' Polytone options (`pack_options_ui`), from the pack screen.
    PackOptions,
    /// Horizon's graphics settings (`graphics_ui`).
    Graphics,
    GraphicsAdvanced,
    GraphicsHdr,
    /// HDR calibration, full screen (`calibration`).
    HdrCalibration,
    /// Horizon's controller settings and button binds.
    Controller,
    PadBinds,
    /// The radial menu's pages (`radial_editor`), on the settings shell, and a page's name.
    ControllerRadial,
    ControllerRadialName,
    /// The list of every action for one of its slots.
    ControllerRadialPick,
    Extras,
    /// PvP & HUD (`pvp`): the extras a PvP player sets, on their own page.
    Pvp,
    /// CreditsAndAttributionScreen.
    Credits,
    /// Horizon Settings: every page Horizon adds, as a mod's settings
    /// screen hangs off vanilla's Options.
    Horizon,
    /// Replay Studio: saved recordings browser and 3D video editor launcher.
    Studio,
    /// AI Agent: agent control on or off, whether an agent is connected, the connection details.
    Agent,
    /// SkinCustomizationScreen: the skin layers and Main Hand, and in the game's own options
    /// (`skin_page`) the player's name and own skin (`own_skin`) before them.
    Skin,
    /// OnlineOptionsScreen: server listing, Realms and friends notifications.
    Online,
    /// Splitscreen: the profile another player joins as (`split_profiles`).
    SplitProfiles,
    /// Splitscreen: the monitor map and draft window arrangement.
    SplitSetup,
    /// The built-in music tracks, by situation: play one, turn any off
    /// (vanilla's `minecraft:audio/background_music` pools).
    Music,
}
#[derive(Clone, Debug, PartialEq)]
enum Act {
    DiagnosticsExport,
    DiagnosticsOpen,
    DiagnosticsRecord,
    DiagnosticsGpu,
    PerfLab(u8),
    None,
    Push(Page),
    Done,
    Cycle(Opt),
    Extra(crate::extras::Item),
    Key(&'static str),
    ResetKey(&'static str),
    ResetKeys,
    KeyboardProfile,
    /// A mod turned on or off on Starling's Mods page; a native mod chosen (or not) for the next
    /// build there (Android).
    ToggleMod(usize),
    /// Starling's Mods page: a mod's Settings beside its switch, its page (`Page::NativeMod`)
    /// with its labels asked of the game again.
    ModSettings(usize),
    /// Starling's Mods page: Open Mod Folder.
    OpenModFolder,
    /// The Custom Buttons page, a new button there, one to change or take away, a control or a
    /// keyboard key for it, and its icon.
    CustomButtons,
    CustomAdd,
    CustomEdit(u32),
    CustomDelete(u32),
    /// Toggled or held.
    CustomToggle(u32),
    CustomKey(u32, &'static str),
    CustomInput(u32, &'static str),
    /// A native mod's key (the mod's index in the installed list, the key's in its page) as the
    /// button's keyboard key: the key the player has set for it now.
    CustomModKey(u32, usize, usize),
    CustomIcon(u32, Option<&'static str>),
    /// Its own image, and what the drawing page does to the canvas.
    CustomDraw(u32),
    CanvasClear,
    CanvasPlate,
    CanvasImport(u32),
    /// PvP & HUD's PvP preset (`pvp::apply_preset`).
    PvpPreset,
    /// PvP & HUD's Edit HUD button.
    HudEditor,
    /// A native mod's key (mod, key) waits for a key; a button of its page.
    /// A button of the Mods page's game with native mods.
    NativeGame(crate::mod_list::NativeGameAct),
    OpenCatalog(CatalogShelf),
    /// The creative screen with the inventory beside it, or as the game draws it.
    BedrockInventory,
    /// The touch layout editor (phones).
    TouchLayout,
    /// A key a native mod registered (mod, mapping) waits for a key, or is reset.
    ModKey(usize, usize),
    ResetModKey(usize, usize),
    /// Setting `.1` of native mod `.0`'s page clicked: a toggle, choice or colour steps on.
    NativeSetting(usize, usize),
    PageRow(usize, usize),
    /// Browse mods: the page of listing `.0` of `mod_browser`.
    ModProject(usize),
    /// The open project page's Install or Update.
    InstallMod,
    /// The open project page's page on the website.
    OpenModPage,
    /// Accessibility's "Accessibility Help" link
    /// (`AccessibilityOptionsScreen.addFooter`'s openLinkButton).
    AccessibilityHelp,
    /// Credits & Attribution's Credits button (WinScreen without the poem).
    Credits,
    CopyAgentDetails,
    /// Skin Customization: the next skin of `skins/`, the other model, the system's file picker
    /// (phones), the folder, Ely.by's skins.
    NextSkin,
    SkinModel,
    PickSkin,
    OpenSkinFolder,
    ElyBySkins,
    /// The chosen skin to the Microsoft account's profile; an Ely.by account's skin page.
    UploadSkin,
    ElySkinSite,
    /// AI Agent: start or stop the repro recording.
    ToggleRecording,
    /// AI Agent: write the repro bundle now.
    ExportRepro,
    /// The Music page: play this track of this event now.
    PlayMusicTrack(String, String),
    /// The Music page: turn this track off, or back on.
    MusicTrackOn(String, bool),
}
enum WKind {
    Button {
        label: Text,
        active: bool,
        act: Act,
        /// A sprite drawn over it, filling it (an icon to pick).
        icon: Option<String>,
    },
    Slider {
        label: Text,
        value: f32,
        opt: Opt,
    },
    Text {
        text: Text,
        color: u32,
        centered: bool,
    },
    LanguageRow {
        label: Text,
        selected: bool,
        code: String,
    },
}
struct Widget {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    kind: WKind,
    /// Scrolls with the list (clipped to the list area).
    list: bool,
    /// The list row it sits in (`ContainerObjectSelectionList`'s entry).
    row: usize,
}

/// The keyboard-focused UI target on an options page.
#[derive(Clone, Debug, PartialEq)]
enum FocusId {
    Search,
    Slider(Opt),
    Cycle(Opt),
    Act(Act),
    Lang(String),
    Index(usize),
}

fn focus_id(i: usize, wd: &Widget) -> FocusId {
    match &wd.kind {
        WKind::Slider { opt, .. } => FocusId::Slider(*opt),
        WKind::Button {
            act: Act::Cycle(opt),
            ..
        } => FocusId::Cycle(*opt),
        WKind::Button { act, .. } if *act != Act::None => FocusId::Act(act.clone()),
        WKind::LanguageRow { code, .. } => FocusId::Lang(code.clone()),
        _ => FocusId::Index(i),
    }
}

fn matches_focus(id: &FocusId, i: usize, wd: &Widget) -> bool {
    match (id, &wd.kind) {
        (FocusId::Slider(a), WKind::Slider { opt, .. }) => a == opt,
        (
            FocusId::Cycle(a),
            WKind::Button {
                act: Act::Cycle(opt),
                ..
            },
        ) => a == opt,
        (FocusId::Act(a), WKind::Button { act, .. }) => a == act,
        (FocusId::Lang(a), WKind::LanguageRow { code, .. }) => a == code,
        (FocusId::Index(idx), _) => *idx == i,
        _ => false,
    }
}
/// The catalog's shelf a page's Catalog button opens. This crate can't depend on
/// `starling-catalog`, so the game maps it to the catalog's own (game/options.rs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogShelf {
    Mods,
    ResourcePacks,
    ShaderPacks,
}

/// What the game must do after an interaction.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    DiagnosticsExport,
    DiagnosticsOpen,
    DiagnosticsRecord,
    DiagnosticsGpu,
    /// One of the performance test's switches (`perf_lab`), by index: the next choice.
    PerfLab(u8),
    /// The Pack Options page closed with changes: the player's choices of its entries, the
    /// ones that differ from their defaults (the others went back to theirs).
    PackOptions {
        entries: Vec<String>,
        chosen: std::collections::BTreeMap<String, horizon_polytone::pack::ConfigValue>,
    },
    /// An accepted button press or the release of a slider.
    ButtonClick,
    /// Horizon's own: download the Modrinth hit at this index of the pack
    /// screen's browser. The game owns the packs folder, not this screen.
    DownloadPack(usize),
    /// Close the options screens (back to the pause menu or title).
    Close,
    /// Settings changed: apply and save them.
    Changed,
    /// The reduced-debug-info option rebuilt the debug entry list.
    DebugEntriesChanged,
    /// A slider is being dragged: apply settings live but defer saving.
    SliderDrag,
    /// A category volume slider (`SoundSource` index) moved to a new value,
    /// after its `SliderDrag` (the sound slider's `onValueUpdate`).
    Volume(usize, f32),
    /// The language changed (reload translations).
    Language(String),
    /// The pack selection changed (reload resources).
    Packs(Vec<String>),
    /// Create World's Data Packs screen closed with a new selection.
    DataPacks(Vec<String>),
    OpenPackFolder,
    /// Data Packs' Open Pack Folder: create the folder if needed and open it.
    OpenDataPackFolder(std::path::PathBuf),
    OpenModFolder,
    /// A link on the Mods screen (Homepage, Issues, a license): confirm and open it.
    OpenLink(String),
    /// A click on the Mods screen's mod id or version: copy it.
    CopyText(String),
    /// Browse mods opened: read the registry's index into `mod_browser`.
    BrowseMods,
    /// Install (or update) the registry's content mod with this id.
    InstallMod(String),
    /// Open a registry project's page on the website.
    OpenModPage(String),
    /// Accessibility's Help link: confirm and open
    /// `CommonLinks.ACCESSIBILITY_HELP` over the options screen.
    AccessibilityHelp,
    /// Android: a page's Catalog button (game/mod_catalog.rs).
    OpenCatalog(CatalogShelf),
    /// The Shader Packs page opened: list `shaderpacks/` into `shader_packs`.
    ListShaderPacks,
    /// The Mods page opened: the native mods folder read again, for a mod copied in meanwhile.
    ListMods,
    /// Installed mod `.0` was turned on or off (after `Changed`, which saved it).
    ModToggled(usize),
    /// Android: a button of the Mods page's game with native mods (`ModList::native_game`).
    NativeGame(crate::mod_list::NativeGameAct),
    /// A shader pack or its profile was picked, or the pack turned off: save and apply it.
    ShaderPack,
    /// The Shader Packs page's folder (on a phone without the Starling folder, its offer).
    OpenShaderPackFolder,
    /// The Shader Options page opened: read the picked pack's options into `shader_options`.
    OpenShaderOptions,
    /// The Shader Options page closed with changes: save them and load the pack again.
    ShaderOptions,
    /// The options of `shader_packs[.0]`, a Horizon pack, changed: save them (the pack reloads
    /// with them).
    NativePackOptions(usize),
    /// Native mod `.0`'s page opened: the game reads its settings' kept values again.
    NativeSettings(usize),
    /// Close the options and edit the touch controls' layout over the world.
    TouchLayout,
    /// The Custom Buttons page opened: `Settings::custom_buttons` takes what the layout has.
    OpenCustomButtons,
    /// The player's own touch buttons changed: the layout takes them.
    CustomButtons,
    /// A custom button's image was drawn: its file takes it.
    CustomImage(u32, Vec<u8>),
    /// Ask the system for a picture for a custom button.
    ImportButtonImage(u32),
    /// Setting `.1` of native mod `.0`'s page changed (`settings.mods`): the game keeps it and
    /// tells the mod (`setting_changed`).
    NativeSetting(usize, usize),
    /// Roll the credits (`CreditsAndAttributionScreen.openCreditsScreen`).
    Credits,
    /// PvP & HUD's Edit HUD: open the HUD editor over the game.
    HudEditor,
    /// Copy how an agent connects (the AI Agent page).
    CopyAgentDetails,
    /// The AI Agent page's recording button: start or stop.
    ToggleRecording,
    /// The AI Agent page's export button.
    ExportRepro,
    /// The narrator option changed (after its `Changed`): the option's
    /// `onValueUpdate`, `GameNarrator.updateNarratorStatus`.
    Narrator,
    /// The Music page opened: fill `music_groups` from the sound registry
    /// (the game owns it).
    ListMusicTracks,
    /// A Music page row: play this track of this event now.
    PlayMusicTrack(String, String),
    /// Open the recordings folder in the system file browser.
    OpenStudioFolder,
    /// Start playback from the selected Studio recording.
    OpenStudioReplay(std::path::PathBuf),
}
/// A row of the Music page: the track, and the pool it is played from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MusicTrack {
    /// The music event the track belongs to (`minecraft:music.game`).
    pub event: String,
    /// The sound id (`minecraft:music/game/calm1`).
    pub id: String,
}
/// One group of rows on the Music page: a situation, or a dimension's biomes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MusicGroup {
    /// The lang key of the heading the page shows (`horizon.pages.music.overworld`).
    pub label: String,
    /// The rows, in `sounds.json` order.
    pub tracks: Vec<MusicTrack>,
}
/// What Skin Customization asks of the game (`OptionsUi::take_skin_actions`), apart from
/// [`Event`]: the menus before the game, which don't show the page, need not know them.
#[derive(Clone, Debug, PartialEq)]
pub enum SkinAction {
    /// The name, checked (`offline_name::is_valid_choice`): keep it.
    Name(String),
    /// The skin or model changed: keep it and put it on.
    Choice(crate::own_skin::SkinChoice),
    /// The skins folder (on a phone without the Starling folder, its offer).
    OpenFolder,
    /// Ask the system for a skin file; it arrives through the import inbox.
    PickFile,
    /// Ely.by Skins turned on or off (`elyby`): keep it.
    ElyBy(bool),
    /// This skin to the playing Microsoft account's profile, for every server to show.
    Upload(crate::own_skin::SkinChoice),
    /// Ely.by's page where the playing Ely.by account changes its skin.
    ElySite,
}
/// `SliderableValueSet.applyValueImmediately`: these apply 600 ms after
/// the last change (`OptionInstanceSliderButton.delayedApplyAt`).
fn delays_apply(opt: Opt) -> bool {
    matches!(
        opt,
        Opt::RenderDistance | Opt::SimulationDistance | Opt::BiomeBlend | Opt::WheelSensitivity
    )
}
/// Whether a slider still shows its own position: while held, or for a
/// delayed one until its value applies (both then `resetValue`).
fn slider_unsnapped(opt: Opt, held: bool, moved: Instant) -> bool {
    if delays_apply(opt) {
        moved.elapsed().as_millis() < 600
    } else {
        held
    }
}
pub struct OptionsUi {
    pub stack: Vec<Page>,
    scroll: f32,
    pub(crate) dragging: Option<Opt>,
    /// Shift is held during the click (it resets a shader pack option to its default).
    pub shift: bool,
    /// The window's modifier keys (GLFW bits) when a key or button is bound: a mod's key keeps them.
    pub modifiers: u32,
    /// The pressed slider's own position (`AbstractSliderButton.value`) and
    /// when it last moved: the handle follows the pointer while the option
    /// keeps its stepped value, then snaps to it (`resetValue`).
    slider_value: Option<(Opt, f32, Instant)>,
    /// The pointer's last y while a scrollbar is held.
    pub(crate) scrollbar_drag_y: Option<f32>,
    /// Key Binds: the mapping waiting for a key (`MOD_KEY` for a registered mod key).
    pub selected_key: Option<&'static str>,
    /// The registered key waiting: (mod, mapping) in `mods.installed`.
    selected_mod_key: Option<(usize, usize)>,
    selected_page_key: Option<(usize, usize)>,
    pending_mod_key: Option<winit::keyboard::KeyCode>,
    /// Bindings: the action waiting for a pad button.
    pub selected_pad_action: Option<&'static str>,
    /// The glyphs menus show (`Glyphs::of`: the setting, else the pad's family, and a Nintendo
    /// pad's letters when it goes by them). The game sets it each frame.
    pub pad_glyphs: crate::pad_glyph::Glyphs,
    /// The pad's focus drives the page: its hidden pointer sits mid-widget, so a tooltip goes
    /// under the widget instead of beside the pointer.
    pub pad_driving: bool,
    pub languages: Vec<LanguageInfo>,
    language_choice: Option<String>,
    language_search: ui::EditBox,
    pub(crate) settings_search: ui::EditBox,
    /// What the custom button on its page sends.
    custom_send: ui::EditBox,
    /// The custom button on its page came from the touch editor's Add, and goes back there.
    back_to_editor: bool,
    /// The canvas of a custom button's image.
    painter: Painter,
    /// Browse mods: the registry's projects, filled by the game.
    pub mod_browser: crate::mod_browser::ModBrowser,
    /// The project whose page is open (`Page::ModProject`).
    pub mod_project: String,
    /// The line about mods that wait for a restart to switch, for Browse mods' message.
    pub mods_note: Option<String>,
    /// The Mods screen (`Page::Mods`), made when the page opens.
    pub mods_screen: Option<crate::mods_screen::ModsScreen>,
    /// Horizon's `packBrowser` extra: the pack screen's Add button.
    pub pack_browser: bool,
    /// Packs: the packs found and the selection the screen opens with.
    pub pack_infos: Vec<crate::pack_screen::PackInfo>,
    pub pack_selection: Vec<String>,
    /// The open pack screen, made at its first frame.
    pub pack_screen: Option<crate::pack_screen::PackScreen>,
    /// The Packs page is Create World's Data Packs screen on this folder.
    data_pack_dir: Option<std::path::PathBuf>,
    /// The game's resources, for the built-in data pack's icon.
    data_pack_resources: std::path::PathBuf,
    /// Additional repository entries retained while the folder watcher refreshes.
    additional_data_pack_infos: Vec<crate::pack_screen::PackInfo>,
    /// The folder's entries when its packs were listed, and when they were
    /// last compared with it.
    data_pack_stamp: (Vec<(String, Option<std::time::SystemTime>)>, Instant),
    /// A reload failure, dismissed before returning to the current page.
    pub resource_error: Option<String>,
    /// When the pack screen's search box took the focus (its cursor blink).
    pack_focus: Instant,
    /// The selection carried over from a pack screen made again over a changed pack list
    /// (`update_pack_infos`) differs from the settings'.
    pack_reselected: bool,
    pub in_world: bool,
    /// AccessibilityOptionsScreen: the loaded world's experimental feature.
    pub minecart_improvements: bool,
    /// Scroll the selected language into the middle on the next layout.
    center_language: bool,
    /// The output of the renderer the options are drawn for, as of the last draw. It lives
    /// here, not on the graphics screen, which opening resets: `shell_mut` hands it over.
    output: crate::gfx::EffectiveOutput,
    graphics: shell::Shell<crate::graphics_ui::GraphicsScreen>,
    /// Controller Settings, on the settings shell.
    controller: shell::Shell<crate::controller_ui::ControllerScreen>,
    /// Horizon Extras, on the settings shell.
    extras: shell::Shell<crate::extras_ui::ExtrasScreen>,
    /// The radial menu's pages, on the settings shell.
    radial: shell::Shell<crate::radial_editor::RadialScreen>,
    /// The list of actions for one of the radial menu's slots.
    radial_pick: shell::Shell<crate::radial_editor::PickScreen>,
    /// Shaders: the shader packs and the picked one's options, on the settings shell.
    shader: shell::Shell<crate::shader_ui::ShaderScreen>,
    /// The Pack Options page, and the packs' Polytone config entries by pack id (the game
    /// fills them beside `pack_infos`).
    pub pack_options: crate::pack_options_ui::PackOptionsUi,
    pub pack_configs: std::collections::BTreeMap<String, horizon_polytone::pack::Configs>,
    pub split_profiles: crate::split_profiles::SplitProfiles,
    /// HDR calibration while its page is open.
    pub calibration: Option<crate::hdr_calibration::Calibration>,
    /// The window as calibration found it before making it fullscreen (the game keeps it).
    pub calibration_window: Option<crate::hdr_calibration::WindowState>,
    pub split_setup: crate::split_ui::SplitUi,
    pub studio: crate::studio_ui::StudioUi,
    /// The AI Agent page's status lines (set by the game while it shows).
    pub agent_status: Vec<String>,
    pub diagnostics_status: Vec<String>,
    /// Starling's performance test on the Diagnostics page (game/starling/perf_lab.rs): its
    /// switches' labels, each a button (`Event::PerfLab` with its index), and the lines after
    /// them. Empty: no test section.
    pub perf_lab: Vec<String>,
    pub perf_lab_notes: Vec<String>,
    /// A repro recording is running: the AI Agent page's button says stop.
    pub agent_recording: bool,
    /// The Music page's groups (set by the game when it opens the page).
    pub music_groups: Vec<MusicGroup>,
    /// Keyboard focus: the interactive target currently focused.
    focus: Option<FocusId>,
    /// The focus came from keyboard or pad navigation, so the mouse wheel steps it
    /// from anywhere (`wheel`). A click focuses its widget too, for the highlight,
    /// but vanilla's wheel ignores that: only the hovered widget then reacts.
    focus_from_keyboard: bool,
    /// Whether a focused slider is in value-editing mode (`AbstractSliderButton.canChangeValue`).
    pub(crate) can_change_value: bool,
    /// The current GUI size (w, h).
    pub size: (f32, f32),
    /// The current window framebuffer dimensions [w, h].
    pub framebuffer: [i32; 2],
    /// Splitscreen: another player's options. Language and resource packs are the whole
    /// process's (every player's text and textures), so they're P1's to change.
    pub split_guest: bool,
    /// The list's rows as last laid out: each row's top and bottom (GUI
    /// pixels), for the narration's row under the mouse.
    row_bands: Vec<[f32; 2]>,
    /// The Shader Packs page's chosen pack's profiles ("" first: its defaults).
    /// The PvP & HUD items that take a whole row (`wide_extras`), measured
    /// when the page is drawn.
    wide_extras: Vec<crate::extras::Item>,
    /// Whether Options opens Skin Customization: the game's options do; the menus before the
    /// game, which can't put a skin on, keep vanilla's greyed button.
    pub skin_page: bool,
    /// A profile selected by the host: names are managed in Accounts, not the legacy alias box.
    pub profile_name: Option<String>,
    /// That profile's key (`accounts::player_key`), under which its own skin is kept
    /// (`own_skin::SkinChoice::for_profile`).
    pub profile_key: Option<String>,
    /// Where that profile's skin can go besides this game, set by the game every frame.
    pub skin_cloud: crate::own_skin::Cloud,
    /// Whether the Mods, Resource Packs and Shader Packs pages have their Catalog button: the
    /// game's options on a phone (`offer_catalog`). The menus before the game can't show it.
    catalog_buttons: bool,
    skin: SkinPage,
}
/// The Skin Customization page: the name being typed and the skins to choose from, read from
/// beside options.txt when the page opens.
struct SkinPage {
    name: ui::EditBox,
    /// The name worlds open with now (`offline_name::for_worlds`).
    saved: String,
    /// The signed-in account's name, which servers get.
    account: Option<String>,
    /// The PNGs of `skins/` (`own_skin::list`).
    files: Vec<String>,
    choice: crate::own_skin::SkinChoice,
    /// Ely.by Skins (`elyby::enabled`).
    elyby: bool,
    /// A name refused, or a skin the game could not put on.
    warning: Option<String>,
    /// For the game, oldest first.
    actions: Vec<SkinAction>,
}
impl Default for SkinPage {
    fn default() -> Self {
        let mut name = ui::EditBox::new(0.0, 0.0, 270.0, 20.0);
        name.max_length = 16;
        name.focused = false;
        Self {
            name,
            saved: String::new(),
            account: None,
            files: Vec::new(),
            choice: Default::default(),
            elyby: true,
            warning: None,
            actions: Vec::new(),
        }
    }
}
impl Default for OptionsUi {
    fn default() -> Self {
        let mut search = ui::EditBox::new(0.0, 0.0, 200.0, 15.0);
        search.max_length = 32;
        let mut settings_search = ui::EditBox::new(0.0, 0.0, 200.0, 15.0);
        settings_search.focused = false;
        let mut custom_send = ui::EditBox::new(0.0, 0.0, 310.0, 20.0);
        custom_send.max_length = 256;
        custom_send.focused = false;
        Self {
            stack: vec![Page::Main],
            scroll: 0.0,
            dragging: None,
            shift: false,
            modifiers: 0,
            slider_value: None,
            scrollbar_drag_y: None,
            selected_key: None,
            selected_mod_key: None,
            selected_page_key: None,
            pending_mod_key: None,
            selected_pad_action: None,
            pad_glyphs: crate::pad_glyph::Glyphs::default(),
            pad_driving: false,
            languages: Vec::new(),
            language_choice: None,
            language_search: search,
            settings_search,
            custom_send,
            back_to_editor: false,
            painter: Painter::default(),
            mod_browser: Default::default(),
            mod_project: String::new(),
            mods_note: None,
            mods_screen: None,
            pack_browser: false,
            pack_infos: Vec::new(),
            pack_selection: Vec::new(),
            pack_screen: None,
            data_pack_dir: None,
            data_pack_resources: Default::default(),
            additional_data_pack_infos: Vec::new(),
            data_pack_stamp: (Vec::new(), Instant::now()),
            resource_error: None,
            pack_focus: Instant::now(),
            pack_reselected: false,
            in_world: false,
            minecart_improvements: false,
            center_language: false,
            output: crate::gfx::EffectiveOutput::NONE,
            graphics: Default::default(),
            controller: Default::default(),
            extras: Default::default(),
            radial: Default::default(),
            radial_pick: Default::default(),
            shader: Default::default(),
            pack_options: Default::default(),
            pack_configs: Default::default(),
            split_profiles: Default::default(),
            calibration: None,
            calibration_window: None,
            split_setup: Default::default(),
            studio: Default::default(),
            agent_status: Vec::new(),
            diagnostics_status: Vec::new(),
            perf_lab: Vec::new(),
            perf_lab_notes: Vec::new(),
            agent_recording: false,
            music_groups: Vec::new(),
            focus: None,
            focus_from_keyboard: false,
            can_change_value: false,
            size: (427.0, 240.0),
            framebuffer: [1280, 720],
            split_guest: false,
            row_bands: Vec::new(),
            skin_page: false,
            profile_name: None,
            profile_key: None,
            skin_cloud: Default::default(),
            catalog_buttons: false,
            skin: SkinPage::default(),
            wide_extras: Vec::new(),
        }
    }
}
/// The longest line of a Mods page note, in characters: about a button's width.
const NOTE_CHARS: usize = 52;

/// `text` in lines of at most `max` characters, broken between words.
fn wrap_words(text: &str, max: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > max {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Android: the Mods page's game with native mods (`ModList::native_game`): its lines, cut to the
/// list's width, its buttons two to a row, and a gap before the mods.
fn native_game_rows(s: &Settings) -> Vec<Row> {
    let Some(game) = &s.mods.native_game else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for (text, warning) in &game.lines {
        for line in wrap_words(text, NOTE_CHARS) {
            rows.push(if *warning {
                Row::Warning(line)
            } else {
                Row::Note(line)
            });
        }
    }
    let widget = |(label, act): &(String, crate::mod_list::NativeGameAct), w: f32| {
        button(
            0.0,
            0.0,
            w,
            Text::literal(label.clone()),
            Act::NativeGame(*act),
        )
    };
    for pair in game.buttons.chunks(2) {
        rows.push(match pair {
            [a, b] => Row::Buttons(widget(a, 150.0), Some(widget(b, 150.0))),
            _ => Row::Buttons(widget(&pair[0], 310.0), None),
        });
    }
    rows
}

/// Starling's Mods page (`Page::NativeGame`, which the Mods buttons open: a phone has no launcher
/// to choose the mods): which game runs and its buttons (Android), then every mod's switch. A
/// native mod is a box ticked when it is chosen for the next build (the buttons' Compile builds
/// with those); one chosen and not in this game says so beside it, and one in it with settings
/// has Settings there. Details opens upstream's Mods screen (`Page::Mods`).
fn native_game_page_rows(s: &Settings) -> Vec<Row> {
    let mut rows = native_game_rows(s);
    if s.mods.installed.is_empty() {
        rows.push(Row::Header("horizon.mods.empty"));
        return rows;
    }
    rows.push(Row::Buttons(
        button(
            0.0,
            0.0,
            310.0,
            tr("starling.mods.details"),
            Act::Push(Page::Mods),
        ),
        None,
    ));
    let mods = s.mods.installed.iter().enumerate().map(|(i, installed)| {
        let on = s.mods.is_on(i);
        let not_built = installed.problem.as_deref() == Some(crate::mod_list::NOT_BUILT);
        let compiled_here = not_built && installed.native.is_some();
        let chosen_box = installed.native.is_some() && !installed.id.is_empty();
        let mut label = if chosen_box {
            let tick = if on { '\u{2611}' } else { '\u{2610}' };
            Text::literal(format!("{tick} {}", installed.title))
        } else {
            Text::literal(format!("{}: ", installed.title)).append(on_off(on))
        };
        if not_built {
            if !compiled_here {
                label = label.append(tr("horizon.mods.not_built"));
            }
        } else if installed.problem.is_some() {
            label = label.append(tr("horizon.mods.stopped"));
        } else if installed.dev {
            label = label.append(tr("horizon.mods.dev"));
        }
        let dead = installed.dev || installed.id.is_empty();
        // Beside it: a warning, as Java's, for a native mod chosen and not in this game (nothing
        // to press: Compile is the page's), Settings for one in it that has settings.
        let side = if compiled_here {
            on.then(|| {
                let warning = Text::literal(format!(
                    "\u{a7}e\u{26a0} {}",
                    lang::get("starling.mods.native.not_compiled")
                ));
                inactive(button(0.0, 0.0, 150.0, warning, Act::ModSettings(i)))
            })
        } else {
            (installed.native.as_ref())
                .is_some_and(|page| !page.settings.is_empty())
                .then(|| {
                    let settings = tr("horizon.mods.settings");
                    button(0.0, 0.0, 150.0, settings, Act::ModSettings(i))
                })
        };
        if let Some(side) = side {
            let toggle = button(0.0, 0.0, 150.0, label, Act::ToggleMod(i));
            Row::Buttons(if dead { inactive(toggle) } else { toggle }, Some(side))
        } else {
            let widget = button(0.0, 0.0, 310.0, label, Act::ToggleMod(i));
            Row::Buttons(if dead { inactive(widget) } else { widget }, None)
        }
    });
    rows.extend(mods);
    rows
}

fn button(x: f32, y: f32, w: f32, label: Text, act: Act) -> Widget {
    Widget {
        x,
        y,
        w,
        h: 20.0,
        kind: WKind::Button {
            label,
            active: true,
            act,
            icon: None,
        },
        list: false,
        row: 0,
    }
}
fn inactive(mut w: Widget) -> Widget {
    if let WKind::Button { active, .. } = &mut w.kind {
        *active = false;
    }
    w
}
fn option_widget(s: &Settings, opt: Opt, x: f32, y: f32, w: f32) -> Widget {
    // OptionsSubScreen.addContents: the narrator button is active only while
    // the narrator is (`narratorButton.active = narrator.isActive()`).
    if !opt.available() || opt == Opt::Narrator && !horizon_narrator::is_active() {
        return inactive(button(x, y, w, opt.label(s), Act::Cycle(opt)));
    }
    // VideoSettingsScreen.tick: Max Anisotropy only works with Anisotropic filtering.
    if opt == Opt::MaxAnisotropy && s.file.int("textureFiltering") != 2 {
        return inactive(button(x, y, w, opt.label(s), Act::Cycle(opt)));
    }
    match opt.kind(s) {
        Kind::Slider(v) => Widget {
            x,
            y,
            w,
            h: 20.0,
            kind: WKind::Slider {
                label: opt.label(s),
                value: v,
                opt,
            },
            list: false,
            row: 0,
        },
        Kind::Cycle => button(x, y, w, opt.label(s), Act::Cycle(opt)),
    }
}
/// The least GUI size a shell screen lays its keys out for, before it has been drawn at its own.
const MIN_KEY_GUI: (f32, f32) = (480.0, 240.0);
/// The PvP & HUD page's rows: its items under their headers.
fn pvp_rows() -> Vec<Result<crate::extras::Item, &'static str>> {
    crate::pvp::rows()
        .into_iter()
        .map(|r| r.map(crate::extras::Item::Pvp))
        .collect()
}
/// The items of PvP & HUD's rows whose label, with any of their first values, is wider than a
/// half-width button holds (`width` measures text with the screen's font): those take the
/// whole row, so no label slides past its edges, in any language.
fn wide_extras(s: &Settings, width: impl Fn(&str) -> f32) -> Vec<crate::extras::Item> {
    // A button's label keeps 2 px off each edge (`AbstractButton.renderString`'s margin),
    // past which it slides back and forth.
    let room = 150.0 - 4.0;
    pvp_rows()
        .into_iter()
        .filter_map(Result::ok)
        .filter(|&item| {
            let mut e = s.extras.clone();
            (0..4).any(|_| {
                let label = Text::option_name_value(
                    Text::literal(item.caption_text()),
                    Text::literal(item.value(&e)),
                )
                .plain();
                item.cycle(&mut e, false);
                width(&label) > room
            })
        })
        .collect()
}
/// Rows of an OptionsList: (big?, options) or a header.
enum Row {
    Big(Opt),
    Pair(Opt, Option<Opt>),
    Header(&'static str),
    /// A heading the page makes at run time, by lang key (the Music page's groups).
    Heading(String),
    /// A line of grey text (a path).
    Note(String),
    /// A button, and another after it.
    Buttons(Widget, Option<Widget>),
    /// Skin Customization's name box, after its label.
    Field,
    /// What a custom button sends.
    SendField,
    /// A row of icons to pick.
    Icons(Vec<Widget>),
    /// A custom button's canvas, drawn by the page itself (`painter`).
    Canvas(u32),
    /// A line of red text: what was refused.
    Warning(String),
}
/// A widget's tooltip (`setTooltip`), unwrapped: drawn while hovered, and
/// its narration's hint.
fn tooltip_text(s: &Settings, wd: &Widget) -> Option<Text> {
    let WKind::Button { act, .. } = &wd.kind else {
        let WKind::Slider { opt, .. } = &wd.kind else {
            return None;
        };
        let key = opt.tooltip_key(s);
        let Opt::Page(page, row) = opt else {
            return lang::has(&key).then(|| tr(&key));
        };
        return pages::tooltip(*page, *row);
    };
    let mut tip = None;
    if let Act::Extra(item) = act {
        tip = Some(Text::literal(item.description_text()));
    }
    if let Act::Push(page) = act
        && let Some(text) = pages::horizon_page_tooltip(*page)
    {
        tip = Some(tr(text));
    }
    let key = match act {
        Act::Cycle(opt) => Some(opt.tooltip_key(s)),
        Act::Push(Page::Language) => Some("options.language.tooltip".into()),
        Act::Push(Page::Accessibility) => Some("options.accessibility.tooltip".into()),
        Act::PvpPreset => Some("horizon.pvp.preset.tooltip".into()),
        Act::HudEditor => Some("horizon.pvp.editor.tooltip".into()),
        _ => None,
    };
    if let Some(key) = key.filter(|key| lang::has(key)) {
        tip = Some(tr(&key));
    }
    if let Act::Cycle(Opt::GraphicsBackend) = act {
        tip = Some(graphics_api_tooltip(s));
    }
    match act {
        Act::Cycle(opt) if !opt.available() => Some(tr("horizon.options.unavailable")),
        Act::PageRow(page, row) => pages::tooltip(*page, *row),
        Act::ToggleMod(i) => match s.mods.installed.get(*i) {
            Some(installed) => {
                let description = match installed.problem.as_deref() {
                    Some(crate::mod_list::NOT_BUILT) => lang::get(crate::mod_list::NOT_BUILT_TIP),
                    Some(problem) => problem.to_owned(),
                    None => installed.description.clone(),
                };
                Some(Text::literal(format!("{}\n{description}", installed.title)))
            }
            None => tip,
        },
        Act::ModKey(i, k) => {
            let installed = s.mods.installed.get(*i)?;
            let mapping = installed.native.as_ref()?.mappings.get(*k)?;
            let (key, modifiers) = s.mods.binding(
                &installed.id,
                &mapping.id,
                (&mapping.key, mapping.modifiers),
            );
            let conflicts = key_conflicts(
                s,
                &installed.id,
                &mapping.id,
                &key,
                modifiers,
                mapping.context,
                key == mapping.key && modifiers == mapping.modifiers,
            );
            if conflicts.is_empty() {
                return tip;
            }
            Some(Text::translate(
                "controls.keybinds.duplicateKeybinds",
                &[Text::literal(conflicts.join(", "))],
            ))
        }
        Act::Key(name) => {
            let conflicts = key_conflicts(
                s,
                "",
                name,
                s.keys.get(name),
                0,
                crate::mod_list::KeyConflictContext::Universal,
                s.keys.is_default(name),
            );
            if conflicts.is_empty() {
                return tip;
            }
            let names = conflicts.join(", ");
            Some(Text::translate(
                "controls.keybinds.duplicateKeybinds",
                &[Text::literal(names)],
            ))
        }
        _ => tip,
    }
}
/// The value part of an option's message (`CycleButton.createLabelForValue`):
/// its `options.generic_value` message without the caption.
fn option_value(opt: Opt, s: &Settings) -> String {
    let full = opt.label(s).plain();
    let marker = "\u{1}";
    let template = generic(opt.caption(), Text::literal(marker)).plain();
    match template.split_once(marker) {
        Some((before, after))
            if full.len() >= before.len() + after.len()
                && full.starts_with(before)
                && full.ends_with(after) =>
        {
            full[before.len()..full.len() - after.len()].to_owned()
        }
        _ => full,
    }
}
/// What widget `wd` narrates, over what `ui::button` or `ui::slider`
/// recorded for it (`n`): a cycle button the value a click moves to (worked
/// out only while it may be narrated: under the mouse or focused), a key
/// bind's buttons `narrator.controls.*`, its tooltip as a hint; a list row's
/// widget belongs to the list keyed `list`.
fn narrate_widget(
    n: &mut crate::widgets::Narratable,
    s: &Settings,
    wd: &Widget,
    hover: bool,
    list: u64,
    framebuffer: [i32; 2],
) {
    use crate::widgets::What;
    if let WKind::Button {
        label, active, act, ..
    } = &wd.kind
    {
        let t = |key: &str, args: &[Text]| Text::translate(key, args).plain();
        let bind = |name: Text, bound: bool| {
            if bound {
                let button = Text::translate("gui.narrate.button", std::slice::from_ref(label));
                t("narrator.controls.bound", &[name, button])
            } else {
                t("narrator.controls.unbound", &[name])
            }
        };
        let active = *active;
        match act {
            Act::Cycle(opt) if active => {
                let next = if hover || crate::widgets::narration_focused(n.rect) {
                    let mut next = s.clone();
                    opt.cycle(&mut next, false, framebuffer);
                    option_value(*opt, &next)
                } else {
                    String::new()
                };
                n.what = What::Cycle {
                    message: label.plain(),
                    next,
                    active,
                };
            }
            Act::ModKey(i, k) | Act::ResetModKey(i, k) => {
                if let Some(installed) = s.mods.installed.get(*i)
                    && let Some(mapping) = installed
                        .native
                        .as_ref()
                        .and_then(|page| page.mappings.get(*k))
                {
                    let title = if matches!(act, Act::ResetModKey(..)) {
                        t("narrator.controls.reset", &[tr(&mapping.label)])
                    } else {
                        let (key, _) = s.mods.binding(
                            &installed.id,
                            &mapping.id,
                            (&mapping.key, mapping.modifiers),
                        );
                        bind(tr(&mapping.label), key != "key.keyboard.unknown")
                    };
                    n.what = What::Named { title, active };
                }
            }
            Act::Key(name) => {
                let bound = s.keys.get(name) != "key.keyboard.unknown";
                n.what = What::Named {
                    title: bind(tr(name), bound),
                    active,
                };
            }
            Act::ResetKey(name) => {
                n.what = What::Named {
                    title: t("narrator.controls.reset", &[tr(name)]),
                    active,
                };
            }
            _ => {}
        }
    }
    n.hint = tooltip_text(s, wd).map(|tip| tip.plain());
    if wd.list {
        // Stable while the list scrolls: its row and column.
        n.key = crate::widgets::key_of(&(list, wd.row, wd.x.round() as i32));
        n.parent = Some((list, wd.row));
    }
}
/// A list's rows as they are laid out: the row of each widget pushed and
/// each row's band, for the narration (`ContainerObjectSelectionList`'s
/// entries and the one under the mouse).
#[derive(Default)]
struct ListRows {
    start: usize,
    top: f32,
    bands: Vec<[f32; 2]>,
}
impl ListRows {
    /// A row starts at `y` with the next widget pushed.
    fn begin(&mut self, out: &[Widget], y: f32) {
        self.start = out.len();
        self.top = y;
    }
    /// The row ends at `y`: the widgets pushed since `begin` are in it.
    fn end(&mut self, out: &mut [Widget], y: f32) {
        let row = self.bands.len();
        out[self.start..].iter_mut().for_each(|w| w.row = row);
        self.bands.push([self.top, y]);
    }
}
fn pairs(opts: &[Opt]) -> Vec<Row> {
    opts.chunks(2)
        .map(|c| Row::Pair(c[0], c.get(1).copied()))
        .collect()
}
/// The focused text field as the on-screen keyboard's editor on Android mirrors it
/// (`OptionsUi::focused_text`).
pub struct FocusedText {
    /// Which field: 0 the language search, 1 the settings search, 2 the pack search, 3 Skin
    /// Customization's name, 4 a custom button's text.
    pub index: usize,
    /// It searches a list.
    pub search: bool,
    pub value: String,
    /// The cursor and the selection's other end, in characters.
    pub selection: (usize, usize),
    /// The most characters it takes.
    pub max: usize,
}
impl OptionsUi {
    /// No widget keeps the highlight a click gave it; a focus the keyboard
    /// moved stays.
    pub fn clear_click_focus(&mut self) {
        if !self.focus_from_keyboard {
            self.focus = None;
        }
        if let Some(shell) = self.shell_mut() {
            shell.blur_search();
        }
    }
    /// A switch of `horizon-extras.txt` as the Extras page draws it: "Caption: On".
    fn extra_button(&self, s: &Settings, item: crate::extras::Item) -> Widget {
        let label = Text::option_name_value(
            Text::literal(item.caption_text()),
            Text::literal(item.value(&s.extras)),
        );
        button(0.0, 0.0, 150.0, label, Act::Extra(item))
    }
    fn settings_search_active(&self) -> bool {
        self.settings_search.focused
            && matches!(
                self.page(),
                Page::Video | Page::Controls | Page::KeyBinds | Page::ModBrowser
            )
    }
    /// Whether a text field shown now has the focus: the language and settings searches, the
    /// pack search, or Skin Customization's name.
    pub fn text_focused(&self) -> bool {
        match self.page() {
            Page::Language => self.language_search.focused,
            Page::Packs => self
                .pack_screen
                .as_ref()
                .is_some_and(|p| p.search_focused()),
            Page::Skin => self.profile_name.is_none() && self.skin.name.focused,
            Page::CustomButton(_) => self.custom_send.focused,
            _ => self.settings_search_active(),
        }
    }
    /// Whether `(x, y)` (GUI pixels) is on the focused text field shown now.
    pub fn text_field_at(&self, x: f32, y: f32) -> bool {
        let on = |e: &ui::EditBox| {
            e.focused && x >= e.x && x < e.x + e.width && y >= e.y && y < e.y + e.height
        };
        match self.page() {
            Page::Language => on(&self.language_search),
            Page::Packs => self.pack_screen.as_ref().is_some_and(|p| p.search_at(x, y)),
            Page::Skin => self.profile_name.is_none() && on(&self.skin.name),
            Page::CustomButton(_) => on(&self.custom_send),
            _ => self.settings_search_active() && on(&self.settings_search),
        }
    }
    /// The text field that has the focus (`text_focused`), for the on-screen keyboard's editor
    /// on Android.
    pub fn focused_text(&self) -> Option<FocusedText> {
        if !self.text_focused() {
            return None;
        }
        let edit = |e: &ui::EditBox| (e.value.clone(), (e.cursor, e.highlight), e.max_length);
        let (index, (value, selection, max)) = match self.page() {
            Page::Language => (0, edit(&self.language_search)),
            Page::Packs => {
                let s = self.pack_screen.as_ref()?.search_field();
                (2, (s.value.clone(), (s.cursor, s.highlight), s.max_length))
            }
            Page::Skin => (3, edit(&self.skin.name)),
            Page::CustomButton(_) => (4, edit(&self.custom_send)),
            _ => (1, edit(&self.settings_search)),
        };
        Some(FocusedText {
            index,
            search: index <= 2,
            value,
            selection,
            max,
        })
    }
    /// The on-screen keyboard's editor's text and selection on the focused field (Android),
    /// through the field's filter and limit as typing would leave them: a position after a
    /// character the filter drops moves back with it. A search's list goes back to its top
    /// when its text changed, as after a typed key.
    pub fn set_focused_text(&mut self, atlas: &HudAtlas, text: &str, cursor: usize, anchor: usize) {
        if !self.text_focused() {
            return;
        }
        let kept = |pos: usize| {
            let before: String = text.chars().take(pos).collect();
            crate::clipboard::filter_text(&before, false)
                .chars()
                .count()
        };
        let (cursor, anchor) = (kept(cursor), kept(anchor));
        let value = crate::clipboard::filter_text(text, false);
        let page = self.page();
        let edit = match page {
            Page::Packs => {
                if let Some(p) = &mut self.pack_screen {
                    p.set_search(
                        &crate::screen_draw::Measure(&atlas.font),
                        &value,
                        cursor,
                        anchor,
                    );
                }
                return;
            }
            Page::Language => &mut self.language_search,
            Page::Skin => &mut self.skin.name,
            Page::CustomButton(_) => &mut self.custom_send,
            _ => &mut self.settings_search,
        };
        let changed = value != edit.value;
        if changed {
            edit.set_value(&value);
        }
        edit.set_cursor(anchor, false);
        edit.set_cursor(cursor, true);
        if changed && !matches!(page, Page::Skin | Page::CustomButton(_)) {
            self.scroll = 0.0;
        }
    }
    /// Read the Skin Customization page's name, skins and choice again (it opened, or a skin
    /// came through the import inbox).
    pub fn refresh_skin_page(&mut self) {
        let dir = std::env::current_dir().unwrap_or_default();
        let page = &mut self.skin;
        page.saved = self
            .profile_name
            .clone()
            .unwrap_or_else(|| crate::offline_name::for_worlds(&dir));
        if self.profile_name.is_some() {
            page.name.focused = false;
        }
        page.account = crate::auth::current().map(|a| a.name.clone());
        page.files = crate::own_skin::list(&dir.join(crate::own_skin::DIR));
        page.choice = crate::own_skin::SkinChoice::for_profile(&dir, self.profile_key.as_deref());
        page.elyby = crate::elyby::enabled(&dir);
        page.warning = None;
        if !page.name.focused {
            page.name.set_value(&page.saved);
        }
    }
    /// A line in red on the Skin Customization page (a skin the game could not put on).
    pub fn skin_warning(&mut self, text: String) {
        self.skin.warning = Some(text);
    }
    /// What Skin Customization asked of the game since the last call.
    pub fn take_skin_actions(&mut self) -> Vec<SkinAction> {
        std::mem::take(&mut self.skin.actions)
    }
    /// What the skin button calls no file of `skins/`: an account's own skin, else the default.
    fn skin_default_name(&self) -> String {
        lang::get(match self.skin_cloud {
            crate::own_skin::Cloud::Local => "starling.skin.default",
            _ => "starling.skin.account",
        })
    }
    /// Skin Customization's rows for the playing profile's account, as Horizon Android's wardrobe
    /// has them: a Microsoft account's upload of the chosen skin to its profile, and how the last
    /// one went; an Ely.by account's page on ely.by, where its skin is changed.
    fn skin_cloud_rows(&self) -> Vec<Row> {
        use crate::own_skin::{Cloud, Upload};
        let chosen = self.skin.choice.file.is_some();
        match &self.skin_cloud {
            Cloud::Local => Vec::new(),
            Cloud::Ely => vec![
                Row::Buttons(
                    button(
                        0.0,
                        0.0,
                        310.0,
                        tr("starling.skin.ely_site"),
                        Act::ElySkinSite,
                    ),
                    None,
                ),
                Row::Note(lang::get("starling.skin.ely_hint")),
            ],
            Cloud::Microsoft(upload) => {
                let sending = *upload == Upload::Sending;
                let label = tr(if sending {
                    "starling.skin.uploading"
                } else {
                    "starling.skin.upload"
                });
                let upload_button = button(0.0, 0.0, 310.0, label, Act::UploadSkin);
                let upload_button = if chosen && !sending {
                    upload_button
                } else {
                    inactive(upload_button)
                };
                let note = match upload {
                    Upload::Failed(why) => Row::Warning(lang::get_or(why, why)),
                    Upload::Sent => Row::Note(lang::get("starling.skin.uploaded")),
                    _ if !chosen => Row::Note(lang::get("starling.skin.upload_choose")),
                    _ => Row::Note(lang::get("starling.skin.upload_hint")),
                };
                vec![Row::Buttons(upload_button, None), note]
            }
        }
    }
    /// The name box loses the focus: a changed name is kept when it is one, else refused and
    /// the old one back in the box.
    fn commit_name(&mut self) {
        if self.profile_name.is_some() {
            self.skin.name.focused = false;
            return;
        }
        let page = &mut self.skin;
        page.name.focused = false;
        let typed = page.name.value.clone();
        if typed == page.saved {
            return;
        }
        if crate::offline_name::is_valid_choice(&typed) {
            page.saved = typed.clone();
            page.warning = None;
            page.actions.push(SkinAction::Name(typed));
            return;
        }
        page.warning = Some(format!(
            "Not a name: 3 to 16 letters, digits or _ ({} kept)",
            page.saved
        ));
        page.name.set_value(&page.saved);
    }
    /// Whether the resource pack screen is open (not Create World's data packs on the same
    /// page).
    pub fn resource_packs_open(&self) -> bool {
        self.page() == Page::Packs && self.data_pack_dir.is_none()
    }
    /// The game is up, so on a phone the pages can open its catalog.
    pub fn offer_catalog(&mut self) {
        self.catalog_buttons = ON_A_PHONE;
    }
    /// The packs in `resourcepacks/` changed while the pack screen is open
    /// (`PackSelectionScreen`'s directory watcher): the screen is made again over
    /// `infos`, keeping the selection made on it so far.
    pub fn update_pack_infos(&mut self, infos: Vec<crate::pack_screen::PackInfo>) {
        // Create World's data packs are what this page lists then, and they have their watcher.
        if self.data_pack_dir.is_some() {
            return;
        }
        if let Some(p) = self.pack_screen.take()
            && p.changed
        {
            self.pack_selection = p.commit();
            self.pack_reselected = true;
        }
        self.pack_infos = infos;
    }
    pub fn page(&self) -> Page {
        *self.stack.last().unwrap_or(&Page::Main)
    }
    /// Whether the screen's focus is a search box that takes the keys (an
    /// `EditBox` that `canConsumeInput`): the language list's, the settings
    /// search, or the pack screen's.
    pub fn typing(&self) -> bool {
        self.resource_error.is_none()
            && match self.page() {
                Page::Language => true,
                Page::Packs => self
                    .pack_screen
                    .as_ref()
                    .is_some_and(|p| p.search_focused()),
                Page::Mods => self
                    .mods_screen
                    .as_ref()
                    .is_some_and(|p| p.search_focused()),
                Page::ControllerRadialName => self.radial_name_typing(),
                _ => match self.shell() {
                    Some(shell) => shell.typing(),
                    None => self.settings_search_active(),
                },
            }
    }
    /// Jump straight to a page (control socket / tests).
    pub fn show(&mut self, page: Page) {
        if page != Page::Main {
            self.push(page);
        }
    }
    pub fn open(&mut self, in_world: bool) {
        self.open_at(in_world, Page::Main);
    }
    /// Create World's Data Packs (`openDataPackSelectionScreen`): the Packs
    /// page on the new world's pack folder `dir`, `selected` chosen;
    /// `resources` holds the built-in pack's icon.
    pub fn open_data_packs(
        &mut self,
        dir: std::path::PathBuf,
        selected: Vec<String>,
        resources: std::path::PathBuf,
        additional: Vec<crate::pack_screen::PackInfo>,
    ) {
        self.open_at(false, Page::Packs);
        self.pack_infos = crate::datapack::screen_packs(&dir, &resources);
        self.pack_infos.extend(additional.iter().cloned());
        self.additional_data_pack_infos = additional;
        self.data_pack_resources = resources;
        self.pack_selection = selected;
        self.data_pack_stamp = (crate::datapack::folder_stamp(&dir), Instant::now());
        self.data_pack_dir = Some(dir);
    }
    /// `PackSelectionScreen.tick`'s watcher: once a second, the Data Packs
    /// folder's packs are listed again if it changed, the selection kept
    /// (the search text and scroll start over).
    fn watch_data_packs(&mut self, atlas: &HudAtlas, size: (f32, f32)) {
        let Some(dir) = &self.data_pack_dir else {
            return;
        };
        if self.data_pack_stamp.1.elapsed().as_secs() < 1 {
            return;
        }
        let stamp = crate::datapack::folder_stamp(dir);
        let changed = stamp != self.data_pack_stamp.0;
        self.data_pack_stamp = (stamp, Instant::now());
        if !changed {
            return;
        }
        self.pack_infos = crate::datapack::screen_packs(dir, &self.data_pack_resources);
        self.pack_infos
            .extend(self.additional_data_pack_infos.iter().cloned());
        if let Some(p) = self.pack_screen.take() {
            self.pack_selection = p.commit();
            self.packs(atlas, size).changed = p.changed;
        }
    }
    /// Steps a slider option forward or backward, derived from its own slider definition.
    pub fn step_slider_option(
        &mut self,
        s: &mut Settings,
        opt: Opt,
        forward: bool,
        width: f32,
    ) -> Vec<Event> {
        let volume = |s: &Settings| match opt {
            Opt::Volume(0) => Some(s.master_volume),
            Opt::Volume(i) => Some(s.volumes[i]),
            _ => None,
        };
        let before_vol = volume(s);
        opt.step_slider(s, forward, width);
        self.slider_value = None;
        let mut events = vec![Event::Changed, Event::SliderDrag];
        if let Opt::ModSetting(i, j) = opt {
            events.push(Event::NativeSetting(usize::from(i), usize::from(j)));
        }
        if let (Opt::Volume(source), Some(after)) = (opt, volume(s))
            && before_vol != Some(after)
        {
            events.push(Event::Volume(source, after));
        }
        events
    }
    /// Open a page whose Done/Escape returns directly to the calling screen.
    pub fn open_at(&mut self, in_world: bool, page: Page) {
        // Starling: the Mods buttons open its Mods page (the switches and Compile: a phone has no
        // launcher to choose the mods); upstream's Mods screen is that page's Details.
        let page = if page == Page::Mods {
            Page::NativeGame
        } else {
            page
        };
        self.stack.clear();
        self.data_pack_dir = None;
        self.focus = None;
        self.focus_from_keyboard = false;
        self.can_change_value = false;
        self.push(page);
        self.in_world = in_world;
        self.selected_key = None;
        self.pending_mod_key = None;
    }
    fn push(&mut self, page: Page) {
        self.stack.push(page);
        self.scroll = 0.0;
        self.scrollbar_drag_y = None;
        self.focus = None;
        self.focus_from_keyboard = false;
        self.can_change_value = false;
        if matches!(
            page,
            Page::Video | Page::Controls | Page::KeyBinds | Page::ModBrowser
        ) {
            self.settings_search.set_value("");
            self.settings_search.focused = false;
        }
        if page == Page::Language {
            self.language_choice = None;
            self.language_search.set_value("");
            self.center_language = true;
        }
        if page == Page::Packs {
            self.pack_screen = None;
        }
        if page == Page::Mods {
            self.mods_screen = None;
        }
        if let Some(shell) = self.shell_mut() {
            shell.open();
        }
        if let Some(at) = Self::shader_page(page) {
            self.shader.show(at);
        }
        if let Some(at) = Self::graphics_page(page) {
            self.graphics.show(at);
        }
        if let Some(at) = Self::controller_page(page) {
            self.controller.show(at);
        }
        if page == Page::SplitSetup {
            self.split_setup.reset();
        }
        if page == Page::SplitProfiles {
            self.split_profiles.reset();
        }
        if page == Page::Skin {
            self.skin.name.focused = false;
            self.refresh_skin_page();
        }
        if page == Page::Studio {
            self.studio.refresh();
        }
    }
    /// A controller's or the keyboard's focus landed on `rect`: a row of a shell screen's list
    /// that is partly hidden scrolls all into view. How far it moved down, when it moved.
    pub fn reveal(&mut self, s: &Settings, size: (f32, f32), rect: [f32; 4]) -> Option<f32> {
        self.shell_mut()?.reveal(s, size, rect)
    }
    /// Whether a video settings screen is open, Horizon's or vanilla's: the fullscreen mode
    /// waits for it to close, and Ctrl with the wheel steps the GUI scale.
    pub fn video_open(&self) -> bool {
        self.page() == Page::Video || Self::graphics_page(self.page()).is_some()
    }
    /// Where the graphics screen opens for `page`, if it is one of its pages.
    fn graphics_page(page: Page) -> Option<crate::graphics_ui::Page> {
        match page {
            Page::Graphics => Some(crate::graphics_ui::Page::General),
            Page::GraphicsAdvanced => Some(crate::graphics_ui::Page::Advanced),
            Page::GraphicsHdr => Some(crate::graphics_ui::Page::Hdr),
            _ => None,
        }
    }
    fn title(page: Page) -> &'static str {
        match page {
            Page::Diagnostics => "starling.diagnostics.title",
            Page::Main => "options.title",
            Page::Video => "options.videoTitle",
            Page::Sound => "options.sounds.title",
            Page::Skin => "options.skinCustomisation.title",
            Page::Online => "options.online.title",
            Page::Controls => "controls.title",
            Page::Touch => "Touch Controls",
            Page::KeyBinds => "controls.keybinds.title",
            Page::Mouse => "options.mouse_settings.title",
            Page::Chat => "options.chat.title",
            Page::Accessibility => "options.accessibility.title",
            Page::Language => "options.language.title",
            Page::Font => "options.font.title",
            Page::Packs => "resourcePack.title",
            Page::Mods => "horizon.pages.mods",
            Page::ModBrowser => "horizon.mods.browse.title",
            Page::ModProject => "horizon.mods.browse.project",
            Page::NativeMod(_) => "Mod Settings",
            Page::NativeGame => "horizon.pages.mods",
            Page::Added(i) => pages::title(i),
            Page::ShaderPacks => "horizon.pages.shader_packs",
            Page::ShaderOptions => "horizon.pages.shader_options",
            Page::CustomButtons
            | Page::CustomButton(_)
            | Page::CustomPick(..)
            | Page::CustomDraw(_) => "Custom Buttons",
            Page::PackOptions => "horizon.pack.options.title",
            Page::Graphics | Page::GraphicsAdvanced | Page::GraphicsHdr | Page::HdrCalibration => {
                "options.videoTitle"
            }
            Page::Controller | Page::PadBinds => "horizon.pages.controller",
            Page::ControllerRadial => "horizon.pages.controller_radial",
            Page::ControllerRadialName => "horizon.radial.name_title",
            Page::ControllerRadialPick => "horizon.radial.pick_title",
            Page::Extras => "horizon.pages.extras",
            Page::Pvp => "horizon.pages.pvp",
            Page::Credits => "credits_and_attribution.screen.title",
            Page::Horizon => "horizon.pages.settings",
            Page::Studio => "horizon.pages.studio",
            Page::Agent => "horizon.pages.agent",
            Page::SplitProfiles => "horizon.pages.split_profiles",
            Page::SplitSetup => "horizon.pages.split_setup",
            Page::Music => "horizon.pages.music",
        }
    }
    fn scrollbar_x(&self, width: f32) -> f32 {
        (width / 2.0).floor()
            + match self.page() {
                Page::KeyBinds => 178.0,
                Page::Language => 118.0,
                _ => 163.0,
            }
    }
    fn filter_option_rows(&self, rows: Vec<Row>) -> Vec<Row> {
        let query = self.settings_search.value.to_lowercase();
        if query.is_empty() {
            return rows;
        }
        let mut filtered = Vec::new();
        let mut pending_header = None;
        for row in rows {
            let matches = |opt: Opt| lang::get(opt.caption()).to_lowercase().contains(&query);
            let visible = match row {
                Row::Header(key) => {
                    pending_header = Some(key);
                    continue;
                }
                Row::Big(opt) if matches(opt) => vec![Row::Big(opt)],
                Row::Pair(a, b) => pairs(
                    &[Some(a), b]
                        .into_iter()
                        .flatten()
                        .filter(|opt| matches(*opt))
                        .collect::<Vec<_>>(),
                ),
                // Keep navigation available regardless of the query.
                Row::Buttons(a, b) => {
                    filtered.push(Row::Buttons(a, b));
                    continue;
                }
                _ => Vec::new(),
            };
            if !visible.is_empty() {
                // Emit a section heading only when its first matching option is found.
                if let Some(key) = pending_header.take() {
                    filtered.push(Row::Header(key));
                }
                filtered.extend(visible);
            }
        }
        filtered
    }

    /// Browse mods: what the registry offers this game, by the search box.
    fn mod_browser_rows(&self) -> Vec<Row> {
        use crate::mod_browser::Status;
        let b = &self.mod_browser;
        match &b.status {
            Status::Idle | Status::Loading => {
                return vec![Row::Note(lang::get("horizon.mods.browse.loading"))];
            }
            Status::Failed(why) => {
                return vec![Row::Note(lang::tr("horizon.mods.browse.failed", &[why]))];
            }
            Status::Ready => {}
        }
        let found = b.found(&self.settings_search.value);
        if found.is_empty() {
            return vec![Row::Note(lang::get("horizon.mods.browse.none"))];
        }
        let mut rows = Vec::new();
        for i in found {
            let l = &b.listings[i];
            let mut label = Text::literal(format!("{} {}", l.name, l.version));
            if l.has_update() {
                label = label.append(tr("horizon.mods.browse.has_update"));
            } else if l.installed.is_some() {
                label = label.append(tr("horizon.mods.browse.is_installed"));
            }
            rows.push(Row::Buttons(
                button(0.0, 0.0, 310.0, label, Act::ModProject(i)),
                None,
            ));
            rows.extend(wrap(&l.summary, 64).into_iter().take(1).map(Row::Note));
        }
        rows
    }
    /// A registry project's page: what it is, Install or Update, its description and
    /// changelog. An installed mod is on already: the install turned it on.
    fn mod_project_rows(&self) -> Vec<Row> {
        let b = &self.mod_browser;
        let Some(l) = b.listing(&self.mod_project) else {
            return vec![Row::Note(lang::get("horizon.mods.browse.none"))];
        };
        let mut rows = vec![Row::Note(format!("{} {}", l.name, l.version))];
        let by = [
            lang::tr("horizon.mods.browse.by", &[&l.authors.join(", ")]),
            l.license.clone(),
            l.published.clone(),
        ];
        let by: Vec<&str> = by
            .iter()
            .map(String::as_str)
            .filter(|t| !t.is_empty())
            .collect();
        rows.push(Row::Note(by.join(" · ")));
        let labels: Vec<&str> = l
            .categories
            .iter()
            .chain(&l.tags)
            .map(String::as_str)
            .collect();
        if !labels.is_empty() {
            rows.push(Row::Note(labels.join(", ")));
        }
        let installing = b.installing.as_deref() == Some(l.id.as_str());
        let install = match (&l.installed, installing) {
            (_, true) => inactive(button(
                0.0,
                0.0,
                150.0,
                tr("horizon.mods.browse.installing"),
                Act::None,
            )),
            (None, false) => button(
                0.0,
                0.0,
                150.0,
                tr("horizon.mods.browse.install"),
                Act::InstallMod,
            ),
            (Some(_), false) if l.has_update() => button(
                0.0,
                0.0,
                150.0,
                tr("horizon.mods.browse.update"),
                Act::InstallMod,
            ),
            (Some(_), false) => inactive(button(
                0.0,
                0.0,
                150.0,
                tr("horizon.mods.browse.installed"),
                Act::None,
            )),
        };
        let page = button(
            0.0,
            0.0,
            150.0,
            tr("horizon.mods.browse.open_page"),
            Act::OpenModPage,
        );
        rows.push(Row::Buttons(install, Some(page)));
        if let Some((message, _)) = &b.message {
            rows.extend(wrap(message, 64).into_iter().map(Row::Note));
        }
        let description = crate::mod_browser::plain_lines(if l.description.is_empty() {
            &l.summary
        } else {
            &l.description
        });
        rows.extend(
            description
                .iter()
                .flat_map(|line| wrap(line, 64))
                .map(Row::Note),
        );
        let changelog = crate::mod_browser::plain_lines(&l.changelog);
        if !changelog.is_empty() {
            rows.push(Row::Heading("horizon.mods.browse.changelog".into()));
            rows.extend(
                changelog
                    .iter()
                    .flat_map(|line| wrap(line, 64))
                    .map(Row::Note),
            );
        }
        rows
    }
    /// A native mod's page: its name, its keys as Key Binds rows (the
    /// player's choice, else the default), then its buttons.
    fn native_rows(&self, s: &Settings, i: usize) -> Vec<Row> {
        let Some(m) = s.mods.installed.get(i) else {
            return Vec::new();
        };
        let mut rows = vec![Row::Note(m.title.clone())];
        let Some(page) = &m.native else {
            return rows;
        };
        for (j, setting) in page.settings.iter().enumerate() {
            let widget = match (&setting.value, u16::try_from(i), u16::try_from(j)) {
                (crate::mod_list::NativeSettingValue::Range { .. }, Ok(i), Ok(j)) => Widget {
                    x: 0.0,
                    y: 0.0,
                    w: 310.0,
                    h: 20.0,
                    kind: WKind::Slider {
                        label: mod_setting_label(setting),
                        value: setting.value.fraction(),
                        opt: Opt::ModSetting(i, j),
                    },
                    list: false,
                    row: 0,
                },
                _ => button(
                    0.0,
                    0.0,
                    310.0,
                    mod_setting_label(setting),
                    Act::NativeSetting(i, j),
                ),
            };
            rows.push(Row::Buttons(widget, None));
        }
        rows
    }

    fn list_rows(&self, s: &Settings) -> Vec<Row> {
        match self.page() {
            Page::Diagnostics => {
                let mut rows = vec![
                    Row::Note(lang::get("starling.diagnostics.notice")),
                    Row::Note(lang::get("starling.diagnostics.overhead")),
                    Row::Buttons(
                        button(
                            0.0,
                            0.0,
                            310.0,
                            tr("starling.diagnostics.export"),
                            Act::DiagnosticsExport,
                        ),
                        None,
                    ),
                    Row::Buttons(
                        button(
                            0.0,
                            0.0,
                            310.0,
                            tr("starling.diagnostics.record"),
                            Act::DiagnosticsRecord,
                        ),
                        None,
                    ),
                    Row::Buttons(
                        button(
                            0.0,
                            0.0,
                            310.0,
                            tr("starling.diagnostics.gpu"),
                            Act::DiagnosticsGpu,
                        ),
                        None,
                    ),
                    Row::Buttons(
                        button(
                            0.0,
                            0.0,
                            310.0,
                            tr("starling.diagnostics.open"),
                            Act::DiagnosticsOpen,
                        ),
                        None,
                    ),
                ];
                rows.extend(self.diagnostics_status.iter().cloned().map(Row::Note));
                if !self.perf_lab.is_empty() {
                    rows.push(Row::Note(lang::get("starling.perf.title")));
                    rows.push(Row::Note(lang::get("starling.perf.about")));
                    for (i, label) in self.perf_lab.iter().enumerate() {
                        rows.push(Row::Buttons(
                            button(
                                0.0,
                                0.0,
                                310.0,
                                Text::literal(label.clone()),
                                Act::PerfLab(i as u8),
                            ),
                            None,
                        ));
                    }
                    rows.extend(self.perf_lab_notes.iter().cloned().map(Row::Note));
                }
                rows
            }
            Page::SplitSetup => Vec::new(),
            Page::Pvp => {
                // Each section's switches two to a row, as vanilla's
                // OptionsList.addSmall lays out an options screen; one whose label would run
                // past half a row in this language (`wide_extras`) gets the whole row.
                let extra = |item: crate::extras::Item, width: f32| {
                    let value = Text::option_name_value(
                        Text::literal(item.caption_text()),
                        Text::literal(item.value(&s.extras)),
                    )
                    .plain();
                    let label = if item.key_name().is_some() && self.selected_key == item.key_name()
                    {
                        Text::literal(format!("> {value} <")).with_color(text::YELLOW)
                    } else {
                        Text::literal(value)
                    };
                    let widget = button(0.0, 0.0, width, label, Act::Extra(item));
                    // A setting of a switch that is off does nothing: inactive,
                    // as vanilla greys out an option that can't apply.
                    if item
                        .parent()
                        .is_some_and(|parent| parent.switch(&s.extras) == Some(false))
                    {
                        inactive(widget)
                    } else {
                        widget
                    }
                };
                // PvP & HUD opens with its preset (Horizon Settings has the way there).
                let preset = button(0.0, 0.0, 150.0, tr("horizon.pvp.preset"), Act::PvpPreset);
                // The editor moves what the game shows: only from a world.
                let edit = button(0.0, 0.0, 150.0, tr("horizon.pvp.editor"), Act::HudEditor);
                let edit = if self.in_world { edit } else { inactive(edit) };
                let mut rows = vec![Row::Buttons(preset, Some(edit))];
                let mut section = Vec::new();
                let flush = |rows: &mut Vec<Row>, section: &mut Vec<crate::extras::Item>| {
                    let mut half = None;
                    for item in section.drain(..) {
                        if self.wide_extras.contains(&item) {
                            rows.extend(half.take().map(|h| Row::Buttons(extra(h, 150.0), None)));
                            rows.push(Row::Buttons(extra(item, 310.0), None));
                        } else if let Some(h) = half.take() {
                            rows.push(Row::Buttons(extra(h, 150.0), Some(extra(item, 150.0))));
                        } else {
                            half = Some(item);
                        }
                    }
                    rows.extend(half.map(|h| Row::Buttons(extra(h, 150.0), None)));
                };
                for row in pvp_rows() {
                    match row {
                        Err(header) => {
                            flush(&mut rows, &mut section);
                            rows.push(Row::Header(header));
                        }
                        Ok(item) => section.push(item),
                    }
                }
                flush(&mut rows, &mut section);
                rows
            }
            Page::Horizon => self.horizon_rows(s),
            Page::Agent => {
                let mut rows = vec![Row::Buttons(
                    self.extra_button(s, crate::extras::Item::AgentControl),
                    Some(button(
                        0.0,
                        0.0,
                        150.0,
                        tr("horizon.agent.copy_details"),
                        Act::CopyAgentDetails,
                    )),
                )];
                rows.push(Row::Buttons(
                    button(
                        0.0,
                        0.0,
                        150.0,
                        tr(if self.agent_recording {
                            "horizon.agent.record_stop"
                        } else {
                            "horizon.agent.record_session"
                        }),
                        Act::ToggleRecording,
                    ),
                    Some(button(
                        0.0,
                        0.0,
                        150.0,
                        tr("horizon.agent.export_repro"),
                        Act::ExportRepro,
                    )),
                ));
                rows.push(Row::Buttons(
                    self.extra_button(s, crate::extras::Item::ReproKeys),
                    None,
                ));
                rows.extend(self.agent_status.iter().cloned().map(Row::Note));
                rows
            }
            Page::Skin => {
                // SkinCustomizationScreen: the seven layers, then Main Hand; in the game's own
                // options, the player's name and skin before them.
                let mut opts: Vec<Opt> = (0..MODEL_PART_KEYS.len()).map(Opt::ModelPart).collect();
                // Horizon's own: the outer layer as real pixels, not one box.
                opts.push(Opt::SkinLayers3d);
                opts.push(Opt::MainHand);
                let layers = pairs(&opts);
                if !self.skin_page {
                    return layers;
                }
                let page = &self.skin;
                let mut rows = if let Some(name) = &self.profile_name {
                    vec![
                        Row::Note(
                            tr("starling.account.active")
                                .plain()
                                .replacen("%s", name, 1),
                        ),
                        Row::Note(tr("starling.account.name_in_accounts").plain()),
                    ]
                } else {
                    vec![Row::Field]
                };
                if self.profile_name.is_none() || page.warning.is_some() {
                    rows.push(match (&page.warning, &page.account) {
                        (Some(warning), _) => Row::Warning(warning.clone()),
                        // Online-mode servers take only the account's name, and the game can't tell
                        // them from the others before it joins.
                        (None, Some(account)) => {
                            Row::Note(format!("Worlds use this name; servers use {account}"))
                        }
                        (None, None) => Row::Note(
                            "Your name in worlds and on servers, from the next one".into(),
                        ),
                    });
                }
                let default = self.skin_default_name();
                let skin = page
                    .choice
                    .file
                    .as_deref()
                    .map_or(default.as_str(), crate::own_skin::shown_name);
                let skin = button(
                    0.0,
                    0.0,
                    150.0,
                    Text::literal(format!("Skin: {}", clipped(skin, 18))),
                    Act::NextSkin,
                );
                let model = if page.choice.slim { "Slim" } else { "Classic" };
                let model = button(
                    0.0,
                    0.0,
                    150.0,
                    Text::literal(format!("Model: {model}")),
                    Act::SkinModel,
                );
                // The default skin's model goes with it (Steve's or Alex's, by the UUID).
                let model = if page.choice.file.is_some() {
                    model
                } else {
                    inactive(model)
                };
                rows.push(Row::Buttons(skin, Some(model)));
                rows.extend(self.skin_cloud_rows());
                let elyby = |width: f32| {
                    let on = if page.elyby { "ON" } else { "OFF" };
                    button(
                        0.0,
                        0.0,
                        width,
                        Text::literal(format!("Ely.by Skins: {on}")),
                        Act::ElyBySkins,
                    )
                };
                // A phone's own file picker: the file chosen comes through the import inbox.
                if cfg!(target_os = "android") {
                    rows.push(Row::Buttons(
                        button(
                            0.0,
                            0.0,
                            150.0,
                            Text::literal("Choose a File..."),
                            Act::PickSkin,
                        ),
                        Some(elyby(150.0)),
                    ));
                } else {
                    rows.push(Row::Buttons(elyby(310.0), None));
                }
                if page.files.is_empty() {
                    rows.push(Row::Note(
                        "No skins yet: put 64x64 PNGs in the skins folder".into(),
                    ));
                }
                // Vanilla's protocol: other players get the skin of the profile from the server.
                rows.push(Row::Note(
                    "Only you see this skin. On a server, others see".into(),
                ));
                rows.push(Row::Note(
                    "the skin Mojang has for your account, or the default".into(),
                ));
                // Offline play's skins: by the player's name, where nothing else gives one.
                if page.elyby {
                    rows.push(Row::Note(
                        "Ely.by Skins: a player without a skin shows the one".into(),
                    ));
                    rows.push(Row::Note(
                        "their name has on ely.by (yours too, on Default)".into(),
                    ));
                }
                rows.extend(layers);
                rows
            }
            Page::Video => {
                // Vanilla's VideoSettingsScreen as vanilla lays it out. No menu opens it (Video
                // Settings... opens the graphics screen, which has all of it); it stays for
                // `horizon-ctl options video-vanilla` and the tests that pin vanilla's screen.
                let mut rows = vec![Row::Header("options.video.display.header")];
                // A phone's app always fills its screen.
                let display: &[Opt] = if cfg!(target_os = "android") {
                    &[
                        Opt::MaxFps,
                        Opt::Vsync,
                        Opt::InactivityFps,
                        Opt::GuiScale,
                        Opt::Gamma,
                    ]
                } else {
                    rows.push(Row::Big(Opt::ExclusiveMode));
                    &[
                        Opt::MaxFps,
                        Opt::Vsync,
                        Opt::InactivityFps,
                        Opt::GuiScale,
                        Opt::Fullscreen,
                        Opt::ExclusiveFullscreen,
                        Opt::Gamma,
                        Opt::GraphicsBackend,
                    ]
                };
                rows.extend(pairs(display));
                rows.push(Row::Header("options.video.quality.header"));
                rows.push(Row::Big(Opt::Preset));
                rows.extend(pairs(&[
                    Opt::BiomeBlend,
                    Opt::RenderDistance,
                    Opt::ChunkBuilder,
                    Opt::SimulationDistance,
                    Opt::SmoothLighting,
                    Opt::Clouds,
                    Opt::Particles,
                    Opt::Mipmaps,
                    Opt::EntityShadows,
                    Opt::EntityDistance,
                    Opt::MenuBlur,
                    Opt::CloudRange,
                    Opt::CutoutLeaves,
                    Opt::ImprovedTransparency,
                    Opt::TextureFiltering,
                    Opt::MaxAnisotropy,
                    Opt::WeatherRadius,
                ]));
                rows.push(Row::Header("options.video.preferences.header"));
                rows.extend(pairs(&[
                    Opt::Autosave,
                    Opt::Vignette,
                    Opt::AttackIndicator,
                    Opt::ChunkFade,
                ]));
                self.filter_option_rows(rows)
            }
            Page::Sound => {
                // SoundOptionsScreen, without Audio Device: Horizon opens the
                // output device itself and cannot switch it.
                let mut rows = vec![Row::Big(Opt::Volume(0))];
                let others: Vec<Opt> = (1..SOUND_SOURCES.len()).map(Opt::Volume).collect();
                rows.extend(pairs(&others));
                rows.extend(pairs(&[Opt::ShowSubtitles, Opt::DirectionalAudio]));
                rows.extend(pairs(&[Opt::MusicFrequency, Opt::MusicToast]));
                rows.push(Row::Big(Opt::SoundDevice));
                // Horizon's music page (an extra): the built-in tracks, by
                // situation, once it is on in Horizon Extras.
                if s.extras.music_page {
                    rows.push(Row::Buttons(
                        button(
                            0.0,
                            0.0,
                            310.0,
                            tr("horizon.pages.open.music"),
                            Act::Push(Page::Music),
                        ),
                        None,
                    ));
                }
                rows
            }
            Page::Music => {
                if self.music_groups.is_empty() {
                    return vec![Row::Header("horizon.pages.music.empty")];
                }
                let mut rows = Vec::new();
                for group in &self.music_groups {
                    rows.push(Row::Heading(group.label.clone()));
                    for track in &group.tracks {
                        // Left: the track, played now. Right: whether its pool
                        // still picks it (vanilla until one is turned off).
                        let on = !s.extras.music_off.contains(&track.id);
                        let play = button(
                            0.0,
                            0.0,
                            150.0,
                            Text::literal(track_name(&track.id).to_owned()),
                            Act::PlayMusicTrack(track.event.clone(), track.id.clone()),
                        );
                        let toggle = button(
                            0.0,
                            0.0,
                            150.0,
                            on_off(on),
                            Act::MusicTrackOn(track.id.clone(), on),
                        );
                        rows.push(Row::Buttons(play, Some(toggle)));
                    }
                }
                rows
            }
            Page::Online => {
                // OnlineOptionsScreen without the Xbox and friends-list parts.
                let mut rows = vec![Row::Header("options.online.friends.header")];
                rows.extend(pairs(&[Opt::InGameNotification, Opt::SharePresence]));
                rows.push(Row::Header("options.online.servers.header"));
                rows.push(Row::Big(Opt::AllowServerListing));
                rows.push(Row::Header("options.online.realms.header"));
                rows.push(Row::Big(Opt::RealmsNotifications));
                rows
            }
            Page::Controls => {
                let android = cfg!(target_os = "android");
                let mut rows = Vec::new();
                if android {
                    rows.push(Row::Big(Opt::AndroidInputSelection));
                    rows.push(Row::Buttons(
                        button(
                            0.,
                            0.,
                            150.,
                            Text::literal("Touch Controls..."),
                            Act::Push(Page::Touch),
                        ),
                        Some(button(
                            0.,
                            0.,
                            150.,
                            tr("horizon.pages.open.controller"),
                            Act::Push(Page::Controller),
                        )),
                    ));
                }
                rows.push(Row::Buttons(
                    button(
                        0.,
                        0.,
                        150.,
                        tr("controls.keybinds"),
                        Act::Push(Page::KeyBinds),
                    ),
                    Some(button(
                        0.,
                        0.,
                        150.,
                        tr("options.mouse_settings"),
                        Act::Push(Page::Mouse),
                    )),
                ));
                if !android {
                    rows.push(Row::Buttons(
                        button(
                            0.,
                            0.,
                            310.,
                            tr("horizon.pages.open.controller"),
                            Act::Push(Page::Controller),
                        ),
                        None,
                    ));
                }
                let mut controls = vanilla_control_options();
                if android {
                    let at = controls.iter().position(|o| *o == Opt::AutoJump).unwrap() + 1;
                    controls.insert(at, Opt::TouchAutoSprint);
                    controls.retain(|o| *o != Opt::OperatorItemsTab);
                }
                rows.extend(pairs(&controls));
                rows.push(Row::Buttons(
                    button(
                        0.,
                        0.,
                        150.,
                        Text::literal("Bedrock Inventory: ").append(on_off(s.bedrock_inventory)),
                        Act::BedrockInventory,
                    ),
                    Some(if android {
                        option_widget(s, Opt::OperatorItemsTab, 0., 0., 150.)
                    } else {
                        button(
                            0.,
                            0.,
                            150.,
                            Text::literal("Customize Layout..."),
                            Act::CustomButtons,
                        )
                    }),
                ));
                self.filter_option_rows(rows)
            }
            Page::Touch => {
                let mut rows = vec![Row::Header("Movement & Look Sensitivity")];
                rows.push(Row::Big(Opt::TouchMode));
                rows.extend(pairs(&[
                    Opt::TouchCrosshairGestures,
                    Opt::TouchAttackButtonLook,
                    Opt::TouchSensitivity,
                    Opt::TouchSensitivityHorizontal,
                    Opt::TouchSensitivityVertical,
                ]));
                rows.push(Row::Header("Control Hub & Feedback"));
                rows.extend(pairs(&[
                    Opt::TouchControlHub,
                    Opt::TouchUiVibration,
                    Opt::TouchGameVibration,
                ]));
                rows.push(Row::Header("HUD Buttons & Layout"));
                rows.extend(pairs(&[
                    Opt::TouchMouseButton,
                    Opt::TouchKeyboardButton,
                    Opt::TouchOffhand,
                    Opt::TouchShield,
                    Opt::TouchButtonScale,
                    Opt::TouchCircleSize,
                    Opt::TouchAttackSide,
                    Opt::GuiScale,
                    Opt::TouchLeftHand,
                    Opt::TouchSwapJumpSneak,
                    Opt::TouchAvoidCutout,
                ]));
                rows.push(Row::Header("Joystick, Actions & Gyroscope"));
                rows.extend(pairs(&[
                    Opt::TouchAnalog,
                    Opt::TouchSprintButton,
                    Opt::TouchSprintToggle,
                    Opt::TouchInventoryControls,
                    Opt::TouchInteract,
                    Opt::TouchDynamicJoystick,
                    Opt::TouchDynamicAreaX,
                    Opt::TouchDynamicAreaY,
                    Opt::AndroidGyroscope,
                    Opt::AndroidGyroSensitivity,
                ]));
                rows.push(Row::Buttons(
                    button(
                        0.0,
                        0.0,
                        310.0,
                        Text::literal("Customize Touch & GoyControl Layout..."),
                        Act::TouchLayout,
                    ),
                    None,
                ));
                self.filter_option_rows(rows)
            }
            Page::Mouse => pairs(MOUSE_OPTIONS),
            Page::Chat => pairs(&[
                Opt::ChatVisibility,
                Opt::ChatColors,
                Opt::ChatLinks,
                Opt::ChatLinksPrompt,
                Opt::ChatOpacity,
                Opt::TextBackgroundOpacity,
                Opt::ChatScale,
                Opt::ChatLineSpacing,
                Opt::ChatWidth,
                Opt::ChatHeightFocused,
                Opt::ChatHeightUnfocused,
                Opt::Narrator,
                Opt::AutoSuggestions,
                Opt::HideMatchedNames,
                Opt::ReducedDebugInfo,
                Opt::OnlySecureChat,
                Opt::SaveChatDrafts,
            ]),
            Page::Accessibility => {
                let mut rows = vec![Row::Buttons(
                    option_widget(s, Opt::Narrator, 0.0, 0.0, 150.0),
                    Some(button(
                        0.0,
                        0.0,
                        150.0,
                        tr("options.controls"),
                        Act::Push(Page::Controls),
                    )),
                )];
                rows.extend(pairs(&ACCESSIBILITY_OPTIONS[1..]));
                rows
            }
            Page::Font => pairs(&[Opt::ForceUnicode, Opt::JapaneseGlyphs]),
            Page::ModBrowser => self.mod_browser_rows(),
            Page::ModProject => self.mod_project_rows(),
            Page::NativeMod(i) => self.native_rows(s, i),
            Page::NativeGame => native_game_page_rows(s),
            Page::CustomButtons => {
                let mut rows = vec![Row::Note(
                    "Buttons of your own: place them in Touch Controls".into(),
                )];
                for b in &s.custom_buttons {
                    rows.push(Row::Buttons(
                        button(
                            0.0,
                            0.0,
                            150.0,
                            Text::literal(b.label()),
                            Act::CustomEdit(b.id),
                        ),
                        Some(button(
                            0.0,
                            0.0,
                            150.0,
                            Text::literal("Delete"),
                            Act::CustomDelete(b.id),
                        )),
                    ));
                }
                rows.push(Row::Buttons(
                    button(0.0, 0.0, 310.0, Text::literal("Add Button"), Act::CustomAdd),
                    None,
                ));
                rows
            }
            Page::CustomButton(id) => {
                use crate::settings::CustomAction as A;
                let b = s.custom_buttons.iter().find(|b| b.id == id);
                let held = |pick: CustomPick| match b.map(|b| &b.action) {
                    Some(A::Key(_)) if pick == CustomPick::Control => b.map(|b| b.label()),
                    Some(A::Input(_)) if pick == CustomPick::Keyboard => b.map(|b| b.label()),
                    _ => None,
                };
                let choice = |caption: &str, pick: CustomPick| {
                    let label = match held(pick) {
                        Some(now) => format!("{caption}: {now}"),
                        None => format!("{caption}..."),
                    };
                    button(
                        0.0,
                        0.0,
                        150.0,
                        Text::literal(label),
                        Act::Push(Page::CustomPick(id, pick)),
                    )
                };
                let look = match b {
                    Some(b) if b.image => "Yours",
                    Some(b) => b.icon.as_deref().unwrap_or("Its Name"),
                    None => "Its Name",
                };
                vec![
                    Row::Note("Sends a /command or a chat line when pressed".into()),
                    Row::SendField,
                    Row::Note("Or holds a key while pressed".into()),
                    Row::Buttons(
                        choice("Game Control", CustomPick::Control),
                        Some(choice("Keyboard Key", CustomPick::Keyboard)),
                    ),
                    Row::Note("How it looks".into()),
                    Row::Buttons(
                        button(
                            0.0,
                            0.0,
                            150.0,
                            Text::literal(format!("Icon: {look}")),
                            Act::Push(Page::CustomPick(id, CustomPick::Icon)),
                        ),
                        Some(button(
                            0.0,
                            0.0,
                            150.0,
                            Text::literal("Draw or Import..."),
                            Act::CustomDraw(id),
                        )),
                    ),
                    Row::Note("While pressed".into()),
                    Row::Buttons(
                        button(
                            0.0,
                            0.0,
                            150.0,
                            Text::literal(if b.is_some_and(|b| b.toggle) {
                                "Type: Toggle"
                            } else {
                                "Type: Hold"
                            }),
                            Act::CustomToggle(id),
                        ),
                        Some(button(
                            0.0,
                            0.0,
                            150.0,
                            Text::literal(if b.is_some_and(|b| b.pressed) {
                                "Pressed Image: Yours"
                            } else {
                                "Pressed Image..."
                            }),
                            Act::CustomDraw(own_buttons::pressed_slot(id)),
                        )),
                    ),
                ]
            }
            Page::CustomDraw(id) => vec![Row::Canvas(id)],
            Page::CustomPick(id, CustomPick::Control) => {
                let mut mappings: Vec<(&'static str, &'static str)> =
                    MAPPINGS.iter().map(|(n, _, c)| (*n, *c)).collect();
                mappings.retain(|(name, _)| keybinds::key_code(s.keys.get(name)).is_some());
                mappings.sort_by(|a, b| {
                    let ca = CATEGORIES.iter().position(|c| *c == a.1).unwrap_or(99);
                    let cb = CATEGORIES.iter().position(|c| *c == b.1).unwrap_or(99);
                    ca.cmp(&cb)
                        .then_with(|| lang::get(a.0).cmp(&lang::get(b.0)))
                });
                let mut rows: Vec<Row> = mappings
                    .into_iter()
                    .map(|(name, _)| {
                        let label = Text::literal(lang::get(name));
                        Row::Buttons(
                            button(0.0, 0.0, 310.0, label, Act::CustomKey(id, name)),
                            None,
                        )
                    })
                    .collect();
                // The keys native mods register, after the game's own.
                for (i, m) in s.mods.installed.iter().enumerate() {
                    let Some(page) = &m.native else {
                        continue;
                    };
                    for (k, mapping) in page.mappings.iter().enumerate() {
                        let label = Text::literal(format!("{}: {}", m.title, mapping.label));
                        rows.push(Row::Buttons(
                            button(0.0, 0.0, 310.0, label, Act::CustomModKey(id, i, k)),
                            None,
                        ));
                    }
                }
                rows
            }
            Page::CustomPick(id, CustomPick::Keyboard) => keybinds::keyboard_keys()
                .chunks(2)
                .map(|pair| {
                    let key = |name: &'static str| {
                        let label = keybinds::display_name(name);
                        button(0.0, 0.0, 150.0, label, Act::CustomInput(id, name))
                    };
                    Row::Buttons(key(pair[0]), pair.get(1).map(|name| key(name)))
                })
                .collect(),
            Page::CustomPick(id, CustomPick::Icon) => {
                let mut rows = vec![Row::Buttons(
                    button(
                        0.0,
                        0.0,
                        310.0,
                        Text::literal("No Icon: Its Name"),
                        Act::CustomIcon(id, None),
                    ),
                    None,
                )];
                for icons in crate::settings::CUSTOM_ICONS.chunks(7) {
                    let widgets = icons
                        .iter()
                        .map(|name| {
                            let mut w = button(
                                0.0,
                                0.0,
                                36.0,
                                Text::literal(*name),
                                Act::CustomIcon(id, Some(name)),
                            );
                            w.h = 36.0;
                            if let WKind::Button { icon, .. } = &mut w.kind {
                                *icon = Some(format!("starling:gui/sprites/touch/custom_{name}"));
                            }
                            w
                        })
                        .collect();
                    rows.push(Row::Icons(widgets));
                }
                rows
            }
            Page::Added(i) => self.added_rows(i),
            _ => {
                let _ = s;
                Vec::new()
            }
        }
    }
    /// The chosen pack's profiles (`profile.<name>` in its shaders.properties, in
    /// `shaderpacks/` beside `options.txt`), for the Shader Packs page.
    fn refresh_shader_profiles(&mut self, s: &Settings) {
        self.shader.screen.refresh_profiles(s);
    }
    /// A section heading of a list (`Row::Header`, `Row::Heading`), at `y`.
    fn heading_row(&self, out: &mut Vec<Widget>, label: Text, y: &mut f32, w: f32, top: f32) {
        let padding = if *y + self.scroll <= top + 2.0 {
            0.0
        } else {
            18.0
        };
        let label = label.with_style(&Style {
            bold: Some(true),
            underlined: Some(true),
            ..Style::default()
        });
        out.push(Widget {
            x: 0.0,
            y: *y + 2.0 + padding + 4.0,
            w,
            h: 9.0,
            kind: WKind::Text {
                text: label,
                color: u32::MAX,
                centered: true,
            },
            list: false,
            row: 0,
        });
        *y += padding + 18.0;
    }
    /// Build every widget of the current page.
    fn layout(&mut self, s: &Settings, w: f32, h: f32) -> (Vec<Widget>, f32, f32, f32) {
        let mut out = Vec::new();
        let mut rows = ListRows::default();
        let page = self.page();
        let cx = (w / 2.0).floor();
        if matches!(
            page,
            Page::Video | Page::Controls | Page::KeyBinds | Page::ModBrowser
        ) {
            self.settings_search.x = cx - 100.0;
            self.settings_search.y = 17.0;
        }
        let done = |y: f32| button(cx - 100.0, y, 200.0, tr("gui.done"), Act::Done);
        // (list top, list bottom, content height)
        let (mut top, mut bottom, mut content) = (33.0, h - 33.0, 0.0);
        match page {
            Page::Main => {
                out.push(Widget {
                    x: 0.0,
                    y: 12.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr("options.title"),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                out.push(option_widget(s, Opt::Fov, cx - 154.0, 29.0, 150.0));
                out.push(button(
                    cx + 4.0,
                    29.0,
                    150.0,
                    tr("options.online"),
                    Act::Push(Page::Online),
                ));
                let entries = [
                    // Everywhere for the layers and Main Hand; `skin_page` adds the name and skin.
                    ("options.skinCustomisation", Some(Page::Skin)),
                    ("options.sounds", Some(Page::Sound)),
                    // Video Settings... opens Horizon's graphics screen, which holds every
                    // video option, as Sodium's replaces vanilla's.
                    ("options.video", Some(Page::Graphics)),
                    ("options.controls", Some(Page::Controls)),
                    (
                        "options.language",
                        (!self.split_guest).then_some(Page::Language),
                    ),
                    ("options.chat", Some(Page::Chat)),
                    (
                        "options.resourcepack",
                        (!self.split_guest).then_some(Page::Packs),
                    ),
                    ("options.accessibility", Some(Page::Accessibility)),
                    // Vanilla's Telemetry Data and Credits slots: Horizon's
                    // settings and its mods, as a mod adds its button here.
                    // Credits & Attribution (WinScreen's credits) follows them.
                    ("horizon.pages.open.horizon_settings", Some(Page::Horizon)),
                    // Starling's Mods page (`open_at`).
                    ("horizon.pages.open.mods", Some(Page::NativeGame)),
                    ("options.credits_and_attribution", Some(Page::Credits)),
                ];
                // horizon-extras: without it, vanilla's last row instead of Horizon Settings and
                // Mods: Telemetry Data (greyed: nothing is sent) and Credits & Attribution.
                let plain;
                let entries: &[_] = if horizon_extras() {
                    &entries
                } else {
                    plain = [
                        &entries[..entries.len() - 3],
                        &[
                            ("options.telemetry", None),
                            ("options.credits_and_attribution", Some(Page::Credits)),
                        ],
                    ]
                    .concat();
                    &plain
                };
                let entries = with_diagnostics(entries);
                let rows = entries.len().div_ceil(2) as f32;
                let gy = (61.0 + 30.0f32).min(h - 33.0 - rows * 24.0);
                for (i, (key, page)) in entries.iter().enumerate() {
                    let x = cx - 158.0 + 4.0 + (i % 2) as f32 * 158.0;
                    let y = gy + (i / 2) as f32 * 24.0;
                    let b = button(x, y, 150.0, tr(key), page.map_or(Act::None, Act::Push));
                    out.push(if page.is_some() { b } else { inactive(b) });
                }
                out.push(done(h - 27.0));
                top = 0.0;
                bottom = 0.0;
            }
            Page::Credits => {
                // CreditsAndAttributionScreen: the title header, three 210-wide
                // buttons 8 apart (HeaderAndFooterLayout: 30 below the header
                // unless the footer is closer), Done.
                out.push(Widget {
                    x: 0.0,
                    y: 12.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr(Self::title(page)),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                let y = (33.0 + 30.0f32).min(h - 33.0 - 76.0);
                let x = cx - 105.0;
                if ON_A_PHONE {
                    // Starling's founders, then the game it is built on (THIRD_PARTY_NOTICES.md).
                    for (i, key) in ["starling.credits.founders", "starling.credits.horizon"]
                        .into_iter()
                        .enumerate()
                    {
                        out.push(Widget {
                            x: 0.0,
                            y: y - 26.0 + 12.0 * i as f32,
                            w,
                            h: 9.0,
                            kind: WKind::Text {
                                text: tr(key),
                                color: u32::MAX,
                                centered: true,
                            },
                            list: false,
                            row: 0,
                        });
                    }
                }
                out.push(button(
                    x,
                    y,
                    210.0,
                    tr("credits_and_attribution.button.credits"),
                    Act::Credits,
                ));
                // ConfirmLinkScreen to minecraft.net: Horizon opens no links.
                for (i, key) in [
                    "credits_and_attribution.button.attribution",
                    "credits_and_attribution.button.licenses",
                ]
                .into_iter()
                .enumerate()
                {
                    let y = y + 28.0 * (i + 1) as f32;
                    out.push(inactive(button(x, y, 210.0, tr(key), Act::None)));
                }
                // horizon-extras: a build without them says where the official one is.
                if !horizon_extras() {
                    out.push(Widget {
                        x: 0.0,
                        y: (y + 92.0).min(h - 42.0),
                        w,
                        h: 9.0,
                        kind: WKind::Text {
                            text: tr("horizon.plain.official"),
                            color: 0xFFA0A0A0,
                            centered: true,
                        },
                        list: false,
                        row: 0,
                    });
                }
                out.push(done(h - 27.0));
                top = 0.0;
                bottom = 0.0;
            }
            Page::KeyBinds => {
                top = 38.0;
                out.push(Widget {
                    x: 0.0,
                    y: 4.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr(Self::title(page)),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                let keypad = s.keypad.enabled;
                if ON_A_PHONE || keypad {
                    top = 64.0;
                    out.push(button(
                        cx - 150.0,
                        39.0,
                        300.0,
                        Text::literal(if keypad {
                            "Input profile: D-pad / keypad"
                        } else {
                            "Input profile: Keyboard"
                        }),
                        Act::KeyboardProfile,
                    ));
                }
                let label = |name: &str| {
                    crate::keypad::ACTIONS
                        .iter()
                        .find(|a| a.0 == name)
                        .map_or_else(|| lang::get(name), |a| a.1.to_owned())
                };
                let mut mappings: Vec<(&str, &str)> = if keypad {
                    crate::keypad::ACTIONS
                        .iter()
                        .map(|a| (a.0, "movement"))
                        .collect()
                } else {
                    MAPPINGS.iter().map(|(n, _, c)| (*n, *c)).collect()
                };
                let query = self.settings_search.value.to_lowercase();
                mappings.retain(|(name, _)| {
                    query.is_empty() || label(name).to_lowercase().contains(&query)
                });
                mappings.sort_by(|a, b| {
                    let ca = CATEGORIES.iter().position(|c| *c == a.1).unwrap_or(99);
                    let cb = CATEGORIES.iter().position(|c| *c == b.1).unwrap_or(99);
                    ca.cmp(&cb).then_with(|| label(a.0).cmp(&label(b.0)))
                });
                let row_left = cx - 170.0;
                let scrollbar_x = row_left + 340.0 + 6.0 + 2.0;
                // VideoSettingsScreen.addTitle: a changed graphics API needs a restart.
                if page == Page::Video && s.horizon.api != s.startup_api {
                    out.push(Widget {
                        x: 0.0,
                        y: 15.0,
                        w,
                        h: 9.0,
                        kind: WKind::Text {
                            text: tr("options.restartRequired"),
                            color: rgba(223, 80, 80, 255),
                            centered: true,
                        },
                        list: false,
                        row: 0,
                    });
                }
                let list_start = out.len();
                let mut y = top + 2.0 - self.scroll;
                let mut last_category = "";
                for (name, category) in mappings {
                    if category != last_category {
                        last_category = category;
                        rows.begin(&out, y);
                        let label =
                            Text::translate(&format!("key.category.minecraft.{category}"), &[])
                                .with_style(&Style {
                                    bold: Some(true),
                                    underlined: Some(true),
                                    ..Style::default()
                                });
                        out.push(Widget {
                            x: 0.0,
                            y: y + 7.0,
                            w,
                            h: 9.0,
                            kind: WKind::Text {
                                text: label,
                                color: u32::MAX,
                                centered: true,
                            },
                            list: false,
                            row: 0,
                        });
                        y += 20.0;
                        rows.end(&mut out, y);
                    }
                    rows.begin(&out, y);
                    out.push(Widget {
                        x: row_left + 2.0,
                        y: y + 6.0,
                        w: 200.0,
                        h: 9.0,
                        kind: WKind::Text {
                            text: if keypad {
                                Text::literal(label(name))
                            } else {
                                tr(name)
                            },
                            color: u32::MAX,
                            centered: false,
                        },
                        list: false,
                        row: 0,
                    });
                    let key = if keypad {
                        s.keypad
                            .keys(name)
                            .first()
                            .cloned()
                            .unwrap_or_else(|| "key.keyboard.unknown".into())
                    } else {
                        s.keys.get(name).to_owned()
                    };
                    let mut message = keybinds::display_name(&key);
                    let conflicts: Vec<_> = if keypad {
                        crate::keypad::ACTIONS
                            .iter()
                            .filter(|a| {
                                a.0 != name
                                    && s.keypad.keys(a.0).contains(&key)
                                    && key != "key.keyboard.unknown"
                            })
                            .map(|a| a.0.to_owned())
                            .collect()
                    } else {
                        key_conflicts(
                            s,
                            "",
                            name,
                            s.keys.get(name),
                            0,
                            crate::mod_list::KeyConflictContext::Universal,
                            s.keys.is_default(name),
                        )
                    };
                    if !conflicts.is_empty() {
                        message = Text::literal("[ ")
                            .append(message.with_color(text::WHITE))
                            .append(" ]")
                            .with_color(text::YELLOW);
                    }
                    if self.selected_key == Some(name) {
                        message = Text::literal("> ")
                            .append(message.with_style(&Style {
                                color: Some(text::WHITE),
                                underlined: Some(true),
                                ..Style::default()
                            }))
                            .append(" <")
                            .with_color(text::YELLOW);
                    }
                    let reset_x = scrollbar_x - 50.0 - 10.0;
                    let change_x = reset_x - 5.0 - 75.0;
                    let name_static = if keypad {
                        crate::keypad::ACTIONS
                            .iter()
                            .find(|a| a.0 == name)
                            .map_or("", |a| a.0)
                    } else {
                        MAPPINGS.iter().find(|m| m.0 == name).map_or("", |m| m.0)
                    };
                    out.push(button(change_x, y, 75.0, message, Act::Key(name_static)));
                    if !conflicts.is_empty() {
                        out.push(Widget {
                            x: change_x - 6.0,
                            y: y + 1.0,
                            w: 3.0,
                            h: 18.0,
                            kind: WKind::Text {
                                text: Text::empty(),
                                color: rgba(255, 255, 0, 255),
                                centered: false,
                            },
                            list: false,
                            row: 0,
                        });
                    }
                    let reset = button(
                        reset_x,
                        y,
                        50.0,
                        tr("controls.reset"),
                        Act::ResetKey(name_static),
                    );
                    out.push(
                        if (keypad && !s.keypad.bindings.contains_key(name))
                            || (!keypad && s.keys.is_default(name))
                        {
                            inactive(reset)
                        } else {
                            reset
                        },
                    );
                    y += 20.0;
                    rows.end(&mut out, y);
                }
                // Each native mod's registered keys, under the mod's title and their categories.
                for (i, m) in s.mods.installed.iter().enumerate() {
                    let Some(page) = m.native.as_ref().filter(|p| !p.mappings.is_empty()) else {
                        continue;
                    };
                    rows.begin(&out, y);
                    out.push(section_title(Text::literal(m.title.clone()), y, w));
                    y += 20.0;
                    rows.end(&mut out, y);
                    let mut last_category = "";
                    for (k, mapping) in page.mappings.iter().enumerate() {
                        if !query.is_empty()
                            && !lang::get(&mapping.label).to_lowercase().contains(&query)
                            && !lang::get(&mapping.category).to_lowercase().contains(&query)
                        {
                            continue;
                        }
                        let category = mapping.category.as_str();
                        if !category.is_empty() && category != last_category {
                            last_category = category;
                            rows.begin(&out, y);
                            out.push(section_title(tr(&mapping.category), y, w));
                            y += 20.0;
                            rows.end(&mut out, y);
                        }
                        let (key, modifiers) = s.mods.binding(
                            &m.id,
                            &mapping.id,
                            (mapping.key.as_str(), mapping.modifiers),
                        );
                        let at_default = key == mapping.key && modifiers == mapping.modifiers;
                        let mut message = binding_text(&key, modifiers);
                        if !key_conflicts(
                            s,
                            &m.id,
                            &mapping.id,
                            &key,
                            modifiers,
                            mapping.context,
                            at_default,
                        )
                        .is_empty()
                        {
                            message = Text::literal("[ ")
                                .append(message.with_color(text::WHITE))
                                .append(" ]")
                                .with_color(text::YELLOW);
                        }
                        if self.selected_mod_key == Some((i, k)) {
                            message = Text::literal("> ")
                                .append(message.with_style(&Style {
                                    color: Some(text::WHITE),
                                    underlined: Some(true),
                                    ..Style::default()
                                }))
                                .append(" <")
                                .with_color(text::YELLOW);
                        }
                        rows.begin(&out, y);
                        out.push(Widget {
                            x: row_left + 2.0,
                            y: y + 6.0,
                            w: 200.0,
                            h: 9.0,
                            kind: WKind::Text {
                                text: tr(&mapping.label),
                                color: u32::MAX,
                                centered: false,
                            },
                            list: false,
                            row: 0,
                        });
                        let reset_x = scrollbar_x - 50.0 - 10.0;
                        let change_x = reset_x - 5.0 - 75.0;
                        out.push(button(change_x, y, 75.0, message, Act::ModKey(i, k)));
                        let reset = button(
                            reset_x,
                            y,
                            50.0,
                            tr("controls.reset"),
                            Act::ResetModKey(i, k),
                        );
                        out.push(if at_default { inactive(reset) } else { reset });
                        y += 20.0;
                        rows.end(&mut out, y);
                    }
                }
                content = y + self.scroll - top + 2.0;
                out[list_start..].iter_mut().for_each(|w| w.list = true);
                out.push(button(
                    cx - 154.0,
                    h - 27.0,
                    150.0,
                    tr("controls.resetAll"),
                    Act::ResetKeys,
                ));
                out.push(button(cx + 4.0, h - 27.0, 150.0, tr("gui.done"), Act::Done));
            }
            Page::Language => {
                top = 36.0;
                bottom = h - 53.0;
                out.push(Widget {
                    x: 0.0,
                    y: 4.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr(Self::title(page)),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                self.language_search.x = cx - 100.0;
                self.language_search.y = 17.0;
                let query = self.language_search.value.to_lowercase();
                let current = self
                    .language_choice
                    .clone()
                    .unwrap_or_else(|| s.language.clone());
                if std::mem::take(&mut self.center_language)
                    && let Some(i) = self.languages.iter().position(|l| l.code == current)
                {
                    // AbstractSelectionList.centerScrollOn.
                    self.scroll = (i as f32 * 18.0 + 9.0 - (bottom - top) / 2.0).max(0.0);
                }
                let list_start = out.len();
                let mut y = top + 2.0 - self.scroll;
                for l in &self.languages {
                    if !query.is_empty()
                        && !l.label().to_lowercase().contains(&query)
                        && !l.code.contains(&query)
                    {
                        continue;
                    }
                    rows.begin(&out, y);
                    out.push(Widget {
                        x: cx - 110.0,
                        y,
                        w: 220.0,
                        h: 18.0,
                        kind: WKind::LanguageRow {
                            label: Text::literal(l.label()),
                            selected: l.code == current,
                            code: l.code.clone(),
                        },
                        list: false,
                        row: 0,
                    });
                    y += 18.0;
                    rows.end(&mut out, y);
                }
                content = y + self.scroll - top + 2.0;
                out[list_start..].iter_mut().for_each(|w| w.list = true);
                out.push(Widget {
                    x: 0.0,
                    y: h - 45.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr("options.languageAccuracyWarning"),
                        color: rgba(0xBA, 0xBA, 0xBA, 255),
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                out.push(button(
                    cx - 154.0,
                    h - 28.0,
                    150.0,
                    tr("options.font"),
                    Act::Push(Page::Font),
                ));
                out.push(button(cx + 4.0, h - 28.0, 150.0, tr("gui.done"), Act::Done));
            }
            // Drawn by `pack_screen` and `mods_screen`.
            Page::Packs | Page::Mods => {}
            _ => {
                if matches!(page, Page::Video | Page::Controls | Page::ModBrowser) {
                    top = 38.0;
                }
                out.push(Widget {
                    x: 0.0,
                    y: if matches!(page, Page::Video | Page::Controls | Page::ModBrowser) {
                        4.0
                    } else {
                        12.0
                    },
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr(Self::title(page)),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                let list_start = out.len();
                let mut y = top + 2.0 - self.scroll;
                for row in self.list_rows(s) {
                    rows.begin(&out, y);
                    match row {
                        Row::Header(key) => {
                            let label = tr(key);
                            self.heading_row(&mut out, label, &mut y, w, top);
                        }
                        Row::Heading(key) => {
                            let label = tr(&key);
                            self.heading_row(&mut out, label, &mut y, w, top);
                        }
                        Row::Note(text) => {
                            out.push(Widget {
                                x: 0.0,
                                y: y + 4.0,
                                w,
                                h: 9.0,
                                kind: WKind::Text {
                                    text: Text::literal(text),
                                    color: 0xFFA0A0A0,
                                    centered: true,
                                },
                                list: false,
                                row: 0,
                            });
                            y += 16.0;
                        }
                        Row::Big(opt) => {
                            out.push(option_widget(s, opt, cx - 155.0, y + 2.0, 310.0));
                            y += 25.0;
                        }
                        Row::Pair(a, b) => {
                            out.push(option_widget(s, a, cx - 155.0, y + 2.0, 150.0));
                            if let Some(b) = b {
                                out.push(option_widget(s, b, cx + 5.0, y + 2.0, 150.0));
                            }
                            y += 25.0;
                        }
                        Row::Buttons(mut a, b) => {
                            a.x = cx - 155.0;
                            a.y = y + 2.0;
                            let after = a.x + a.w + 10.0;
                            out.push(a);
                            if let Some(mut b) = b {
                                b.x = after;
                                b.y = y + 2.0;
                                out.push(b);
                            }
                            y += 25.0;
                        }
                        Row::Field => {
                            out.push(Widget {
                                x: cx - 155.0,
                                y: y + 8.0,
                                w: 40.0,
                                h: 9.0,
                                kind: WKind::Text {
                                    text: Text::literal("Name"),
                                    color: 0xFFA0_A0A0,
                                    centered: false,
                                },
                                list: false,
                                row: 0,
                            });
                            // Drawn and edited by the page itself (`skin.name`).
                            let field = &mut self.skin.name;
                            (field.x, field.y) = (cx - 115.0, y + 2.0);
                            (field.width, field.height) = (270.0, 20.0);
                            y += 25.0;
                        }
                        Row::SendField => {
                            // Drawn and edited by the page itself.
                            let field = &mut self.custom_send;
                            (field.x, field.y) = (cx - 155.0, y + 2.0);
                            (field.width, field.height) = (310.0, 20.0);
                            y += 25.0;
                        }
                        Row::Icons(icons) => {
                            let mut x = cx - 155.0;
                            for mut icon in icons {
                                (icon.x, icon.y) = (x, y + 2.0);
                                icon.list = true;
                                x += 44.0;
                                out.push(icon);
                            }
                            y += 42.0;
                        }
                        Row::Canvas(id) => {
                            // As big as the page has room for, its tools beside it.
                            let side = own_buttons::SIZE as f32;
                            let cell = ((h - 72.0) / side).floor().clamp(3.0, 8.0);
                            let x = cx - 155.0;
                            self.painter.at = [x, y + 2.0, cell];
                            let tools = x + side * cell + 46.0;
                            for (i, (label, act)) in [
                                ("Clear", Act::CanvasClear),
                                ("Plate", Act::CanvasPlate),
                                ("Import PNG", Act::CanvasImport(id)),
                            ]
                            .into_iter()
                            .enumerate()
                            {
                                let mut tool = button(
                                    tools,
                                    y + 2.0 + i as f32 * 24.0,
                                    80.0,
                                    Text::literal(label),
                                    act,
                                );
                                tool.list = true;
                                out.push(tool);
                            }
                            y += side * cell + 6.0;
                        }
                        Row::Warning(text) => {
                            out.push(Widget {
                                x: 0.0,
                                y: y + 4.0,
                                w,
                                h: 9.0,
                                kind: WKind::Text {
                                    text: Text::literal(text),
                                    color: rgba(255, 85, 85, 255),
                                    centered: true,
                                },
                                list: false,
                                row: 0,
                            });
                            y += 16.0;
                        }
                    }
                    rows.end(&mut out, y);
                }
                content = y + self.scroll - top + 2.0;
                out[list_start..].iter_mut().for_each(|w| w.list = true);
                let folder = match page {
                    Page::Accessibility => {
                        Some(("options.accessibility.link", Act::AccessibilityHelp))
                    }
                    // Starling's Mods page.
                    Page::NativeGame => Some(("horizon.mods.open_folder", Act::OpenModFolder)),
                    Page::Skin if self.skin_page => Some(("Open Skin Folder", Act::OpenSkinFolder)),
                    _ => None,
                };
                if let Some((label, act)) = folder {
                    // Android's way to find mods, resource packs and shader packs.
                    let shelf = catalog_shelf(page).filter(|_| self.catalog_buttons);
                    let (open_w, catalog) =
                        crate::pack_screen::open_folder_slot(cx - 154.0, shelf.is_some());
                    if let (Some(shelf), Some((x, width))) = (shelf, catalog) {
                        out.push(button(
                            x,
                            h - 27.0,
                            width,
                            tr("Catalog"),
                            Act::OpenCatalog(shelf),
                        ));
                    }
                    let label = if catalog.is_some() {
                        crate::pack_screen::SHORT_OPEN_FOLDER
                    } else {
                        label
                    };
                    out.push(button(cx - 154.0, h - 27.0, open_w, tr(label), act));
                    out.push(button(cx + 4.0, h - 27.0, 150.0, tr("gui.done"), Act::Done));
                } else {
                    out.push(done(h - 27.0));
                }
            }
        }
        // Layout may change the offset (language centering), or the list may
        // shrink after a resize/search. Correct the rows in this same frame.
        let previous_scroll = self.scroll;
        self.clamp_scroll(top, bottom, content);
        for widget in &mut out {
            if widget.list {
                widget.y += previous_scroll - self.scroll;
            }
            if let WKind::Button {
                act: Act::Cycle(Opt::RotateWithMinecarts),
                active,
                ..
            } = &mut widget.kind
            {
                *active = self.in_world && self.minecart_improvements;
            }
        }
        if page == Page::Skin && self.skin_page {
            self.skin.name.y += previous_scroll - self.scroll;
        }
        self.row_bands = rows
            .bands
            .into_iter()
            .map(|band| band.map(|y| y + previous_scroll - self.scroll))
            .collect();
        (out, top, bottom, content)
    }
    fn clamp_scroll(&mut self, top: f32, bottom: f32, content: f32) {
        let max = (content - (bottom - top)).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max);
    }
    /// The page's list as the narrator reads it: Language's
    /// `ObjectSelectionList` ("Selected: <language>"), or the
    /// `ContainerObjectSelectionList` the rows' widgets belong to, with the row
    /// under the mouse (`getEntryAtPosition`: within the row's width).
    fn narrate_list(
        &self,
        widgets: &[Widget],
        key: u64,
        (top, bottom): (f32, f32),
        w: f32,
        cursor: (f32, f32),
    ) {
        use crate::widgets::{ListKind, Narratable, Row as Narrated};
        let rect = [0.0, top, w, bottom - top];
        let in_list = cursor.1 >= top && cursor.1 < bottom;
        if self.page() == Page::Language {
            crate::widgets::narratable(|| {
                let rows: Vec<(&Widget, &Text, bool)> = widgets
                    .iter()
                    .filter_map(|wd| match &wd.kind {
                        WKind::LanguageRow {
                            label, selected, ..
                        } => Some((wd, label, *selected)),
                        _ => None,
                    })
                    .collect();
                let narrated = |(wd, label, _): &(&Widget, &Text, bool)| {
                    Narrated::new(
                        wd.row,
                        Text::translate("narrator.select", &[(*label).clone()]).plain(),
                    )
                };
                let hovered = rows
                    .iter()
                    .find(|(wd, ..)| in_list && ui::hovered(cursor, wd.x, wd.y, wd.w, wd.h));
                let selected = rows.iter().find(|(_, _, selected)| *selected);
                Narratable::list(
                    ListKind::Object,
                    rows.len(),
                    hovered.map(narrated),
                    selected.map(narrated),
                    rect,
                )
                .key(key)
            });
            return;
        }
        // OptionsList rows are 310 wide, KeyBindsList's 340.
        let half = if self.page() == Page::KeyBinds {
            170.0
        } else {
            155.0
        };
        let hovered_row = (in_list && (cursor.0 - (w / 2.0).floor()).abs() < half)
            .then(|| {
                self.row_bands
                    .iter()
                    .position(|[a, b]| cursor.1 >= *a && cursor.1 < *b)
            })
            .flatten();
        crate::widgets::narratable(|| {
            Narratable::container_list(self.row_bands.len(), hovered_row, rect).key(key)
        });
    }
    /// The finger lifted from a touch screen: the widget it tapped keeps no focus, so neither its
    /// highlight nor its tooltip stays there. A keyboard's or a controller's focus stays.
    pub fn drop_pointer_focus(&mut self) {
        if !self.focus_from_keyboard {
            self.focus = None;
        }
    }

    /// `output`: that of the renderer the options are drawn for, each draw, so the graphics
    /// screen's rows and calibration read its offer and report as they are now, not another
    /// window's, and not one from before the page opened.
    pub fn draw(
        &mut self,
        s: &Settings,
        output: crate::gfx::EffectiveOutput,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
    ) {
        self.output = output;
        let (w, h) = (batch.width, batch.height);
        if let Some(error) = &self.resource_error {
            ui::menu_background(batch, atlas, 0.0, 0.0, w, h, self.in_world);
            let title = tr("resourcePack.load_fail");
            // An alert: its title and its message.
            crate::widgets::begin_screen(|| {
                crate::widgets::ScreenInfo::new(
                    "options/resource_error",
                    crate::widgets::join_for_narration(&[&title.plain(), error]),
                )
            });
            batch.text_styled(
                atlas,
                &title,
                ((w - atlas.font.width_text(&title)) / 2.0).floor(),
                30.0,
                u32::MAX,
                true,
            );
            let lines = atlas.font.split(&Text::literal(error), (w - 50.0).max(1.0));
            let limit = ((h - 110.0) / 9.0).max(1.0) as usize;
            for (i, line) in lines.iter().take(limit).enumerate() {
                batch.text_styled(
                    atlas,
                    line,
                    ((w - atlas.font.width_text(line)) / 2.0).floor(),
                    60.0 + i as f32 * 9.0,
                    u32::MAX,
                    true,
                );
            }
            ui::action_button(
                batch,
                atlas,
                crate::widgets::Action::Done,
                w / 2.0 - 100.0,
                h - 28.0,
                200.0,
                20.0,
                &tr("gui.done"),
                true,
                ui::hovered(cursor, w / 2.0 - 100.0, h - 28.0, 200.0, 20.0),
            );
            return;
        }
        // The page is its own screen (OptionsScreen, VideoSettingsScreen...).
        let page = self.page();
        crate::widgets::begin_screen(|| {
            crate::widgets::ScreenInfo::new(
                format!("options/{page:?}"),
                tr(Self::title(page)).plain(),
            )
        });
        if self.page() == Page::HdrCalibration
            && let Some(c) = &self.calibration
        {
            c.draw(batch, atlas, cursor);
            return;
        }
        if self.page() == Page::PackOptions {
            self.pack_options.draw(batch, atlas, cursor, self.in_world);
            return;
        }
        let waiting = self.waiting();
        let in_world = self.in_world;
        if self.draw_radial_name(batch, atlas, cursor, in_world) {
            return;
        }
        if let Some(shell) = self.shell_mut() {
            shell.draw(s, batch, atlas, cursor, in_world, waiting);
            // The keys lay the screen out at the size it was drawn at.
            self.size = (batch.width, batch.height);
            return;
        }
        if self.page() == Page::SplitSetup {
            self.split_setup.on = s.extras.splitscreen;
            // The controller picks a tile up between presses: the screen's size from the draw.
            self.split_setup.size = (batch.width, batch.height);
            self.split_setup.draw(batch, atlas, cursor, self.in_world);
            return;
        }
        if self.page() == Page::SplitProfiles {
            self.split_profiles
                .draw(batch, atlas, cursor, self.in_world, self.pad_glyphs);
            return;
        }
        if self.page() == Page::Studio {
            self.studio.draw(batch, atlas, cursor, self.in_world);
            return;
        }
        if self.page() == Page::Mods {
            ui::menu_background(batch, atlas, 0.0, 0.0, w, h, self.in_world);
            let blink = self.pack_focus.elapsed().as_millis() as u64;
            let m = crate::screen_draw::Measure(&atlas.font);
            let mouse = (cursor.0.floor() as i32, cursor.1.floor() as i32);
            let screen = self.mods_page(s, (w, h));
            let mut draws = screen.draw(&m, mouse, blink, crate::hud_overlays::now_ms());
            screen.bind_icons(&mut draws, atlas, &mut batch.patches);
            crate::screen_draw::paint(batch, atlas, &draws);
            return;
        }
        if self.page() == Page::Packs {
            self.watch_data_packs(atlas, (w, h));
            ui::menu_background(batch, atlas, 0.0, 0.0, w, h, self.in_world);
            let blink = self.pack_focus.elapsed().as_millis() as u64;
            let m = crate::screen_draw::Measure(&atlas.font);
            let mouse = (cursor.0.floor() as i32, cursor.1.floor() as i32);
            let packs = self.packs(atlas, (w, h));
            let mut draws = packs.draw(&m, mouse, blink, crate::hud_overlays::now_ms());
            packs.bind_icons(&mut draws, atlas, &mut batch.patches);
            crate::screen_draw::paint(batch, atlas, &draws);
            return;
        }
        if self.page() == Page::Pvp {
            self.wide_extras = wide_extras(s, |t| atlas.font.width(t));
        }
        let (widgets, top, bottom, content) = self.layout(s, w, h);
        self.size = (w, h);
        let in_world = self.in_world;
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, in_world);
        let list = bottom > top;
        if list {
            ui::list_background(batch, atlas, top, bottom, in_world);
        }
        let visible = |wd: &Widget| !wd.list || (wd.y < bottom && wd.y + wd.h > top);
        // The list's narration: the language list is an ObjectSelectionList,
        // the others ContainerObjectSelectionLists whose rows' widgets are
        // recorded as they are drawn.
        let list_key = crate::widgets::key_of(&format!("options/{page:?}/list"));
        let framebuffer = [(w * batch.scale) as i32, (h * batch.scale) as i32];
        if list {
            self.narrate_list(&widgets, list_key, (top, bottom), w, cursor);
        }
        let mut tooltip = None;
        // The widget a keyboard-focused tooltip belongs to: `MenuTooltipPositioner`
        // keeps it against that widget, not at the pointer.
        let mut tooltip_rect: Option<[f32; 4]> = None;
        for (wd_idx, wd) in widgets.iter().enumerate() {
            if !visible(wd) {
                continue;
            }
            let is_focused = self
                .focus
                .as_ref()
                .is_some_and(|fid| matches_focus(fid, wd_idx, wd));
            let start = batch.vertices.len();
            let pointer_over = ui::hovered(cursor, wd.x, wd.y, wd.w, wd.h)
                && (!wd.list || cursor.1 >= top && cursor.1 < bottom);
            let hover = pointer_over || is_focused;
            // The tooltip is the pointer's (`setTooltip`, drawn while hovered);
            // a keyboard-focused widget keeps it against the widget
            // (`MenuTooltipPositioner`). Focus a mouse click leaves behind
            // does not: the description goes when the mouse leaves, like
            // vanilla's.
            let tip_hover = pointer_over || (is_focused && self.focus_from_keyboard);
            if tip_hover && let Some(tip) = tooltip_text(s, wd) {
                tooltip = Some(match &wd.kind {
                    // The duplicate keybinds tooltip keeps its own lines.
                    WKind::Button {
                        act: Act::Key(_), ..
                    } => tip
                        .plain()
                        .split('\n')
                        .map(|l| Text::literal(l.to_owned()))
                        .collect::<Vec<_>>(),
                    _ => atlas.font.split(&tip, 170.0),
                });
                tooltip_rect =
                    (!pointer_over || self.pad_driving).then_some([wd.x, wd.y, wd.w, wd.h]);
            }
            match &wd.kind {
                WKind::Button {
                    label,
                    active,
                    icon,
                    act,
                } => {
                    match icon.as_deref().filter(|i| atlas.sprites.contains_key(*i)) {
                        Some(icon) => {
                            ui::sprite_sized(batch, atlas, icon, wd.x, wd.y, wd.w, wd.h, u32::MAX)
                        }
                        None => ui::focused_button(
                            batch,
                            atlas,
                            [wd.x, wd.y, wd.w, wd.h],
                            label,
                            *active,
                            pointer_over,
                            is_focused,
                            matches!(act, Act::Done).then_some(crate::widgets::Action::Done),
                        ),
                    }
                    crate::widgets::last_narratable(|n| {
                        narrate_widget(n, s, wd, hover, list_key, framebuffer);
                    });
                }
                WKind::Slider { label, value, opt } => {
                    let dragging = self.dragging == Some(*opt);
                    let shown = self.slider_shown(*opt, *value);
                    ui::slider_ext(
                        batch,
                        atlas,
                        wd.x,
                        wd.y,
                        wd.w,
                        label,
                        shown,
                        hover || dragging,
                        is_focused,
                        self.can_change_value && is_focused,
                        true,
                        dragging,
                    );
                    crate::widgets::last_narratable(|n| {
                        narrate_widget(n, s, wd, hover, list_key, framebuffer);
                    });
                }
                WKind::Text {
                    text,
                    color,
                    centered,
                } => {
                    if text.is_empty() {
                        batch.fill(atlas, wd.x, wd.y, wd.w, wd.h, *color);
                    } else if *centered {
                        let tw = atlas.font.width_text(text);
                        // A list's header row (OptionsList.HeaderEntry,
                        // KeyBindsList.CategoryEntry): a FocusableTextWidget
                        // narrating its text, 4 pixels of padding round it.
                        if wd.list && *color == u32::MAX {
                            crate::widgets::narratable(|| {
                                let x = wd.x + ((wd.w - tw) / 2.0).floor();
                                crate::widgets::Narratable::text(
                                    text.plain(),
                                    [x - 4.0, wd.y - 4.0, tw + 8.0, 17.0],
                                )
                                .key(crate::widgets::key_of(&(list_key, wd.row)))
                                .child_of(list_key, wd.row)
                            });
                        }
                        batch.text_styled(
                            atlas,
                            text,
                            wd.x + ((wd.w - tw) / 2.0).floor(),
                            wd.y,
                            *color,
                            true,
                        );
                    } else {
                        batch.text_styled(atlas, text, wd.x, wd.y, *color, true);
                    }
                }
                WKind::LanguageRow {
                    label, selected, ..
                } => {
                    let rect = [wd.x, wd.y, wd.w, wd.h];
                    crate::widgets::record_as(
                        &label.plain(),
                        rect,
                        true,
                        crate::widgets::Kind::Row,
                    );
                    crate::widgets::target_as(rect, crate::widgets::Kind::Row);
                    if *selected || is_focused {
                        batch.fill(atlas, wd.x, wd.y, wd.w, wd.h, u32::MAX);
                        batch.fill(
                            atlas,
                            wd.x + 1.0,
                            wd.y + 1.0,
                            wd.w - 2.0,
                            wd.h - 2.0,
                            rgba(0, 0, 0, 255),
                        );
                    }
                    let tw = atlas.font.width_text(label);
                    batch.text_styled(
                        atlas,
                        label,
                        wd.x + ((wd.w - tw) / 2.0).floor(),
                        wd.y + 2.0 + 7.0 - 4.0,
                        u32::MAX,
                        true,
                    );
                }
            }
            if wd.list && (wd.y < top || wd.y + wd.h > bottom) {
                batch.clip_since(start, 0.0, top, w, bottom);
            }
        }
        if list {
            ui::separators(batch, atlas, top, bottom, in_world);
            let sx = self.scrollbar_x(w);
            ui::scrollbar(batch, atlas, sx, top, bottom, content, self.scroll);
        }
        if self.page() == Page::Language {
            let mut search = self.language_search.clone();
            if self.focus == Some(FocusId::Search) {
                search.focused = true;
            }
            search.draw(batch, atlas, None);
            // LanguageSelectScreen's header search box (no message), added
            // before the list.
            crate::widgets::last_narratable(|n| n.order = -1);
            if search.value.is_empty() && !search.focused {
                batch.text_styled(
                    atlas,
                    &tr("gui.language.search").with_color(text::GRAY).italic(),
                    search.text_x(),
                    search.text_y(),
                    u32::MAX,
                    true,
                );
            }
            self.language_search = search;
        }
        if matches!(
            self.page(),
            Page::Video | Page::Controls | Page::KeyBinds | Page::ModBrowser
        ) {
            let mut search = self.settings_search.clone();
            if self.focus == Some(FocusId::Search) {
                search.focused = true;
            }
            search.draw(batch, atlas, None);
            let hint = match self.page() {
                Page::ModBrowser => "horizon.mods.browse.search",
                _ => "horizon.options.search",
            };
            // Horizon's search box sits in the header, before the list.
            crate::widgets::last_narratable(|n| {
                n.order = -1;
                if let crate::widgets::What::EditBox { message, .. } = &mut n.what {
                    *message = tr(hint).plain();
                }
            });
            if search.value.is_empty() && !search.focused {
                batch.text_styled(
                    atlas,
                    &tr(hint).with_color(text::GRAY).italic(),
                    search.text_x(),
                    search.text_y(),
                    u32::MAX,
                    true,
                );
            }
            self.settings_search = search;
        }
        if self.page() == Page::Skin && self.skin_page && self.profile_name.is_none() {
            let mut name = self.skin.name.clone();
            if name.y >= top && name.y + name.height <= bottom {
                // Red while what is typed is no name, which leaving the box refuses.
                name.text_color = if crate::offline_name::is_valid_choice(&name.value) {
                    rgba(224, 224, 224, 255)
                } else {
                    rgba(255, 85, 85, 255)
                };
                name.draw(batch, atlas, None);
            }
            self.skin.name = name;
        }
        if matches!(self.page(), Page::CustomButton(_)) {
            let mut field = self.custom_send.clone();
            if field.y >= top && field.y + field.height <= bottom {
                field.draw(batch, atlas, None);
            }
            self.custom_send = field;
        }
        if matches!(self.page(), Page::CustomDraw(_)) {
            self.painter.draw(batch, atlas);
        }
        // An image just kept, into its slot of the atlas: the button wears it at once.
        if let Some((id, pixels)) = self.painter.upload.take()
            && let Some(r) = atlas.sprites.get(&own_buttons::sprite(id))
        {
            batch.patches.push(crate::font::Patch {
                x: (r.u0 * atlas.width as f32).round() as u32,
                y: (r.v0 * atlas.height as f32).round() as u32,
                width: own_buttons::SIZE,
                height: own_buttons::SIZE,
                rgba: pixels,
            });
        }
        if let Some(lines) = tooltip {
            match tooltip_rect {
                Some(rect) if self.pad_driving => ui::tooltip_under(batch, atlas, &lines, rect),
                Some(rect) => ui::tooltip_menu(batch, atlas, &lines, cursor, rect),
                None => ui::tooltip(batch, atlas, &lines, cursor),
            }
        }
    }
    /// Mouse press/drag. Returns events for the game.
    pub fn click(
        &mut self,
        s: &mut Settings,
        atlas: &HudAtlas,
        x: f32,
        y: f32,
        size: (f32, f32),
        framebuffer: [i32; 2],
        drag: bool,
        right: bool,
    ) -> Vec<Event> {
        if self.resource_error.is_some() {
            if !drag
                && !right
                && ui::hovered((x, y), size.0 / 2.0 - 100.0, size.1 - 28.0, 200.0, 20.0)
            {
                self.resource_error = None;
                return vec![Event::ButtonClick];
            }
            return Vec::new();
        }
        if self.page() == Page::SplitSetup {
            use crate::split_ui::Click;
            if right {
                return Vec::new();
            }
            self.split_setup.on = s.extras.splitscreen;
            return match self.split_setup.click((x, y), size, drag) {
                Click::Nothing => Vec::new(),
                Click::Pressed => vec![Event::ButtonClick],
                Click::Switch => {
                    crate::extras::Item::Splitscreen.cycle(&mut s.extras, false);
                    vec![Event::ButtonClick, Event::Changed]
                }
                Click::Done => {
                    let mut events = vec![Event::ButtonClick];
                    events.extend(self.back(s));
                    events
                }
            };
        }
        if self.page() == Page::SplitProfiles {
            use crate::split_profiles::Click;
            if right || drag {
                return Vec::new();
            }
            return match self.split_profiles.click(atlas, (x, y), size) {
                Click::Nothing => Vec::new(),
                Click::Pressed => vec![Event::ButtonClick],
                Click::Done => {
                    let mut events = vec![Event::ButtonClick];
                    events.extend(self.back(s));
                    events
                }
            };
        }
        if self.page() == Page::Studio {
            if right || drag {
                return Vec::new();
            }
            use crate::studio_ui::StudioAction;
            return match self.studio.click((x, y), size) {
                StudioAction::None => Vec::new(),
                StudioAction::Refresh => vec![Event::ButtonClick],
                StudioAction::OpenReplay(path) => {
                    vec![Event::ButtonClick, Event::OpenStudioReplay(path)]
                }
                StudioAction::OpenFolder => vec![Event::ButtonClick, Event::OpenStudioFolder],
                StudioAction::Back => {
                    let mut events = vec![Event::ButtonClick];
                    events.extend(self.back(s));
                    events
                }
            };
        }
        if self.page() == Page::HdrCalibration
            && let Some(c) = &mut self.calibration
        {
            if right {
                return Vec::new();
            }
            let outcome = c.click(&atlas.font, (x, y), size, drag);
            return self.calibration_outcome(s, outcome);
        }
        if self.page() == Page::ControllerRadialName {
            return self.click_radial_name(s, atlas, (x, y), size, drag || right);
        }
        if self.shell().is_some() {
            // Waiting on a controller button: a click cancels the wait.
            if !drag && self.selected_pad_action.take().is_some() {
                return Vec::new();
            }
            // Waiting on a key: a mouse button binds, as on Key Binds.
            if self.selected_key.is_some() && !drag {
                let button = if right {
                    winit::event::MouseButton::Right
                } else {
                    winit::event::MouseButton::Left
                };
                return self.bind_mouse(s, button);
            }
            let shift = self.shift;
            self.framebuffer = framebuffer;
            self.shell_framebuffers();
            let outputs = self.shell_mut().map_or_else(Vec::new, |shell| {
                shell.click(s, (x, y), size, drag, right, shift)
            });
            return self.shell_events(s, outputs);
        }
        if self.page() == Page::Mods {
            let mut events = Vec::new();
            if drag {
                self.mods_page(s, size).drag(y as f64);
                return events;
            }
            if right {
                return events;
            }
            let m = crate::screen_draw::Measure(&atlas.font);
            let screen = self.mods_page(s, size);
            let before = screen.search_focused();
            let (action, sound) = screen.click(&m, x as f64, y as f64);
            if !before && screen.search_focused() {
                self.pack_focus = Instant::now();
            }
            if sound {
                events.push(Event::ButtonClick);
            }
            use crate::mods_screen::Action as Do;
            match action {
                Do::None => {}
                Do::OpenFolder => events.push(Event::OpenModFolder),
                Do::Back => events.extend(self.back(s)),
                Do::Config(i) => {
                    self.push(Page::NativeMod(i));
                    events.push(Event::NativeSettings(i));
                }
                Do::Link(url) => events.push(Event::OpenLink(url)),
                Do::Copy(text) => events.push(Event::CopyText(text)),
            }
            return events;
        }
        if self.page() == Page::PackOptions {
            let mut events = Vec::new();
            for action in self
                .pack_options
                .click((x, y), size, drag, right || self.shift)
            {
                match action {
                    crate::pack_options_ui::Action::ButtonClick => events.push(Event::ButtonClick),
                    crate::pack_options_ui::Action::SliderDrag => {}
                    crate::pack_options_ui::Action::Done => events.extend(self.back(s)),
                }
            }
            return events;
        }
        if self.page() == Page::Packs {
            let mut events = Vec::new();
            if drag {
                self.packs(atlas, size).drag(y as f64);
                return events;
            }
            if right {
                return events;
            }
            let m = crate::screen_draw::Measure(&atlas.font);
            let p = self.packs(atlas, size);
            if p.button_at(&m, x as f64, y as f64) {
                events.push(Event::ButtonClick);
            }
            let before = p.search_focused();
            let action = p.click(&m, x as f64, y as f64);
            if !before && p.search_focused() {
                self.pack_focus = Instant::now();
            }
            match action {
                crate::pack_screen::Action::OpenFolder => events.push(match &self.data_pack_dir {
                    Some(dir) => Event::OpenDataPackFolder(dir.clone()),
                    None => Event::OpenPackFolder,
                }),
                crate::pack_screen::Action::Done => events.extend(self.back(s)),
                crate::pack_screen::Action::BrowsePick(i) => {
                    events.push(Event::DownloadPack(i));
                }
                crate::pack_screen::Action::Catalog => {
                    events.push(Event::OpenCatalog(CatalogShelf::ResourcePacks));
                }
                crate::pack_screen::Action::Options(id) => {
                    let mut configs = horizon_polytone::pack::Configs::default();
                    if let Some(own) = self.pack_configs.get(&id) {
                        configs.merge(own);
                    }
                    self.pack_options = crate::pack_options_ui::PackOptionsUi::open(configs);
                    self.push(Page::PackOptions);
                }
                crate::pack_screen::Action::None => {}
            }
            return events;
        }
        let (widgets, top, bottom, content) = self.layout(s, size.0, size.1);
        self.size = size;
        self.framebuffer = framebuffer;
        let mut events = Vec::new();
        if !right && self.painting() && self.painter.press(x, y, drag) {
            return events;
        }
        if !drag
            && matches!(
                self.page(),
                Page::Video | Page::Controls | Page::KeyBinds | Page::ModBrowser
            )
        {
            let search = &mut self.settings_search;
            search.focused = ui::hovered((x, y), search.x, search.y, search.width, search.height);
            if search.focused {
                self.focus = Some(FocusId::Search);
                self.focus_from_keyboard = false;
                self.can_change_value = false;
                search.click(atlas, x, false, false);
                self.scroll = 0.0;
                events.push(Event::ButtonClick);
                return events;
            }
        }
        if !drag
            && !right
            && self.page() == Page::Skin
            && self.skin_page
            && self.profile_name.is_none()
        {
            let field = &mut self.skin.name;
            if ui::hovered((x, y), field.x, field.y, field.width, field.height)
                && y >= top
                && y < bottom
            {
                if !field.focused {
                    field.reset_blink();
                }
                field.focused = true;
                field.click(atlas, x, false, false);
                return events;
            }
            // A press anywhere else leaves the box.
            if field.focused {
                self.commit_name();
            }
        }
        if !drag && !right && matches!(self.page(), Page::CustomButton(_)) {
            let field = &mut self.custom_send;
            if ui::hovered((x, y), field.x, field.y, field.width, field.height)
                && y >= top
                && y < bottom
            {
                if !field.focused {
                    field.reset_blink();
                }
                field.focused = true;
                field.click(atlas, x, false, false);
                return events;
            }
            field.focused = false;
        }
        if !drag && self.page() == Page::Language {
            let search = &mut self.language_search;
            search.focused = ui::hovered((x, y), search.x, search.y, search.width, search.height);
            if search.focused {
                self.focus = Some(FocusId::Search);
                self.focus_from_keyboard = false;
                self.can_change_value = false;
                search.click(atlas, x, false, false);
                self.scroll = 0.0;
                return events;
            }
        }
        if drag {
            if let Some(previous_y) = self.scrollbar_drag_y {
                if let Some((_, thumb)) = ui::scrollbar_thumb(top, bottom, content, self.scroll) {
                    let max = content - (bottom - top);
                    self.scroll = if y < top {
                        0.0
                    } else if y > bottom {
                        max
                    } else {
                        self.scroll + (y - previous_y) * max / (bottom - top - thumb)
                    };
                    self.clamp_scroll(top, bottom, content);
                    self.scrollbar_drag_y = Some(y);
                } else {
                    self.scrollbar_drag_y = None;
                }
                return events;
            }
            if let Some(opt) = self.dragging
                && let Some(wd) = widgets
                    .iter()
                    .find(|wd| matches!(&wd.kind, WKind::Slider { opt: o, .. } if *o == opt))
            {
                let v = ui::slider_value(wd.x, wd.w, x);
                self.slider_value = Some((opt, v, Instant::now()));
                drag_slider(opt, s, v, &mut events);
            }
            return events;
        }
        // Key Binds: a mouse button pressed while waiting binds it.
        if self.selected_key.is_some() {
            let button = if right {
                winit::event::MouseButton::Right
            } else {
                winit::event::MouseButton::Left
            };
            return self.bind_mouse(s, button);
        }
        // AbstractWidget accepts only the primary button. Rebinding above
        // accepts other mouse buttons, but is not a widget activation.
        if right {
            return events;
        }
        if ui::scrollbar_thumb(top, bottom, content, self.scroll).is_some()
            && ui::hovered((x, y), self.scrollbar_x(size.0), top, 6.0, bottom - top)
        {
            self.scrollbar_drag_y = Some(y);
            return events;
        }
        for (wd_idx, wd) in widgets.into_iter().enumerate() {
            if !ui::hovered((x, y), wd.x, wd.y, wd.w, wd.h) {
                continue;
            }
            if wd.list && (y < top || y >= bottom) {
                continue;
            }
            // AbstractWidget.mouseClicked rejects inactive buttons before the
            // container can focus them.
            if matches!(wd.kind, WKind::Button { active: false, .. }) {
                return events;
            }
            // A click focuses the widget for its highlight only: vanilla's wheel
            // steps a keyboard-focused widget from anywhere, not a clicked one.
            self.focus = Some(focus_id(wd_idx, &wd));
            self.focus_from_keyboard = false;
            match wd.kind {
                WKind::Button {
                    active: true, act, ..
                } => {
                    self.can_change_value = false;
                    events.push(Event::ButtonClick);
                    match act {
                        Act::Push(p) => {
                            if p == Page::ShaderPacks {
                                // The pack list, and the picked pack's own settings with it.
                                events.push(Event::ListShaderPacks);
                                self.refresh_shader_profiles(s);
                                s.shader_options = None;
                                events.push(Event::OpenShaderOptions);
                            }
                            if p == Page::Mods || p == Page::NativeGame {
                                events.push(Event::ListMods);
                            }
                            if p == Page::Music {
                                events.push(Event::ListMusicTracks);
                            }
                            if p == Page::ModBrowser {
                                events.push(Event::BrowseMods);
                            }
                            self.push(p)
                        }
                        Act::Done => events.extend(self.back(s)),
                        Act::Cycle(opt) => {
                            events.extend(self.cycle_option(s, opt, self.shift, framebuffer));
                        }
                        Act::Extra(item) => {
                            if let Some(name) = item.key_name() {
                                self.selected_key = Some(name);
                            } else {
                                item.cycle(&mut s.extras, self.shift);
                                events.push(Event::Changed);
                            }
                        }
                        Act::PlayMusicTrack(event, track) => {
                            events.push(Event::PlayMusicTrack(event, track));
                        }
                        Act::MusicTrackOn(track, was_on) => {
                            s.extras
                                .music_off
                                .retain(|off| off.as_str() != track.as_str());
                            if was_on {
                                s.extras.music_off.push(track);
                            }
                            events.push(Event::Changed);
                        }
                        Act::Key(name) => self.selected_key = Some(name),
                        Act::KeyboardProfile => {
                            s.keypad.enabled ^= true;
                            self.selected_key = None;
                            self.scroll = 0.;
                            events.push(Event::Changed);
                        }
                        Act::ResetKey(name) => {
                            if name.starts_with("key.keypad.") {
                                s.keypad.bindings.remove(name);
                            } else {
                                s.keys.set(name, KeyBinds::default_of(name));
                            }
                            events.push(Event::Changed);
                        }
                        Act::ResetKeys => {
                            if s.keypad.enabled {
                                s.keypad.bindings.clear();
                            } else {
                                s.keys.reset_all();
                                s.mods.keys.clear();
                            }
                            events.push(Event::Changed);
                        }
                        Act::ToggleMod(i) => {
                            s.mods.toggle(i);
                            events.push(Event::Changed);
                            events.push(Event::ModToggled(i));
                        }
                        Act::ModSettings(i) => {
                            self.push(Page::NativeMod(i));
                            events.push(Event::NativeSettings(i));
                        }
                        Act::OpenModFolder => events.push(Event::OpenModFolder),
                        Act::ModKey(i, k) => {
                            self.selected_key = Some(MOD_KEY);
                            self.selected_mod_key = Some((i, k));
                            self.pending_mod_key = None;
                        }
                        Act::ResetModKey(i, k) => {
                            if let Some((id, mapping)) = mod_mapping(s, i, k) {
                                s.mods.reset_binding(&id, &mapping);
                                events.push(Event::Changed);
                            }
                        }
                        Act::PageRow(p, r) => events.extend(self.page_act(p, r, self.shift)),
                        Act::NativeSetting(i, j) => {
                            let setting = s
                                .mods
                                .installed
                                .get_mut(i)
                                .and_then(|m| m.native.as_mut())
                                .and_then(|page| page.settings.get_mut(j));
                            if let Some(setting) = setting {
                                setting.value.click();
                                events.push(Event::NativeSetting(i, j));
                            }
                        }
                        Act::NativeGame(act) => events.push(Event::NativeGame(act)),
                        Act::OpenCatalog(shelf) => events.push(Event::OpenCatalog(shelf)),
                        Act::ModProject(i) => {
                            if let Some(listing) = self.mod_browser.listings.get(i) {
                                self.mod_project = listing.id.clone();
                                self.push(Page::ModProject);
                            }
                        }
                        Act::CustomButtons => {
                            self.push(Page::CustomButtons);
                            events.push(Event::OpenCustomButtons);
                        }
                        Act::CustomAdd => {
                            self.custom_add(s);
                        }
                        Act::CustomEdit(id) => {
                            let line =
                                s.custom_buttons.iter().find(|b| b.id == id).map(|b| {
                                    match &b.action {
                                        crate::settings::CustomAction::Send(line) => line.as_str(),
                                        crate::settings::CustomAction::Key(_)
                                        | crate::settings::CustomAction::Input(_) => "",
                                    }
                                });
                            self.custom_send.set_value(line.unwrap_or_default());
                            self.custom_send.focused = false;
                            self.push(Page::CustomButton(id));
                        }
                        Act::CustomDelete(id) => {
                            own_buttons::remove_all(id);
                            s.custom_buttons.retain(|b| b.id != id);
                            events.push(Event::CustomButtons);
                        }
                        Act::CustomToggle(id) => {
                            if let Some(b) = s.custom_buttons.iter_mut().find(|b| b.id == id) {
                                b.toggle = !b.toggle;
                            }
                        }
                        Act::CustomKey(id, name) => {
                            if let Some(b) = s.custom_buttons.iter_mut().find(|b| b.id == id) {
                                b.action = crate::settings::CustomAction::Key(name.to_owned());
                            }
                            self.custom_send.set_value("");
                            self.custom_send.focused = false;
                            events.extend(self.back(s));
                        }
                        Act::CustomInput(id, name) => {
                            if let Some(b) = s.custom_buttons.iter_mut().find(|b| b.id == id) {
                                b.action = crate::settings::CustomAction::Input(name.to_owned());
                            }
                            self.custom_send.set_value("");
                            self.custom_send.focused = false;
                            events.extend(self.back(s));
                        }
                        Act::CustomModKey(id, i, k) => {
                            let key = s.mods.installed.get(i).and_then(|m| {
                                let mapping = m.native.as_ref()?.mappings.get(k)?;
                                let chosen =
                                    s.mods.keys.get(&m.id).and_then(|c| c.get(&mapping.id));
                                Some(chosen.unwrap_or(&mapping.key).clone())
                            });
                            if let Some(key) = key
                                && let Some(b) = s.custom_buttons.iter_mut().find(|b| b.id == id)
                            {
                                b.action = crate::settings::CustomAction::Input(key);
                            }
                            self.custom_send.set_value("");
                            self.custom_send.focused = false;
                            events.extend(self.back(s));
                        }
                        Act::CustomIcon(id, icon) => {
                            if let Some(b) = s.custom_buttons.iter_mut().find(|b| b.id == id) {
                                b.icon = icon.map(str::to_owned);
                                if b.image {
                                    b.image = false;
                                    own_buttons::remove(id);
                                }
                            }
                            events.extend(self.back(s));
                        }
                        Act::CustomDraw(id) => {
                            if id < own_buttons::SLOTS {
                                self.painter
                                    .open(atlas, s.custom_buttons.iter().find(|b| b.id == id));
                            } else {
                                self.painter.open_pressed(atlas, id);
                            }
                            self.push(Page::CustomDraw(id));
                        }
                        Act::CanvasClear => self.painter.clear(),
                        Act::CanvasPlate => self.painter.plate(atlas),
                        Act::CanvasImport(id) => events.push(Event::ImportButtonImage(id)),
                        Act::InstallMod => {
                            self.mod_browser.message = None;
                            events.push(Event::InstallMod(self.mod_project.clone()));
                        }
                        Act::OpenModPage => {
                            if let Some(l) = self.mod_browser.listing(&self.mod_project) {
                                events.push(Event::OpenModPage(l.page.clone()));
                            }
                        }
                        Act::PvpPreset => {
                            crate::pvp::apply_preset(s);
                            events.push(Event::Changed);
                        }
                        Act::TouchLayout => events.push(Event::TouchLayout),
                        Act::BedrockInventory => s.bedrock_inventory = !s.bedrock_inventory,
                        Act::HudEditor => events.push(Event::HudEditor),
                        Act::AccessibilityHelp => events.push(Event::AccessibilityHelp),
                        Act::Credits => events.push(Event::Credits),
                        Act::CopyAgentDetails => events.push(Event::CopyAgentDetails),
                        Act::NextSkin => {
                            let page = &mut self.skin;
                            page.choice.file =
                                crate::own_skin::next(&page.files, page.choice.file.as_deref());
                            page.warning = None;
                            page.actions.push(SkinAction::Choice(page.choice.clone()));
                        }
                        Act::SkinModel => {
                            let page = &mut self.skin;
                            page.choice.slim = !page.choice.slim;
                            page.actions.push(SkinAction::Choice(page.choice.clone()));
                        }
                        Act::PickSkin => self.skin.actions.push(SkinAction::PickFile),
                        Act::ElyBySkins => {
                            let page = &mut self.skin;
                            page.elyby = !page.elyby;
                            page.actions.push(SkinAction::ElyBy(page.elyby));
                        }
                        Act::OpenSkinFolder => self.skin.actions.push(SkinAction::OpenFolder),
                        Act::UploadSkin => {
                            let page = &mut self.skin;
                            page.warning = None;
                            page.actions.push(SkinAction::Upload(page.choice.clone()));
                        }
                        Act::ElySkinSite => self.skin.actions.push(SkinAction::ElySite),
                        Act::ToggleRecording => events.push(Event::ToggleRecording),
                        Act::DiagnosticsExport => events.push(Event::DiagnosticsExport),
                        Act::DiagnosticsOpen => events.push(Event::DiagnosticsOpen),
                        Act::DiagnosticsRecord => events.push(Event::DiagnosticsRecord),
                        Act::DiagnosticsGpu => events.push(Event::DiagnosticsGpu),
                        Act::PerfLab(i) => events.push(Event::PerfLab(i)),
                        Act::ExportRepro => events.push(Event::ExportRepro),
                        Act::None => {}
                    }
                }
                WKind::Slider { opt, .. } => {
                    self.can_change_value = true;
                    self.dragging = Some(opt);
                    let v = ui::slider_value(wd.x, wd.w, x);
                    self.slider_value = Some((opt, v, Instant::now()));
                    drag_slider(opt, s, v, &mut events);
                }
                WKind::LanguageRow { code, .. } => {
                    self.can_change_value = false;
                    if self.language_choice.as_deref() == Some(code.as_str()) {
                        events.extend(self.back(s));
                    } else {
                        self.language_choice = Some(code);
                    }
                }
                _ => {}
            }
            break;
        }
        events
    }
    /// Where a slider's handle sits: its own position while held (or until
    /// a delayed one applies), else the option's stepped value.
    fn slider_shown(&self, opt: Opt, value: f32) -> f32 {
        match self.slider_value {
            Some((o, v, at)) if o == opt && slider_unsnapped(o, self.dragging == Some(o), at) => v,
            _ => value,
        }
    }
    pub fn release(&mut self) -> Vec<Event> {
        self.scrollbar_drag_y = None;
        if let Some(p) = &mut self.pack_screen {
            p.release();
        }
        if let Some(p) = &mut self.mods_screen {
            p.release();
        }
        // AbstractSliderButton suppresses the press sound and plays once on
        // release, even if the pointer has left the widget or its value stayed.
        let slider = self.dragging.take().is_some();
        let shell_slider = self.shell_mut().and_then(|shell| shell.release());
        self.pack_options.release();
        let calibration_slider = self.calibration.as_mut().is_some_and(|c| c.release());
        let split_drop = self.split_setup.release();
        if slider || shell_slider.is_some() || calibration_slider {
            let mut events = vec![Event::ButtonClick, Event::Changed];
            // What the screen does once a drag ends: a shader pack's option is saved, and the
            // pack rebuilt, once per drag.
            let done = shell_slider.into_iter().flatten();
            events.extend(done.filter_map(|output| match output {
                shell::Output::Event(event) => Some(event),
                _ => None,
            }));
            events
        } else if split_drop {
            vec![Event::ButtonClick]
        } else {
            Vec::new()
        }
    }
    pub fn scroll(&mut self, s: &Settings, size: (f32, f32), amount: f32) {
        let (_, top, bottom, content) = self.layout(s, size.0, size.1);
        self.scroll -= amount * 18.0;
        self.clamp_scroll(top, bottom, content);
    }
    /// `CycleButton.cycleValue` on an option, then what the option changes:
    /// the preset's values, High Contrast's built-in pack.
    fn cycle_option(
        &mut self,
        s: &mut Settings,
        opt: Opt,
        back: bool,
        framebuffer: [i32; 2],
    ) -> Vec<Event> {
        let mut events = Vec::new();
        opt.cycle(s, back, framebuffer);
        if opt == Opt::ReducedDebugInfo {
            events.push(Event::DebugEntriesChanged);
        }
        if opt == Opt::Preset {
            apply_preset(s);
        }
        if opt == Opt::HighContrast {
            // Options.highContrast: the built-in pack follows the option.
            let on = s.resource_packs.iter().any(|id| id == "high_contrast");
            if on != s.high_contrast {
                s.resource_packs.retain(|id| id != "high_contrast");
                if s.high_contrast {
                    s.resource_packs.push("high_contrast".into());
                }
                // PackSelectionScreen reads the repository each time it
                // opens: the next one starts from this selection.
                self.pack_selection = s.resource_packs.clone();
                self.pack_screen = None;
                events.push(Event::Packs(s.resource_packs.clone()));
            }
        }
        events.push(Event::Changed);
        if opt == Opt::Narrator {
            events.push(Event::Narrator);
        }
        events
    }
    /// The mouse wheel at the cursor. A vanilla list keeps the wheel for itself:
    /// `AbstractContainerWidget` inherits `AbstractScrollArea.mouseScrolled`, which
    /// scrolls and never reaches the list's rows. A cycle button outside a list is
    /// the screen's own child, so it takes the step (`CycleButton.mouseScrolled`: up
    /// goes back). `AbstractSliderButton` does not override it, so a slider outside
    /// a list does nothing.
    ///
    /// **Horizon extra (diverges from Vanilla 26.3, off by default):** "Scroll wheel
    /// changes settings" lets a hovered list row take the step too, and a slider or
    /// cycle button focused by the keyboard (Tab, the arrows, a pad) step on the
    /// wheel wherever the cursor is. Focus left by a click does not count
    /// (`focus_from_keyboard`): honouring it toggled the clicked option on every
    /// later wheel event, from anywhere on the screen, while the list never scrolled.
    pub fn wheel(
        &mut self,
        s: &mut Settings,
        size: (f32, f32),
        cursor: (f32, f32),
        framebuffer: [i32; 2],
        amount: f32,
    ) -> Vec<Event> {
        self.size = size;
        self.framebuffer = framebuffer;
        if self.resource_error.is_some() {
            return Vec::new();
        }
        self.shell_framebuffers();
        if let Some(shell) = self.shell_mut() {
            let outputs = shell.wheel(s, size, cursor, amount);
            return self.shell_events(s, outputs);
        }
        if self.page() == Page::Studio {
            self.studio.wheel(amount);
            return Vec::new();
        }
        if self.page() == Page::PackOptions {
            self.pack_options.scroll(size, amount);
            return Vec::new();
        }
        let plain = self.resource_error.is_none()
            && !(self.pack_screen.is_some() && self.stack.last() == Some(&Page::Packs))
            && self.stack.last() != Some(&Page::Mods);
        if !plain {
            self.scroll_at(s, size, cursor, amount);
            return Vec::new();
        }
        let extra = s.extras.wheel_cycles_settings;
        // The extra: a keyboard-focused slider or cycle button intercepts the
        // wheel before hover detection (see doc-comment above). A click focuses
        // its widget for the highlight too, and the wheel must not act on that:
        // it would toggle the clicked option from anywhere on the screen and the
        // list would never scroll.
        if extra && amount != 0.0 && self.focus_from_keyboard {
            let (widgets, _, _, _) = self.layout(s, size.0, size.1);
            match &self.focus {
                Some(FocusId::Slider(opt)) => {
                    let opt = *opt;
                    let width = widgets
                        .iter()
                        .find(|wd| matches!(&wd.kind, WKind::Slider { opt: o, .. } if *o == opt))
                        .map_or(150.0, |wd| wd.w);
                    return self.step_slider_option(s, opt, amount > 0.0, width);
                }
                Some(FocusId::Cycle(opt)) => {
                    let opt = *opt;
                    let mut events = vec![Event::ButtonClick];
                    events.extend(self.cycle_option(s, opt, amount > 0.0, framebuffer));
                    return events;
                }
                _ => {}
            }
        }
        enum HoveredControl {
            Cycle(Opt),
            Extra(crate::extras::Item),
        }
        let (widgets, top, bottom, _) = self.layout(s, size.0, size.1);
        // A list's row takes the wheel only with the extra on.
        let takes_wheel = |wd: &Widget| {
            ui::hovered(cursor, wd.x, wd.y, wd.w, wd.h)
                && (!wd.list || (extra && cursor.1 >= top && cursor.1 < bottom))
        };
        let hovered = widgets.iter().find_map(|wd| match &wd.kind {
            WKind::Button {
                active: true,
                act: Act::Cycle(opt),
                ..
            } if takes_wheel(wd) => Some(HoveredControl::Cycle(*opt)),
            WKind::Button {
                active: true,
                act: Act::Extra(item),
                ..
            } if takes_wheel(wd) => Some(HoveredControl::Extra(*item)),
            _ => None,
        });
        match hovered {
            Some(HoveredControl::Cycle(opt)) if amount != 0.0 => {
                self.cycle_option(s, opt, amount > 0.0, framebuffer)
            }
            Some(HoveredControl::Extra(item)) if amount != 0.0 => {
                item.cycle(&mut s.extras, amount > 0.0);
                vec![Event::Changed]
            }
            Some(_) => Vec::new(),
            None => {
                self.scroll_at(s, size, cursor, amount);
                Vec::new()
            }
        }
    }
    /// The mouse wheel at the cursor.
    pub fn scroll_at(&mut self, s: &Settings, size: (f32, f32), cursor: (f32, f32), amount: f32) {
        if self.resource_error.is_some() {
            return;
        }
        if let Some(shell) = self.shell_mut() {
            shell.scroll(s, size, amount);
            return;
        }
        if self.page() == Page::Studio {
            self.studio.wheel(amount);
            return;
        }
        if self.page() == Page::Mods {
            if let Some(p) = &mut self.mods_screen {
                p.scroll(cursor.0 as f64, cursor.1 as f64, amount as f64);
            }
            return;
        }
        if self.page() == Page::PackOptions {
            self.pack_options.scroll(size, amount);
            return;
        }
        match &mut self.pack_screen {
            Some(p) if self.stack.last() == Some(&Page::Packs) => {
                p.scroll(cursor.0 as f64, cursor.1 as f64, amount as f64)
            }
            _ => self.scroll(s, size, amount),
        }
    }
    /// Whether `cursor` is on a button that a finger moving from it should scroll past rather than
    /// press (touch.rs, `prefer_scroll`): any active button or language row, and on Video Settings
    /// any row but a slider's. Sliders keep their sideways drag, and the pages with drags of their
    /// own (packs, the canvas, calibration, split screen) are left as they are.
    pub fn list_button_at(&mut self, s: &Settings, size: (f32, f32), cursor: (f32, f32)) -> bool {
        if self.resource_error.is_some() || self.painting() {
            return false;
        }
        // Upstream's settings shell lays these pages' rows out.
        if let Some(shell) = self.shell_mut() {
            return shell.pressable_at(s, size, cursor);
        }
        if matches!(
            self.page(),
            Page::Packs | Page::SplitSetup | Page::SplitProfiles | Page::HdrCalibration
        ) {
            return false;
        }
        let (widgets, top, bottom, _) = self.layout(s, size.0, size.1);
        widgets.iter().any(|wd| {
            matches!(
                wd.kind,
                WKind::Button { active: true, .. } | WKind::LanguageRow { .. }
            ) && ui::hovered(cursor, wd.x, wd.y, wd.w, wd.h)
                && !(wd.list && (cursor.1 < top || cursor.1 >= bottom))
        })
    }
    /// Whether `cursor` is on a slider of the settings shell's pages, which a finger takes only
    /// moving clearly sideways (touch.rs, `prefer_slider`).
    pub fn list_slider_at(&mut self, s: &Settings, size: (f32, f32), cursor: (f32, f32)) -> bool {
        if self.resource_error.is_some() || self.painting() {
            return false;
        }
        self.shell_mut()
            .is_some_and(|shell| shell.slider_at(s, size, cursor))
    }
    /// `VideoSettingsScreen.mouseScrolled`: with Ctrl held the wheel steps the GUI scale
    /// (auto counts as one past the maximum) and the list goes back to the top.
    pub fn video_wheel_gui_scale(
        &mut self,
        s: &mut Settings,
        framebuffer: [i32; 2],
        amount: f32,
    ) -> Vec<Event> {
        let max = i64::from(crate::window::calculate_scale(
            framebuffer,
            0,
            s.force_unicode,
        ));
        let old = if s.gui_scale == 0 {
            max + 1
        } else {
            i64::from(s.gui_scale)
        };
        let step = i64::from(amount > 0.0) - i64::from(amount < 0.0);
        let new = old + step;
        if new == 0 || new > max {
            return Vec::new();
        }
        s.gui_scale = new as u32;
        self.scroll = 0.0;
        vec![Event::Changed]
    }
    /// The Mods screen at this GUI size, made on first use from the installed mods.
    fn mods_page(&mut self, s: &Settings, size: (f32, f32)) -> &mut crate::mods_screen::ModsScreen {
        let (w, h) = (size.0 as i32, size.1 as i32);
        let in_world = self.in_world;
        let screen = self
            .mods_screen
            .get_or_insert_with(|| crate::mods_screen::ModsScreen::new(w, h, in_world, &s.mods));
        if screen.width != w || screen.height != h {
            screen.resize(w, h);
        }
        screen
    }
    /// The pack screen at this GUI size, made on first use.
    fn packs(&mut self, atlas: &HudAtlas, size: (f32, f32)) -> &mut crate::pack_screen::PackScreen {
        let m = crate::screen_draw::Measure(&atlas.font);
        let (w, h) = (size.0 as i32, size.1 as i32);
        match &mut self.pack_screen {
            Some(p) => {
                if p.width != w || p.height != h {
                    p.resize(&m, w, h);
                }
            }
            None => {
                self.pack_focus = Instant::now();
                let title = if self.data_pack_dir.is_some() {
                    "dataPack.title"
                } else {
                    "resourcePack.title"
                };
                let mut screen = crate::pack_screen::PackScreen::new(
                    &m,
                    crate::text::Text::translate(title, &[]),
                    w,
                    h,
                    self.in_world,
                    self.pack_infos.clone(),
                    &self.pack_selection,
                );
                screen.browsing_allowed = self.pack_browser;
                screen.changed = std::mem::take(&mut self.pack_reselected);
                self.pack_screen = Some(screen);
            }
        }
        // Here, not when it's made: the menu's screen goes on as the game's (`offer_catalog`).
        let catalog = self.catalog_buttons && self.data_pack_dir.is_none();
        // The cards of packs with Polytone options carry their button (data packs have none).
        let with_options = if self.data_pack_dir.is_none() {
            self.pack_configs
                .iter()
                .filter(|(_, c)| !c.entries().is_empty())
                .map(|(id, _)| id.clone())
                .collect()
        } else {
            Default::default()
        };
        let screen = self.pack_screen.as_mut().expect("made above");
        screen.catalog = catalog;
        screen.with_options = with_options;
        screen
    }
    /// Escape or Done: leave the current page, applying page results.
    /// After an input on HDR calibration: Save keeps what it landed on, Cancel what was
    /// set before; both go back to the graphics screen's HDR page.
    fn calibration_outcome(
        &mut self,
        s: &mut Settings,
        outcome: crate::hdr_calibration::Outcome,
    ) -> Vec<Event> {
        use crate::hdr_calibration::Outcome;
        match outcome {
            Outcome::Nothing | Outcome::Changed => Vec::new(),
            Outcome::Click => vec![Event::ButtonClick],
            Outcome::Save | Outcome::Cancel => {
                if let Some(c) = self.calibration.take()
                    && outcome == Outcome::Save
                {
                    c.save(s);
                }
                let mut events = vec![Event::ButtonClick];
                events.extend(self.back(s));
                events
            }
        }
    }
    /// HDR calibration's test picture, while its page is open, in a GUI of `size`.
    pub fn calibration_picture(&self, atlas: &HudAtlas, size: (f32, f32)) -> Option<Picture> {
        self.calibration
            .as_ref()
            .filter(|_| self.page() == Page::HdrCalibration)
            .map(|c| c.picture(&atlas.font, size))
    }
    /// A lifted finger no longer hovers a button. Text editing retains its focus.
    pub fn release_touch_focus(&mut self) {
        if !self.focus_from_keyboard && self.focus != Some(FocusId::Search) {
            self.focus = None;
        }
    }
    pub fn back(&mut self, s: &mut Settings) -> Vec<Event> {
        let mut events = Vec::new();
        // A pack's screen goes back to the one that opened it.
        if self.page() == Page::ShaderOptions
            && let Some(options) = &mut s.shader_options
            && options.path.pop().is_some()
        {
            self.scroll = 0.0;
            return events;
        }
        let page = self.stack.pop().unwrap_or(Page::Main);
        self.scroll = 0.0;
        self.scrollbar_drag_y = None;
        self.dragging = None;
        self.slider_value = None;
        self.selected_key = None;
        self.pending_mod_key = None;
        self.selected_pad_action = None;
        self.focus = None;
        self.focus_from_keyboard = false;
        self.settings_search.focused = false;
        self.language_search.focused = false;
        self.can_change_value = false;
        match page {
            Page::PackOptions => {
                let ui = std::mem::take(&mut self.pack_options);
                if ui.changed {
                    let entries = ui.configs.entries().keys().cloned().collect();
                    events.push(Event::PackOptions {
                        entries,
                        chosen: ui.configs.chosen(),
                    });
                }
            }
            Page::ShaderOptions | Page::ShaderPacks => {
                if s.shader_options.as_ref().is_some_and(|o| o.changed) {
                    events.push(Event::ShaderOptions);
                } else {
                    s.shader_options = None;
                }
            }
            Page::Language => {
                if let Some(code) = self.language_choice.take()
                    && code != s.language
                {
                    s.language = code.clone();
                    events.push(Event::Language(code));
                }
            }
            Page::Mods => self.mods_screen = None,
            // Browse mods may have installed some: the list is made again.
            Page::ModBrowser => self.mods_screen = None,
            Page::Packs => {
                // PackSelectionScreen.onClose: PackSelectionModel.commit.
                let data = self.data_pack_dir.take();
                if let Some(p) = self.pack_screen.take()
                    && p.changed
                {
                    let ids = p.commit();
                    if data.is_some() {
                        events.push(Event::DataPacks(ids));
                    } else {
                        // `PackSelectionModel.commit` already drops `server/…`
                        // ids; keep the same filter if the screen is rebuilt.
                        let ids = crate::packs::player_pack_ids(ids);
                        s.resource_packs = ids.clone();
                        self.pack_selection = ids.clone();
                        events.push(Event::Packs(ids));
                    }
                }
            }
            // Done or Escape with the name box focused keeps what it holds, as leaving it does.
            Page::Skin if self.profile_name.is_none() && self.skin.name.focused => {
                self.commit_name()
            }
            // Done keeps a canvas that changed: the file, the atlas's slot, and the button's look.
            Page::CustomDraw(id) => {
                if std::mem::take(&mut self.painter.dirty) {
                    let pixels = self.painter.pixels.clone();
                    self.painter.upload = Some((id, pixels.clone()));
                    if let Some(b) = s.custom_buttons.iter_mut().find(|b| b.id == id) {
                        b.image = true;
                        b.icon = None;
                    }
                    // Its pressed image: a canvas cleared takes it away.
                    if let Some(b) = s
                        .custom_buttons
                        .iter_mut()
                        .find(|b| own_buttons::pressed_slot(b.id) == id)
                    {
                        b.pressed = pixels.chunks(4).any(|p| p[3] != 0);
                    }
                    events.push(Event::CustomImage(id, pixels));
                }
            }
            // What the box holds is what it sends; one that does nothing goes.
            Page::CustomButton(id) => {
                let line = self.custom_send.value.trim().to_owned();
                self.custom_send.focused = false;
                if let Some(b) = s.custom_buttons.iter_mut().find(|b| b.id == id)
                    && (!line.is_empty()
                        || matches!(b.action, crate::settings::CustomAction::Send(_)))
                {
                    b.action = crate::settings::CustomAction::Send(line);
                }
                s.custom_buttons
                    .retain(|b| b.action != crate::settings::CustomAction::Send(String::new()));
                events.push(Event::CustomButtons);
            }
            _ => {}
        }
        events.push(Event::Changed);
        if self.stack.is_empty() {
            events.push(Event::Close);
        }
        if self.back_to_editor && matches!(page, Page::CustomButton(_)) {
            self.back_to_editor = false;
            events.push(Event::TouchLayout);
        }
        events
    }
    /// A new custom button's page. False with every slot taken.
    fn custom_add(&mut self, s: &mut Settings) -> bool {
        // A button's id is the slot of its image: the first one free.
        let free = (0..own_buttons::SLOTS).find(|id| !s.custom_buttons.iter().any(|b| b.id == *id));
        let Some(id) = free else {
            return false;
        };
        s.custom_buttons.push(crate::settings::CustomButton {
            id,
            action: crate::settings::CustomAction::Send(String::new()),
            icon: None,
            image: false,
            pressed: false,
            toggle: false,
        });
        self.custom_send.set_value("");
        self.custom_send.focused = false;
        self.push(Page::CustomButton(id));
        true
    }
    /// The touch editor's Add: a new button's page, over the page of them all; done with it, the
    /// editor again.
    pub fn add_custom_button(&mut self, s: &mut Settings) {
        if self.page() != Page::CustomButtons {
            self.push(Page::CustomButtons);
        }
        self.back_to_editor = self.custom_add(s);
    }
    /// Whether a custom button's canvas shows: a finger on the page drags, it never scrolls.
    pub fn painting(&self) -> bool {
        matches!(self.page(), Page::CustomDraw(_))
    }
    /// A picture for the canvas showing now.
    pub fn import_button_image(&mut self, width: u32, height: u32, rgba: &[u8]) {
        if self.painting() {
            self.painter.import(width, height, rgba);
        }
    }
    /// Typed text (the language search box).
    pub fn type_text(&mut self, atlas: &HudAtlas, text: &str) {
        if self.resource_error.is_some() {
            return;
        }
        if self.page() == Page::Packs
            && let Some(p) = &mut self.pack_screen
        {
            p.type_text(&crate::screen_draw::Measure(&atlas.font), text);
            return;
        }
        if self.page() == Page::Mods
            && let Some(p) = &mut self.mods_screen
        {
            p.type_text(&crate::screen_draw::Measure(&atlas.font), text);
            return;
        }
        if self.page() == Page::Language {
            self.language_search.insert_text(text);
            self.scroll = 0.0;
        } else if self.page() == Page::SplitProfiles {
            self.split_profiles.type_text(text);
        } else if self.page() == Page::ControllerRadialName {
            self.type_radial_name(text);
        } else if let Some(shell) = self.shell_mut() {
            if shell.typing() {
                shell.type_text(text);
            }
        } else if self.settings_search_active() {
            self.settings_search.insert_text(text);
            self.scroll = 0.0;
        } else if self.page() == Page::Skin && self.profile_name.is_none() && self.skin.name.focused
        {
            self.skin.name.insert_text(text);
        } else if matches!(self.page(), Page::CustomButton(_)) && self.custom_send.focused {
            self.custom_send.insert_text(text);
        }
    }
    /// Ctrl+A/C/V/X in the language or pack search box; false when neither
    /// is shown with the focus.
    pub fn shortcut(
        &mut self,
        atlas: &HudAtlas,
        shortcut: crate::clipboard::EditShortcut,
        clipboard: &mut dyn crate::clipboard::Clipboard,
    ) -> bool {
        if self.resource_error.is_some() || self.selected_key.is_some() {
            return false;
        }
        if self.page() == Page::Packs
            && let Some(p) = &mut self.pack_screen
        {
            let m = crate::screen_draw::Measure(&atlas.font);
            return p.shortcut(&m, shortcut, clipboard);
        }
        if self.page() == Page::Mods
            && let Some(p) = &mut self.mods_screen
        {
            let m = crate::screen_draw::Measure(&atlas.font);
            return p.shortcut(&m, shortcut, clipboard);
        }
        if self.page() == Page::Language {
            if self.language_search.shortcut(shortcut, clipboard) {
                self.scroll = 0.0;
            }
            return true;
        }
        if self.page() == Page::SplitProfiles {
            return self.split_profiles.shortcut(shortcut, clipboard);
        }
        if self.page() == Page::ControllerRadialName {
            return self.radial_name_shortcut(shortcut, clipboard);
        }
        if let Some(shell) = self.shell_mut() {
            return shell.shortcut(shortcut, clipboard);
        }
        if self.settings_search_active() {
            if self.settings_search.shortcut(shortcut, clipboard) {
                self.scroll = 0.0;
            }
            return true;
        }
        if self.page() == Page::Skin && self.profile_name.is_none() && self.skin.name.focused {
            self.skin.name.shortcut(shortcut, clipboard);
            return true;
        }
        if matches!(self.page(), Page::CustomButton(_)) && self.custom_send.focused {
            self.custom_send.shortcut(shortcut, clipboard);
            return true;
        }
        false
    }
    fn bind_key(&mut self, s: &mut Settings, value: String) -> Vec<Event> {
        self.pending_mod_key = None;
        let Some(name) = self.selected_key.take() else {
            return Vec::new();
        };
        if name.starts_with("key.keypad.") {
            s.keypad.set(name, value);
            return vec![Event::Changed];
        }
        if name == pages::PAGE_KEY {
            return self.bind_page_key(value);
        }
        if name == MOD_KEY {
            let (i, k) = self.selected_mod_key.take().unwrap_or_default();
            let Some((id, mapping)) = mod_mapping(s, i, k) else {
                return Vec::new();
            };
            // A modifier key bound on its own is the key, not a modifier held with itself.
            let own = match value.as_str() {
                "key.keyboard.left.shift" | "key.keyboard.right.shift" => crate::mod_list::SHIFT,
                "key.keyboard.left.control" | "key.keyboard.right.control" => {
                    crate::mod_list::CONTROL
                }
                "key.keyboard.left.alt" | "key.keyboard.right.alt" => crate::mod_list::ALT,
                _ => 0,
            };
            let modifiers = self.modifiers
                & (crate::mod_list::SHIFT | crate::mod_list::CONTROL | crate::mod_list::ALT)
                & !own;
            s.mods.set_binding(&id, &mapping, &value, modifiers);
            return vec![Event::Changed];
        }
        if !s.extras.set_key(name, value.clone()) {
            s.keys.set(name, &value);
        }
        vec![Event::Changed]
    }
    /// Mouse bindings use the same settings and persistence as keyboard bindings.
    pub fn bind_mouse(
        &mut self,
        s: &mut Settings,
        button: winit::event::MouseButton,
    ) -> Vec<Event> {
        self.bind_key(s, keybinds::mouse_name(button))
    }
    /// Completes a modifier-only mod binding when its key is released.
    pub fn key_released(&mut self, s: &mut Settings, key: winit::keyboard::KeyCode) -> Vec<Event> {
        if self.selected_key == Some(MOD_KEY) && self.pending_mod_key == Some(key) {
            return self.bind_key(
                s,
                keybinds::key_name(key)
                    .unwrap_or("key.keyboard.unknown")
                    .to_owned(),
            );
        }
        Vec::new()
    }
    /// Keys while the options are open. Returns events; `handled` false lets
    /// Escape fall through.
    pub fn key(
        &mut self,
        s: &mut Settings,
        atlas: &HudAtlas,
        key: winit::keyboard::KeyCode,
        text: Option<&str>,
    ) -> Vec<Event> {
        self.key_modifiers(s, atlas, key, text, false)
    }

    /// Keys while the options are open, including Shift modifier state.
    pub fn key_modifiers(
        &mut self,
        s: &mut Settings,
        atlas: &HudAtlas,
        key: winit::keyboard::KeyCode,
        text: Option<&str>,
        shift: bool,
    ) -> Vec<Event> {
        use winit::keyboard::KeyCode as K;
        if self.page() == Page::HdrCalibration
            && let Some(c) = &mut self.calibration
        {
            let outcome = c.key(key);
            return self.calibration_outcome(s, outcome);
        }
        if self.resource_error.is_some() {
            if matches!(key, K::Escape | K::Enter | K::NumpadEnter) {
                self.resource_error = None;
            }
            return Vec::new();
        }
        if self.page() == Page::Packs
            && let Some(p) = &mut self.pack_screen
        {
            let m = crate::screen_draw::Measure(&atlas.font);
            match key {
                // ConfirmScreen.keyPressed: Escape answers no.
                K::Escape if p.confirming() => {
                    p.cancel_confirm(&m);
                    return Vec::new();
                }
                K::Backspace => {
                    p.erase(&m, 1);
                    return Vec::new();
                }
                _ => {}
            }
        }
        if self.page() == Page::Mods
            && let Some(p) = &mut self.mods_screen
        {
            let m = crate::screen_draw::Measure(&atlas.font);
            match key {
                K::Backspace => {
                    p.erase(&m, 1);
                    return Vec::new();
                }
                K::ArrowUp => {
                    p.step(-1);
                    return Vec::new();
                }
                K::ArrowDown => {
                    p.step(1);
                    return Vec::new();
                }
                _ => {}
            }
        }
        if let Some(action) = self.selected_pad_action.take() {
            // As on Key Binds, Escape unbinds; other keys cancel.
            if key == K::Escape {
                s.controller.bind(action, None);
            }
            return vec![Event::Changed];
        }
        if self.selected_key.is_some() {
            // KeyBindsScreen.keyPressed: Escape unbinds.
            if self.selected_key == Some(MOD_KEY)
                && matches!(
                    key,
                    K::ShiftLeft
                        | K::ShiftRight
                        | K::ControlLeft
                        | K::ControlRight
                        | K::AltLeft
                        | K::AltRight
                )
            {
                self.pending_mod_key = Some(key);
                return Vec::new();
            }
            let value = if key == K::Escape {
                "key.keyboard.unknown".to_owned()
            } else {
                keybinds::key_name(key)
                    .unwrap_or("key.keyboard.unknown")
                    .to_owned()
            };
            return self.bind_key(s, value);
        }
        if self.page() == Page::SplitProfiles && self.split_profiles.key(key, text) {
            return Vec::new();
        }
        if self.page() == Page::ControllerRadialName {
            return self.radial_name_key(s, key, text);
        }

        // The size it was drawn at; before the first draw, one its keys can lay out at.
        let gui = if self.size.0 > 0.0 && self.size.1 > 0.0 {
            self.size
        } else {
            MIN_KEY_GUI
        };
        self.shell_framebuffers();
        if let Some(shell) = self.shell_mut() {
            let outputs = shell.key(s, key, text, shift, gui);
            return self.shell_events(s, outputs);
        }

        if key == K::Escape {
            return self.back(s);
        }

        // Search field text entry when search box is focused
        if self.focus == Some(FocusId::Search) {
            if key != K::Tab && key != K::ArrowDown {
                let search = if self.page() == Page::Language {
                    &mut self.language_search
                } else {
                    &mut self.settings_search
                };
                if !search.key(key, false, shift)
                    && let Some(t) = text
                {
                    let typed: String = t.chars().filter(|c| !c.is_control()).collect();
                    if !typed.is_empty() {
                        search.insert_text(&typed);
                    }
                }
                self.scroll = 0.0;
                return Vec::new();
            }
        } else if self.page() == Page::Language && self.focus.is_none() {
            if !self.language_search.key(key, false, shift)
                && let Some(t) = text
            {
                let typed: String = t.chars().filter(|c| !c.is_control()).collect();
                if !typed.is_empty() {
                    self.language_search.insert_text(&typed);
                }
            }
            self.scroll = 0.0;
        } else if self.settings_search_active() && self.focus.is_none() {
            if !self.settings_search.key(key, false, shift)
                && let Some(t) = text
            {
                let typed: String = t.chars().filter(|c| !c.is_control()).collect();
                if !typed.is_empty() {
                    self.settings_search.insert_text(&typed);
                }
            }
            self.scroll = 0.0;
        } else if self.page() == Page::Skin && self.profile_name.is_none() && self.skin.name.focused
        {
            // Enter (the on-screen keyboard's Done) keeps the name.
            if matches!(key, K::Enter | K::NumpadEnter) {
                self.commit_name();
                return Vec::new();
            }
            let field = &mut self.skin.name;
            if !field.key(key, false, false)
                && let Some(t) = text
            {
                let typed: String = t.chars().filter(|c| !c.is_control()).collect();
                if !typed.is_empty() {
                    field.insert_text(&typed);
                }
            }
            field.reset_blink();
        } else if matches!(self.page(), Page::CustomButton(_)) && self.custom_send.focused {
            // Enter (the on-screen keyboard's Done) leaves the box; Done on the page keeps it.
            let field = &mut self.custom_send;
            if matches!(key, K::Enter | K::NumpadEnter) {
                field.focused = false;
                return Vec::new();
            }
            if !field.key(key, false, false)
                && let Some(t) = text
            {
                let typed: String = t.chars().filter(|c| !c.is_control()).collect();
                if !typed.is_empty() {
                    field.insert_text(&typed);
                }
            }
            field.reset_blink();
        }

        // Gather focusables on the current page
        let (widgets, top, bottom, content) = self.layout(s, self.size.0, self.size.1);
        let has_search = (self.page() == Page::Language) || self.settings_search_active();
        let mut focusables: Vec<(FocusId, [f32; 4], bool)> = Vec::new();
        if has_search {
            let search = if self.page() == Page::Language {
                &self.language_search
            } else {
                &self.settings_search
            };
            focusables.push((
                FocusId::Search,
                [search.x, search.y, search.width, search.height],
                false,
            ));
        }
        for (i, wd) in widgets.iter().enumerate() {
            match wd.kind {
                WKind::Button { active: true, .. }
                | WKind::Slider { .. }
                | WKind::LanguageRow { .. } => {
                    focusables.push((focus_id(i, wd), [wd.x, wd.y, wd.w, wd.h], wd.list));
                }
                _ => {}
            }
        }
        if focusables.is_empty() {
            return Vec::new();
        }

        // 1. Tab / Shift+Tab navigation
        if key == K::Tab {
            let at = self
                .focus
                .as_ref()
                .and_then(|fid| focusables.iter().position(|(id, ..)| id == fid));
            let next_idx = match at {
                None => {
                    if shift {
                        focusables.len() - 1
                    } else {
                        0
                    }
                }
                Some(p) => {
                    if shift {
                        (p + focusables.len() - 1) % focusables.len()
                    } else {
                        (p + 1) % focusables.len()
                    }
                }
            };
            let (next_id, rect, in_list) = &focusables[next_idx];
            self.focus = Some(next_id.clone());
            self.focus_from_keyboard = true;
            self.can_change_value = matches!(next_id, FocusId::Slider(_));
            if *in_list {
                if rect[1] < top {
                    self.scroll -= (top - rect[1]) + 4.0;
                } else if rect[1] + rect[3] > bottom {
                    self.scroll += (rect[1] + rect[3] - bottom) + 4.0;
                }
                self.clamp_scroll(top, bottom, content);
            }
            return Vec::new();
        }

        // 2. Left / Right Arrow keys
        if matches!(key, K::ArrowLeft | K::ArrowRight) {
            let forward = key == K::ArrowRight;
            // If focused on a slider and in can_change_value mode: adjust the slider value!
            if let Some(FocusId::Slider(opt)) = self.focus
                && self.can_change_value
            {
                let width = widgets
                    .iter()
                    .find(|wd| matches!(wd.kind, WKind::Slider { opt: o, .. } if o == opt))
                    .map_or(150.0, |wd| wd.w);
                return self.step_slider_option(s, opt, forward, width);
            }
            // Otherwise, navigate horizontally using 2D focus stepping
            if let Some(fid) = &self.focus {
                if let Some(cur_pos) = focusables.iter().position(|(id, ..)| id == fid) {
                    let from = focusables[cur_pos].1;
                    let rects: Vec<[f32; 4]> = focusables.iter().map(|f| f.1).collect();
                    let dir = if forward {
                        gamepad::Button::DPadRight
                    } else {
                        gamepad::Button::DPadLeft
                    };
                    if let Some(next_idx) = gamepad::focus_step_rect(from, dir, &rects) {
                        let (next_id, rect, in_list) = &focusables[next_idx];
                        self.focus = Some(next_id.clone());
                        self.focus_from_keyboard = true;
                        self.can_change_value = false;
                        if *in_list {
                            if rect[1] < top {
                                self.scroll -= (top - rect[1]) + 4.0;
                            } else if rect[1] + rect[3] > bottom {
                                self.scroll += (rect[1] + rect[3] - bottom) + 4.0;
                            }
                            self.clamp_scroll(top, bottom, content);
                        }
                    }
                }
            } else {
                self.focus = Some(focusables[0].0.clone());
                self.focus_from_keyboard = true;
                self.can_change_value = false;
            }
            return Vec::new();
        }

        // 3. Up / Down Arrow keys
        if matches!(key, K::ArrowUp | K::ArrowDown) {
            let down = key == K::ArrowDown;
            if let Some(fid) = &self.focus {
                if let Some(cur_pos) = focusables.iter().position(|(id, ..)| id == fid) {
                    let from = focusables[cur_pos].1;
                    let rects: Vec<[f32; 4]> = focusables.iter().map(|f| f.1).collect();
                    let dir = if down {
                        gamepad::Button::DPadDown
                    } else {
                        gamepad::Button::DPadUp
                    };
                    let step = gamepad::focus_step_strict(from, dir, &rects)
                        .or_else(|| gamepad::focus_step_vague(from, dir, &rects));
                    if let Some(next_idx) = step {
                        let (next_id, rect, in_list) = &focusables[next_idx];
                        self.focus = Some(next_id.clone());
                        self.focus_from_keyboard = true;
                        self.can_change_value = false;
                        if *in_list {
                            if rect[1] < top {
                                self.scroll -= (top - rect[1]) + 4.0;
                            } else if rect[1] + rect[3] > bottom {
                                self.scroll += (rect[1] + rect[3] - bottom) + 4.0;
                            }
                            self.clamp_scroll(top, bottom, content);
                        }
                    } else {
                        // Off the edge of visible rows: scroll the list
                        self.scroll += if down { 18.0 } else { -18.0 };
                        self.clamp_scroll(top, bottom, content);
                    }
                }
            } else {
                self.focus = Some(focusables[0].0.clone());
                self.focus_from_keyboard = true;
                self.can_change_value = false;
            }
            return Vec::new();
        }

        // 4. Enter / Space selection keys
        if matches!(key, K::Enter | K::NumpadEnter | K::Space) {
            match &self.focus {
                Some(FocusId::Slider(_)) => {
                    self.can_change_value = !self.can_change_value;
                    return Vec::new();
                }
                Some(FocusId::Cycle(opt)) => {
                    let mut events = vec![Event::ButtonClick];
                    events.extend(self.cycle_option(s, *opt, shift, self.framebuffer));
                    return events;
                }
                Some(FocusId::Act(act)) => {
                    let mut events = vec![Event::ButtonClick];
                    let act = act.clone();
                    match act {
                        Act::Push(p) => {
                            self.focus = None;
                            self.focus_from_keyboard = false;
                            self.can_change_value = false;
                            if p == Page::ShaderPacks {
                                // The pack list, and the picked pack's own settings with it.
                                events.push(Event::ListShaderPacks);
                                s.shader_options = None;
                                events.push(Event::OpenShaderOptions);
                            }
                            if p == Page::Mods || p == Page::NativeGame {
                                events.push(Event::ListMods);
                            }
                            if p == Page::Music {
                                events.push(Event::ListMusicTracks);
                            }
                            if p == Page::ModBrowser {
                                events.push(Event::BrowseMods);
                            }
                            self.push(p);
                        }
                        Act::Done => {
                            self.focus = None;
                            self.focus_from_keyboard = false;
                            self.can_change_value = false;
                            events.extend(self.back(s));
                        }
                        Act::Cycle(opt) => {
                            events.extend(self.cycle_option(s, opt, shift, self.framebuffer));
                        }
                        Act::Extra(item) => {
                            if let Some(name) = item.key_name() {
                                self.selected_key = Some(name);
                            } else {
                                item.cycle(&mut s.extras, shift);
                                events.push(Event::Changed);
                            }
                        }
                        Act::PlayMusicTrack(event, track) => {
                            events.push(Event::PlayMusicTrack(event, track));
                        }
                        Act::MusicTrackOn(track, was_on) => {
                            s.extras
                                .music_off
                                .retain(|off| off.as_str() != track.as_str());
                            if was_on {
                                s.extras.music_off.push(track);
                            }
                            events.push(Event::Changed);
                        }
                        Act::Key(name) => self.selected_key = Some(name),
                        Act::KeyboardProfile => {
                            s.keypad.enabled ^= true;
                            self.selected_key = None;
                            self.scroll = 0.;
                            events.push(Event::Changed);
                        }
                        Act::ResetKey(name) => {
                            if name.starts_with("key.keypad.") {
                                s.keypad.bindings.remove(name);
                            } else {
                                s.keys.set(name, KeyBinds::default_of(name));
                            }
                            events.push(Event::Changed);
                        }
                        Act::ResetKeys => {
                            if s.keypad.enabled {
                                s.keypad.bindings.clear();
                            } else {
                                s.keys.reset_all();
                                s.mods.keys.clear();
                            }
                            events.push(Event::Changed);
                        }
                        Act::ToggleMod(i) => {
                            s.mods.toggle(i);
                            events.push(Event::Changed);
                            events.push(Event::ModToggled(i));
                        }
                        Act::ModSettings(i) => {
                            self.push(Page::NativeMod(i));
                            events.push(Event::NativeSettings(i));
                        }
                        Act::OpenModFolder => events.push(Event::OpenModFolder),
                        Act::ModKey(i, k) => {
                            self.selected_key = Some(MOD_KEY);
                            self.selected_mod_key = Some((i, k));
                            self.pending_mod_key = None;
                        }
                        Act::ResetModKey(i, k) => {
                            if let Some((id, mapping)) = mod_mapping(s, i, k) {
                                s.mods.reset_binding(&id, &mapping);
                                events.push(Event::Changed);
                            }
                        }
                        Act::PageRow(p, r) => events.extend(self.page_act(p, r, shift)),
                        Act::NativeSetting(i, j) => {
                            let setting = s
                                .mods
                                .installed
                                .get_mut(i)
                                .and_then(|m| m.native.as_mut())
                                .and_then(|page| page.settings.get_mut(j));
                            if let Some(setting) = setting {
                                setting.value.click();
                                events.push(Event::NativeSetting(i, j));
                            }
                        }
                        Act::NativeGame(act) => events.push(Event::NativeGame(act)),
                        Act::OpenCatalog(shelf) => events.push(Event::OpenCatalog(shelf)),
                        Act::ModProject(i) => {
                            if let Some(listing) = self.mod_browser.listings.get(i) {
                                self.mod_project = listing.id.clone();
                                self.push(Page::ModProject);
                            }
                        }
                        Act::InstallMod => {
                            self.mod_browser.message = None;
                            events.push(Event::InstallMod(self.mod_project.clone()));
                        }
                        Act::OpenModPage => {
                            if let Some(l) = self.mod_browser.listing(&self.mod_project) {
                                events.push(Event::OpenModPage(l.page.clone()));
                            }
                        }
                        Act::PvpPreset => {
                            crate::pvp::apply_preset(s);
                            events.push(Event::Changed);
                        }
                        Act::HudEditor => events.push(Event::HudEditor),
                        Act::AccessibilityHelp => events.push(Event::AccessibilityHelp),
                        Act::Credits => events.push(Event::Credits),
                        Act::CopyAgentDetails => events.push(Event::CopyAgentDetails),
                        Act::ToggleRecording => events.push(Event::ToggleRecording),
                        Act::DiagnosticsExport => events.push(Event::DiagnosticsExport),
                        Act::DiagnosticsOpen => events.push(Event::DiagnosticsOpen),
                        Act::DiagnosticsRecord => events.push(Event::DiagnosticsRecord),
                        Act::DiagnosticsGpu => events.push(Event::DiagnosticsGpu),
                        Act::PerfLab(i) => events.push(Event::PerfLab(i)),
                        Act::ExportRepro => events.push(Event::ExportRepro),
                        // Starling's pages answer taps only.
                        Act::CustomButtons
                        | Act::CustomAdd
                        | Act::CustomEdit(_)
                        | Act::CustomDelete(_)
                        | Act::CustomKey(..)
                        | Act::CustomInput(..)
                        | Act::CustomModKey(..)
                        | Act::CustomIcon(..)
                        | Act::CustomDraw(_)
                        | Act::CustomToggle(_)
                        | Act::CanvasClear
                        | Act::CanvasPlate
                        | Act::CanvasImport(_)
                        | Act::TouchLayout
                        | Act::BedrockInventory
                        | Act::NextSkin
                        | Act::SkinModel
                        | Act::PickSkin
                        | Act::OpenSkinFolder
                        | Act::ElyBySkins
                        | Act::UploadSkin
                        | Act::ElySkinSite => {}
                        Act::None => {}
                    }
                    return events;
                }
                Some(FocusId::Lang(code)) => {
                    // `LanguageSelectionList.Entry.keyPressed`: the selection
                    // key (`isSelection`: Enter, Space, keypad Enter) selects
                    // the row and finishes the screen at once (`onDone`), so
                    // the language applies on that one press.
                    let mut events = vec![Event::ButtonClick];
                    self.language_choice = Some(code.clone());
                    events.extend(self.back(s));
                    return events;
                }
                _ => {}
            }
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extras_ui::ExtraRow;
    use crate::gfx::EffectiveOutput;

    /// A renderer whose destination offers nothing and whose display reports nothing.
    const NONE: EffectiveOutput = EffectiveOutput::NONE;

    /// The window allows the input method only while `typing` says a search
    /// box has the focus (`Game::text_field_focused`), so a page without one
    /// must not claim the keys: the movement and menu keys keep working there.
    #[test]
    fn only_a_focused_search_box_takes_text() {
        let mut ui = OptionsUi::default();
        assert!(!ui.typing(), "the main page has no text box");
        ui.show(Page::Language);
        assert!(ui.typing(), "the language list's search takes the keys");
        ui.show(Page::Video);
        assert!(!ui.typing(), "the settings search is not focused");
        ui.settings_search.focused = true;
        assert!(ui.typing(), "the settings search, once focused, does");
        ui.settings_search.focused = false;
        ui.show(Page::Packs);
        assert!(!ui.typing(), "the pack screen's search is not open yet");
    }

    #[test]
    fn accessibility_sliders_and_toggles_survive_options_round_trip() {
        let mut s = Settings::default();
        for opt in [
            Opt::FovEffects,
            Opt::DarknessPulsing,
            Opt::DamageTilt,
            Opt::PanoramaSpeed,
        ] {
            for position in [0.0, 0.5, 1.0] {
                opt.set_slider(&mut s, position);
                let loaded = Settings::parse(&s.serialize());
                assert_eq!(opt.slider_position(&loaded), Some(position));
                let expected = if matches!(opt, Opt::FovEffects | Opt::DarknessPulsing) {
                    f64::from(position).powi(2)
                } else {
                    f64::from(position)
                };
                assert_eq!(loaded.file.double(opt.file_key().unwrap()), expected);
            }
        }
        assert!(Opt::MonochromeLogo.available());
        for opt in [
            Opt::TextBackground,
            Opt::HideLightningFlashes,
            Opt::HideSplashTexts,
            Opt::MonochromeLogo,
        ] {
            let key = opt.file_key().unwrap();
            let before = s.file.bool(key);
            opt.cycle(&mut s, false, [854, 480]);
            assert_eq!(Settings::parse(&s.serialize()).file.bool(key), !before);
        }
        Opt::TextBackground.cycle(&mut s, false, [854, 480]);
        let chat_only = s.text_background(0.0);
        Opt::TextBackground.cycle(&mut s, false, [854, 480]);
        assert_ne!(s.text_background(0.0), chat_only);
    }

    #[test]
    fn mouse_settings_match_java_26_3_option_order() {
        let mut ui = OptionsUi::default();
        ui.show(Page::Mouse);
        let mut options = Vec::new();
        for row in ui.list_rows(&Settings::default()) {
            if let Row::Pair(a, b) = row {
                options.push(a);
                if let Some(b) = b {
                    options.push(b);
                }
            }
        }
        assert_eq!(
            options,
            vec![
                Opt::Sensitivity,
                Opt::WheelSensitivity,
                Opt::DiscreteScroll,
                Opt::InvertX,
                Opt::InvertY,
                Opt::AllowCursorChanges,
            ]
        );
    }

    #[test]
    fn mouse_settings_round_trip_with_java_defaults() {
        let mut settings = Settings::default();
        assert!(!settings.invert_x);
        assert!(settings.allow_cursor_changes);
        Opt::InvertX.cycle(&mut settings, false, [854, 480]);
        Opt::AllowCursorChanges.cycle(&mut settings, false, [854, 480]);
        let loaded = Settings::parse(&settings.serialize());
        assert!(loaded.invert_x);
        assert!(!loaded.allow_cursor_changes);
        assert!(!loaded.file.bool("allowCursorChanges"));
    }

    #[test]
    fn accessibility_has_all_options_and_disabled_placeholders() {
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Accessibility);
        let (widgets, _, _, _) = ui.layout(&s, 480.0, 1200.0);
        let options: Vec<_> = widgets
            .iter()
            .filter_map(|w| match w.kind {
                WKind::Button {
                    act: Act::Cycle(opt),
                    ..
                }
                | WKind::Slider { opt, .. } => Some(opt),
                _ => None,
            })
            .collect();
        assert_eq!(options, ACCESSIBILITY_OPTIONS);
        for &opt in ACCESSIBILITY_OPTIONS {
            let widget = widgets
                .iter()
                .find(|w| match w.kind {
                    WKind::Button {
                        act: Act::Cycle(o), ..
                    }
                    | WKind::Slider { opt: o, .. } => o == opt,
                    _ => false,
                })
                .expect("every accessibility option has a control");
            if !opt.available() {
                assert!(matches!(widget.kind, WKind::Button { active: false, .. }));
                let before = s.serialize();
                opt.cycle(&mut s, false, [854, 480]);
                opt.set_slider(&mut s, 0.0);
                assert_eq!(s.serialize(), before);
            }
        }
        assert!(widgets.iter().any(|w| matches!(
            w.kind,
            WKind::Button {
                act: Act::Push(Page::Controls),
                active: true,
                ..
            }
        )));
        // `AccessibilityOptionsScreen.addFooter`: the "Accessibility Help"
        // link button sits beside Done.
        assert!(widgets.iter().any(|w| matches!(
            w.kind,
            WKind::Button {
                act: Act::AccessibilityHelp,
                active: true,
                ..
            }
        )));
    }

    #[test]
    fn the_narrator_option_cycles_its_four_statuses_and_announces_them() {
        use horizon_narrator::Status;
        let mut s = Settings::default();
        for want in [Status::All, Status::Chat, Status::System, Status::Off] {
            Opt::Narrator.cycle(&mut s, false, [854, 480]);
            assert_eq!(s.narrator(), want);
        }
        Opt::Narrator.cycle(&mut s, true, [854, 480]);
        assert_eq!(s.narrator(), Status::System);
        let mut ui = OptionsUi::default();
        assert_eq!(
            ui.cycle_option(&mut s, Opt::Narrator, false, [854, 480]),
            [Event::Changed, Event::Narrator]
        );
        assert_eq!(
            ui.cycle_option(&mut s, Opt::NarratorHotkey, false, [854, 480]),
            [Event::Changed]
        );
        assert!(!s.file.bool("narratorHotkey"));
    }

    #[test]
    fn without_speech_the_narrator_button_is_inactive_and_not_available() {
        // Tests never open the narrator: text2speech's Narrator.EMPTY.
        let s = Settings::default();
        let widget = option_widget(&s, Opt::Narrator, 0.0, 0.0, 150.0);
        assert!(matches!(widget.kind, WKind::Button { active: false, .. }));
        assert_eq!(
            Opt::Narrator.value_text(&s).plain(),
            tr("options.narrator.notavailable").plain()
        );
        let hotkey = option_widget(&s, Opt::NarratorHotkey, 0.0, 0.0, 150.0);
        assert!(matches!(hotkey.kind, WKind::Button { active: true, .. }));
    }

    #[test]
    fn chat_settings_show_the_narrator_where_vanilla_does() {
        let mut ui = OptionsUi::default();
        ui.show(Page::Chat);
        let options: Vec<Opt> = ui
            .list_rows(&Settings::default())
            .into_iter()
            .flat_map(|row| match row {
                Row::Big(opt) => vec![opt],
                Row::Pair(a, b) => std::iter::once(a).chain(b).collect(),
                _ => Vec::new(),
            })
            .collect();
        let at = options.iter().position(|o| *o == Opt::Narrator).unwrap();
        assert_eq!(options[at - 1], Opt::ChatHeightUnfocused);
        assert_eq!(options[at + 1], Opt::AutoSuggestions);
    }

    #[test]
    fn controls_match_java_26_3_option_order() {
        let mut ui = OptionsUi::default();
        ui.show(Page::Controls);
        let options =
            ui.list_rows(&Settings::default())
                .into_iter()
                .fold(Vec::new(), |mut options, row| {
                    match row {
                        Row::Big(opt) => options.push(opt),
                        Row::Pair(a, b) => {
                            options.push(a);
                            options.extend(b);
                        }
                        _ => {}
                    }
                    options
                });
        assert_eq!(options, vanilla_control_options());
    }

    #[test]
    fn sprint_window_and_new_control_switches_round_trip() {
        let mut settings = Settings::default();
        set_slider(Opt::SprintWindow, &mut settings, 0.5);
        settings.operator_items_tab = true;
        settings.quit_shortcuts = false;
        settings.ctrl_click_emulates_right_click = true;
        settings.toggle_attack = true;
        settings.toggle_use = true;
        let loaded = Settings::parse(&settings.serialize());
        assert_eq!(loaded.sprint_window, 5);
        assert!(loaded.operator_items_tab);
        assert!(!loaded.quit_shortcuts);
        assert!(loaded.ctrl_click_emulates_right_click);
        assert!(loaded.toggle_attack && loaded.toggle_use);
    }

    #[test]
    fn quit_shortcuts_option_can_be_switched() {
        assert!(Opt::QuitShortcuts.available());
        let s = Settings::default();
        assert_eq!(Opt::QuitShortcuts.caption(), "options.quitShortcuts");
        assert_eq!(
            Opt::QuitShortcuts.tooltip_key(&s),
            "options.quitShortcuts.tooltip"
        );
    }

    /// The gating is for genuinely unread keys only: every option a system
    /// already consumes must stay clickable.
    #[test]
    fn read_options_are_not_greyed_out() {
        for opt in [
            Opt::ChunkFade,
            Opt::ImprovedTransparency,
            Opt::ChatDelay,
            Opt::SaveChatDrafts,
            Opt::MusicToast,
            Opt::CtrlClickEmulatesRightClick,
        ] {
            assert!(opt.available(), "{opt:?} is read and must be clickable");
        }
        for opt in [
            Opt::HideMatchedNames,
            Opt::OnlySecureChat,
            Opt::RealmsNotifications,
            Opt::InGameNotification,
            Opt::SharePresence,
            Opt::DirectionalAudio,
            Opt::ChunkBuilder,
        ] {
            assert!(!opt.available(), "{opt:?} still has no reader");
        }
    }

    #[test]
    fn operator_items_tab_option_can_be_switched() {
        // The creative screen reads it (CreativeModeInventoryScreen.hasPermissions).
        assert!(Opt::OperatorItemsTab.available());
        let mut s = Settings::default();
        assert!(!s.operator_items_tab);
        Opt::OperatorItemsTab.cycle(&mut s, false, [854, 480]);
        assert!(s.operator_items_tab);
    }

    #[test]
    fn render_distance_slider_round_trips_through_extended_options_range() {
        let mut s = Settings::default();
        for (position, distance) in [(0.0, 2), (0.5, 65), (1.0, 128)] {
            set_slider(Opt::RenderDistance, &mut s, position);
            assert_eq!(s.render_distance, distance);
            let reloaded = Settings::parse(&s.serialize());
            assert_eq!(reloaded.render_distance, distance);
            assert_eq!(
                Opt::RenderDistance.slider_position(&reloaded),
                Some(position)
            );
        }
        let mut file = crate::options_file::OptionsFile::default();
        file.set_text("renderDistance", "128").unwrap();
        assert!(file.set_text("renderDistance", "129").is_err());
        assert!(file.set_text("renderDistance", "1").is_err());
        assert_eq!(file.int("renderDistance"), 128);
    }
    pub(super) fn atlas() -> HudAtlas {
        HudAtlas::build(&horizon_assets::resource_source::ResourceSource::directory(
            (std::path::Path::new("/nonexistent/horizon-test-assets")).to_owned(),
        ))
        .unwrap()
    }

    /// Picking another pack keeps the last one's changed options before the next one's are
    /// read, and leaving the shader screen applies them.
    #[test]
    fn the_shader_screen_saves_changed_options_before_reading_the_next_packs() {
        use crate::shader_pack_list::{Entry, Kind};
        let entry = |file: &str| Entry {
            file: file.into(),
            key: file.into(),
            title: file.into(),
            kind: Kind::Classic,
            options: Vec::new(),
            values: Vec::new(),
            pages: Vec::new(),
        };
        let mut s = Settings {
            shader_packs: vec![entry("A.zip"), entry("B.zip")],
            ..Settings::default()
        };
        s.horizon.shader_pack = "A.zip".into();
        let mut options = crate::shader_options::ShaderOptions::new(
            "A.zip".into(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
        );
        options.changed = true;
        s.shader_options = Some(options);
        let mut ui = OptionsUi::default();
        ui.open(false);
        ui.push(Page::ShaderPacks);
        let size = (854.0, 480.0);
        let (x, y) = ui
            .shader
            .point_at(&s, size, crate::shader_ui::Item::Pack(1));
        let events = ui.click(&mut s, &atlas(), x, y, size, [1708, 960], false, false);
        assert!(matches!(
            events[..],
            [
                Event::ButtonClick,
                Event::ShaderOptions,
                Event::ShaderPack,
                Event::OpenShaderOptions
            ]
        ));
        assert_eq!(s.horizon.shader_pack, "B.zip");
        let events = ui.back(&mut s);
        assert!(matches!(events[0], Event::ShaderOptions));
    }

    #[test]
    fn resource_reload_error_blocks_options_until_dismissed() {
        use winit::keyboard::KeyCode;
        let atlas = atlas();
        let mut settings = Settings::default();
        let mut ui = OptionsUi {
            resource_error: Some("broken pack".into()),
            ..Default::default()
        };
        let stack = ui.stack.clone();
        assert!(
            ui.click(
                &mut settings,
                &atlas,
                10.0,
                10.0,
                (480.0, 270.0),
                [960, 540],
                false,
                false
            )
            .is_empty()
        );
        assert!(ui.resource_error.is_some());
        assert!(
            ui.key(&mut settings, &atlas, KeyCode::KeyW, None)
                .is_empty()
        );
        assert!(ui.resource_error.is_some());
        ui.key(&mut settings, &atlas, KeyCode::Escape, None);
        assert!(ui.resource_error.is_none());
        assert_eq!(ui.stack, stack);
        ui.resource_error = Some("broken pack".into());
        assert_eq!(
            ui.click(
                &mut settings,
                &atlas,
                240.0,
                250.0,
                (480.0, 270.0),
                [960, 540],
                false,
                false
            ),
            vec![Event::ButtonClick]
        );
        assert!(ui.resource_error.is_none());
    }

    /// The Extras page joins caption and value with the language's `options.generic_value`
    /// (26.3's templates), not an English ": " (#6072).
    #[test]
    fn extras_captions_use_the_languages_separator() {
        struct Vanilla;
        impl lang::ResourceSource for Vanilla {
            fn read(&self, path: &str) -> Option<Vec<u8>> {
                let generic = match path {
                    "assets/minecraft/lang/fr_fr.json" => "%s : %s",
                    "assets/minecraft/lang/zh_cn.json" => "%s：%s",
                    _ => return None,
                };
                serde_json::to_vec(&serde_json::json!({ "options.generic_value": generic })).ok()
            }
        }
        let s = Settings::default();
        let ui = OptionsUi::default();
        for (code, shown) in [("fr_fr", "Sans IA : Non"), ("zh_cn", "关闭 AI：关")] {
            let (label, _) = lang::scoped(lang::Language::load(&[&Vanilla], code), || {
                match ui.extra_button(&s, crate::extras::Item::NoAi).kind {
                    WKind::Button { label, .. } => label.plain(),
                    _ => String::new(),
                }
            });
            assert_eq!(label, shown, "{code}");
        }
    }

    /// The texts of an options page, as laid out in `code`'s language, and the keys it lacked.
    fn page_texts(code: &str, pages: &[Page]) -> (Vec<String>, Vec<String>) {
        let s = Settings::default();
        lang::scoped(lang::Language::load(&[], code), || {
            let mut texts = Vec::new();
            for page in pages {
                let mut ui = OptionsUi::default();
                ui.show(*page);
                for w in ui.layout(&s, 480.0, 4000.0).0 {
                    match w.kind {
                        WKind::Button { label: t, .. }
                        | WKind::Slider { label: t, .. }
                        | WKind::Text { text: t, .. } => texts.push(t.plain()),
                        _ => {}
                    }
                }
                if ui.controller_open() {
                    texts.extend(ui.controller.row_names(&s));
                }
                if ui.page() == Page::Extras {
                    texts.extend(ui.extras.row_labels(&s));
                }
            }
            texts
        })
    }

    /// Horizon's own options pages said "Splitscreen...", "Hide block-break particles" and
    /// "Hotbar: Previous Slot" in every language (#1927).
    #[test]
    fn horizon_option_pages_follow_the_language() {
        let (texts, missed) = page_texts("de_de", &[Page::Horizon, Page::Extras, Page::PadBinds]);
        for want in [
            "Splitscreen …",
            "Weicher Chat: Aus",
            "Nachrichtenmarkierung: An",
            "Kanäle für UI-Töne: Aus",
            "Schnellzugriffsleiste: Vorheriger Platz",
            "Schnellzugriffsleiste: Nächster Platz",
        ] {
            assert!(texts.iter().any(|t| t == want), "{want:?} not in {texts:?}");
        }
        // Every key these pages ask for is in the table; vanilla's own keys (`options.*`,
        // `key.*`: dotted ids without spaces) come from the prepared resources, which this
        // language does not have.
        let missed: Vec<_> = missed
            .into_iter()
            .filter(|k| k.starts_with("horizon.") || k.contains(' ') || !k.contains('.'))
            .collect();
        assert_eq!(missed, Vec::<String>::new());
        let (texts, _) = page_texts("pl_pl", &[Page::Extras, Page::PadBinds]);
        for want in [
            "Płynny czat: Wył.",
            "Pasek szybkiego wyboru: poprzednie pole",
        ] {
            assert!(texts.iter().any(|t| t == want), "{want:?} not in {texts:?}");
        }
    }

    /// The Mods page is NeoForge's list screen: no switches, a footer of folder and
    /// Back, and Config on the selected native mod. Positions are the layout at 480x240:
    /// 150 px buttons 4 px apart, centred 27 px above the bottom; the sidebar at x 41 with its
    /// list at y 32 and 28 px rows; the panel at x 188 with its buttons at y 182, 84 px apart.
    #[test]
    fn the_mods_page_opens_the_folder_and_a_native_mods_settings() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        let size = (480.0, 240.0);
        // Starling: Options' Mods button opens its Mods page, whose Details is this screen.
        assert!(ui.layout(&s, size.0, size.1).0.iter().any(|w| {
            matches!(
                w.kind,
                WKind::Button {
                    active: true,
                    act: Act::Push(Page::NativeGame),
                    ..
                }
            )
        }));
        s.mods.installed.push(crate::mod_list::Installed {
            id: "native".into(),
            title: "Native 1.0".into(),
            description: String::new(),
            problem: None,
            dev: false,
            native: Some(crate::mod_list::NativePage {
                settings: vec![crate::mod_list::NativeSetting {
                    key: "trails".into(),
                    label: "Trails".into(),
                    tooltip: None,
                    value: crate::mod_list::NativeSettingValue::Toggle(true),
                }],
                ..Default::default()
            }),
        });
        let click = |ui: &mut OptionsUi, s: &mut Settings, x: f32, y: f32| {
            ui.click(s, &atlas, x, y, size, [480, 240], false, false)
        };
        ui.show(Page::Mods);
        assert_eq!(
            click(&mut ui, &mut s, 160.0, 223.0),
            vec![Event::ButtonClick, Event::OpenModFolder]
        );
        // Config does nothing until a native mod with settings is picked from the list
        // (Horizon, Minecraft, then Native).
        assert!(click(&mut ui, &mut s, 397.0, 192.0).is_empty());
        assert!(click(&mut ui, &mut s, 61.0, 104.0).is_empty());
        let events = click(&mut ui, &mut s, 397.0, 192.0);
        // Its labels were made with the list, maybe in another language.
        assert_eq!(events, vec![Event::ButtonClick, Event::NativeSettings(0)]);
        assert_eq!(ui.page(), Page::NativeMod(0));
        ui.back(&mut s);
        assert_eq!(ui.page(), Page::Mods);
        // Back leaves the page.
        let events = click(&mut ui, &mut s, 320.0, 223.0);
        assert_eq!(events.first(), Some(&Event::ButtonClick));
        assert_eq!(ui.page(), Page::Main);
    }

    #[test]
    fn a_mod_page_draws_its_settings_and_a_change_reaches_the_game() {
        use crate::mod_list::{NativeSetting, NativeSettingValue as V};
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        let size = (480.0, 240.0);
        let setting = |key: &str, value: V| NativeSetting {
            key: key.into(),
            label: key.into(),
            tooltip: None,
            value,
        };
        s.mods.installed.push(crate::mod_list::Installed {
            id: "native".into(),
            title: "Native 1.0".into(),
            description: String::new(),
            problem: None,
            dev: false,
            native: Some(crate::mod_list::NativePage {
                settings: vec![
                    setting("trails", V::Toggle(true)),
                    setting(
                        "speed",
                        V::Range {
                            min: 0.0,
                            max: 4.0,
                            step: 1.0,
                            value: 1.0,
                        },
                    ),
                ],
                ..Default::default()
            }),
        });
        ui.show(Page::NativeMod(0));
        let (widgets, ..) = ui.layout(&s, size.0, size.1);
        let toggle = widgets
            .iter()
            .find(|w| {
                matches!(
                    w.kind,
                    WKind::Button {
                        act: Act::NativeSetting(0, 0),
                        ..
                    }
                )
            })
            .expect("a toggle is a button");
        let slider = widgets
            .iter()
            .find(|w| {
                matches!(
                    w.kind,
                    WKind::Slider {
                        opt: Opt::ModSetting(0, 1),
                        ..
                    }
                )
            })
            .expect("a range is a slider");
        let (slider_x, slider_y, slider_w) = (slider.x, slider.y, slider.w);
        let events = ui.click(
            &mut s,
            &atlas,
            toggle.x + 1.0,
            toggle.y + 1.0,
            size,
            [480, 240],
            false,
            false,
        );
        assert!(events.contains(&Event::NativeSetting(0, 0)), "{events:?}");
        let page = |s: &Settings| s.mods.installed[0].native.clone().unwrap();
        assert_eq!(page(&s).settings[0].value, V::Toggle(false));
        let events = ui.click(
            &mut s,
            &atlas,
            slider_x + slider_w - 1.0,
            slider_y + 1.0,
            size,
            [480, 240],
            false,
            false,
        );
        assert!(events.contains(&Event::NativeSetting(0, 1)), "{events:?}");
        assert_eq!(
            page(&s).settings[1].value,
            V::Range {
                min: 0.0,
                max: 4.0,
                step: 1.0,
                value: 4.0
            }
        );
    }

    /// Typing goes to the Mods search box once a click focuses it; the one match is selected,
    /// so Homepage opens Minecraft's link.
    #[test]
    fn the_mods_search_takes_the_typing_after_a_click() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        let size = (480.0, 240.0);
        ui.show(Page::Mods);
        assert!(!ui.typing());
        ui.click(&mut s, &atlas, 60.0, 16.0, size, [480, 240], false, false);
        assert!(ui.typing());
        ui.type_text(&atlas, "mine");
        let events = ui.click(&mut s, &atlas, 229.0, 192.0, size, [480, 240], false, false);
        assert_eq!(
            events,
            vec![
                Event::ButtonClick,
                Event::OpenLink("https://minecraft.net".into())
            ]
        );
        assert!(!ui.typing());
    }

    #[test]
    fn mod_mapping_labels_and_category_search_use_translations() {
        let mut settings = Settings::default();
        settings.mods.installed.push(crate::mod_list::Installed {
            id: "sample".into(),
            title: "Sample".into(),
            description: String::new(),
            problem: None,
            dev: false,
            native: Some(crate::mod_list::NativePage {
                mappings: vec![crate::mod_list::KeyMapping {
                    id: "zoom".into(),
                    label: "controls.reset".into(),
                    category: "controls.keybinds.title".into(),
                    key: "key.keyboard.z".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
        });
        let mut ui = OptionsUi::default();
        ui.show(Page::KeyBinds);
        for (key, bound) in [("key.keyboard.z", true), ("key.keyboard.unknown", false)] {
            settings.mods.set_binding("sample", "zoom", key, 0);
            for act in [Act::ModKey(0, 0), Act::ResetModKey(0, 0)] {
                let reset = matches!(act, Act::ResetModKey(..));
                let widget = button(0.0, 0.0, 75.0, Text::literal("Z"), act);
                let mut narration = crate::widgets::Narratable::button("", [0.0; 4], true);
                narrate_widget(&mut narration, &settings, &widget, false, 0, [480, 240]);
                let name = tr("controls.reset");
                let expected = if reset {
                    Text::translate("narrator.controls.reset", &[name])
                } else if bound {
                    Text::translate(
                        "narrator.controls.bound",
                        &[
                            name,
                            Text::translate("gui.narrate.button", &[Text::literal("Z")]),
                        ],
                    )
                } else {
                    Text::translate("narrator.controls.unbound", &[name])
                };
                assert_eq!(
                    narration.what,
                    crate::widgets::What::Named {
                        title: expected.plain(),
                        active: true,
                    }
                );
                assert_eq!(
                    narration.hint,
                    tooltip_text(&settings, &widget).map(|tip| tip.plain())
                );
            }
        }
        for query in [
            lang::get("controls.reset"),
            lang::get("controls.keybinds.title"),
        ] {
            ui.settings_search.set_value(&query);
            let (widgets, ..) = ui.layout(&settings, 480.0, 240.0);
            assert!(widgets.iter().any(|widget| matches!(
                widget.kind,
                WKind::Button {
                    act: Act::ModKey(0, 0),
                    ..
                }
            )));
            assert!(widgets.iter().any(|widget| matches!(
                &widget.kind,
                WKind::Text { text, .. } if text.plain() == lang::get("controls.reset")
            )));
        }
    }

    #[test]
    fn conflicts_include_vanilla_mods_and_contexts() {
        use crate::mod_list::{Installed, KeyConflictContext as Context, KeyMapping, NativePage};
        let mut settings = Settings::default();
        for id in ["first", "second"] {
            settings.mods.installed.push(Installed {
                id: id.into(),
                title: id.into(),
                description: String::new(),
                problem: None,
                dev: false,
                native: Some(NativePage {
                    mappings: vec![KeyMapping {
                        id: "zoom".into(),
                        label: "key.zoom".into(),
                        key: "key.keyboard.z".into(),
                        context: Context::InGame,
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            });
        }
        settings
            .mods
            .set_binding("first", "zoom", "key.keyboard.w", 0);
        assert!(
            !key_conflicts(
                &settings,
                "first",
                "zoom",
                "key.keyboard.w",
                0,
                Context::InGame,
                false
            )
            .is_empty()
        );
        assert!(
            !key_conflicts(
                &settings,
                "",
                "key.forward",
                "key.keyboard.w",
                0,
                Context::Universal,
                true
            )
            .is_empty()
        );
        settings
            .mods
            .set_binding("first", "zoom", "key.keyboard.f24", 0);
        settings
            .mods
            .set_binding("second", "zoom", "key.keyboard.f24", 0);
        assert_eq!(
            key_conflicts(
                &settings,
                "first",
                "zoom",
                "key.keyboard.f24",
                0,
                Context::InGame,
                false
            ),
            [lang::get("key.zoom")]
        );
        assert!(
            key_conflicts(
                &settings,
                "first",
                "zoom",
                "key.keyboard.f24",
                0,
                Context::Gui,
                false
            )
            .is_empty()
        );
        assert!(
            key_conflicts(
                &settings,
                "first",
                "zoom",
                "key.keyboard.f24",
                1,
                Context::InGame,
                false
            )
            .is_empty()
        );
        assert!(
            key_conflicts(
                &settings,
                "first",
                "zoom",
                "key.keyboard.unknown",
                0,
                Context::Universal,
                false
            )
            .is_empty()
        );
    }

    #[test]
    fn mod_key_capture_waits_for_a_chord_or_modifier_release() {
        use winit::keyboard::KeyCode as K;
        let atlas = atlas();
        let mut settings = Settings::default();
        settings.mods.installed.push(crate::mod_list::Installed {
            id: "sample".into(),
            title: "Sample".into(),
            description: String::new(),
            problem: None,
            dev: false,
            native: Some(crate::mod_list::NativePage {
                mappings: vec![crate::mod_list::KeyMapping {
                    id: "zoom".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
        });
        let mut ui = OptionsUi::default();
        for (keys, modifiers, mouse, expected) in [
            (
                vec![K::ShiftLeft, K::KeyZ],
                1,
                false,
                "shift+key.keyboard.z",
            ),
            (
                vec![K::ControlLeft, K::AltLeft, K::KeyZ],
                6,
                false,
                "ctrl+alt+key.keyboard.z",
            ),
            (vec![K::ShiftLeft], 1, true, "shift+key.mouse.right"),
            (vec![K::ShiftLeft], 1, false, "key.keyboard.left.shift"),
        ] {
            ui.selected_key = Some(MOD_KEY);
            ui.selected_mod_key = Some((0, 0));
            ui.modifiers = modifiers;
            for key in &keys {
                ui.key(&mut settings, &atlas, *key, None);
            }
            if mouse {
                ui.bind_mouse(&mut settings, winit::event::MouseButton::Right);
            } else if keys.len() == 1 {
                ui.key_released(&mut settings, keys[0]);
            }
            assert_eq!(settings.mods.keys["sample"]["zoom"], expected);
            assert_eq!(ui.selected_key, None);
        }
    }

    #[test]
    fn the_native_game_page_says_which_game_runs_above_the_mods_and_its_buttons_act() {
        use crate::mod_list::{NativeGame, NativeGameAct};
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        let size = (480.0, 400.0);
        let now = "Now: Starling 0.8.0 from the APK, without native mods.";
        let old = "The build with Wrench was made for an earlier install of Starling, so it no \
                   longer loads. Compile again.";
        s.mods.native_game = Some(NativeGame {
            lines: vec![(now.into(), false), (old.into(), true)],
            buttons: vec![
                ("Compile".into(), NativeGameAct::Compile),
                ("Use GitHub (slower)".into(), NativeGameAct::CompileOnGithub),
                (
                    "Play without native mods".into(),
                    NativeGameAct::PlayWithout,
                ),
            ],
        });
        s.mods.installed.push(crate::mod_list::Installed {
            id: "wrench".into(),
            title: "Wrench 0.1.0".into(),
            description: String::new(),
            problem: Some(crate::mod_list::NOT_BUILT.into()),
            dev: false,
            native: Some(crate::mod_list::NativePage::default()),
        });
        ui.show(Page::NativeGame);
        let (widgets, ..) = ui.layout(&s, size.0, size.1);
        // Each line in pieces that fit a button's width, red for the build that no longer loads.
        let lines = |color: u32| -> Vec<String> {
            widgets
                .iter()
                .filter_map(|w| match &w.kind {
                    WKind::Text { text, color: c, .. } if *c == color => Some(text.plain()),
                    _ => None,
                })
                .filter(|line| !line.is_empty())
                .collect()
        };
        let grey = lines(0xFFA0A0A0);
        let red = lines(rgba(255, 85, 85, 255));
        assert!(grey.len() > 1 && red.len() > 1, "{grey:?} {red:?}");
        for line in wrap_words(old, NOTE_CHARS) {
            assert!(
                line.chars().count() <= NOTE_CHARS && red.contains(&line),
                "{line}"
            );
        }
        assert!(grey.join(" ").contains(now), "{grey:?}");
        let old_words = old.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(red.join(" ").contains(&old_words), "{red:?}");
        let shown = |act: NativeGameAct| {
            widgets
                .iter()
                .find(
                    |w| matches!(w.kind, WKind::Button { act: Act::NativeGame(a), .. } if a == act),
                )
                .unwrap_or_else(|| panic!("{act:?}'s button"))
        };
        // Two to a row; the one left over takes the row.
        assert_eq!(shown(NativeGameAct::Compile).w, 150.0);
        assert_eq!(shown(NativeGameAct::CompileOnGithub).w, 150.0);
        let without = shown(NativeGameAct::PlayWithout);
        assert_eq!(without.w, 310.0);
        let (x, y) = (without.x + 1.0, without.y + 1.0);
        let events = ui.click(&mut s, &atlas, x, y, size, [480, 400], false, false);
        assert_eq!(
            events.last(),
            Some(&Event::NativeGame(NativeGameAct::PlayWithout))
        );
        // Above the mods. The mod is chosen and not in this game: its row is a ticked box, without
        // "(Not built)" after it, and beside it a warning that presses nothing (Compile is the
        // page's, at the top).
        // The row's toggle (where, how wide, its label) and the button beside it, if any.
        let row = |widgets: &[Widget]| {
            let toggle = widgets
                .iter()
                .find_map(|w| match &w.kind {
                    WKind::Button {
                        label,
                        act: Act::ToggleMod(0),
                        ..
                    } => Some(([w.x, w.y, w.w], label.plain())),
                    _ => None,
                })
                .expect("the mod's row");
            let side = widgets.iter().find_map(|w| match &w.kind {
                WKind::Button {
                    act: Act::ModSettings(0),
                    ..
                } => Some([w.x, w.y, w.w]),
                _ => None,
            });
            (toggle, side)
        };
        let ((toggle, label), warning) = row(&widgets);
        assert!(toggle[1] > without.y);
        assert!(label.starts_with('\u{2611}'), "{label}");
        assert!(
            !label.contains(&tr("horizon.mods.not_built").plain()),
            "{label}"
        );
        let warning = warning.expect("the warning beside the row");
        assert_eq!(
            (toggle[2], warning[2], warning[1]),
            (150.0, 150.0, toggle[1])
        );
        let (x, y) = (warning[0] + 1.0, warning[1] + 1.0);
        let events = ui.click(&mut s, &atlas, x, y, size, [480, 400], false, false);
        assert!(!events.contains(&Event::NativeSettings(0)), "{events:?}");
        // Not chosen, it has nothing to compile: the row is the box alone.
        let (x, y) = (toggle[0] + 1.0, toggle[1] + 1.0);
        ui.click(&mut s, &atlas, x, y, size, [480, 400], false, false);
        assert!(!s.mods.is_on(0));
        let (widgets, ..) = ui.layout(&s, size.0, size.1);
        let ((toggle, label), warning) = row(&widgets);
        assert!(label.starts_with('\u{2610}'), "{label}");
        assert!(warning.is_none());
        assert_eq!(toggle[2], 310.0);
    }

    #[test]
    fn starlings_mods_page_switches_every_mod_and_keeps_dev_and_invalid_ones_inactive() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        let size = (480.0, 240.0);
        // The title's and the pause menu's Mods open it (a phone has no launcher).
        ui.open_at(false, Page::Mods);
        assert_eq!(ui.page(), Page::NativeGame);
        let empty = ui.layout(&s, size.0, size.1).0;
        assert!(empty.iter().any(|w| {
            matches!(&w.kind, WKind::Text { text, .. } if text.plain() == "horizon.mods.empty")
        }));
        let open_folder = empty
            .iter()
            .find(|w| {
                matches!(
                    &w.kind,
                    WKind::Button {
                        act: Act::OpenModFolder,
                        ..
                    }
                )
            })
            .expect("the Mods page has an Open Mod Folder button");
        let (x, y) = (open_folder.x + 1.0, open_folder.y + 1.0);
        assert_eq!(
            ui.click(&mut s, &atlas, x, y, size, [480, 240], false, false),
            vec![Event::ButtonClick, Event::OpenModFolder]
        );
        for (id, dev) in [("normal", false), ("dev", true), ("", false)] {
            s.mods.installed.push(crate::mod_list::Installed {
                id: id.into(),
                title: id.into(),
                description: "May show HUD panels.".into(),
                problem: id.is_empty().then(|| "Invalid package".into()),
                dev,
                native: None,
            });
        }
        let (widgets, ..) = ui.layout(&s, size.0, size.1);
        // Details opens upstream's Mods screen.
        assert!(widgets.iter().any(|w| matches!(
            w.kind,
            WKind::Button {
                act: Act::Push(Page::Mods),
                ..
            }
        )));
        let rows: Vec<(f32, f32)> = widgets
            .iter()
            .filter(|w| {
                matches!(
                    w.kind,
                    WKind::Button {
                        act: Act::ToggleMod(_),
                        ..
                    }
                )
            })
            .map(|w| (w.x + 1.0, w.y + 1.0))
            .collect();
        assert_eq!(rows.len(), 3);
        for (i, &(x, y)) in rows.iter().enumerate() {
            let events = ui.click(&mut s, &atlas, x, y, size, [480, 240], false, false);
            if i == 0 {
                assert!(events.contains(&Event::Changed));
                assert!(events.contains(&Event::ModToggled(0)));
            } else {
                assert!(events.is_empty(), "{events:?}");
            }
        }
        // A mod that is not native starts off: the click turned it on. A dev mod is always on,
        // the invalid one never.
        assert!(s.mods.is_on(0));
        assert!(s.mods.is_on(1));
        assert!(!s.mods.is_on(2));
    }

    #[test]
    fn a_native_mod_with_settings_has_them_beside_its_switch() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        let size = (480.0, 240.0);
        s.mods.installed.push(crate::mod_list::Installed {
            id: "native".into(),
            title: "Native 1.0".into(),
            description: String::new(),
            problem: None,
            dev: false,
            native: Some(crate::mod_list::NativePage {
                settings: vec![crate::mod_list::NativeSetting {
                    key: "trails".into(),
                    label: "Trails".into(),
                    tooltip: None,
                    value: crate::mod_list::NativeSettingValue::Toggle(true),
                }],
                ..Default::default()
            }),
        });
        s.mods.installed.push(crate::mod_list::Installed {
            id: "no_settings".into(),
            title: "Plain 1.0".into(),
            description: String::new(),
            problem: None,
            dev: false,
            native: Some(crate::mod_list::NativePage::default()),
        });
        ui.show(Page::NativeGame);
        let (widgets, ..) = ui.layout(&s, size.0, size.1);
        let toggle_w = |i: usize| {
            widgets
                .iter()
                .find(|w| matches!(w.kind, WKind::Button { act: Act::ToggleMod(j), .. } if j == i))
                .map(|w| w.w)
                .unwrap()
        };
        // Without settings, the switch takes the row.
        assert_eq!(toggle_w(1), 310.0);
        assert_eq!(toggle_w(0), 150.0);
        let (x, y) = widgets
            .iter()
            .find(|w| {
                matches!(
                    w.kind,
                    WKind::Button {
                        active: true,
                        act: Act::ModSettings(0),
                        ..
                    }
                )
            })
            .map(|w| (w.x + 1.0, w.y + 1.0))
            .expect("Settings beside the switch");
        let events = ui.click(&mut s, &atlas, x, y, size, [480, 240], false, false);
        // Its labels were made with the list, maybe in another language.
        assert_eq!(events.last(), Some(&Event::NativeSettings(0)));
        assert_eq!(ui.page(), Page::NativeMod(0));
        ui.back(&mut s);
        assert_eq!(ui.page(), Page::NativeGame);
    }

    #[test]
    fn a_tapped_row_keeps_no_tooltip_once_the_finger_lifts() {
        let atlas = atlas();
        let mut s = Settings::default();
        let size = (480.0, 400.0);
        s.mods.installed.push(crate::mod_list::Installed {
            id: "zoom".into(),
            title: "Zoom 2.0.0".into(),
            description: "Zooms in.".into(),
            problem: None,
            dev: false,
            native: Some(crate::mod_list::NativePage::default()),
        });
        let mut tapped = OptionsUi::default();
        tapped.show(Page::NativeGame);
        let mut never = OptionsUi::default();
        never.show(Page::NativeGame);
        let (widgets, ..) = tapped.layout(&s, size.0, size.1);
        let toggle = widgets
            .iter()
            .find(|w| {
                matches!(
                    w.kind,
                    WKind::Button {
                        act: Act::ToggleMod(0),
                        ..
                    }
                )
            })
            .expect("the mod's row");
        let (x, y) = (toggle.x + 1.0, toggle.y + 1.0);
        tapped.click(&mut s, &atlas, x, y, size, [480, 400], false, false);
        // The finger went away (the game draws with no pointer then).
        let away = (-10_000.0, -10_000.0);
        let drawn = |ui: &mut OptionsUi| {
            let mut batch = HudBatch::new(
                [480, 400],
                crate::hud::GuiScale {
                    requested: 0,
                    unicode: false,
                },
            );
            ui.draw(&s, NONE, &mut batch, &atlas, away);
            batch.vertices
        };
        let plain = drawn(&mut never);
        // A click's focus draws no tooltip away from the pointer (upstream follows vanilla
        // there), and a lifted finger lets go of its highlight too.
        tapped.drop_pointer_focus();
        let after = drawn(&mut tapped);
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&after),
            bytemuck::cast_slice::<_, u8>(&plain)
        );
    }

    #[test]
    fn a_native_mods_key_shows_and_keeps_the_key_pressed() {
        // #3539: Zoom's key set to R showed another key.
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        let size = (480.0, 240.0);
        s.mods.installed.push(crate::mod_list::Installed {
            id: "zoom".into(),
            title: "Zoom 2.0.0".into(),
            description: String::new(),
            problem: None,
            dev: false,
            native: Some(crate::mod_list::NativePage {
                mappings: vec![crate::mod_list::KeyMapping {
                    id: "zoom".into(),
                    label: "key.zoom".into(),
                    key: "key.keyboard.z".into(),
                    ..Default::default()
                }],
                settings: Vec::new(),
            }),
        });
        ui.show(Page::KeyBinds);
        ui.scroll(&s, size, -10_000.0);
        let key_label = |ui: &mut OptionsUi, s: &Settings| {
            let (widgets, ..) = ui.layout(s, size.0, size.1);
            widgets.into_iter().find_map(|w| match w.kind {
                WKind::Button {
                    label,
                    act: Act::ModKey(0, 0),
                    ..
                } => Some((label.plain(), w.x, w.y)),
                _ => None,
            })
        };
        let (label, x, y) = key_label(&mut ui, &s).expect("the key's row");
        let (_, top, bottom, _) = ui.layout(&s, size.0, size.1);
        assert!(y >= top && y + 20.0 <= bottom);
        assert_eq!(label, "Z");
        ui.click(
            &mut s,
            &atlas,
            x + 1.0,
            y + 1.0,
            size,
            [480, 240],
            false,
            false,
        );
        assert_eq!(ui.selected_key, Some(MOD_KEY));
        assert_eq!(
            ui.key(&mut s, &atlas, winit::keyboard::KeyCode::KeyR, Some("r")),
            vec![Event::Changed]
        );
        assert_eq!(key_label(&mut ui, &s).unwrap().0, "R");
        let saved = crate::mod_list::ModList::parse(&s.mods.serialize());
        assert_eq!(saved.keys_of("zoom"), vec![("zoom", "key.keyboard.r")]);
    }

    fn mode(width: i32, rate: f32) -> crate::window::VideoMode {
        crate::window::VideoMode {
            width,
            height: width * 9 / 16,
            red_bits: 8,
            green_bits: 8,
            blue_bits: 8,
            refresh_rate: rate,
        }
    }

    #[test]
    fn video_page_lists_vanilla_display_and_quality_options_in_order() {
        let mut ui = OptionsUi::default();
        ui.show(Page::Video);
        let rows = ui.list_rows(&Settings::default());
        let opts: Vec<Opt> = rows
            .iter()
            .flat_map(|r| match r {
                Row::Big(o) => vec![*o],
                Row::Pair(a, b) => std::iter::once(*a).chain(*b).collect(),
                _ => Vec::new(),
            })
            .collect();
        // VideoSettingsScreen.addOptions: the exclusive mode row, displayOptions, the preset,
        // qualityOptions, then the preferences.
        assert_eq!(
            opts[..9],
            [
                Opt::ExclusiveMode,
                Opt::MaxFps,
                Opt::Vsync,
                Opt::InactivityFps,
                Opt::GuiScale,
                Opt::Fullscreen,
                Opt::ExclusiveFullscreen,
                Opt::Gamma,
                Opt::GraphicsBackend,
            ]
        );
        let at = |o| opts.iter().position(|x| *x == o).unwrap();
        assert_eq!(at(Opt::ImprovedTransparency) + 1, at(Opt::TextureFiltering));
        assert_eq!(at(Opt::TextureFiltering) + 1, at(Opt::MaxAnisotropy));
        assert_eq!(at(Opt::MaxAnisotropy) + 1, at(Opt::WeatherRadius));
    }

    #[test]
    fn exclusive_fullscreen_mode_picks_a_monitor_mode_or_the_current_one() {
        let mut s = Settings::default();
        // No monitor mode: the slider changes nothing.
        Opt::ExclusiveMode.set_slider(&mut s, 1.0);
        assert_eq!(s.file.fullscreen_resolution, None);
        s.video_modes = vec![mode(1280, 60.0), mode(1920, 144.0), mode(2560, 59.94)];
        assert_eq!(s.video_mode_index(), -1);
        // IntRange(-1, 2): the slider's thirds are -1, 0, 1, 2.
        Opt::ExclusiveMode.set_slider(&mut s, 2.0 / 3.0);
        assert_eq!(s.video_mode_index(), 1);
        assert_eq!(Opt::ExclusiveMode.slider_position(&s), Some(2.0 / 3.0));
        Opt::ExclusiveMode.set_slider(&mut s, 1.0);
        assert_eq!(s.video_mode_index(), 2);
        Opt::ExclusiveMode.set_slider(&mut s, 0.0);
        assert_eq!(s.file.fullscreen_resolution, None);
    }

    #[test]
    fn pointer_cursor_leaves_a_clicked_option_while_its_focus_stays() {
        use crate::widgets::{Cursor, take_cursor};
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Video);
        let size = (480.0, 480.0);
        let (widgets, top, bottom, _) = ui.layout(&s, size.0, size.1);
        let button = widgets
            .iter()
            .find(|wd| {
                matches!(
                    wd.kind,
                    WKind::Button {
                        active: true,
                        act: Act::Cycle(_),
                        ..
                    }
                ) && (!wd.list || wd.y >= top && wd.y + wd.h <= bottom)
            })
            .expect("visible cycle button");
        let over = (button.x + 1.0, button.y + 1.0);
        ui.click(
            &mut s,
            &atlas,
            over.0,
            over.1,
            size,
            [480, 480],
            false,
            false,
        );
        ui.release();
        assert!(
            ui.focus.is_some(),
            "the accepted click keeps vanilla's focus"
        );
        assert!(!ui.focus_from_keyboard);
        let _ = take_cursor();
        ui.draw(
            &s,
            NONE,
            &mut HudBatch::new(
                [480, 480],
                crate::hud::GuiScale {
                    requested: 0,
                    unicode: false,
                },
            ),
            &atlas,
            over,
        );
        assert_eq!(take_cursor(), Cursor::PointingHand);
        ui.draw(
            &s,
            NONE,
            &mut HudBatch::new(
                [480, 480],
                crate::hud::GuiScale {
                    requested: 0,
                    unicode: false,
                },
            ),
            &atlas,
            (-10.0, -10.0),
        );
        assert_eq!(take_cursor(), Cursor::Arrow, "focus is not mouse hover");
        assert!(
            ui.focus.is_some(),
            "moving the pointer does not clear focus"
        );
        ui.focus_from_keyboard = true;
        ui.draw(
            &s,
            NONE,
            &mut HudBatch::new(
                [480, 480],
                crate::hud::GuiScale {
                    requested: 0,
                    unicode: false,
                },
            ),
            &atlas,
            (-10.0, -10.0),
        );
        assert_eq!(
            take_cursor(),
            Cursor::Arrow,
            "keyboard focus is not mouse hover either"
        );
    }

    #[test]
    fn pointer_cursor_leaves_a_disabled_option_without_focusing_it() {
        use crate::widgets::{Cursor, take_cursor};
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::KeyBinds);
        let size = (480.0, 480.0);
        let (widgets, top, bottom, _) = ui.layout(&s, size.0, size.1);
        let button = widgets
            .iter()
            .find(|wd| {
                matches!(wd.kind, WKind::Button { active: false, .. })
                    && (!wd.list || wd.y >= top && wd.y + wd.h <= bottom)
            })
            .expect("visible disabled button");
        let over = (button.x + 1.0, button.y + 1.0);
        let _ = take_cursor();
        ui.draw(
            &s,
            NONE,
            &mut HudBatch::new(
                [480, 480],
                crate::hud::GuiScale {
                    requested: 0,
                    unicode: false,
                },
            ),
            &atlas,
            over,
        );
        assert_eq!(take_cursor(), Cursor::NotAllowed);
        assert!(
            ui.click(
                &mut s,
                &atlas,
                over.0,
                over.1,
                size,
                [480, 480],
                false,
                false
            )
            .is_empty()
        );
        assert!(
            ui.focus.is_none(),
            "an inactive widget rejects the click before focus"
        );
        ui.draw(
            &s,
            NONE,
            &mut HudBatch::new(
                [480, 480],
                crate::hud::GuiScale {
                    requested: 0,
                    unicode: false,
                },
            ),
            &atlas,
            (-10.0, -10.0),
        );
        assert_eq!(take_cursor(), Cursor::Arrow);
    }

    #[test]
    fn a_clicked_extras_tooltip_leaves_with_the_pointer() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Pvp);
        let (widgets, _, _, _) = ui.layout(&s, 480.0, 480.0);
        let extra = widgets
            .iter()
            .find(|w| {
                matches!(
                    &w.kind,
                    WKind::Button {
                        act: Act::Extra(item),
                        ..
                    } if item.key_name().is_none()
                )
            })
            .unwrap();
        ui.click(
            &mut s,
            &atlas,
            extra.x + 1.0,
            extra.y + 1.0,
            (480.0, 480.0),
            [960, 540],
            false,
            false,
        );
        // The pointer is still on the clicked widget: it carries the
        // description (`setTooltip`, drawn while hovered).
        let mut on = HudBatch::scratch([480, 480]);
        ui.draw(
            &s,
            NONE,
            &mut on,
            &atlas,
            (extra.x + extra.w / 2.0, extra.y + extra.h / 2.0),
        );
        // The pointer moved off: the description goes with it. A mouse
        // focus a click left behind (the look-up keeps the click's widget,
        // `focus_from_keyboard` false) does not keep the tooltip against
        // the widget, like vanilla's `MenuTooltipPositioner` does for a
        // keyboard focus.
        let mut off = HudBatch::scratch([480, 480]);
        ui.draw(
            &s,
            NONE,
            &mut off,
            &atlas,
            (extra.x + extra.w + 40.0, extra.y + extra.h + 40.0),
        );
        assert!(
            off.vertices.len() < on.vertices.len(),
            "a click left {} tooltip vertices behind when the pointer left ({} vs {})",
            on.vertices.len() - off.vertices.len(),
            on.vertices.len(),
            off.vertices.len(),
        );
    }

    #[test]
    fn extras_reaches_the_zoom_mod_that_replaced_the_extra() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Extras);
        let english = || lang::Language::load(&[], "en_us");
        // Without the mod, Extras says where it is installed (a note, not a second way into
        // Options > Mods).
        let labels = lang::scoped(english(), || ui.extras.row_labels(&s)).0;
        assert!(
            labels.iter().any(|l| l == "Zoom: install from Mods"),
            "Extras has no hint for the Zoom mod: {labels:?}"
        );
        // Built in, Zoom's settings live on its page: Extras opens that page.
        s.mods.installed.push(crate::mod_list::Installed {
            id: "zoom".into(),
            title: "Zoom 2.0.0".into(),
            description: String::new(),
            problem: None,
            dev: false,
            native: Some(crate::mod_list::NativePage {
                mappings: Vec::new(),
                settings: vec![crate::mod_list::NativeSetting {
                    key: "trails".into(),
                    label: "Trails".into(),
                    tooltip: None,
                    value: crate::mod_list::NativeSettingValue::Toggle(true),
                }],
            }),
        });
        let labels = lang::scoped(english(), || ui.extras.row_labels(&s)).0;
        assert!(labels.iter().any(|l| l == "Settings"), "{labels:?}");
        // The mod's switch is beside it, so Extras keeps the Zoom setting it had.
        assert!(labels.iter().any(|l| l.starts_with("Zoom: ")), "{labels:?}");
        let size = (480.0, 480.0);
        let on = s.mods.is_on(0);
        let at = ui.extras.point_at(&s, size, ExtraRow::Zoom(0));
        assert_eq!(
            ui.click(&mut s, &atlas, at.0, at.1, size, [960, 960], false, false),
            vec![Event::ButtonClick, Event::Changed]
        );
        assert_ne!(s.mods.is_on(0), on);
        let at = ui.extras.point_at(&s, size, ExtraRow::ZoomSettings(0));
        assert_eq!(
            ui.click(&mut s, &atlas, at.0, at.1, size, [960, 960], false, false),
            vec![
                Event::ButtonClick,
                Event::ButtonClick,
                Event::NativeSettings(0)
            ]
        );
        assert_eq!(ui.page(), Page::NativeMod(0));
    }

    #[test]
    fn exclusive_fullscreen_and_its_tooltips_follow_the_option() {
        let mut s = Settings::default();
        assert!(!s.file.bool("exclusiveFullscreen"));
        assert_eq!(
            Opt::ExclusiveFullscreen.tooltip_key(&s),
            "options.exclusiveFullscreen.off.tooltip"
        );
        Opt::ExclusiveFullscreen.cycle(&mut s, false, [854, 480]);
        assert!(s.file.bool("exclusiveFullscreen"));
        assert_eq!(
            Opt::ExclusiveFullscreen.tooltip_key(&s),
            "options.exclusiveFullscreen.on.tooltip"
        );
        // The mode row's own tooltip says it only matters with Exclusive Fullscreen.
        assert_eq!(
            Opt::ExclusiveMode.tooltip_key(&s),
            "options.fullscreen.exclusive.mode.tooltip"
        );
    }

    #[test]
    fn the_graphics_api_option_cycles_default_opengl_vulkan_and_asks_for_a_restart() {
        use crate::graphics::GraphicsApi;
        let mut s = Settings::default();
        s.startup_api = s.horizon.api;
        let offered = GraphicsApi::choices();
        let mut seen = vec![s.horizon.api];
        for _ in 1..offered.len() {
            Opt::GraphicsBackend.cycle(&mut s, false, [854, 480]);
            seen.push(s.horizon.api);
        }
        // Dx12 is Horizon's own Graphics page's; this option has the three of vanilla.
        assert!(!seen.contains(&GraphicsApi::Dx12));
        assert_eq!(seen[0], GraphicsApi::Auto);
        if offered.len() == 3 {
            assert_eq!(seen[1..], [GraphicsApi::OpenGl, GraphicsApi::Vulkan]);
            assert_eq!(s.file.name("preferredGraphicsBackend"), "vulkan");
            Opt::GraphicsBackend.cycle(&mut s, false, [854, 480]);
            assert_eq!(s.horizon.api, GraphicsApi::Auto);
            assert_eq!(s.file.name("preferredGraphicsBackend"), "default");
        }
    }

    #[test]
    fn texture_filtering_rows_exist_and_the_presets_set_them() {
        let mut s = Settings::default();
        for (preset, bit, filtering) in [("fast", 1, 0), ("fancy", 1, 1), ("fabulous", 2, 2)] {
            s.graphics_preset = preset.into();
            apply_preset(&mut s);
            assert_eq!(s.file.int("maxAnisotropyBit"), bit, "{preset}");
            assert_eq!(s.file.int("textureFiltering"), filtering, "{preset}");
        }
        assert!(Opt::TextureFiltering.available());
        assert!(Opt::MaxAnisotropy.available());
        // The anisotropy slider only works with Anisotropic filtering (VideoSettingsScreen.tick).
        let grey = |s: &Settings| {
            matches!(
                option_widget(s, Opt::MaxAnisotropy, 0.0, 0.0, 150.0).kind,
                WKind::Button { active: false, .. }
            )
        };
        s.file
            .set("textureFiltering", crate::options_file::V::Int(1));
        assert!(grey(&s));
        s.file
            .set("textureFiltering", crate::options_file::V::Int(2));
        assert!(!grey(&s));
    }

    #[test]
    #[ignore = "requires prepared language data in HORIZON_TEST_RESOURCES"]
    fn the_new_video_options_read_as_vanilla_words_them() {
        use crate::graphics::GraphicsApi;
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES"),
        );
        lang::scoped(lang::Language::load(&[&root], "en_us"), || {
            let mut s = Settings::default();
            assert_eq!(Opt::ExclusiveMode.label(&s).plain(), "Setting unavailable");
            s.video_modes = vec![mode(1280, 60.0), mode(1920, 144.0), mode(2560, 59.94)];
            assert_eq!(
                Opt::ExclusiveMode.label(&s).plain(),
                "Exclusive Fullscreen Mode: Current"
            );
            Opt::ExclusiveMode.set_slider(&mut s, 2.0 / 3.0);
            assert_eq!(
                Opt::ExclusiveMode.label(&s).plain(),
                "Exclusive Fullscreen Mode: 1920x1080@144 (24bit)"
            );
            Opt::ExclusiveMode.set_slider(&mut s, 1.0);
            assert_eq!(
                Opt::ExclusiveMode.label(&s).plain(),
                "Exclusive Fullscreen Mode: 2560x1440@59.94 (24bit)"
            );
            s.horizon.api = GraphicsApi::Vulkan;
            assert_eq!(
                Opt::GraphicsBackend.label(&s).plain(),
                "Graphics API: Prefer Vulkan (Experimental)"
            );
            assert!(
                graphics_api_tooltip(&s)
                    .plain()
                    .starts_with("Requires a game restart to take effect.\n\n")
            );
            s.startup_api = GraphicsApi::Vulkan;
            assert!(
                graphics_api_tooltip(&s)
                    .plain()
                    .starts_with("Which graphics API")
            );
            s.file
                .set("maxAnisotropyBit", crate::options_file::V::Int(2));
            s.file
                .set("textureFiltering", crate::options_file::V::Int(2));
            assert_eq!(
                Opt::MaxAnisotropy.label(&s).plain(),
                "Anisotropic Filtering: 4x"
            );
            assert_eq!(
                Opt::TextureFiltering.label(&s).plain(),
                "Texture Filtering: Anisotropic"
            );
        });
    }

    #[test]
    fn option_tooltip_keys_follow_value_dependent_descriptions() {
        let mut s = Settings::default();
        assert_eq!(
            Opt::InactivityFps.tooltip_key(&s),
            "options.inactivityFpsLimit.afk.tooltip"
        );
        s.inactivity_fps_limit = "minimized".into();
        assert_eq!(
            Opt::InactivityFps.tooltip_key(&s),
            "options.inactivityFpsLimit.minimized.tooltip"
        );
        for (value, name) in ["none", "byPlayer", "nearby"].iter().enumerate() {
            s.prioritize_chunk_updates = value as u8;
            assert_eq!(
                Opt::ChunkBuilder.tooltip_key(&s),
                format!("options.prioritizeChunkUpdates.{name}.tooltip")
            );
        }
        assert_eq!(
            Opt::MenuBlur.tooltip_key(&s),
            "options.accessibility.menu_background_blurriness.tooltip"
        );
    }

    #[test]
    #[ignore = "requires prepared language data in HORIZON_TEST_RESOURCES"]
    fn classic_hover_draws_the_selected_options_wrapped_tooltip() {
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES"),
        );
        lang::scoped(lang::Language::load(&[&root], "en_us"), || {
            let atlas = atlas();
            let mut s = Settings::default();
            let mut ui = OptionsUi::default();
            ui.show(Page::Video);
            for mode in ["afk", "minimized"] {
                s.inactivity_fps_limit = mode.into();
                let (widgets, _, _, _) = ui.layout(&s, 480.0, 480.0);
                let option = widgets
                    .iter()
                    .find(|w| {
                        matches!(
                            w.kind,
                            WKind::Button {
                                act: Act::Cycle(Opt::InactivityFps),
                                ..
                            }
                        )
                    })
                    .unwrap();
                let cursor = (option.x + 1.0, option.y + 1.0);
                let mut actual = HudBatch::scratch([480, 480]);
                ui.draw(&s, NONE, &mut actual, &atlas, cursor);
                let key = format!("options.inactivityFpsLimit.{mode}.tooltip");
                assert!(lang::has(&key));
                let mut expected = HudBatch::scratch([480, 480]);
                ui::tooltip(
                    &mut expected,
                    &atlas,
                    &atlas.font.split(&tr(&key), 170.0),
                    cursor,
                );
                assert!(!expected.vertices.is_empty());
                let tail = &actual.vertices[actual.vertices.len() - expected.vertices.len()..];
                assert_eq!(
                    bytemuck::cast_slice::<_, u8>(tail),
                    bytemuck::cast_slice::<_, u8>(&expected.vertices)
                );
            }
        });
    }

    #[test]
    fn classic_video_search_keeps_only_headings_with_matching_options() {
        let mut ui = OptionsUi::default();
        ui.show(Page::Video);
        let settings = Settings::default();
        let headers = |rows: Vec<Row>| {
            rows.into_iter()
                .filter_map(|row| match row {
                    Row::Header(key) => Some(key),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let all_headers = vec![
            "options.video.display.header",
            "options.video.quality.header",
            "options.video.preferences.header",
        ];
        assert_eq!(headers(ui.list_rows(&settings)), all_headers);
        for (opt, header) in [
            (Opt::Fullscreen, all_headers[0]),
            (Opt::RenderDistance, all_headers[1]),
            (Opt::Preset, all_headers[1]),
            (Opt::Autosave, all_headers[2]),
        ] {
            ui.settings_search
                .set_value(&lang::get(opt.caption()).to_uppercase());
            let rows = ui.list_rows(&settings);
            assert_eq!(headers(ui.list_rows(&settings)), vec![header]);
            assert!(rows.iter().any(|row| match row {
                Row::Big(found) => *found == opt,
                Row::Pair(a, b) => *a == opt || *b == Some(opt),
                _ => false,
            }));
        }
        ui.settings_search.set_value("no matching video setting");
        let rows = ui.list_rows(&settings);
        assert!(headers(ui.list_rows(&settings)).is_empty());
        assert!(rows.is_empty(), "{}", rows.len());
        ui.settings_search.set_value("");
        assert_eq!(headers(ui.list_rows(&settings)), all_headers);
    }

    #[test]
    fn classic_search_emits_each_matching_heading_once_in_section_order() {
        let mut ui = OptionsUi::default();
        ui.settings_search
            .set_value(&lang::get(Opt::Fullscreen.caption()));
        let rows = ui.filter_option_rows(vec![
            Row::Header("empty"),
            Row::Header("display"),
            Row::Pair(Opt::Fullscreen, Some(Opt::Fullscreen)),
            Row::Big(Opt::Fullscreen),
            Row::Header("quality"),
            Row::Big(Opt::Fullscreen),
            Row::Header("trailing empty"),
        ]);
        assert!(matches!(
            rows.as_slice(),
            [
                Row::Header("display"),
                Row::Pair(Opt::Fullscreen, Some(Opt::Fullscreen)),
                Row::Big(Opt::Fullscreen),
                Row::Header("quality"),
                Row::Big(Opt::Fullscreen),
            ]
        ));
    }

    #[test]
    fn classic_video_gui_scale_cycles_on_press_without_dragging() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Video);
        let size = (480.0, 480.0);
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let button = widgets
            .iter()
            .find(|wd| {
                matches!(
                    wd.kind,
                    WKind::Button {
                        active: true,
                        act: Act::Cycle(Opt::GuiScale),
                        ..
                    }
                )
            })
            .expect("GUI Scale must be a cycle button in classic Video Settings");
        let (x, y) = (button.x + 1.0, button.y + 1.0);
        assert!(Opt::GuiScale.slider_position(&s).is_none());
        assert!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], false, true)
                .is_empty()
        );
        assert_eq!(s.gui_scale, 0);
        for expected in [1, 2, 0] {
            assert_eq!(
                ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false),
                vec![Event::ButtonClick, Event::Changed]
            );
            assert_eq!(s.gui_scale, expected);
            assert!(
                ui.click(&mut s, &atlas, x + 100.0, y, size, [854, 480], true, false)
                    .is_empty()
            );
            assert_eq!(s.gui_scale, expected);
            assert!(ui.release().is_empty());
        }
    }

    #[test]
    fn extras_from_horizon_settings_edit_shared_settings_and_return() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        let size = (480.0, 480.0);
        let click = |ui: &mut OptionsUi, s: &mut Settings, act: Act| {
            let (widgets, _, _, _) = ui.layout(s, size.0, size.1);
            let widget = widgets
                .iter()
                .find(
                    |w| matches!(&w.kind, WKind::Button { act: a, active: true, .. } if *a == act),
                )
                .unwrap();
            ui.click(
                s,
                &atlas,
                widget.x + 1.0,
                widget.y + 1.0,
                size,
                [960, 960],
                false,
                false,
            )
        };
        for page in [Page::Horizon, Page::Extras] {
            assert_eq!(
                click(&mut ui, &mut s, Act::Push(page)),
                vec![Event::ButtonClick]
            );
            assert_eq!(ui.page(), page);
        }
        assert_eq!(
            ui.extras.row_labels(&s).len(),
            // And the note about the Zoom mod over them.
            1 + crate::extras::rows().iter().filter(|r| r.is_ok()).count()
        );
        let press = |ui: &mut OptionsUi, s: &mut Settings, item: crate::extras::Item| {
            let at = ui.extras.point_at(s, size, ExtraRow::Item(item));
            ui.click(s, &atlas, at.0, at.1, size, [960, 960], false, false)
        };
        assert_eq!(
            press(&mut ui, &mut s, crate::extras::Item::Sort),
            vec![Event::ButtonClick, Event::Changed]
        );
        assert!(s.extras.sort);
        assert_eq!(
            press(&mut ui, &mut s, crate::extras::Item::Pregen),
            vec![Event::ButtonClick, Event::Changed]
        );
        assert!(s.extras.pregen);
        assert_eq!(
            press(&mut ui, &mut s, crate::extras::Item::SortKey),
            vec![Event::ButtonClick]
        );
        assert_eq!(ui.selected_key, Some(crate::extras::SORT_KEY));
        assert_eq!(
            ui.key(&mut s, &atlas, winit::keyboard::KeyCode::KeyC, None),
            vec![Event::Changed]
        );
        assert_eq!(s.extras.sort_key, "key.keyboard.c");
        assert_eq!(
            crate::extras::Extras::parse(&s.extras.serialize()),
            s.extras
        );
        assert_eq!(
            ui.key(&mut s, &atlas, winit::keyboard::KeyCode::Escape, None),
            vec![Event::Changed]
        );
        assert_eq!(ui.page(), Page::Horizon);
    }

    /// A Bindings row pressed waits for a controller button: the next click or key cancels the
    /// wait, Escape unbinds the action as on Key Binds, and none of them leaves the screen.
    #[test]
    fn a_controller_binding_waits_and_a_click_or_a_key_ends_the_wait() {
        use crate::controller_ui::Item;
        use winit::keyboard::KeyCode as K;
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::PadBinds);
        let size = (854.0, 480.0);
        ui.size = size;
        let jump = Item::Bind("key.jump");
        let (x, y) = ui.controller.point_at(&s, size, jump);
        let press = |ui: &mut OptionsUi, s: &mut Settings| {
            ui.click(s, &atlas, x, y, size, [1708, 960], false, false)
        };
        assert_eq!(press(&mut ui, &mut s), vec![Event::ButtonClick]);
        assert_eq!(ui.selected_pad_action, Some("key.jump"));
        assert!(press(&mut ui, &mut s).is_empty());
        assert_eq!(ui.selected_pad_action, None);
        press(&mut ui, &mut s);
        assert_eq!(ui.key(&mut s, &atlas, K::KeyQ, None), vec![Event::Changed]);
        assert_eq!(
            (ui.selected_pad_action, s.controller.is_default("key.jump")),
            (None, true)
        );
        press(&mut ui, &mut s);
        assert_eq!(
            ui.key(&mut s, &atlas, K::Escape, None),
            vec![Event::Changed]
        );
        assert_eq!(s.controller.binding("key.jump"), None);
        assert_eq!(ui.page(), Page::PadBinds);
    }

    /// Every way a pad or a wheel scrolls a page on the shell moves the same list: Replay Studio's
    /// took the wheel and missed `scroll_at`, so a stick couldn't scroll it.
    #[test]
    fn the_wheel_and_the_stick_scroll_a_page_on_the_shell_alike() {
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Extras);
        // Too short for the page.
        let short = (480.0, 180.0);
        ui.wheel(&mut s, short, (0.0, 0.0), [960, 360], -1.0);
        let one = ui.extras.scroll_amount();
        assert!(one > 0.0);
        ui.scroll_at(&s, short, (0.0, 0.0), -1.0);
        assert_eq!(ui.extras.scroll_amount(), 2.0 * one);
        ui.scroll_at(&s, short, (0.0, 0.0), 2.0);
        assert_eq!(ui.extras.scroll_amount(), 0.0);
    }

    #[test]
    fn classic_graphics_presets_set_the_distances() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Video);
        let size = (480.0, 480.0);
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let preset = widgets
            .iter()
            .find(|wd| {
                matches!(
                    wd.kind,
                    WKind::Slider {
                        opt: Opt::Preset,
                        ..
                    }
                )
            })
            .expect("the Graphics preset slider is in classic Video Settings");
        // Dragging the slider across its range (GraphicsPreset.apply on each change).
        let y = preset.y + 1.0;
        let mut seen = Vec::new();
        ui.click(
            &mut s,
            &atlas,
            preset.x + 1.0,
            y,
            size,
            [854, 480],
            false,
            false,
        );
        for step in 0..=20 {
            let x = preset.x + preset.w * step as f32 / 20.0;
            ui.click(&mut s, &atlas, x, y, size, [854, 480], true, false);
            seen.push((
                s.graphics_preset.clone(),
                s.render_distance,
                s.simulation_distance,
                s.cloud_range,
            ));
        }
        ui.release();
        for expected in [
            ("fast", 8, 6, 32),
            ("fancy", 16, 12, 64),
            ("fabulous", 32, 12, 128),
        ] {
            assert!(
                seen.iter()
                    .any(|(p, r, d, c)| (p.as_str(), *r, *d, *c) == expected),
                "{expected:?} in {seen:?}"
            );
        }
    }

    #[test]
    fn gui_scale_cycle_handles_reverse_and_out_of_range_saved_values() {
        for (framebuffer, force_unicode, max) in [
            ([854, 480], false, 2),
            ([1920, 1080], false, 4),
            ([1280, 720], true, 4),
            ([3840, 2160], false, 9),
            ([320, 240], false, 1),
        ] {
            let mut s = Settings {
                force_unicode,
                ..Settings::default()
            };
            for expected in (0..=max).rev() {
                Opt::GuiScale.cycle(&mut s, true, framebuffer);
                assert_eq!(s.gui_scale, expected);
            }
            for expected in (1..=max).chain([0]) {
                Opt::GuiScale.cycle(&mut s, false, framebuffer);
                assert_eq!(s.gui_scale, expected);
            }
            s.gui_scale = u32::MAX;
            Opt::GuiScale.cycle(&mut s, false, framebuffer);
            assert_eq!(s.gui_scale, 0);
            s.gui_scale = u32::MAX;
            Opt::GuiScale.cycle(&mut s, true, framebuffer);
            assert_eq!(s.gui_scale, max - 1);
        }
        let mut s = Settings {
            gui_scale: 4,
            ..Settings::default()
        };
        // A resized window supplies a new maximum on the next interaction.
        Opt::GuiScale.cycle(&mut s, true, [854, 480]);
        assert_eq!(s.gui_scale, 1);
    }

    #[test]
    fn classic_buttons_sound_but_disabled_and_nonprimary_clicks_are_silent() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Mouse);
        let size = (480.0, 320.0);
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let button = widgets
            .iter()
            .find(|wd| {
                matches!(
                    wd.kind,
                    WKind::Button {
                        active: true,
                        act: Act::Cycle(_),
                        ..
                    }
                )
            })
            .unwrap();
        let (x, y) = (button.x + 1.0, button.y + 1.0);
        assert!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], false, true)
                .is_empty()
        );
        assert_eq!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false),
            vec![Event::ButtonClick, Event::Changed]
        );
        assert!(ui.release().is_empty());
        assert!(
            ui.click(&mut s, &atlas, 0.0, 0.0, size, [854, 480], false, false)
                .is_empty()
        );
        ui.show(Page::KeyBinds);
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let disabled = widgets
            .iter()
            .find(|wd| matches!(wd.kind, WKind::Button { active: false, .. }))
            .unwrap();
        assert!(
            ui.click(
                &mut s,
                &atlas,
                disabled.x + 1.0,
                disabled.y + 1.0,
                size,
                [854, 480],
                false,
                false
            )
            .is_empty()
        );
    }

    #[test]
    fn classic_sliders_sound_once_on_release_including_unchanged_values() {
        let atlas = atlas();
        let size = (480.0, 320.0);
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Sound);
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let slider = widgets
            .iter()
            .find(|wd| {
                matches!(
                    wd.kind,
                    WKind::Slider {
                        opt: Opt::Volume(0),
                        ..
                    }
                )
            })
            .unwrap();
        let (x, y) = (slider.x + slider.w - 1.0, slider.y + 1.0);
        assert_eq!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false),
            vec![Event::SliderDrag]
        );
        assert_eq!(s.master_volume, 1.0);
        assert_eq!(ui.release(), vec![Event::ButtonClick, Event::Changed]);
        assert!(ui.release().is_empty());
        assert_eq!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false),
            vec![Event::SliderDrag]
        );
        for cx in [-100.0, 250.0, 1000.0] {
            assert_eq!(
                ui.click(&mut s, &atlas, cx, -100.0, size, [854, 480], true, false),
                vec![Event::SliderDrag, Event::Volume(0, s.master_volume)]
            );
        }
        assert_eq!(ui.release(), vec![Event::ButtonClick, Event::Changed]);
        assert!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], true, false)
                .is_empty()
        );
    }

    #[test]
    fn a_moved_volume_slider_reports_its_category_and_value() {
        let atlas = atlas();
        let size = (480.0, 320.0);
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Sound);
        let hostile = SOUND_SOURCES.iter().position(|n| *n == "hostile").unwrap();
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let slider = widgets
            .iter()
            .find(|wd| matches!(wd.kind, WKind::Slider { opt: Opt::Volume(i), .. } if i == hostile))
            .unwrap();
        let y = slider.y + 1.0;
        let x = slider.x + 4.0 + (slider.w - 8.0) * 0.5;
        let events = ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false);
        let volume = s.volumes[hostile];
        assert!((volume - 0.5).abs() < 1e-3, "{volume}");
        assert_eq!(
            events,
            vec![Event::SliderDrag, Event::Volume(hostile, volume)]
        );
        // OptionInstance.set: an unchanged value runs no callback.
        assert_eq!(
            ui.click(&mut s, &atlas, x, -50.0, size, [854, 480], true, false),
            vec![Event::SliderDrag]
        );
    }

    #[test]
    fn classic_sliders_follow_the_pointer_and_snap_on_release() {
        let atlas = atlas();
        let size = (480.0, 320.0);
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Sound);
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let slider = widgets
            .iter()
            .find(|wd| {
                matches!(
                    wd.kind,
                    WKind::Slider {
                        opt: Opt::Volume(0),
                        ..
                    }
                )
            })
            .unwrap();
        let (x, y) = (slider.x + 4.0 + (slider.w - 8.0) * 0.3, slider.y + 1.0);
        ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false);
        let raw = ui::slider_value(slider.x, slider.w, x);
        // Held: the handle stays under the pointer whatever the stepped value.
        assert_eq!(ui.slider_shown(Opt::Volume(0), 0.25), raw);
        assert_eq!(ui.slider_shown(Opt::Volume(1), 0.25), 0.25);
        ui.click(
            &mut s,
            &atlas,
            x + 10.0,
            -50.0,
            size,
            [854, 480],
            true,
            false,
        );
        let raw = ui::slider_value(slider.x, slider.w, x + 10.0);
        assert_eq!(ui.slider_shown(Opt::Volume(0), 0.25), raw);
        ui.release();
        assert_eq!(ui.slider_shown(Opt::Volume(0), 0.25), 0.25);
        // Render distance applies late (applyValueImmediately = false) and
        // keeps its position until then, even once released.
        ui.slider_value = Some((Opt::RenderDistance, 0.51, Instant::now()));
        assert_eq!(ui.slider_shown(Opt::RenderDistance, 0.5), 0.51);
        ui.slider_value = Some((
            Opt::RenderDistance,
            0.51,
            Instant::now() - std::time::Duration::from_millis(600),
        ));
        assert_eq!(ui.slider_shown(Opt::RenderDistance, 0.5), 0.5);
    }

    #[test]
    fn classic_language_rows_and_text_fields_are_silent() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi {
            languages: languages(),
            ..Default::default()
        };
        ui.show(Page::Language);
        let size = (480.0, 320.0);
        let (widgets, top, bottom, _) = ui.layout(&s, size.0, size.1);
        let row = widgets
            .iter()
            .find(|wd| {
                matches!(wd.kind, WKind::LanguageRow { .. }) && wd.y >= top && wd.y + wd.h <= bottom
            })
            .unwrap();
        assert!(
            ui.click(
                &mut s,
                &atlas,
                row.x + 1.0,
                row.y + 1.0,
                size,
                [854, 480],
                false,
                false
            )
            .is_empty()
        );
        assert!(ui.release().is_empty());
        let search = ui.language_search.clone();
        assert!(
            ui.click(
                &mut s,
                &atlas,
                search.x + 1.0,
                search.y + 1.0,
                size,
                [854, 480],
                false,
                false
            )
            .is_empty()
        );
    }

    /// `LanguageSelectionList.Entry.keyPressed`: the selection key picks the
    /// row's language and finishes the screen in one press (`onDone`), so the
    /// page is gone after Enter and the language is already applied.
    #[test]
    fn the_selection_key_picks_a_language_and_finishes_the_page() {
        use winit::keyboard::KeyCode;
        let atlas = atlas();
        let rows = languages();
        let code = {
            let s = Settings::default();
            let mut ui = OptionsUi {
                languages: rows.clone(),
                ..Default::default()
            };
            ui.size = (480.0, 320.0);
            ui.show(Page::Language);
            let (widgets, ..) = ui.layout(&s, 480.0, 320.0);
            widgets
                .iter()
                .find_map(|wd| match &wd.kind {
                    WKind::LanguageRow { code, .. } => Some(code.clone()),
                    _ => None,
                })
                .expect("a language row")
        };
        for key in [KeyCode::Enter, KeyCode::NumpadEnter, KeyCode::Space] {
            let mut s = Settings::default();
            assert_ne!(s.language, code, "the row starts unchosen");
            let mut ui = OptionsUi {
                languages: rows.clone(),
                ..Default::default()
            };
            ui.size = (480.0, 320.0);
            ui.show(Page::Language);
            ui.focus = Some(FocusId::Lang(code.clone()));
            ui.focus_from_keyboard = true;
            let events = ui.key(&mut s, &atlas, key, None);
            assert_eq!(s.language, code, "{key:?} applies the row's language");
            assert!(events.contains(&Event::Language(code.clone())), "{key:?}");
            assert_eq!(ui.page(), Page::Main, "{key:?} finishes the language page");
        }
    }

    #[test]
    fn language_search_hint_does_not_overlap_the_focused_caret() {
        let atlas = atlas();
        let settings = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Language);
        let draw_hint = |ui: &mut OptionsUi| {
            let mut batch = HudBatch::scratch([960, 640]);
            ui.draw(&settings, NONE, &mut batch, &atlas, (-1.0, -1.0));
            let search = &ui.language_search;
            batch.vertices.iter().any(|v| {
                v.color == rgba(170, 170, 170, 255)
                    && v.pos[0] >= search.text_x()
                    && v.pos[0] < search.x + search.width
                    && v.pos[1] >= search.text_y()
                    && v.pos[1] < search.y + search.height
            })
        };
        ui.language_search.focused = false;
        assert!(draw_hint(&mut ui));
        ui.language_search.focused = true;
        assert!(!draw_hint(&mut ui));
        ui.language_search.focused = false;
        ui.language_search.set_value("english");
        assert!(!draw_hint(&mut ui));
        ui.language_search.set_value("");
        assert!(draw_hint(&mut ui));
    }

    fn languages() -> Vec<LanguageInfo> {
        (0..30)
            .map(|i| LanguageInfo {
                code: format!("language_{i}"),
                name: format!("Language {i}"),
                region: String::new(),
                bidirectional: false,
            })
            .collect()
    }

    #[test]
    fn classic_scroll_stops_at_both_limits_before_layout() {
        let s = Settings::default();
        let size = (480.0, 120.0);
        for page in [Page::KeyBinds, Page::Chat, Page::Sound, Page::Language] {
            let mut ui = OptionsUi {
                languages: languages(),
                ..Default::default()
            };
            ui.show(page);
            let (_, top, bottom, content) = ui.layout(&s, size.0, size.1);
            let max = content - (bottom - top);
            assert!(max > 4.5, "{page:?}");
            for _ in 0..3 {
                ui.scroll_at(&s, size, (240.0, 60.0), 1000.0);
                assert_eq!(ui.scroll, 0.0, "{page:?}");
            }
            for _ in 0..3 {
                ui.scroll_at(&s, size, (240.0, 60.0), -1000.0);
                assert_eq!(ui.scroll, max, "{page:?}");
            }
            ui.scroll_at(&s, size, (240.0, 60.0), 0.25);
            assert_eq!(ui.scroll, max - 4.5, "{page:?}");
            ui.scroll_at(&s, size, (240.0, 60.0), 1000.0);
            ui.scroll_at(&s, size, (240.0, 60.0), -0.25);
            assert_eq!(ui.scroll, 4.5, "{page:?}");
        }
    }

    #[test]
    fn partially_visible_keybind_rows_are_clipped_and_clickable() {
        let mut atlas = atlas();
        let button = crate::hud::Rect {
            u0: 0.25,
            v0: 0.25,
            u1: 0.5,
            v1: 0.5,
            w: 20,
            h: 20,
        };
        for name in ["button", "button_disabled", "button_highlighted"] {
            atlas
                .sprites
                .insert(format!("gui/sprites/widget/{name}"), button);
        }
        for above in [true, false] {
            let mut s = Settings::default();
            let mut ui = OptionsUi::default();
            ui.show(Page::KeyBinds);
            let mut batch = HudBatch::scratch([480, 240]);
            let size = (batch.width, batch.height);
            let (widgets, top, bottom, _) = ui.layout(&s, size.0, size.1);
            let row = widgets
                .iter()
                .filter(|w| {
                    matches!(
                        w.kind,
                        WKind::Button {
                            act: Act::Key(_),
                            ..
                        }
                    )
                })
                .nth(10)
                .unwrap();
            let WKind::Button {
                act: Act::Key(key), ..
            } = row.kind
            else {
                unreachable!()
            };
            let row_y = if above { top - 5.0 } else { bottom - 16.0 };
            ui.scroll = row.y - row_y;
            let (x, width, height) = (row.x, row.w, row.h);
            ui.draw(&s, NONE, &mut batch, &atlas, (-1.0, -1.0));
            let vertices: Vec<_> = batch
                .vertices
                .iter()
                .filter(|v| {
                    (0.25..=0.5).contains(&v.uv[0])
                        && (0.25..=0.5).contains(&v.uv[1])
                        && v.pos[0] >= x
                        && v.pos[0] <= x + width
                        && v.pos[1] >= row_y
                        && v.pos[1] <= row_y + height
                })
                .collect();
            assert!(!vertices.is_empty());
            let lo = vertices
                .iter()
                .map(|v| v.pos[1])
                .fold(f32::INFINITY, f32::min);
            let hi = vertices
                .iter()
                .map(|v| v.pos[1])
                .fold(f32::NEG_INFINITY, f32::max);
            assert_eq!(lo, row_y.max(top));
            assert_eq!(hi, (row_y + height).min(bottom));
            let hidden_y = if above {
                row_y + 1.0
            } else {
                row_y + height - 1.0
            };
            assert!(
                ui.click(
                    &mut s,
                    &atlas,
                    x + 1.0,
                    hidden_y,
                    size,
                    [480, 240],
                    false,
                    false
                )
                .is_empty()
            );
            assert_eq!(ui.selected_key, None);
            assert_eq!(
                ui.click(
                    &mut s,
                    &atlas,
                    x + 1.0,
                    (lo + hi) / 2.0,
                    size,
                    [480, 240],
                    false,
                    false
                ),
                vec![Event::ButtonClick]
            );
            assert_eq!(ui.selected_key, Some(key));
        }
    }

    #[test]
    fn classic_scrollbar_drag_is_bounded_silent_and_stops_on_release() {
        let atlas = atlas();
        let size = (480.0, 160.0);
        for page in [Page::KeyBinds, Page::Language, Page::Chat] {
            let mut s = Settings::default();
            let mut ui = OptionsUi {
                languages: languages(),
                ..Default::default()
            };
            ui.show(page);
            let (_, top, bottom, content) = ui.layout(&s, size.0, size.1);
            let max = content - (bottom - top);
            assert!(max > 0.0, "{page:?}");
            let x = ui.scrollbar_x(size.0) + 2.0;
            let (_, thumb) = ui::scrollbar_thumb(top, bottom, content, ui.scroll).unwrap();
            let y = top + thumb / 2.0;
            assert!(
                ui.click(&mut s, &atlas, x, y, size, [960, 480], false, true)
                    .is_empty()
            );
            assert_eq!(ui.scrollbar_drag_y, None);
            assert!(
                ui.click(&mut s, &atlas, x, y, size, [960, 480], false, false)
                    .is_empty()
            );
            assert_eq!(ui.scrollbar_drag_y, Some(y));
            let delta = 5.0;
            assert!(
                ui.click(
                    &mut s,
                    &atlas,
                    x - 100.0,
                    y + delta,
                    size,
                    [960, 480],
                    true,
                    false
                )
                .is_empty()
            );
            assert!((ui.scroll - delta * max / (bottom - top - thumb)).abs() < 0.001);
            for (y, expected) in [(bottom + 100.0, max), (top - 100.0, 0.0)] {
                assert!(
                    ui.click(&mut s, &atlas, x, y, size, [960, 480], true, false)
                        .is_empty()
                );
                assert!((ui.scroll - expected).abs() < 0.001, "{page:?}");
            }
            assert!(ui.release().is_empty());
            assert_eq!(ui.scrollbar_drag_y, None);
            assert!(
                ui.click(
                    &mut s,
                    &atlas,
                    x,
                    bottom + 100.0,
                    size,
                    [960, 480],
                    true,
                    false
                )
                .is_empty()
            );
            assert_eq!(ui.scroll, 0.0);
            assert_eq!(s.keys, Settings::default().keys);
            ui.click(&mut s, &atlas, x, top + 1.0, size, [960, 480], false, false);
            assert!(ui.scrollbar_drag_y.is_some());
            ui.show(Page::Mouse);
            assert_eq!(ui.scrollbar_drag_y, None);
        }
    }

    #[test]
    fn extras_and_vanilla_bindings_accept_mouse_buttons() {
        use crate::extras::{Extras, SORT_KEY};
        use winit::event::MouseButton;
        for button in [
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::Back,
            MouseButton::Forward,
        ] {
            for name in [SORT_KEY, "key.jump"] {
                let mut ui = OptionsUi::default();
                let mut s = Settings::default();
                ui.selected_key = Some(name);
                assert_eq!(ui.bind_mouse(&mut s, button), vec![Event::Changed]);
                assert_eq!(ui.selected_key, None);
                let value = keybinds::mouse_name(button);
                match name {
                    SORT_KEY => assert_eq!(s.extras.sort_key, value),
                    _ => assert_eq!(s.keys.get(name), value),
                }
                assert_eq!(Extras::parse(&s.extras.serialize()), s.extras);
                if name != "key.jump" {
                    assert_eq!(s.keys, Settings::default().keys);
                }
                assert!(ui.bind_mouse(&mut s, button).is_empty());
            }
        }
    }

    #[test]
    fn horizon_extra_mouse_binding_consumes_the_click() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Extras);
        for (right, expected) in [(false, "key.mouse.left"), (true, "key.mouse.right")] {
            ui.selected_key = Some(crate::extras::SORT_KEY);
            assert_eq!(
                ui.click(
                    &mut s,
                    &atlas,
                    2.0,
                    2.0,
                    (480.0, 240.0),
                    [960, 480],
                    false,
                    right
                ),
                vec![Event::Changed]
            );
            assert_eq!(s.extras.sort_key, expected);
            assert_eq!(ui.selected_key, None);
            assert_eq!(ui.page(), Page::Extras);
        }
    }

    #[test]
    fn key_rebinding_takes_precedence_over_scrollbar_dragging() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::KeyBinds);
        ui.selected_key = Some("key.jump");
        let x = ui.scrollbar_x(480.0) + 2.0;
        assert_eq!(
            ui.click(
                &mut s,
                &atlas,
                x,
                50.0,
                (480.0, 240.0),
                [960, 480],
                false,
                false
            ),
            vec![Event::Changed]
        );
        assert_eq!(s.keys.get("key.jump"), "key.mouse.left");
        assert_eq!(ui.scrollbar_drag_y, None);
        assert!(ui.release().is_empty());
    }

    #[test]
    fn classic_pages_without_overflow_do_not_scroll() {
        let s = Settings::default();
        for page in [Page::Main, Page::Font, Page::Mouse] {
            let mut ui = OptionsUi::default();
            ui.show(page);
            for amount in [-1000.0, 1000.0, -0.25] {
                ui.scroll(&s, (480.0, 720.0), amount);
                assert_eq!(ui.scroll, 0.0, "{page:?}");
            }
        }
    }

    #[test]
    fn classic_resize_positions_rows_at_the_clamped_offset() {
        let s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::KeyBinds);
        ui.scroll(&s, (480.0, 120.0), -1000.0);
        assert!(ui.scroll > 0.0);
        let (widgets, _, _, _) = ui.layout(&s, 480.0, 4000.0);
        assert_eq!(ui.scroll, 0.0);
        let (expected, _, _, _) = ui.layout(&s, 480.0, 4000.0);
        assert_eq!(
            widgets.iter().map(|w| w.y).collect::<Vec<_>>(),
            expected.iter().map(|w| w.y).collect::<Vec<_>>()
        );
    }

    #[test]
    fn language_centering_and_filtering_use_bounded_row_positions() {
        let s = Settings {
            language: "language_29".into(),
            ..Default::default()
        };
        let mut ui = OptionsUi {
            languages: languages(),
            ..Default::default()
        };
        ui.show(Page::Language);
        let (widgets, top, bottom, content) = ui.layout(&s, 480.0, 240.0);
        assert_eq!(ui.scroll, content - (bottom - top));
        let (expected, _, _, _) = ui.layout(&s, 480.0, 240.0);
        assert_eq!(
            widgets.iter().map(|w| w.y).collect::<Vec<_>>(),
            expected.iter().map(|w| w.y).collect::<Vec<_>>()
        );
        ui.language_search.set_value("language_29");
        let (widgets, top, _, _) = ui.layout(&s, 480.0, 240.0);
        assert_eq!(ui.scroll, 0.0);
        assert_eq!(widgets.iter().find(|w| w.list).unwrap().y, top + 2.0);
    }

    #[test]
    fn title_shortcuts_initialize_pages_and_return_directly_to_the_title() {
        let mut ui = OptionsUi::default();
        let mut s = Settings::default();
        for page in [Page::Accessibility, Page::Language] {
            ui.language_choice = Some("de_de".into());
            ui.language_search.set_value("old query");
            ui.open_at(false, page);
            assert_eq!(ui.page(), page);
            assert!(!ui.in_world);
            if page == Page::Language {
                assert_eq!(ui.language_choice, None);
                assert!(ui.language_search.value.is_empty());
                assert!(ui.center_language);
            }
            assert!(ui.back(&mut s).contains(&Event::Close));
            assert_eq!(s.language, "en_us");
            ui.open(false);
            ui.show(page);
            assert!(!ui.back(&mut s).contains(&Event::Close));
            assert_eq!(ui.page(), Page::Main);
        }
        ui.open_at(false, Page::Language);
        ui.language_choice = Some("de_de".into());
        let events = ui.back(&mut s);
        assert!(events.contains(&Event::Language("de_de".into())));
        assert!(events.contains(&Event::Close));
        assert_eq!(s.language, "de_de");
    }

    /// An Extras or PvP & HUD label wider than half a row takes the whole row, so every label
    /// fits its button: "Keep channels for UI sounds: Off" did not.
    #[test]
    fn pvp_labels_fit_their_buttons() {
        // Vanilla's font: 6 px a glyph, a few narrower ones.
        let width = |t: &str| {
            t.chars()
                .map(|c| match c {
                    'i' | '\'' | ':' | '.' | ',' | '!' => 2.0,
                    'l' => 3.0,
                    ' ' | 't' | 'I' => 4.0,
                    'f' | 'k' => 5.0,
                    _ => 6.0,
                })
                .sum::<f32>()
        };
        let s = Settings::default();
        let mut ui = OptionsUi::default();
        {
            ui.stack = vec![Page::Main, Page::Pvp];
            ui.wide_extras = wide_extras(&s, width);
            let rows = ui.list_rows(&s);
            let buttons: Vec<&Widget> = rows
                .iter()
                .flat_map(|r| match r {
                    Row::Buttons(a, b) => std::iter::once(a).chain(b.as_ref()).collect(),
                    _ => Vec::new(),
                })
                .collect();
            for b in &buttons {
                let WKind::Button { label, .. } = &b.kind else {
                    continue;
                };
                let label = label.plain();
                assert!(width(&label) <= b.w - 4.0, "{label} on {}", b.w);
            }
            // Short ones keep vanilla's two to a row.
            assert!(buttons.iter().any(|b| b.w == 150.0));
        }
    }

    /// Every setting a player reaches, from the Options screen through every button that opens
    /// a page, is in one place, and every page has one way in. Vanilla's own repeats stay as
    /// vanilla has them; the graphics screen stands for Video Settings, so the vanilla video
    /// options it shows count as Video Settings' (Menu Background Blur is in both of
    /// vanilla's). Vanilla's Video Settings rows are all on the graphics screen.
    #[test]
    fn every_setting_and_page_is_reached_from_one_place() {
        use std::collections::{BTreeMap, BTreeSet};
        let mut s = Settings::default();
        // The Music page shows once its extra is on.
        s.extras.music_page = true;
        let mut ui = OptionsUi::default();
        let mut places: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut ways_in: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut put = |id: String, page: &str| {
            places.entry(id).or_default().insert(page.to_owned());
        };
        // The graphics screen, drawn by `graphics_ui`.
        let mut graphics_rows = BTreeSet::new();
        for (page, groups) in crate::graphics_ui::rows() {
            for row in groups.into_iter().flat_map(|(_, rows)| rows) {
                use crate::graphics_ui::{H, Item};
                let id = match row.item {
                    // Vanilla's Graphics API button sets the same setting.
                    Item::Horizon(H::GraphicsApi) => "graphics-api".to_owned(),
                    Item::Horizon(H::ShaderPacks) => {
                        ways_in
                            .entry(format!("{:?}", Page::ShaderPacks))
                            .or_default()
                            .insert(format!("Graphics/{page:?}"));
                        continue;
                    }
                    Item::Vanilla(o) => format!("{o:?}"),
                    Item::Horizon(h) => format!("graphics:{h:?}"),
                    Item::Extra(e) => format!("extra:{e:?}"),
                };
                graphics_rows.insert(id.clone());
                put(id, "Graphics");
            }
        }
        // Every page from the Options screen, through its buttons.
        let mut seen = BTreeSet::new();
        let mut todo = vec![Page::Main];
        while let Some(page) = todo.pop() {
            if !seen.insert(format!("{page:?}")) {
                continue;
            }
            ui.stack = vec![page];
            let (widgets, _, _, _) = ui.layout(&s, 427.0, 4000.0);
            let name = format!("{page:?}");
            for w in &widgets {
                let id = match &w.kind {
                    WKind::Slider { opt, .. } => format!("{opt:?}"),
                    WKind::Button { act, .. } => match act {
                        Act::Cycle(Opt::GraphicsBackend) => "graphics-api".to_owned(),
                        Act::Cycle(opt) => format!("{opt:?}"),
                        Act::Extra(e) => format!("extra:{e:?}"),
                        Act::PvpPreset => "pvp-preset".to_owned(),
                        Act::Push(to) => {
                            ways_in
                                .entry(format!("{to:?}"))
                                .or_default()
                                .insert(name.clone());
                            // The graphics screen's pages are one screen, listed above.
                            if OptionsUi::graphics_page(*to).is_none() {
                                todo.push(*to);
                            }
                            continue;
                        }
                        _ => continue,
                    },
                    _ => continue,
                };
                put(id, &name);
            }
        }
        // Vanilla's own: the same option on two of its screens.
        let vanilla_repeats: BTreeMap<&str, &[&str]> = [
            ("MenuBlur", &["Accessibility", "Graphics"][..]),
            ("ShowSubtitles", &["Accessibility", "Sound"][..]),
            ("Narrator", &["Accessibility", "Chat"][..]),
            ("ChatOpacity", &["Accessibility", "Chat"][..]),
            ("TextBackgroundOpacity", &["Accessibility", "Chat"][..]),
            ("ChatLineSpacing", &["Accessibility", "Chat"][..]),
        ]
        .into();
        for (id, pages) in &places {
            let pages: Vec<&str> = pages.iter().map(String::as_str).collect();
            match vanilla_repeats.get(id.as_str()) {
                Some(expected) => assert_eq!(&pages, expected, "{id}"),
                None => assert_eq!(pages.len(), 1, "{id} is on {pages:?}"),
            }
        }
        // AccessibilityOptionsScreen links Controls, as vanilla's does.
        for (page, from) in &ways_in {
            let from: Vec<&str> = from.iter().map(String::as_str).collect();
            match page.as_str() {
                "Controls" => assert_eq!(from, ["Accessibility", "Main"]),
                _ => assert_eq!(from.len(), 1, "{page} opens from {from:?}"),
            }
        }
        assert_eq!(
            ways_in.get("ShaderPacks").map(|f| f.len()),
            Some(1),
            "{ways_in:?}"
        );
        assert!(
            !seen.contains("Video"),
            "vanilla's own layout has no way in"
        );
        // Every row of vanilla's Video Settings is on the graphics screen.
        ui.stack = vec![Page::Video];
        for row in ui.list_rows(&s) {
            let opts = match row {
                Row::Big(o) => vec![o],
                Row::Pair(a, b) => std::iter::once(a).chain(b).collect(),
                _ => Vec::new(),
            };
            for o in opts {
                let id = match o {
                    Opt::GraphicsBackend => "graphics-api".to_owned(),
                    o => format!("{o:?}"),
                };
                // macOS has one graphics API: the graphics screen leaves the row out.
                if id == "graphics-api" && cfg!(target_os = "macos") {
                    continue;
                }
                assert!(
                    graphics_rows.contains(&id),
                    "{id} is not on the graphics screen"
                );
            }
        }
    }

    #[test]
    fn horizon_settings_hang_off_options_and_reach_every_horizon_page() {
        let s = Settings::default();
        let mut ui = OptionsUi::default();
        let pushes = |ui: &mut OptionsUi| -> Vec<Page> {
            let (widgets, _, _, _) = ui.layout(&s, 427.0, 240.0);
            widgets
                .iter()
                .filter_map(|w| match &w.kind {
                    WKind::Button {
                        act: Act::Push(page),
                        active: true,
                        ..
                    } => Some(*page),
                    _ => None,
                })
                .collect()
        };
        // Vanilla's Options buttons, Horizon's two in the Telemetry and Credits slots (Mods is
        // Starling's Mods page), then Credits & Attribution (WinScreen's credits).
        let main = pushes(&mut ui);
        assert_eq!(
            &main[main.len() - 3..],
            [Page::Horizon, Page::NativeGame, Page::Credits]
        );
        // Video Settings... is the graphics screen, as Sodium's.
        assert!(main.contains(&Page::Graphics) && !main.contains(&Page::Video));
        ui.push(Page::Horizon);
        assert_eq!(
            pushes(&mut ui),
            [
                Page::Extras,
                Page::Pvp,
                Page::SplitSetup,
                Page::Studio,
                Page::Agent,
                Page::ModBrowser,
            ]
        );
        // No AI hides the AI Agent page.
        let mut no_ai = Settings::default();
        no_ai.extras.no_ai = true;
        let (widgets, _, _, _) = ui.layout(&no_ai, 427.0, 240.0);
        assert!(!widgets.iter().any(|w| matches!(
            &w.kind,
            WKind::Button {
                act: Act::Push(Page::Agent),
                ..
            }
        )));
        // The AI Agent page: the switch, Copy and the status the game sets.
        ui.push(Page::Agent);
        ui.agent_status = vec!["Waiting for an agent.".into()];
        let (widgets, _, _, _) = ui.layout(&s, 427.0, 240.0);
        let acts: Vec<&Act> = widgets
            .iter()
            .filter_map(|w| match &w.kind {
                WKind::Button { act, .. } => Some(act),
                _ => None,
            })
            .collect();
        assert!(acts.contains(&&Act::Extra(crate::extras::Item::AgentControl)));
        assert!(acts.contains(&&Act::CopyAgentDetails));
        ui.stack = vec![Page::Main, Page::Controls];
        assert!(pushes(&mut ui).contains(&Page::Controller));
    }

    #[test]
    fn custom_buttons_take_a_line_or_a_key_and_empty_ones_go() {
        use crate::settings::CustomAction;
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi {
            stack: vec![Page::Main, Page::CustomButtons],
            ..Default::default()
        };
        let size = (427.0, 480.0);
        let press = |ui: &mut OptionsUi, s: &mut Settings, wanted: Act| {
            let (widgets, ..) = ui.layout(s, size.0, size.1);
            let at = widgets
                .iter()
                .find_map(|w| match &w.kind {
                    WKind::Button { act, .. } if *act == wanted => Some((w.x + 2.0, w.y + 2.0)),
                    _ => None,
                })
                .expect("the button shows");
            ui.click(s, &atlas, at.0, at.1, size, [1281, 960], false, false)
        };
        // Added and left empty, it goes.
        press(&mut ui, &mut s, Act::CustomAdd);
        assert_eq!(ui.page(), Page::CustomButton(0));
        ui.layout(&s, size.0, size.1);
        assert!(ui.back(&mut s).contains(&Event::CustomButtons));
        assert!(s.custom_buttons.is_empty());
        // What the box holds, it sends.
        press(&mut ui, &mut s, Act::CustomAdd);
        ui.layout(&s, size.0, size.1);
        let field = (ui.custom_send.x + 10.0, ui.custom_send.y + 4.0);
        ui.click(
            &mut s,
            &atlas,
            field.0,
            field.1,
            size,
            [1281, 960],
            false,
            false,
        );
        assert!(ui.text_focused() && ui.text_field_at(field.0, field.1));
        ui.type_text(&atlas, "/home");
        ui.back(&mut s);
        assert_eq!(
            s.custom_buttons[0].action,
            CustomAction::Send("/home".into())
        );
        assert_eq!(s.custom_buttons[0].label(), "/home");
        // A key takes its place, and stays without a line.
        let id = s.custom_buttons[0].id;
        press(&mut ui, &mut s, Act::CustomEdit(id));
        assert_eq!(ui.custom_send.value, "/home");
        ui.push(Page::CustomPick(id, CustomPick::Control));
        press(&mut ui, &mut s, Act::CustomKey(id, "key.sprint"));
        assert_eq!(ui.page(), Page::CustomButton(id));
        assert_eq!(
            s.custom_buttons[0].action,
            CustomAction::Key("key.sprint".into())
        );
        // Any key of the keyboard, F1 first among them.
        ui.push(Page::CustomPick(id, CustomPick::Keyboard));
        press(&mut ui, &mut s, Act::CustomInput(id, "key.keyboard.f1"));
        assert_eq!(
            s.custom_buttons[0].action,
            CustomAction::Input("key.keyboard.f1".into())
        );
        assert_eq!(
            s.custom_buttons[0].label(),
            keybinds::display_name("key.keyboard.f1").plain()
        );
        // An icon, and back to none.
        ui.push(Page::CustomPick(id, CustomPick::Icon));
        press(&mut ui, &mut s, Act::CustomIcon(id, Some("star")));
        assert_eq!(s.custom_buttons[0].icon.as_deref(), Some("star"));
        ui.push(Page::CustomPick(id, CustomPick::Icon));
        press(&mut ui, &mut s, Act::CustomIcon(id, None));
        assert_eq!(s.custom_buttons[0].icon, None);
        // Its own image: a stroke across the canvas, with the swatch picked first, is kept by
        // Done; a canvas nobody touched changes nothing.
        press(&mut ui, &mut s, Act::CustomDraw(id));
        assert!(ui.painting());
        assert!(
            !ui.back(&mut s)
                .iter()
                .any(|e| matches!(e, Event::CustomImage(..)))
        );
        assert!(!s.custom_buttons[0].image);
        press(&mut ui, &mut s, Act::CustomDraw(id));
        ui.layout(&s, size.0, size.1);
        let [left, top, cell] = ui.painter.at;
        let (sx, sy) = ui.painter.swatch(4);
        let tap = |ui: &mut OptionsUi, s: &mut Settings, x: f32, y: f32, drag: bool| {
            ui.click(s, &atlas, x, y, size, [1281, 960], drag, false)
        };
        tap(&mut ui, &mut s, sx + 2.0, sy + 2.0, false);
        assert_eq!(ui.painter.color, 4);
        let at = |c: f32| c * cell + cell / 2.0;
        tap(&mut ui, &mut s, left + at(1.0), top + at(1.0), false);
        // Fast: the pixels between are painted too.
        tap(&mut ui, &mut s, left + at(9.0), top + at(1.0), true);
        let pixel = |ui: &OptionsUi, x: usize, y: usize| {
            let at = (y * own_buttons::SIZE as usize + x) * 4;
            [ui.painter.pixels[at], ui.painter.pixels[at + 3]]
        };
        assert_eq!(pixel(&ui, 5, 1), [PALETTE[4][0], 255]);
        let kept = ui.back(&mut s);
        assert!(
            kept.iter()
                .any(|e| matches!(e, Event::CustomImage(i, p) if *i == id && !p.is_empty()))
        );
        assert!(s.custom_buttons[0].image);
        // An icon picked after it takes its place.
        ui.push(Page::CustomPick(id, CustomPick::Icon));
        press(&mut ui, &mut s, Act::CustomIcon(id, Some("star")));
        assert!(!s.custom_buttons[0].image);
        ui.back(&mut s);
        assert_eq!(
            s.custom_buttons[0].action,
            CustomAction::Input("key.keyboard.f1".into())
        );
        // And it can be taken away.
        assert!(press(&mut ui, &mut s, Act::CustomDelete(id)).contains(&Event::CustomButtons));
        assert!(s.custom_buttons.is_empty());
    }

    #[test]
    fn a_selected_profile_prevents_the_legacy_skin_page_from_changing_its_name() {
        let atlas = atlas();
        let mut settings = Settings::default();
        let mut ui = OptionsUi {
            skin_page: true,
            profile_name: Some("TabletAlpha".into()),
            ..Default::default()
        };
        ui.stack.push(Page::Skin);
        ui.skin.saved = "OldAlias".into();
        ui.skin.name.set_value("OldAlias");
        ui.skin.name.focused = true;
        assert!(!ui.text_focused());
        assert!(!ui.text_field_at(ui.skin.name.x, ui.skin.name.y));
        ui.type_text(&atlas, "X");
        ui.key(
            &mut settings,
            &atlas,
            winit::keyboard::KeyCode::KeyA,
            Some("X"),
        );
        ui.commit_name();
        assert_eq!(ui.skin.name.value, "OldAlias");
        assert!(ui.take_skin_actions().is_empty());
    }

    #[test]
    fn the_skin_page_takes_a_name_and_cycles_the_skins() {
        use winit::keyboard::KeyCode;
        let atlas = atlas();
        let mut s = Settings::default();
        // The game's own options; on the stack without `push`, which reads the page from beside
        // options.txt.
        let mut ui = OptionsUi {
            skin_page: true,
            ..Default::default()
        };
        ui.stack.push(Page::Skin);
        ui.skin.saved = "Player00042".into();
        ui.skin.name.set_value("Player00042");
        ui.skin.files = vec!["alex.png".into(), "steve.png".into()];
        let size = (427.0, 240.0);
        let click = |ui: &mut OptionsUi, s: &mut Settings, (x, y): (f32, f32)| {
            ui.click(s, &atlas, x, y, size, [1281, 720], false, false)
        };
        let find = |ui: &mut OptionsUi, wanted: Act| {
            let (widgets, ..) = ui.layout(&Settings::default(), size.0, size.1);
            widgets
                .iter()
                .find_map(|w| match &w.kind {
                    WKind::Button { act, active, .. } if *act == wanted => {
                        Some(((w.x + 2.0, w.y + 2.0), *active))
                    }
                    _ => None,
                })
                .unwrap()
        };
        // The default skin's model is the default's.
        assert!(!find(&mut ui, Act::SkinModel).1);
        // At the box's right end: the cursor after the text.
        let field = (ui.skin.name.x + 260.0, ui.skin.name.y + 4.0);
        assert!(click(&mut ui, &mut s, field).is_empty());
        assert!(ui.text_focused() && ui.text_field_at(field.0, field.1));
        let retype = |ui: &mut OptionsUi, s: &mut Settings, text: &str| {
            for _ in 0..16 {
                ui.key(s, &atlas, KeyCode::Backspace, None);
            }
            for c in text.chars() {
                ui.key(s, &atlas, KeyCode::KeyA, Some(&c.to_string()));
            }
            ui.key(s, &atlas, KeyCode::Enter, None)
        };
        // Refused: the old name back, a note.
        retype(&mut ui, &mut s, "no");
        assert!(ui.take_skin_actions().is_empty());
        assert_eq!(ui.skin.name.value, "Player00042");
        assert!(!ui.text_focused());
        assert!(ui.skin.warning.as_deref().unwrap().contains("Player00042"));
        click(&mut ui, &mut s, field);
        retype(&mut ui, &mut s, "Steve_2");
        assert_eq!(ui.take_skin_actions(), [SkinAction::Name("Steve_2".into())]);
        assert_eq!(ui.skin.warning, None);
        // Unchanged, nothing to keep; leaving the box by a press elsewhere keeps a new one.
        click(&mut ui, &mut s, field);
        ui.key(&mut s, &atlas, KeyCode::Enter, None);
        assert!(ui.take_skin_actions().is_empty());
        click(&mut ui, &mut s, field);
        ui.type_text(&atlas, "X");
        click(&mut ui, &mut s, (1.0, 1.0));
        assert_eq!(
            ui.take_skin_actions(),
            [SkinAction::Name("Steve_2X".into())]
        );
        // Default, each skin, Default again; the model once there is a skin.
        let next = find(&mut ui, Act::NextSkin).0;
        let mut choice_after = |ui: &mut OptionsUi, at: (f32, f32)| {
            assert!(click(ui, &mut s, at).contains(&Event::ButtonClick));
            match ui.take_skin_actions().as_slice() {
                [SkinAction::Choice(choice)] => choice.clone(),
                other => panic!("{other:?}"),
            }
        };
        let alex = choice_after(&mut ui, next);
        assert_eq!(alex.file.as_deref(), Some("alex.png"));
        let (model, active) = find(&mut ui, Act::SkinModel);
        assert!(active);
        assert!(choice_after(&mut ui, model).slim);
        assert_eq!(
            choice_after(&mut ui, next).file.as_deref(),
            Some("steve.png")
        );
        assert_eq!(choice_after(&mut ui, next).file, None);
        let (folder, active) = find(&mut ui, Act::OpenSkinFolder);
        assert!(active);
        click(&mut ui, &mut s, folder);
        assert_eq!(ui.take_skin_actions(), [SkinAction::OpenFolder]);
        // The layers and Main Hand follow the skin.
        find(&mut ui, Act::Cycle(Opt::MainHand));
        // The menus before the game open the page too, with only the layers and Main Hand: the
        // name and skin are the game's own options'.
        let mut menu = OptionsUi::default();
        let (main, ..) = menu.layout(&s, size.0, size.1);
        assert!(main.iter().any(|w| matches!(
            &w.kind,
            WKind::Button {
                act: Act::Push(Page::Skin),
                active: true,
                ..
            }
        )));
        menu.stack.push(Page::Skin);
        let (widgets, ..) = menu.layout(&s, size.0, size.1);
        let acts: Vec<&Act> = widgets
            .iter()
            .filter_map(|w| match &w.kind {
                WKind::Button { act, .. } => Some(act),
                _ => None,
            })
            .collect();
        assert!(acts.contains(&&Act::Cycle(Opt::ModelPart(0))));
        assert!(!acts.contains(&&Act::NextSkin) && !acts.contains(&&Act::OpenSkinFolder));
    }

    #[test]
    fn an_account_profile_sends_its_skin_where_its_account_keeps_it() {
        use crate::own_skin::{Cloud, SkinChoice, Upload};
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi {
            skin_page: true,
            profile_name: Some("Notch".into()),
            skin_cloud: Cloud::Microsoft(Upload::None),
            ..Default::default()
        };
        ui.stack.push(Page::Skin);
        ui.skin.files = vec!["alex.png".into()];
        let size = (427.0, 240.0);
        let buttons = |ui: &mut OptionsUi| {
            let (widgets, ..) = ui.layout(&Settings::default(), size.0, size.1);
            widgets
                .iter()
                .filter_map(|w| match &w.kind {
                    WKind::Button { act, active, .. } => {
                        Some((act.clone(), (w.x + 2.0, w.y + 2.0), *active))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let find = |ui: &mut OptionsUi, wanted: Act| {
            buttons(ui)
                .into_iter()
                .find(|(act, ..)| *act == wanted)
                .map(|(_, at, active)| (at, active))
        };
        // The account's own skin is nothing to upload.
        assert_eq!(find(&mut ui, Act::UploadSkin).map(|b| b.1), Some(false));
        ui.skin.choice = SkinChoice {
            file: Some("alex.png".into()),
            slim: true,
        };
        let (at, active) = find(&mut ui, Act::UploadSkin).unwrap();
        assert!(active);
        ui.click(&mut s, &atlas, at.0, at.1, size, [1281, 720], false, false);
        assert_eq!(
            ui.take_skin_actions(),
            [SkinAction::Upload(ui.skin.choice.clone())]
        );
        // One upload at a time; a refusal shows in red.
        ui.skin_cloud = Cloud::Microsoft(Upload::Sending);
        assert_eq!(find(&mut ui, Act::UploadSkin).map(|b| b.1), Some(false));
        ui.skin_cloud = Cloud::Microsoft(Upload::Failed("Mojang said no".into()));
        assert!(
            ui.list_rows(&s)
                .iter()
                .any(|r| matches!(r, Row::Warning(w) if w == "Mojang said no"))
        );
        // Ely.by takes no uploads: its page instead.
        ui.skin_cloud = Cloud::Ely;
        assert_eq!(find(&mut ui, Act::UploadSkin), None);
        let (at, active) = find(&mut ui, Act::ElySkinSite).unwrap();
        assert!(active);
        ui.click(&mut s, &atlas, at.0, at.1, size, [1281, 720], false, false);
        assert_eq!(ui.take_skin_actions(), [SkinAction::ElySite]);
        // An offline profile's skin stays in this game.
        ui.skin_cloud = Cloud::Local;
        let acts: Vec<Act> = buttons(&mut ui).into_iter().map(|b| b.0).collect();
        assert!(!acts.contains(&Act::UploadSkin) && !acts.contains(&Act::ElySkinSite));
    }

    #[test]
    fn sound_device_button_in_sound_page_and_cycles() {
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Sound);
        let rows = ui.list_rows(&s);
        assert!(
            rows.iter().any(|r| matches!(r, Row::Big(Opt::SoundDevice))),
            "Sound page should contain Row::Big(Opt::SoundDevice)"
        );
        assert_eq!(s.sound_device, "");
        let devices = crate::audio::output_devices();
        let count = 1 + devices.len();
        // Cycling forward 'count' times wraps around back to the initial selection ("")
        for _ in 0..count {
            Opt::SoundDevice.cycle(&mut s, false, [854, 480]);
        }
        assert_eq!(s.sound_device, "");
        // Cycling backward 'count' times also wraps around back to ""
        for _ in 0..count {
            Opt::SoundDevice.cycle(&mut s, true, [854, 480]);
        }
        assert_eq!(s.sound_device, "");
    }

    #[test]
    fn slider_keyboard_navigation_and_selection_toggle_parity() {
        use winit::keyboard::KeyCode;
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Main);
        s.fov = 70.0;

        // 1. Tab focuses the first interactive widget: FOV slider
        ui.key(&mut s, &atlas, KeyCode::Tab, None);
        assert_eq!(ui.focus, Some(FocusId::Slider(Opt::Fov)));
        assert!(
            ui.can_change_value,
            "Tab onto slider enables can_change_value (Vanilla behavior)"
        );

        // 2. Right Arrow steps FOV up to 71
        let events = ui.key(&mut s, &atlas, KeyCode::ArrowRight, None);
        assert_eq!(s.fov, 71.0);
        assert!(events.contains(&Event::Changed));

        // 3. Left Arrow steps FOV down back to 70, then 69
        ui.key(&mut s, &atlas, KeyCode::ArrowLeft, None);
        assert_eq!(s.fov, 70.0);
        ui.key(&mut s, &atlas, KeyCode::ArrowLeft, None);
        assert_eq!(s.fov, 69.0);

        // 4. Space / Enter toggles can_change_value to false
        ui.key(&mut s, &atlas, KeyCode::Space, None);
        assert!(
            !ui.can_change_value,
            "Space toggles can_change_value to false"
        );

        // 5. With can_change_value false, Left/Right arrow keys navigate focus instead of changing value!
        ui.key(&mut s, &atlas, KeyCode::ArrowRight, None);
        assert_eq!(
            s.fov, 69.0,
            "FOV must not change when can_change_value is false"
        );
        assert_ne!(
            ui.focus,
            Some(FocusId::Slider(Opt::Fov)),
            "Focus moved to next widget horizontally"
        );

        // 6. Navigate back to FOV slider with Left Arrow
        ui.key(&mut s, &atlas, KeyCode::ArrowLeft, None);
        assert_eq!(ui.focus, Some(FocusId::Slider(Opt::Fov)));
        assert!(
            !ui.can_change_value,
            "Arrow navigation onto slider keeps can_change_value false"
        );

        // 7. Space toggles can_change_value back to true
        ui.key(&mut s, &atlas, KeyCode::Enter, None);
        assert!(ui.can_change_value);
        ui.key(&mut s, &atlas, KeyCode::ArrowRight, None);
        assert_eq!(s.fov, 70.0);

        // 8. Escape closes the options page immediately, even while a slider
        // is focused for value changes.
        let events = ui.key(&mut s, &atlas, KeyCode::Escape, None);
        assert!(!ui.can_change_value);
        assert_eq!(
            ui.page(),
            Page::Main,
            "Escape must close the options page from value edit"
        );
        assert!(events.contains(&Event::Close));
    }

    #[test]
    fn escape_closes_options_with_search_focused() {
        use winit::keyboard::KeyCode;
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Video);
        ui.focus = Some(FocusId::Search);
        ui.settings_search.focused = true;

        let events = ui.key(&mut s, &atlas, KeyCode::Escape, None);
        assert!(events.contains(&Event::Changed));
        assert_eq!(ui.page(), Page::Main);
        assert_eq!(ui.focus, None);
        assert!(!ui.settings_search.focused);
    }

    #[test]
    fn slider_mouse_wheel_scrolls_list_parity() {
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Main);
        s.fov = 70.0;
        let size: (f32, f32) = (427.0, 240.0);
        let cx = (size.0 / 2.0_f32).floor();

        // Cursor over FOV slider (cx - 154.0, 29.0, 150.0, 20.0)
        let cursor = (cx - 100.0, 35.0);
        // In vanilla parity, mouse wheel over a slider does not change the slider value.
        ui.wheel(&mut s, size, cursor, [854, 480], 1.0);
        assert_eq!(s.fov, 70.0);

        // Sound page: Master Volume slider
        ui.show(Page::Sound);
        s.master_volume = 0.5;
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let master = widgets
            .iter()
            .find(|w| {
                matches!(
                    w.kind,
                    WKind::Slider {
                        opt: Opt::Volume(0),
                        ..
                    }
                )
            })
            .unwrap();
        let cursor = (master.x + 10.0, master.y + 10.0);
        let events = ui.wheel(&mut s, size, cursor, [854, 480], 1.0);
        assert_eq!(s.master_volume, 0.5);
        assert!(!events.iter().any(|e| matches!(e, Event::Volume(0, _))));
    }

    #[test]
    fn a_finger_scrolls_from_a_cycle_button_and_drags_a_slider() {
        let s = Settings::default();
        let mut ui = OptionsUi::default();
        let size = (480.0, 600.0);
        ui.show(Page::Extras);
        // The Extras page is upstream's settings shell: its Sort switch is a row there.
        let on_sort = ui
            .extras
            .point_at(&s, size, ExtraRow::Item(crate::extras::Item::Sort));
        assert!(ui.list_button_at(&s, size, on_sort));
        ui.show(Page::Sound);
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let master = widgets
            .iter()
            .find(|w| {
                matches!(
                    w.kind,
                    WKind::Slider {
                        opt: Opt::Volume(0),
                        ..
                    }
                )
            })
            .unwrap();
        let on_master = (master.x + 10.0, master.y + 10.0);
        assert!(!ui.list_button_at(&s, size, on_master));
    }

    #[test]
    fn the_graphics_screens_wheel_scrolls_even_over_a_slider_being_changed() {
        use crate::graphics_ui::{H, Item};
        use winit::keyboard::KeyCode as K;
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Graphics);
        ui.graphics.show(crate::graphics_ui::Page::Performance);
        let size = (640.0, 360.0);
        s.horizon.chunk_updates = 16;
        let chunk_updates = Item::Horizon(H::ChunkUpdates);
        // Tab onto it: the arrows change it.
        while ui.graphics.focused_row() != Some(chunk_updates) {
            shell::Hosted::key(&mut ui.graphics, &mut s, K::Tab, None, false, size);
        }
        assert!(ui.graphics.changes_value());
        let before = ui.graphics.scroll_amount();
        ui.wheel(&mut s, size, (320.0, 150.0), [1920, 1080], -1.0);
        assert_eq!(s.horizon.chunk_updates, 16);
        assert!(ui.graphics.scroll_amount() > before);
    }

    #[test]
    fn horizon_pages_slider_and_wheel_parity() {
        use winit::keyboard::KeyCode as K;
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();

        // 1. Extras page: with the wheel extra on, the wheel on the Sort switch cycles it
        s.extras.wheel_cycles_settings = true;
        ui.show(Page::Extras);
        let size = (480.0, 600.0);
        let cursor = ui
            .extras
            .point_at(&s, size, ExtraRow::Item(crate::extras::Item::Sort));
        assert!(!s.extras.sort);
        ui.wheel(&mut s, size, cursor, [854, 480], 1.0);
        assert!(s.extras.sort);
        ui.wheel(&mut s, size, cursor, [854, 480], 1.0);
        assert!(!s.extras.sort);

        // 2. Horizon Graphics page through OptionsUi:
        // Tab onto ChunkUpdates slider, ArrowRight adjusts value, Space toggles can_change_value
        ui.show(Page::Graphics);
        ui.graphics.show(crate::graphics_ui::Page::Performance);
        s.horizon.chunk_updates = 10;
        let chunk_updates = crate::graphics_ui::Item::Horizon(crate::graphics_ui::H::ChunkUpdates);
        ui.size = size;
        ui.framebuffer = [854, 480];

        // Tab into controls
        for _ in 0..80 {
            ui.key(&mut s, &atlas, K::Tab, None);
            if ui.graphics.focused_row() == Some(chunk_updates) {
                break;
            }
        }
        assert_eq!(ui.graphics.focused_row(), Some(chunk_updates));
        assert!(ui.graphics.changes_value());

        // Right arrow steps ChunkUpdates to 11
        let events = ui.key(&mut s, &atlas, K::ArrowRight, None);
        assert_eq!(s.horizon.chunk_updates, 11);
        assert!(events.contains(&Event::SliderDrag));

        // Space toggles can_change_value to false
        ui.key(&mut s, &atlas, K::Space, None);
        assert!(!ui.graphics.changes_value());

        // ArrowRight does not change ChunkUpdates when can_change_value is false
        ui.key(&mut s, &atlas, K::ArrowRight, None);
        assert_eq!(s.horizon.chunk_updates, 11);
    }

    /// Vanilla's wheel scrolls the list (`AbstractScrollArea.mouseScrolled`); with the
    /// extra on, a widget focused by the keyboard takes the step from anywhere on the
    /// screen. A click focuses its widget for the highlight, and honouring that focus made a
    /// clicked option toggle on every later wheel event while the list never scrolled.
    #[test]
    fn a_clicked_option_lets_the_wheel_scroll_the_list() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Video);
        let framebuffer = [854, 480];
        let click_size = (427.0, 400.0);
        let (widgets, click_top, click_bottom, _) = ui.layout(&s, click_size.0, click_size.1);

        // A click leaves the widget focused for its highlight, not for the wheel.
        let wd = widgets
            .iter()
            .find(|wd| {
                matches!(
                    wd.kind,
                    WKind::Button {
                        act: Act::Cycle(_),
                        ..
                    }
                ) && (!wd.list || (wd.y >= click_top && wd.y + wd.h <= click_bottom))
            })
            .expect("a visible cycle button");
        ui.click(
            &mut s,
            &atlas,
            wd.x + 5.0,
            wd.y + 5.0,
            click_size,
            framebuffer,
            false,
            false,
        );
        assert!(
            matches!(ui.focus, Some(FocusId::Cycle(_))),
            "the click focuses the cycle button it hit"
        );
        assert!(
            !ui.focus_from_keyboard,
            "a click does not leave keyboard focus"
        );

        // The wheel over the list's empty margin scrolls it at a size where the
        // page overflows, and the clicked cycle option stays put: no ButtonClick,
        // no Changed.
        let size = (427.0, 240.0);
        let (_, top, bottom, content) = ui.layout(&s, size.0, size.1);
        assert!(
            content > bottom - top,
            "the video page scrolls at this size"
        );
        let before = s.view_bobbing;
        let start = ui.scroll;
        let cursor = (2.0, (top + bottom) / 2.0);
        let events = ui.wheel(&mut s, size, cursor, framebuffer, -1.0);
        assert_eq!(s.view_bobbing, before, "the clicked option does not toggle");
        assert!(!events.contains(&Event::ButtonClick), "no click reaches it");
        assert!(!events.contains(&Event::Changed));
        assert!(ui.scroll > start, "the list scrolls down instead");

        // With the extra on, keyboard focus keeps the documented divergence: the
        // wheel steps it wherever the cursor is, even off the widget.
        s.extras.wheel_cycles_settings = true;
        ui.focus = Some(FocusId::Cycle(Opt::ViewBobbing));
        ui.focus_from_keyboard = true;
        let events = ui.wheel(&mut s, size, cursor, framebuffer, -1.0);
        assert_ne!(s.view_bobbing, before, "a keyboard-focused option cycles");
        assert!(events.contains(&Event::ButtonClick));
        assert!(events.contains(&Event::Changed));
    }

    #[test]
    fn the_wheel_scrolls_a_list_past_its_cycle_buttons_with_the_extra_off() {
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Video);
        let framebuffer = [854, 480];
        let size = (427.0, 240.0);
        let (widgets, top, bottom, content) = ui.layout(&s, size.0, size.1);
        assert!(
            content > bottom - top,
            "the video page scrolls at this size"
        );
        let (opt, over, list) = widgets
            .iter()
            .find_map(|wd| match wd.kind {
                WKind::Button {
                    act: Act::Cycle(opt),
                    ..
                } if wd.y >= top && wd.y + wd.h <= bottom => {
                    Some((opt, (wd.x + 5.0, wd.y + 5.0), wd.list))
                }
                _ => None,
            })
            .expect("a visible cycle button");

        assert!(list, "{opt:?} is a row of the video list");
        // The default, as in vanilla (#9633).
        assert!(!s.extras.wheel_cycles_settings);
        let before = format!("{s:?}");
        let start = ui.scroll;
        let events = ui.wheel(&mut s, size, over, framebuffer, -1.0);
        assert_eq!(format!("{s:?}"), before, "the hovered option stays put");
        assert!(!events.contains(&Event::Changed));
        assert!(
            ui.scroll > start,
            "the list scrolls under the cursor instead"
        );
        // Keyboard focus doesn't take the wheel either.
        ui.focus = Some(FocusId::Cycle(opt));
        ui.focus_from_keyboard = true;
        ui.wheel(&mut s, size, over, framebuffer, -1.0);
        assert_eq!(format!("{s:?}"), before);

        // On, the row under the cursor takes the step.
        s.extras.wheel_cycles_settings = true;
        let before = format!("{s:?}");
        let events = ui.wheel(&mut s, size, over, framebuffer, -1.0);
        assert_ne!(format!("{s:?}"), before, "{opt:?} cycles");
        assert!(events.contains(&Event::Changed));
    }

    #[test]
    fn options_step_slider_matches_set_slider_for_all_stepped_options() {
        let stepped_opts = [
            Opt::Fov,
            Opt::RenderDistance,
            Opt::SimulationDistance,
            Opt::MaxFps,
            Opt::CloudRange,
            Opt::EntityDistance,
            Opt::BiomeBlend,
            Opt::Preset,
            Opt::Mipmaps,
            Opt::WeatherRadius,
            Opt::ChunkFade,
            Opt::MenuBlur,
            Opt::NotificationDisplayTime,
            Opt::PadMoveDeadzone,
            Opt::PadLookDeadzone,
            Opt::PadTriggerThreshold,
            Opt::SprintWindow,
        ];
        for opt in stepped_opts {
            let steps = opt.slider_steps().expect("slider steps");
            for i in 0..steps {
                let pos = i as f32 / steps as f32;
                let mut s1 = Settings::default();
                let mut s2 = Settings::default();
                opt.set_slider(&mut s1, pos);

                opt.step_slider(&mut s1, true, 200.0);
                let next_pos = (i + 1) as f32 / steps as f32;
                opt.set_slider(&mut s2, next_pos);
                assert_eq!(
                    opt.slider_position(&s1),
                    opt.slider_position(&s2),
                    "{opt:?} forward from {i}"
                );
            }
            for i in 1..=steps {
                let pos = i as f32 / steps as f32;
                let mut s1 = Settings::default();
                let mut s2 = Settings::default();
                opt.set_slider(&mut s1, pos);

                opt.step_slider(&mut s1, false, 200.0);
                let prev_pos = (i - 1) as f32 / steps as f32;
                opt.set_slider(&mut s2, prev_pos);
                assert_eq!(
                    opt.slider_position(&s1),
                    opt.slider_position(&s2),
                    "{opt:?} backward from {i}"
                );
            }
        }
    }

    #[test]
    fn options_keyboard_space_enter_emits_button_click_for_cycle_and_default_buttons() {
        use winit::keyboard::KeyCode;
        let mut ui = OptionsUi::default();
        let mut s = Settings::default();
        let atlas = atlas();

        // 1. Focus a cycle button (e.g. Opt::ViewBobbing)
        ui.focus = Some(FocusId::Cycle(Opt::ViewBobbing));
        let events = ui.key(&mut s, &atlas, KeyCode::Space, None);
        assert!(
            events.contains(&Event::ButtonClick),
            "Space on FocusId::Cycle must emit Event::ButtonClick"
        );
        assert!(events.contains(&Event::Changed));

        let events = ui.key(&mut s, &atlas, KeyCode::Enter, None);
        assert!(
            events.contains(&Event::ButtonClick),
            "Enter on FocusId::Cycle must emit Event::ButtonClick"
        );
        assert!(events.contains(&Event::Changed));

        // 2. Focus a default/action button (e.g. Act::Done)
        ui.focus = Some(FocusId::Act(Act::Done));
        let events = ui.key(&mut s, &atlas, KeyCode::Space, None);
        assert!(
            events.contains(&Event::ButtonClick),
            "Space on FocusId::Act must emit Event::ButtonClick"
        );

        ui.focus = Some(FocusId::Act(Act::Done));
        let events = ui.key(&mut s, &atlas, KeyCode::Enter, None);
        assert!(
            events.contains(&Event::ButtonClick),
            "Enter on FocusId::Act must emit Event::ButtonClick"
        );
    }

    /// Browse Mods is on the Horizon settings page (the Mods footer has only Open Mods Folder
    /// and Back): clicking it lists the registry's mods and opens the browser.
    #[test]
    fn browse_mods_opens_the_registry_browser_from_the_horizon_page() {
        let atlas = atlas();
        let size = (960.0, 540.0);
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Horizon);
        let (widgets, ..) = ui.layout(&s, size.0, size.1);
        let browse = widgets
            .into_iter()
            .find(|w| {
                matches!(&w.kind, WKind::Button { act, .. } if *act == Act::Push(Page::ModBrowser))
            })
            .expect("Browse Mods... on the Horizon page");
        let events = ui.click(
            &mut s,
            &atlas,
            browse.x + browse.w / 2.0,
            browse.y + browse.h / 2.0,
            size,
            [854, 480],
            false,
            false,
        );
        assert_eq!(events, vec![Event::ButtonClick, Event::BrowseMods]);
        assert_eq!(ui.page(), Page::ModBrowser);
    }

    #[test]
    fn the_music_page_button_follows_the_extra() {
        let atlas = atlas();
        let size = (960.0, 540.0);
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Sound);
        let open = |ui: &mut OptionsUi, s: &Settings| {
            let (widgets, ..) = ui.layout(s, size.0, size.1);
            widgets.into_iter().find(
                |w| matches!(&w.kind, WKind::Button { act, .. } if *act == Act::Push(Page::Music)),
            )
        };
        // The extra is off by default: Music & Sounds is vanilla's page.
        assert!(open(&mut ui, &s).is_none());
        s.extras.music_page = true;
        assert!(open(&mut ui, &s).is_some(), "Music... once the extra is on");
        // It is the last row: scroll the list to its end before clicking it.
        ui.scroll(&s, size, -1000.0);
        let (widgets, ..) = ui.layout(&s, size.0, size.1);
        let open_button = widgets
            .into_iter()
            .find(
                |w| matches!(&w.kind, WKind::Button { act, .. } if *act == Act::Push(Page::Music)),
            )
            .expect("Music... on the Music & Sounds page");
        let events = ui.click(
            &mut s,
            &atlas,
            open_button.x + open_button.w / 2.0,
            open_button.y + open_button.h / 2.0,
            size,
            [854, 480],
            false,
            false,
        );
        assert_eq!(ui.page(), Page::Music);
        assert!(events.contains(&Event::ListMusicTracks));
    }

    #[test]
    fn the_music_page_plays_a_track_now_and_turns_tracks_off() {
        let atlas = atlas();
        let size = (960.0, 540.0);
        let mut s = Settings::default();
        let mut ui = OptionsUi {
            music_groups: vec![MusicGroup {
                label: "horizon.pages.music.overworld".into(),
                tracks: vec![
                    MusicTrack {
                        event: "minecraft:music.game".into(),
                        id: "minecraft:music/game/calm1".into(),
                    },
                    MusicTrack {
                        event: "minecraft:music.overworld.forest".into(),
                        id: "minecraft:music/game/bromeliad".into(),
                    },
                ],
            }],
            ..OptionsUi::default()
        };
        ui.show(Page::Music);
        let at = |ui: &mut OptionsUi, s: &Settings, want: &Act| {
            let (widgets, ..) = ui.layout(s, size.0, size.1);
            let w = widgets
                .into_iter()
                .find(|w| matches!(&w.kind, WKind::Button { act, .. } if act == want))
                .expect("button");
            (w.x + w.w / 2.0, w.y + w.h / 2.0)
        };
        // The left of a row plays that track now, from its own pool (a biome
        // pool's track plays from that pool).
        let calm1 = "minecraft:music/game/calm1";
        let (x, y) = at(
            &mut ui,
            &s,
            &Act::PlayMusicTrack("minecraft:music.game".into(), calm1.into()),
        );
        let events = ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false);
        assert!(events.contains(&Event::PlayMusicTrack(
            "minecraft:music.game".into(),
            calm1.into()
        )));
        let bromeliad = "minecraft:music/game/bromeliad";
        let (x, y) = at(
            &mut ui,
            &s,
            &Act::PlayMusicTrack("minecraft:music.overworld.forest".into(), bromeliad.into()),
        );
        let events = ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false);
        assert!(events.contains(&Event::PlayMusicTrack(
            "minecraft:music.overworld.forest".into(),
            bromeliad.into()
        )));
        // The right of a row turns it off, and the game saves that on `Changed`.
        let (x, y) = at(&mut ui, &s, &Act::MusicTrackOn(calm1.into(), true));
        let events = ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false);
        assert!(events.contains(&Event::Changed));
        assert_eq!(s.extras.music_off, [calm1.to_owned()]);
        // And back on.
        let (x, y) = at(&mut ui, &s, &Act::MusicTrackOn(calm1.into(), false));
        let events = ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false);
        assert!(events.contains(&Event::Changed));
        assert!(s.extras.music_off.is_empty());
    }

    #[test]
    fn options_cycle_button_cycles_forward_and_shift_click_cycles_backward_with_wrapping() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Chat);
        let size = (480.0, 480.0);
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let button = widgets
            .iter()
            .find(|wd| {
                matches!(
                    wd.kind,
                    WKind::Button {
                        active: true,
                        act: Act::Cycle(Opt::ChatVisibility),
                        ..
                    }
                )
            })
            .expect("ChatVisibility must be a cycle button");
        let (x, y) = (button.x + 1.0, button.y + 1.0);

        // Initial chat_visibility is 0
        assert_eq!(s.chat_visibility, 0);

        // 1. Feature without shift (normal click): cycles forward (+1)
        assert_eq!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false),
            vec![Event::ButtonClick, Event::Changed]
        );
        assert_eq!(s.chat_visibility, 1);

        assert_eq!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false),
            vec![Event::ButtonClick, Event::Changed]
        );
        assert_eq!(s.chat_visibility, 2);

        // 3. Wrap-around forward (at max value, normal click wraps to min / 0)
        assert_eq!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false),
            vec![Event::ButtonClick, Event::Changed]
        );
        assert_eq!(s.chat_visibility, 0);

        // 4. Wrap-around backward (at min / 0, shift-click wraps to max)
        ui.shift = true;
        assert_eq!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false),
            vec![Event::ButtonClick, Event::Changed]
        );
        assert_eq!(s.chat_visibility, 2);

        // 2. Feature with shift: cycles backward (-1)
        assert_eq!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false),
            vec![Event::ButtonClick, Event::Changed]
        );
        assert_eq!(s.chat_visibility, 1);

        assert_eq!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false),
            vec![Event::ButtonClick, Event::Changed]
        );
        assert_eq!(s.chat_visibility, 0);
        ui.shift = false;

        // Also test Opt::Particles on Page::Video
        ui.show(Page::Video);
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let button = widgets
            .iter()
            .find(|wd| {
                matches!(
                    wd.kind,
                    WKind::Button {
                        active: true,
                        act: Act::Cycle(Opt::Particles),
                        ..
                    }
                )
            })
            .expect("Particles must be a cycle button");
        let (px, py) = (button.x + 1.0, button.y + 1.0);

        // Initial particles is 0
        assert_eq!(s.particles, 0);

        // Forward
        ui.click(&mut s, &atlas, px, py, size, [854, 480], false, false);
        assert_eq!(s.particles, 1);
        ui.click(&mut s, &atlas, px, py, size, [854, 480], false, false);
        assert_eq!(s.particles, 2);
        // Wrap forward
        ui.click(&mut s, &atlas, px, py, size, [854, 480], false, false);
        assert_eq!(s.particles, 0);

        // Wrap backward with shift
        ui.shift = true;
        ui.click(&mut s, &atlas, px, py, size, [854, 480], false, false);
        assert_eq!(s.particles, 2);
        // Backward with shift
        ui.click(&mut s, &atlas, px, py, size, [854, 480], false, false);
        assert_eq!(s.particles, 1);
        ui.click(&mut s, &atlas, px, py, size, [854, 480], false, false);
        assert_eq!(s.particles, 0);
        ui.shift = false;
    }

    /// The GUI sizes a landscape phone has, across its GUI scales.
    const PHONE_GUI: [(f32, f32); 5] = [
        (427.0, 240.0),
        (533.0, 240.0),
        (600.0, 270.0),
        (800.0, 360.0),
        (1200.0, 540.0),
    ];

    /// A pack as the pack screen lists it.
    fn pack_info(id: &str) -> crate::pack_screen::PackInfo {
        crate::pack_screen::PackInfo {
            id: id.into(),
            title: Text::literal(id),
            description: Text::literal(""),
            description_style: crate::text::Style::default(),
            source: crate::pack_screen::Source::Default,
            compat: crate::pack_screen::Compat::Compatible,
            icon: None,
            required: false,
            position: crate::packs::Position::Top,
            fixed: false,
        }
    }

    /// Starling's Mods page has it in its footer; the shader packs page, upstream's settings shell,
    /// as a row under Open Folder (`shader_ui::Item::Catalog`).
    #[test]
    fn the_mods_page_opens_the_catalog_on_its_own_shelf() {
        let atlas = atlas();
        let mut s = Settings::default();
        for size in PHONE_GUI {
            for (page, shelf) in [(Page::NativeGame, CatalogShelf::Mods)] {
                let mut ui = OptionsUi {
                    catalog_buttons: true,
                    ..OptionsUi::default()
                };
                ui.show(page);
                let (widgets, ..) = ui.layout(&s, size.0, size.1);
                let footer: Vec<&Widget> = widgets
                    .iter()
                    .filter(|w| !w.list && matches!(w.kind, WKind::Button { .. }))
                    .collect();
                assert_eq!(footer.len(), 3, "{page:?} at {size:?}");
                let catalog = footer
                    .iter()
                    .find(|w| {
                        matches!(&w.kind, WKind::Button { act: Act::OpenCatalog(on), .. } if *on == shelf)
                    })
                    .expect("Catalog button");
                // On the screen, on the footer's row, and clear of its two buttons.
                assert!(catalog.x >= 0.0, "{page:?} at {size:?}");
                for other in &footer {
                    assert_eq!(other.y, size.1 - 27.0);
                    if std::ptr::eq(*other, *catalog) {
                        continue;
                    }
                    // Open Folder is left of it and Done right of it, with room between.
                    let clear = if other.x < catalog.x {
                        other.x + other.w < catalog.x
                    } else {
                        catalog.x + catalog.w < other.x
                    };
                    assert!(
                        clear,
                        "{page:?} at {size:?}: Catalog runs into the button at {}",
                        other.x
                    );
                    if other.x < catalog.x {
                        // Together they fill the 150 Open Folder had.
                        assert_eq!(catalog.x + catalog.w - other.x, 150.0);
                    }
                }
                let (x, y) = (catalog.x + catalog.w / 2.0, catalog.y + catalog.h / 2.0);
                let events = ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false);
                assert_eq!(events, vec![Event::ButtonClick, Event::OpenCatalog(shelf)]);
            }
        }
    }

    #[test]
    fn the_resource_packs_page_opens_the_catalog_on_its_shelf() {
        let atlas = atlas();
        let mut s = Settings::default();
        let size = (800.0, 360.0);
        let mut ui = OptionsUi {
            catalog_buttons: true,
            ..OptionsUi::default()
        };
        ui.show(Page::Packs);
        // The right half of Open Folder's slot: the footer starts at (800 - 308) / 2.
        let events = ui.click(&mut s, &atlas, 330.0, 344.0, size, [854, 480], false, false);
        assert_eq!(
            events,
            vec![
                Event::ButtonClick,
                Event::OpenCatalog(CatalogShelf::ResourcePacks)
            ]
        );
        assert_eq!(ui.page(), Page::Packs);
    }

    #[test]
    fn without_the_offer_no_page_has_a_catalog_button() {
        let atlas = atlas();
        let mut s = Settings::default();
        let size = (800.0, 360.0);
        let mut ui = OptionsUi::default();
        for page in [Page::Mods, Page::ShaderPacks] {
            ui.show(page);
            let (widgets, ..) = ui.layout(&s, size.0, size.1);
            assert!(!widgets.iter().any(|w| matches!(
                w.kind,
                WKind::Button {
                    act: Act::OpenCatalog(_),
                    ..
                }
            )));
        }
        ui.show(Page::Packs);
        // Left of the footer, where the button used to be.
        let events = ui.click(&mut s, &atlas, 200.0, 344.0, size, [854, 480], false, false);
        assert_eq!(events, Vec::new());
    }

    #[test]
    fn only_a_phone_takes_the_offer_of_the_catalog() {
        let mut ui = OptionsUi::default();
        ui.offer_catalog();
        assert_eq!(ui.catalog_buttons, cfg!(target_os = "android"));
    }

    #[test]
    fn create_worlds_data_packs_have_no_catalog_button() {
        let atlas = atlas();
        let mut s = Settings::default();
        let dir = std::env::temp_dir().join(format!("horizon-catalog-{}", std::process::id()));
        let mut ui = OptionsUi {
            catalog_buttons: true,
            ..OptionsUi::default()
        };
        ui.open_data_packs(dir.clone(), Vec::new(), dir, Vec::new());
        let events = ui.click(
            &mut s,
            &atlas,
            192.0,
            344.0,
            (800.0, 360.0),
            [854, 480],
            false,
            false,
        );
        assert_eq!(events, Vec::new());
    }

    /// The catalog installs with Options open: from the Mods page there was no reading the folder
    /// again, as `pack_infos` is read when Options opens and the watcher runs on this page only.
    #[test]
    fn a_pack_installed_with_options_open_is_on_the_resource_packs_page() {
        let atlas = atlas();
        let mut ui = OptionsUi {
            pack_infos: vec![pack_info("file/old")],
            ..OptionsUi::default()
        };
        ui.show(Page::Mods);
        ui.update_pack_infos(vec![pack_info("file/old"), pack_info("file/new")]);
        ui.show(Page::Packs);
        let listed = &ui.packs(&atlas, (800.0, 360.0)).model.unselected;
        assert!(listed.iter().any(|p| p.id == "file/new"));
    }

    #[test]
    fn a_pack_installed_over_the_page_keeps_what_was_chosen_on_it() {
        let atlas = atlas();
        let size = (800.0, 360.0);
        let mut ui = OptionsUi {
            pack_infos: vec![pack_info("file/a"), pack_info("file/b")],
            ..OptionsUi::default()
        };
        ui.show(Page::Packs);
        let screen = ui.packs(&atlas, size);
        screen.model.activate(false, 0, true);
        screen.changed = true;
        let chosen = screen.commit();
        assert_eq!(chosen.len(), 1);
        ui.update_pack_infos(vec![
            pack_info("file/a"),
            pack_info("file/b"),
            pack_info("file/new"),
        ]);
        let screen = ui.packs(&atlas, size);
        assert_eq!(screen.commit(), chosen);
        assert!(screen.changed, "Done still applies it");
        assert!(screen.model.unselected.iter().any(|p| p.id == "file/new"));
    }

    #[test]
    fn a_resource_pack_installed_leaves_create_worlds_data_packs_alone() {
        let dir = std::env::temp_dir().join(format!("horizon-catalog-{}", std::process::id()));
        let mut ui = OptionsUi::default();
        ui.open_data_packs(dir.clone(), Vec::new(), dir, Vec::new());
        let listed = ui.pack_infos.len();
        ui.update_pack_infos(vec![pack_info("file/new")]);
        assert_eq!(ui.pack_infos.len(), listed);
        assert!(!ui.pack_infos.iter().any(|p| p.id == "file/new"));
    }
}

#[cfg(test)]
#[path = "options_ui_graphics_tests.rs"]
mod graphics_tests;
#[path = "options_ui_pages.rs"]
mod pages;
