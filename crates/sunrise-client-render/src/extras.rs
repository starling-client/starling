//! Sunrise's optional extras (lighting, inventory tools): their
//! settings, kept in `sunrise-extras.txt` next to `options.txt` so that file
//! stays exactly vanilla's, and the logic behind them. Every extra is off by
//! default; see `docs/extras.md`. Sunrise's own switches outside the Extras page
//! (No AI, Agent Control) are kept here too.
use std::{collections::VecDeque, path::Path};

pub const FILE: &str = "sunrise-extras.txt";
/// The name the options screen uses while the sort key waits for a key.
pub const SORT_KEY: &str = "sunrise.sort";
/// The name the options screen uses while the shader pack key waits for a key.
pub const SHADER_PACK_KEY: &str = "sunrise.shader_pack_key";

/// A screen corner for an overlay.
pub use mod_api::client::extras::Corner;

#[derive(Clone, Debug, PartialEq)]
pub struct Extras {
    /// The Select Resource Packs screen's Add button, which browses
    /// Modrinth. Off by default: it is Sunrise's own and the screen is
    /// otherwise vanilla's (tools/test_pack_screen_parity.py).
    pub pack_browser: bool,
    /// Make /pregen available in single-player and hosted LAN worlds.
    pub pregen: bool,
    pub fullbright: bool,
    /// Held, dropped and burning light sources light their surroundings (`dynamic_light`).
    pub dynamic_lights: bool,
    /// Far terrain a zoom magnifies is planned finer, so a spyglass does not
    /// show the coarse cells it was given for an unzoomed screen
    /// (`lod::Aim`). Costs far tiles while the zoom is held and gives them
    /// back after, and does nothing at all unless far terrain is on
    /// (`lodDistance`) and the view is actually zoomed. Off by default until
    /// a measurement shows the extra tiles land while the zoom is still held.
    pub lod_zoom_detail: bool,
    /// Positional sounds leave a few static sound channels free for UI sounds
    /// and others played to the listener (`sound_engine::HEARD_RESERVE`).
    pub ui_sound_reserve: bool,
    /// The player's own name tag over its model in third person.
    pub own_name_tag: bool,
    /// NeoForge's `skipModdedSplashesIfVanillaModified`, on by default: a
    /// pack's own `minecraft` splashes replace every namespace's
    /// (`sunrise_client::splashes::pool`). No Extras page entry.
    pub skip_modded_splashes_if_vanilla_modified: bool,
    /// No particles from breaking or mining a block (other particles stay).
    pub hide_block_particles: bool,
    /// Mouse Tweaks-style drags in the inventory and storage containers.
    pub mouse_tweaks: bool,
    /// With them, the wheel over a slot moves its items one at a time.
    pub mouse_tweaks_wheel: bool,
    /// Sort buttons on the inventory and chest-like containers.
    pub sort: bool,
    /// A vanilla key name: sorts the part under the cursor.
    pub sort_key: String,
    /// A vanilla key name: turns the picked shader pack off and on.
    pub shader_pack_key: String,
    /// The last non-empty shader pack pick, for the toggle key to resume. It follows every
    /// non-empty pick, whether by menu or by key, and turning shaders off keeps it.
    pub last_shader_pack: String,
    /// A new chat line slides in from the left while it fades in (`Chat::layout`).
    pub chat_animation: bool,
    /// The indicator bar at the left of a chat line (on by default, as vanilla).
    pub chat_indicator: bool,
    /// No AI: every AI feature hidden and off (`sunrise_core::no_ai`; the launcher writes it too).
    pub no_ai: bool,
    /// Agents may control the game the player starts: it opens its control socket for
    /// sunrise-ctl and MCP clients (off by default; an agent's own launch always opens it).
    pub agent_control: bool,
    /// F3+R and F3+E start/stop a repro recording and write its bundle (off by
    /// default: a recording copies the world).
    pub repro_keys: bool,
    /// The wheel over a cycle button in the settings changes it (`CycleButton.mouseScrolled`,
    /// on by default as vanilla); off, the wheel only scrolls the list.
    pub wheel_cycles_settings: bool,
    /// The built-in music tracks the Music page turned off (sound ids): they
    /// leave every pool they play in (`musicOff`, empty by default: vanilla's
    /// random selection).
    pub music_off: Vec<String>,
    /// The Music page: the Music... button on Music & Sounds, playing a
    /// built-in track and leaving the off ones out (`musicPage`, off by
    /// default: vanilla plays exactly as before).
    pub music_page: bool,
    /// Lines no setting here reads, by key and as written, kept in file order so a setting that moved to a store survives a run without it.
    unknown: Vec<(String, String)>,
    /// The file is there but couldn't be read whole (an I/O error, or bytes that aren't UTF-8):
    /// it is never written over, so nothing in it is lost.
    keep_file: bool,
    /// The PvP & HUD page's options (`pvp`).
    pub pvp: crate::pvp::Pvp,
}
impl Default for Extras {
    fn default() -> Self {
        Self {
            pack_browser: false,
            pregen: false,
            fullbright: false,
            dynamic_lights: false,
            lod_zoom_detail: false,
            ui_sound_reserve: false,
            own_name_tag: false,
            skip_modded_splashes_if_vanilla_modified: true,
            hide_block_particles: false,
            mouse_tweaks: false,
            mouse_tweaks_wheel: true,
            sort: false,
            sort_key: "key.keyboard.unknown".into(),
            shader_pack_key: "key.keyboard.k".into(),
            last_shader_pack: String::new(),
            chat_animation: false,
            chat_indicator: true,
            no_ai: false,
            agent_control: false,
            repro_keys: false,
            wheel_cycles_settings: false,
            music_off: Vec::new(),
            music_page: false,
            unknown: Vec::new(),
            keep_file: false,
            pvp: Default::default(),
        }
    }
}
fn bool_text(v: bool) -> &'static str {
    if v { "true" } else { "false" }
}
impl Extras {
    pub fn load(dir: &Path) -> Self {
        let (text, keep) = Self::read_text(dir);
        Self::from_text(&text, keep)
    }
    pub fn parse(text: &str) -> Self {
        let mut e = Self::default();
        for raw in text.split_inclusive('\n') {
            let line = raw.trim_end_matches(['\n', '\r']);
            let Some((key, value)) = line.split_once(':') else {
                // Not `key:value`: no setting reads it, and it is kept as written.
                e.unknown.push((String::new(), raw.to_owned()));
                continue;
            };
            let value = value.trim();
            let flag = |d: bool| match value {
                "true" => true,
                "false" => false,
                _ => d,
            };
            match key.trim() {
                "packBrowser" => e.pack_browser = flag(e.pack_browser),
                "pregen" => e.pregen = flag(e.pregen),
                "fullbright" => e.fullbright = flag(e.fullbright),
                "dynamicLights" => e.dynamic_lights = flag(e.dynamic_lights),
                "lodZoomDetail" => e.lod_zoom_detail = flag(e.lod_zoom_detail),
                "uiSoundReserve" => e.ui_sound_reserve = flag(e.ui_sound_reserve),
                "ownNameTag" => e.own_name_tag = flag(e.own_name_tag),
                "skipModdedSplashesIfVanillaModified" => {
                    e.skip_modded_splashes_if_vanilla_modified =
                        flag(e.skip_modded_splashes_if_vanilla_modified)
                }
                "hideBlockParticles" => e.hide_block_particles = flag(e.hide_block_particles),
                "mouseTweaks" => e.mouse_tweaks = flag(e.mouse_tweaks),
                "mouseTweaksWheel" => e.mouse_tweaks_wheel = flag(e.mouse_tweaks_wheel),
                "sort" => e.sort = flag(e.sort),
                "sortKey" => {
                    if value.starts_with("key.") {
                        e.sort_key = value.to_owned();
                    }
                }
                "shaderPackKey" => {
                    if value.starts_with("key.") {
                        e.shader_pack_key = value.to_owned();
                    }
                }
                "lastShaderPack" => e.last_shader_pack = value.to_owned(),
                "chatAnimation" => e.chat_animation = flag(e.chat_animation),
                "chatIndicator" => e.chat_indicator = flag(e.chat_indicator),
                sunrise_core::no_ai::KEY => e.no_ai = flag(e.no_ai),
                "agentControl" => e.agent_control = flag(e.agent_control),
                "reproKeys" => e.repro_keys = flag(e.repro_keys),
                // Retired: every install wrote `wheelCyclesSettings` as true while it was the default.
                "wheelCyclesSettings" => {}
                "wheelCyclesListSettings" => {
                    e.wheel_cycles_settings = flag(e.wheel_cycles_settings)
                }
                "musicOff" => {
                    e.music_off = value
                        .split(',')
                        .map(str::trim)
                        .filter(|id| !id.is_empty())
                        .map(str::to_owned)
                        .collect()
                }
                "musicPage" => e.music_page = flag(e.music_page),
                key => {
                    if !e.pvp.parse_line(key, value) {
                        e.unknown.push((key.to_owned(), raw.to_owned()));
                    }
                }
            }
        }
        e
    }
    /// Every line, the kept ones included.
    pub fn serialize(&self) -> String {
        self.serialize_but(&|_| false)
    }
    /// `serialize`, leaving out the kept lines `owned` claims: a store wrote those itself.
    fn serialize_but(&self, owned: &dyn Fn(&str) -> bool) -> String {
        let mut text = String::new();
        let mut push = |key: &str, value: &dyn std::fmt::Display| {
            text.push_str(&format!("{key}:{value}\n"));
        };
        push("packBrowser", &bool_text(self.pack_browser));
        push("pregen", &bool_text(self.pregen));
        push("fullbright", &bool_text(self.fullbright));
        push("dynamicLights", &bool_text(self.dynamic_lights));
        push("lodZoomDetail", &bool_text(self.lod_zoom_detail));
        push("uiSoundReserve", &bool_text(self.ui_sound_reserve));
        push("ownNameTag", &bool_text(self.own_name_tag));
        push(
            "skipModdedSplashesIfVanillaModified",
            &bool_text(self.skip_modded_splashes_if_vanilla_modified),
        );
        push("hideBlockParticles", &bool_text(self.hide_block_particles));
        push("mouseTweaks", &bool_text(self.mouse_tweaks));
        push("mouseTweaksWheel", &bool_text(self.mouse_tweaks_wheel));
        push("sort", &bool_text(self.sort));
        push("sortKey", &self.sort_key);
        push("shaderPackKey", &self.shader_pack_key);
        push("lastShaderPack", &self.last_shader_pack);
        push("chatAnimation", &bool_text(self.chat_animation));
        push("chatIndicator", &bool_text(self.chat_indicator));
        push(sunrise_core::no_ai::KEY, &bool_text(self.no_ai));
        push("agentControl", &bool_text(self.agent_control));
        push("reproKeys", &bool_text(self.repro_keys));
        push(
            "wheelCyclesListSettings",
            &bool_text(self.wheel_cycles_settings),
        );
        push("musicOff", &self.music_off.join(","));
        push("musicPage", &bool_text(self.music_page));
        self.pvp.serialize(&mut text);
        // As written: only the file's last record can lack a line end, and nothing follows it here.
        for (key, raw) in &self.unknown {
            if !owned(key) {
                text.push_str(raw);
            }
        }
        text
    }
    /// Binds the extra key `name` (`SORT_KEY`); false for
    /// another name (a vanilla mapping).
    pub fn set_key(&mut self, name: &str, value: String) -> bool {
        match name {
            SORT_KEY => self.sort_key = value,
            SHADER_PACK_KEY => self.shader_pack_key = value,
            crate::pvp::HUD_EDITOR_KEY => self.pvp.editor_key = value,
            _ => return false,
        }
        true
    }
    /// Written only once something differs from the defaults, so a vanilla
    /// setup gains no file. The settings stores of this file follow these lines
    /// (`sunrise_hooks::settings`).
    pub fn save(&self, dir: &Path) {
        let mut stores = String::new();
        let stores_default = sunrise_hooks::settings::engine::append(FILE, &mut stores);
        self.save_with(dir, &stores, stores_default);
    }
    /// `save` with the stores' lines of this file made: a kept line goes only when a store
    /// wrote its key. A file that couldn't be read, or a failed write, leaves the old file.
    fn save_with(&self, dir: &Path, stores: &str, stores_default: bool) {
        use sunrise_hooks::settings::engine;
        let path = dir.join(FILE);
        if self.keep_file || (*self == Self::default() && stores_default && !path.exists()) {
            return;
        }
        let written = engine::keys(stores);
        let mut text = self.serialize_but(&|key| written.contains(&key));
        if !stores.is_empty() && !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(stores);
        if let Err(e) = engine::replace(&path, &text) {
            log::warn!("Sunrise: {FILE} is not saved: {e}");
        }
    }
    /// The file's text in `dir` (empty when there is none), and whether the file must be kept
    /// as it is: it couldn't be read (its settings stay at their defaults) or had bytes that
    /// aren't UTF-8 (its other lines load).
    pub fn read_text(dir: &Path) -> (String, bool) {
        match sunrise_hooks::settings::engine::read(&dir.join(FILE)) {
            Ok(None) => (String::new(), false),
            Ok(Some(read)) => {
                if read.lossy {
                    log::warn!(
                        "Sunrise: {FILE} has bytes that aren't UTF-8, so it won't be saved over"
                    );
                }
                (read.text, read.lossy)
            }
            Err(e) => {
                log::warn!("Sunrise: {FILE} could not be read, so it won't be saved over: {e}");
                (String::new(), true)
            }
        }
    }
    /// The settings of `text` from [`Self::read_text`]; `keep` leaves the file as it is.
    pub fn from_text(text: &str, keep: bool) -> Self {
        Self {
            keep_file: keep,
            ..Self::parse(text)
        }
    }
}

/// The field-of-view modifier target `Camera.tickFov` eases toward: vanilla's
/// (`getFieldOfViewModifier`) divided by a mod's zoom (`native_zoom`). A
/// zoom of 1 leaves it as is.
pub fn fov_target(vanilla: f32, zoom: f32) -> f32 {
    if zoom == 1.0 { vanilla } else { vanilla / zoom }
}
/// Frame times over the last few seconds: frames per second, the minimum
/// and the 1% low, recomputed twice a second.
pub struct FpsCounter {
    /// (end time in seconds, frame milliseconds), oldest first.
    frames: VecDeque<(f64, f32)>,
    shown: FpsReading,
    computed_at: f64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FpsReading {
    pub fps: u32,
    pub min: u32,
    pub low: u32,
}
/// Seconds the minimum and 1% low look back.
const LOW_WINDOW: f64 = 5.0;
impl Default for FpsCounter {
    fn default() -> Self {
        Self {
            frames: VecDeque::new(),
            shown: FpsReading::default(),
            computed_at: f64::NEG_INFINITY,
        }
    }
}
impl FpsCounter {
    /// A frame that ended at `now` (seconds) and took `ms`.
    pub fn record(&mut self, now: f64, ms: f32) {
        self.frames.push_back((now, ms));
        while self
            .frames
            .front()
            .is_some_and(|&(t, _)| now - t > LOW_WINDOW)
        {
            self.frames.pop_front();
        }
        if now - self.computed_at >= 0.5 {
            self.computed_at = now;
            self.shown = self.compute(now);
        }
    }
    fn compute(&self, now: f64) -> FpsReading {
        let fps = self.frames.iter().filter(|&&(t, _)| now - t < 1.0).count() as u32;
        let mut times: Vec<f32> = self.frames.iter().map(|f| f.1).collect();
        if times.is_empty() {
            return FpsReading::default();
        }
        times.sort_by(|a, b| b.total_cmp(a));
        let to_fps = |ms: f32| (1000.0 / ms.max(0.001)).round() as u32;
        // The 1% low: the frame rate of the 99th percentile frame time.
        let low = times[(times.len() - 1) / 100];
        FpsReading {
            fps,
            min: to_fps(times[0]),
            low: to_fps(low),
        }
    }
    pub fn reading(&self) -> FpsReading {
        self.shown
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Clicks of one button in the last second.
#[derive(Default)]
pub struct Cps {
    clicks: VecDeque<f64>,
}
impl Cps {
    pub fn click(&mut self, now: f64) {
        self.clicks.push_back(now);
        self.forget(now);
    }
    fn forget(&mut self, now: f64) {
        while self.clicks.front().is_some_and(|&t| now - t >= 1.0) {
            self.clicks.pop_front();
        }
    }
    pub fn per_second(&mut self, now: f64) -> usize {
        self.forget(now);
        self.clicks.len()
    }
}
/// The settings the Extras page shows, one per row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Fullbright,
    DynamicLights,
    OwnNameTag,
    HideBlockParticles,
    UiSoundReserve,
    /// The Music... page on Music & Sounds (`Music page`).
    MusicPage,
    MouseTweaks,
    MouseTweaksWheel,
    Sort,
    SortKey,
    /// Turns the picked shader pack off and on, from the Extras page's key.
    ShaderPackKey,
    /// A new chat line slides in and fades in (`Chat::layout`).
    ChatAnimation,
    /// The coloured bar at the left of a chat line.
    ChatIndicator,
    /// Not on the Extras page: Sunrise Settings shows No AI, its AI Agent page Agent Control.
    NoAi,
    AgentControl,
    /// On the AI Agent page, next to Agent Control: the F3 recording shortcuts.
    ReproKeys,
    WheelCyclesSettings,
    /// The Select Resource Packs screen's Add button (Modrinth).
    PackBrowser,
    Pregen,
    /// On the PvP & HUD page (`pvp::rows`).
    Pvp(crate::pvp::Opt),
}
/// The Extras page: headers (language keys) and items.
pub fn rows() -> Vec<Result<Item, &'static str>> {
    use Item::*;
    // Fullbright, Dynamic lights and Hide block-break particles change the picture: they
    // are on the graphics screen's Quality page (`graphics_ui::rows`), not here. A section a crate
    // adds joins the group its title names (`sunrise_hooks::pages::Place::Extras`).
    vec![
        Err("sunrise.extras.header.third_person"),
        Ok(OwnNameTag),
        Err("sunrise.extras.header.inventory"),
        Ok(MouseTweaks),
        Ok(MouseTweaksWheel),
        Ok(Sort),
        Ok(SortKey),
        Err("sunrise.extras.header.chat"),
        Ok(ChatAnimation),
        Ok(ChatIndicator),
        Err("sunrise.extras.header.world"),
        Ok(Pregen),
        Ok(ShaderPackKey),
        Err("sunrise.extras.header.multiplayer"),
        Err("sunrise.extras.header.sound"),
        Ok(UiSoundReserve),
        Ok(MusicPage),
        Err("sunrise.extras.header.menus"),
        Ok(WheelCyclesSettings),
        Ok(PackBrowser),
    ]
}
impl Item {
    /// The language key of the caption (`sunrise.*`, assets/sunrise/lang).
    pub fn caption_key(self) -> &'static str {
        match self {
            Item::Fullbright => "sunrise.extras.fullbright",
            Item::DynamicLights => "sunrise.extras.dynamic_lights",
            Item::OwnNameTag => "sunrise.extras.own_name_tag",
            Item::HideBlockParticles => "sunrise.extras.hide_block_particles",
            Item::UiSoundReserve => "sunrise.extras.ui_sound_reserve",
            Item::MusicPage => "sunrise.extras.music_page",
            Item::MouseTweaks => "sunrise.extras.mouse_tweaks",
            Item::MouseTweaksWheel => "sunrise.extras.mouse_tweaks_wheel",
            Item::Sort => "sunrise.extras.sort",
            Item::SortKey => "sunrise.extras.sort_key",
            Item::ShaderPackKey => "sunrise.extras.shader_pack_key",
            Item::NoAi => "sunrise.extras.no_ai",
            Item::AgentControl => "sunrise.extras.agent_control",
            Item::ReproKeys => "sunrise.extras.repro_keys",
            Item::ChatAnimation => "sunrise.extras.chat_animation",
            Item::ChatIndicator => "sunrise.extras.chat_indicator",
            Item::WheelCyclesSettings => "sunrise.extras.wheel_cycles_settings",
            Item::PackBrowser => "sunrise.extras.pack_browser",
            Item::Pregen => "sunrise.extras.pregen",
            Item::Pvp(o) => o.caption_key(),
        }
    }
    /// The language key of the description shown as the row's tooltip.
    pub fn description_key(self) -> &'static str {
        match self {
            Item::Fullbright => "sunrise.extras.fullbright.tooltip",
            Item::DynamicLights => "sunrise.extras.dynamic_lights.tooltip",
            Item::OwnNameTag => "sunrise.extras.own_name_tag.tooltip",
            Item::HideBlockParticles => "sunrise.extras.hide_block_particles.tooltip",
            Item::UiSoundReserve => "sunrise.extras.ui_sound_reserve.tooltip",
            Item::MusicPage => "sunrise.extras.music_page.tooltip",
            Item::MouseTweaks => "sunrise.extras.mouse_tweaks.tooltip",
            Item::MouseTweaksWheel => "sunrise.extras.mouse_tweaks_wheel.tooltip",
            Item::Sort => "sunrise.extras.sort.tooltip",
            Item::SortKey => "sunrise.extras.sort_key.tooltip",
            Item::ShaderPackKey => "sunrise.extras.shader_pack_key.tooltip",
            Item::NoAi => "sunrise.extras.no_ai.tooltip",
            Item::AgentControl => "sunrise.extras.agent_control.tooltip",
            Item::ReproKeys => "sunrise.extras.repro_keys.tooltip",
            Item::ChatAnimation => "sunrise.extras.chat_animation.tooltip",
            Item::ChatIndicator => "sunrise.extras.chat_indicator.tooltip",
            Item::WheelCyclesSettings => "sunrise.extras.wheel_cycles_list_settings.tooltip",
            Item::PackBrowser => "sunrise.extras.pack_browser.tooltip",
            Item::Pregen => "sunrise.extras.pregen.tooltip",
            Item::Pvp(o) => o.description_key(),
        }
    }
    /// The caption in the current language (English when the key is missing).
    pub fn caption_text(self) -> String {
        crate::lang::get_or(self.caption_key(), self.caption())
    }
    /// The description in the current language (English when the key is missing).
    pub fn description_text(self) -> String {
        crate::lang::get_or(self.description_key(), self.description())
    }
    /// The English caption: the fallback of [`Item::caption_text`], and the text of
    /// `sunrise.extras.*` in en_us.json.
    pub fn caption(self) -> &'static str {
        match self {
            Item::Fullbright => "Fullbright",
            Item::DynamicLights => "Dynamic lights",
            Item::OwnNameTag => "Own name tag",
            Item::HideBlockParticles => "Hide block-break particles",
            Item::UiSoundReserve => "Keep channels for UI sounds",
            Item::MusicPage => "Music page",
            Item::MouseTweaks => "Mouse Tweaks controls",
            Item::MouseTweaksWheel => "Scroll wheel moves items",
            Item::Sort => "Sort buttons",
            Item::SortKey => "Sort key",
            Item::ShaderPackKey => "Shader pack key",
            Item::ChatAnimation => "Smooth chat",
            Item::ChatIndicator => "Message indicator",
            Item::NoAi => "No AI",
            Item::AgentControl => "Agent Control",
            Item::ReproKeys => "Recording shortcuts",
            Item::WheelCyclesSettings => "Scroll wheel changes settings",
            Item::PackBrowser => "Resource pack browser",
            Item::Pregen => "Chunk pregeneration",
            Item::Pvp(o) => o.caption(),
        }
    }
    /// The switch this setting belongs to: it does nothing while that is off.
    pub fn parent(self) -> Option<Item> {
        use Item::*;
        Some(match self {
            UiSoundReserve => return None,
            MouseTweaksWheel => MouseTweaks,
            SortKey => Sort,
            Pvp(o) => return o.parent().map(Pvp),
            Fullbright | DynamicLights | OwnNameTag | HideBlockParticles | MouseTweaks | Sort
            | ChatAnimation | ChatIndicator | NoAi | AgentControl | ReproKeys
            | WheelCyclesSettings | MusicPage | PackBrowser | Pregen | ShaderPackKey => {
                return None;
            }
        })
    }
    /// The English description: the fallback of [`Item::description_text`].
    pub fn description(self) -> &'static str {
        match self {
            Item::Pregen => {
                "Enables /pregen in your single-player and LAN worlds. With cheats enabled, use /pregen start to generate an area before exploring it. Does not enable the command on servers you join."
            }
            Item::Fullbright => "Show blocks and entities at full light, even in dark areas.",
            Item::DynamicLights => {
                "Torches and other light sources held, dropped or on fire light the blocks around them."
            }
            Item::OwnNameTag => {
                "Shows your name above your head in third person, as other players see it. Hidden when the server hides your team's names or you're invisible."
            }
            Item::HideBlockParticles => {
                "Hides the debris particles that burst from a broken block and crumble off the one you mine. Other particles stay."
            }
            Item::UiSoundReserve => {
                "Sounds in the world leave 16 of the 247 sound channels free, so a flood of them (a big explosion) can't silence menu clicks."
            }
            Item::MusicPage => {
                "Music & Sounds gets a Music... page: play any built-in track now, or turn one off so the game stops picking it. Nothing changes until you turn a track off."
            }
            Item::MouseTweaks => {
                "Right-drag places one per slot, left-drag gathers the held item, Shift + left-drag moves every slot."
            }
            Item::MouseTweaksWheel => {
                "Down sends one item of the slot to the other side, up brings one back."
            }
            Item::Sort => {
                "Sorts the inventory, chests, barrels, ender chests and shulker boxes in creative tab order, merging stacks."
            }
            Item::SortKey => {
                "Sorts the part under the cursor. Click, then press a key; Escape unbinds it."
            }
            Item::ShaderPackKey => {
                "Turns the picked shader pack off and on. Click, then press a key; Escape unbinds it. The pick is remembered across restarts."
            }
            Item::ChatAnimation => {
                "A new chat line slides in from the left while it fades in, instead of appearing whole."
            }
            Item::ChatIndicator => "The coloured bar at the left of a chat line.",
            Item::NoAi => {
                "Hides and turns off every AI feature: agent control, sunrise-ctl and the MCP servers. The launcher's switch too."
            }
            Item::AgentControl => {
                "Your own AI agent can play this game through sunrise-ctl or an MCP client, on this computer only."
            }
            Item::ReproKeys => {
                "F3+R starts or stops a repro recording and F3+E writes its bundle. Off by default: a recording copies the world."
            }
            Item::Pvp(o) => o.description(),
            Item::WheelCyclesSettings => {
                "The scroll wheel over a button in the settings changes its value, as in vanilla. Off, the wheel only scrolls the list."
            }
            Item::PackBrowser => {
                "Select Resource Packs gets an Add button that searches Modrinth and installs a pack into your packs folder. Off, the game never contacts Modrinth."
            }
        }
    }
    /// The frame-time cost the main switch of an extra shows.
    pub fn cost(self) -> Option<crate::graphics_style::Cost> {
        use crate::graphics_style::Cost;
        match self {
            Item::Fullbright => Some(Cost::None),
            Item::UiSoundReserve => Some(Cost::None),
            Item::MusicPage => Some(Cost::None),
            Item::DynamicLights => Some(Cost::Low),
            Item::ChatAnimation => Some(Cost::None),
            Item::OwnNameTag
            | Item::HideBlockParticles
            | Item::MouseTweaks
            | Item::Sort
            | Item::WheelCyclesSettings
            | Item::PackBrowser => Some(Cost::None),
            _ => None,
        }
    }
    /// On/off items and their value.
    pub fn switch(self, e: &Extras) -> Option<bool> {
        Some(match self {
            Item::Fullbright => e.fullbright,
            Item::DynamicLights => e.dynamic_lights,
            Item::OwnNameTag => e.own_name_tag,
            Item::HideBlockParticles => e.hide_block_particles,
            Item::UiSoundReserve => e.ui_sound_reserve,
            Item::MusicPage => e.music_page,
            Item::MouseTweaks => e.mouse_tweaks,
            Item::MouseTweaksWheel => e.mouse_tweaks_wheel,
            Item::Sort => e.sort,
            Item::ChatAnimation => e.chat_animation,
            Item::ChatIndicator => e.chat_indicator,
            Item::NoAi => e.no_ai,
            Item::AgentControl => e.agent_control,
            Item::ReproKeys => e.repro_keys,
            Item::WheelCyclesSettings => e.wheel_cycles_settings,
            Item::PackBrowser => e.pack_browser,
            Item::Pregen => e.pregen,
            Item::Pvp(o) => return o.switch(&e.pvp),
            _ => return None,
        })
    }
    /// The value of the other items, as their button shows it.
    pub fn value(self, e: &Extras) -> String {
        match self {
            Item::SortKey => crate::keybinds::display_name(&e.sort_key).plain(),
            Item::ShaderPackKey => crate::keybinds::display_name(&e.shader_pack_key).plain(),
            Item::Pvp(o) if o.switch(&e.pvp).is_none() => o.value(&e.pvp).unwrap_or_default(),
            other => match other.switch(e) {
                Some(true) => crate::lang::get_or("On", "On"),
                _ => crate::lang::get_or("Off", "Off"),
            },
        }
    }
    /// A click (`back` for the right button: a PvP & HUD choice steps back; a switch flips
    /// either way). The sort key binds instead.
    pub fn cycle(self, e: &mut Extras, back: bool) {
        match self {
            Item::Fullbright => e.fullbright = !e.fullbright,
            Item::DynamicLights => e.dynamic_lights = !e.dynamic_lights,
            Item::OwnNameTag => e.own_name_tag = !e.own_name_tag,
            Item::HideBlockParticles => e.hide_block_particles = !e.hide_block_particles,
            Item::UiSoundReserve => e.ui_sound_reserve = !e.ui_sound_reserve,
            Item::MusicPage => e.music_page = !e.music_page,
            Item::MouseTweaks => e.mouse_tweaks = !e.mouse_tweaks,
            Item::MouseTweaksWheel => e.mouse_tweaks_wheel = !e.mouse_tweaks_wheel,
            Item::Sort => e.sort = !e.sort,
            Item::ChatAnimation => e.chat_animation = !e.chat_animation,
            Item::ChatIndicator => e.chat_indicator = !e.chat_indicator,
            Item::NoAi => e.no_ai = !e.no_ai,
            Item::AgentControl => e.agent_control = !e.agent_control,
            Item::ReproKeys => e.repro_keys = !e.repro_keys,
            Item::WheelCyclesSettings => e.wheel_cycles_settings = !e.wheel_cycles_settings,
            Item::PackBrowser => e.pack_browser = !e.pack_browser,
            Item::Pregen => e.pregen = !e.pregen,
            Item::Pvp(o) => o.cycle(&mut e.pvp, back),
            Item::SortKey | Item::ShaderPackKey => {}
        }
    }
    /// The key name the options screen waits on for a key item.
    pub fn key_name(self) -> Option<&'static str> {
        match self {
            Item::SortKey => Some(SORT_KEY),
            Item::ShaderPackKey => Some(SHADER_PACK_KEY),
            Item::Pvp(crate::pvp::Opt::HudEditorKey) => Some(crate::pvp::HUD_EDITOR_KEY),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn all_set() -> Extras {
        Extras {
            pack_browser: true,
            pregen: true,
            fullbright: true,
            music_off: vec!["a".into(), "b".into()],
            own_name_tag: true,
            ..Extras::default()
        }
    }

    #[test]
    fn serialize_keeps_the_format_of_main() {
        let default = include_str!("../tests/fixtures/extras_default.txt");
        assert_eq!(Extras::default().serialize(), default);
        let set = include_str!("../tests/fixtures/extras_set.txt");
        assert_eq!(all_set().serialize(), set);
    }

    #[test]
    fn lines_nobody_owns_survive_a_load_and_save() {
        let text = "ownNameTag:true\ndiscordPresence:true\nfutureKey:7\n";
        let e = Extras::parse(text);
        assert!(e.own_name_tag);
        assert_eq!(
            e.unknown,
            [
                ("discordPresence".into(), "discordPresence:true\n".into()),
                ("futureKey".into(), "futureKey:7\n".into())
            ]
        );
        let out = e.serialize();
        assert!(out.contains("\nownNameTag:true\n"));
        assert!(out.ends_with("discordPresence:true\nfutureKey:7\n"));
        assert_eq!(Extras::parse(&out), e);
        let dir = std::env::temp_dir().join(format!("extras-unknown-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let only_moved = Extras::parse("discordPresence:true\n");
        only_moved.save(&dir);
        assert!(
            fs::read_to_string(dir.join(FILE))
                .unwrap()
                .contains("discordPresence:true\n")
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_bad_value_for_a_known_key_is_dropped_not_kept() {
        let e = Extras::parse("sortKey:foo\n");
        assert!(e.unknown.is_empty());
        let out = e.serialize();
        assert_eq!(out.matches("\nsortKey:").count(), 1, "{out}");
        assert!(!out.contains("foo"));
        let e = Extras::parse("hudEditorKey:foo\n");
        assert!(e.unknown.is_empty());
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("extras-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_kept_line_goes_only_when_a_store_wrote_its_key() {
        let dir = scratch("store-wrote");
        let e = Extras::parse("discordPresence:true\nother:1\n");
        // A store that skipped the key this time (another seat's save): the line stays.
        e.save_with(&dir, "", false);
        let text = fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(text.contains("\ndiscordPresence:true\nother:1\n"), "{text}");
        // A store that wrote it: its line, once.
        e.save_with(&dir, "discordPresence:false\n", false);
        let text = fs::read_to_string(dir.join(FILE)).unwrap();
        assert_eq!(text.matches("discordPresence:").count(), 1, "{text}");
        assert!(text.ends_with("other:1\ndiscordPresence:false\n"), "{text}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn kept_lines_keep_their_spacing_and_line_ends() {
        let e = Extras::parse("futureKey : 7 \r\nno colon here\nownNameTag:true\nlast:x");
        let out = e.serialize();
        assert!(out.contains("futureKey : 7 \r\nno colon here\n"), "{out:?}");
        assert!(out.ends_with("\nlast:x"), "{out:?}");
        assert!(e.own_name_tag);
        // A store's lines after an unterminated last record start on a line of their own.
        let dir = scratch("unterminated");
        e.save_with(&dir, "discordPresence:true\n", false);
        let text = fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(
            text.ends_with("\nlast:x\ndiscordPresence:true\n"),
            "{text:?}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_that_cannot_be_read_is_never_saved_over() {
        let dir = scratch("unreadable");
        // Bytes that aren't UTF-8 cost only themselves: every other line still loads, and a
        // save leaves the file's bytes as they were.
        let bytes = b"ownNameTag:true\nname:\xff\nno colon \xfe\n";
        fs::write(dir.join(FILE), bytes).unwrap();
        let mut e = Extras::load(&dir);
        assert!(e.own_name_tag);
        e.fullbright = true;
        e.save(&dir);
        assert_eq!(fs::read(dir.join(FILE)).unwrap(), bytes);
        // A file that can't be read at all (a folder in its place) is left alone.
        let other = scratch("unreadable-dir");
        fs::create_dir_all(other.join(FILE).join("kept")).unwrap();
        let e = Extras::load(&other);
        assert!(e.keep_file);
        let mut changed = e.clone();
        changed.own_name_tag = true;
        changed.save(&other);
        assert!(other.join(FILE).join("kept").is_dir());
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(other);
    }

    #[test]
    fn a_saved_wheel_setting_from_before_the_default_changed_is_dropped() {
        let old = Extras::parse("wheelCyclesSettings:true\n");
        assert!(!old.wheel_cycles_settings && old.unknown.is_empty());
        let on = Extras {
            wheel_cycles_settings: true,
            ..Extras::default()
        };
        assert!(Extras::parse(&on.serialize()).wheel_cycles_settings);
    }

    #[test]
    fn everything_is_off_by_default_and_round_trips() {
        let d = Extras::default();
        assert!(!d.fullbright && !d.dynamic_lights);
        assert!(!d.mouse_tweaks && !d.sort && !d.own_name_tag && !d.ui_sound_reserve);
        assert!(!d.hide_block_particles && !d.pregen);
        assert!(!d.music_page && d.music_off.is_empty());
        assert!(!d.wheel_cycles_settings);
        assert_eq!(Extras::parse(""), d);
        assert_eq!(Extras::parse(&d.serialize()), d);
        let mut e = d.clone();
        Item::Fullbright.cycle(&mut e, false);
        assert_eq!(Item::Fullbright.switch(&e), Some(true));
        Item::DynamicLights.cycle(&mut e, false);
        assert_eq!(Item::DynamicLights.switch(&e), Some(true));
        assert_eq!(Extras::parse(&e.serialize()), e);
        Item::Pregen.cycle(&mut e, false);
        assert_eq!(Item::Pregen.switch(&e), Some(true));
        Item::OwnNameTag.cycle(&mut e, false);
        assert_eq!(Item::OwnNameTag.switch(&e), Some(true));
        assert_eq!(Item::HideBlockParticles.switch(&e), Some(false));
        Item::HideBlockParticles.cycle(&mut e, false);
        assert!(e.hide_block_particles);
        assert_eq!(Item::HideBlockParticles.switch(&e), Some(true));
        let only = Extras::parse("hideBlockParticles:true\n");
        assert!(only.hide_block_particles && !only.own_name_tag);
        e.mouse_tweaks = true;
        e.mouse_tweaks_wheel = false;
        e.sort = true;
        e.sort_key = "key.keyboard.r".into();
        e.chat_animation = true;
        e.chat_indicator = false;
        Item::WheelCyclesSettings.cycle(&mut e, false);
        assert_eq!(Item::WheelCyclesSettings.switch(&e), Some(true));
        e.music_off = vec![
            "minecraft:music/game/calm1".into(),
            "minecraft:music.menu".into(),
        ];
        assert!(!e.music_page);
        Item::MusicPage.cycle(&mut e, false);
        assert_eq!(Item::MusicPage.switch(&e), Some(true));
        assert_eq!(Extras::parse(&e.serialize()), e);
    }

    #[test]
    fn bad_lines_keep_defaults() {
        let e = Extras::parse("sort:maybe\nsortKey:r\nnoise");
        assert!(!e.sort);
        assert_eq!(e.sort_key, "key.keyboard.unknown");
    }

    #[test]
    fn a_mods_zoom_eases_like_the_spyglass() {
        // Camera.tickFov halves the distance to the target each tick.
        let mut camera = crate::camera::Camera::default();
        camera.tick(None, 1.0);
        for _ in 0..30 {
            camera.tick(None, 1.0);
        }
        let target = fov_target(1.0, 4.0);
        camera.tick(None, target);
        let first = camera.calculate_fov(1.0, 70, 1.0, None, Default::default(), false);
        assert!((first - 70.0 * 0.625).abs() < 1e-3, "{first}");
        for _ in 0..30 {
            camera.tick(None, target);
        }
        let settled = camera.calculate_fov(1.0, 70, 1.0, None, Default::default(), false);
        assert!((settled - 70.0 / 4.0).abs() < 1e-3, "{settled}");
        // Deeper than the 0.1 floor stops there, as vanilla clamps.
        for _ in 0..30 {
            camera.tick(None, fov_target(1.0, 20.0));
        }
        let floor = camera.calculate_fov(1.0, 70, 1.0, None, Default::default(), false);
        assert!((floor - 7.0).abs() < 1e-3, "{floor}");
    }

    #[test]
    fn fps_counter_reports_rate_minimum_and_low() {
        let mut c = FpsCounter::default();
        let mut t = 0.0;
        // Two seconds at 100 fps with one 50 ms hitch.
        for i in 0..200 {
            let ms = if i == 150 { 50.0 } else { 10.0 };
            t += f64::from(ms) / 1000.0;
            c.record(t, ms);
        }
        c.computed_at = f64::NEG_INFINITY;
        c.record(t + 0.01, 10.0);
        let r = c.reading();
        assert!((95..=101).contains(&r.fps), "{r:?}");
        assert_eq!(r.min, 20);
        assert_eq!(r.low, 100, "one frame in 200 is under the 1% mark");
        // Hitches beyond 1% of frames pull the 1% low down.
        let mut c = FpsCounter::default();
        let mut t = 0.0;
        for i in 0..100 {
            let ms = if i % 25 == 0 { 40.0 } else { 10.0 };
            t += f64::from(ms) / 1000.0;
            c.record(t, ms);
        }
        c.computed_at = f64::NEG_INFINITY;
        c.record(t, 10.0);
        assert_eq!(c.reading().low, 25);
    }

    #[test]
    fn old_frames_leave_the_window() {
        let mut c = FpsCounter::default();
        c.record(0.0, 100.0);
        for i in 1..=700 {
            c.record(f64::from(i) / 100.0, 10.0);
        }
        assert_eq!(c.reading().min, 100, "the 100 ms frame is 7 s old");
    }

    #[test]
    fn cps_counts_the_last_second() {
        let mut cps = Cps::default();
        for i in 0..10 {
            cps.click(f64::from(i) * 0.1);
        }
        assert_eq!(cps.per_second(0.95), 10);
        assert_eq!(cps.per_second(1.25), 7);
        assert_eq!(cps.per_second(3.0), 0);
    }

    #[test]
    fn items_cycle_through_their_values() {
        let mut e = Extras::default();
        Item::Sort.cycle(&mut e, true);
        assert_eq!(Item::Sort.value(&e), "On");
        for item in rows().into_iter().flatten() {
            assert!(!item.caption().is_empty() && !item.description().is_empty());
        }
    }

    /// The shader pack key is bound to K out of the box, rebinds through the generic extra-key
    /// machinery, and the remembered pick round-trips with it.
    #[test]
    fn the_shader_pack_key_binds_and_remembers_the_pick() {
        let mut e = Extras::default();
        assert_eq!(e.shader_pack_key, "key.keyboard.k");
        assert_eq!(e.last_shader_pack, "");
        assert_eq!(Item::ShaderPackKey.key_name(), Some(SHADER_PACK_KEY));
        assert!(Item::ShaderPackKey.value(&e).contains('K'));
        assert!(e.set_key(SHADER_PACK_KEY, "key.keyboard.j".into()));
        assert_eq!(e.shader_pack_key, "key.keyboard.j");
        assert!(!e.set_key("key.toggleGui", "key.keyboard.j".into()));
        e.last_shader_pack = "A.zip".into();
        let back = Extras::parse(&e.serialize());
        assert_eq!(
            (
                back.shader_pack_key.as_str(),
                back.last_shader_pack.as_str()
            ),
            ("key.keyboard.j", "A.zip")
        );
        // A bad value is dropped, not kept: the default stays.
        let bad = Extras::parse("shaderPackKey:foo\n");
        assert_eq!(bad.shader_pack_key, "key.keyboard.k");
        assert!(bad.unknown.is_empty());
    }
}
