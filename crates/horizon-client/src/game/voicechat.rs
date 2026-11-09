// Complex Voice Chat: talk to players on servers running Simple Voice Chat
// 2.6.x, with groups, a menu of its own and a lot to customize.
// The protocol, Opus, UDP and audio are in the voicechat_engine crate; this
// is the game side: its keys, settings and the connection's bookkeeping.

use std::sync::Mutex;

use horizon_client::options_ui::voicechat_menu as menu;

pub const ID: &str = "voicechat";
pub const NAME: &str = "Complex Voice Chat";
pub const VERSION: &str = "2.0.0";
pub const DESCRIPTION: &str = "Proximity voice chat with groups, for servers with the Simple Voice Chat plugin (2.6.x). V opens the menu: Players (who's talking, each one's volume and mute), Groups (make one with a name, password and type, join or leave), Audio (microphone, speakers, volumes), Talking (push to talk or voice activation, the level meter, a microphone test) and Customize (theme, opacity, size, animations, mic icon colour, the on-screen status's style, corner and size). Hold Caps Lock to talk, M mutes, J turns voice chat off.";

pub const PUSH_TO_TALK: [&str; 3] = ["push_to_talk", "Push to talk", "key.keyboard.caps.lock"];
pub const MUTE: [&str; 3] = ["mute", "Mute microphone", "key.keyboard.m"];
pub const DISABLE: [&str; 3] = ["disable", "Turn voice chat off/on", "key.keyboard.j"];
pub const MENU: [&str; 3] = ["menu", "Voice chat menu", "key.keyboard.v"];

// The steps the Mods page's buttons cycle through (the menu has sliders).
const VOLUMES: [f32; 9] = [0.0, 0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];
const GAINS: [f32; 7] = [0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0];
const THRESHOLDS: [f32; 7] = [-60.0, -55.0, -50.0, -45.0, -40.0, -35.0, -30.0];

static PAYLOADS: Mutex<Vec<(String, Vec<u8>)>> = Mutex::new(Vec::new());

/// Queues an incoming `voicechat:*` custom payload from the network/remote handler.
pub fn push_payload(channel: String, data: Vec<u8>) {
    let mut g = PAYLOADS.lock().unwrap_or_else(|e| e.into_inner());
    if g.len() < 256 {
        g.push((channel, data));
    }
}

fn take_payloads() -> Vec<(String, Vec<u8>)> {
    std::mem::take(&mut *PAYLOADS.lock().unwrap_or_else(|e| e.into_inner()))
}

/// The next step after `v` in `table`, wrapping round.
fn next(table: &[f32], v: f32) -> f32 {
    table
        .iter()
        .copied()
        .find(|t| *t > v + 1e-3)
        .unwrap_or(table[0])
}

/// The next device after `current` (None = default, first) in `list`.
fn next_device(list: &[String], current: &Option<String>) -> Option<String> {
    match current {
        None => list.first().cloned(),
        Some(c) => list
            .iter()
            .position(|n| n == c)
            .and_then(|i| list.get(i + 1).cloned()),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub voice_activation: bool,
    pub rnnoise: bool,
    pub volume: f32,
    pub mic_gain: f32,
    pub threshold_db: f32,
    pub microphone: Option<String>,
    pub speakers: Option<String>,
    /// The icons' colour, one of `voicechat_engine::icons::COLOURS`.
    pub colour: String,
    /// The menu's and the on-screen status's look (and chat messages).
    pub look: menu::Prefs,
}

/// The look's settings, by the keys they're kept under.
const LOOK_KEYS: [&str; 9] = [
    "theme",
    "opacity",
    "size",
    "animations",
    "show_hud",
    "hud_style",
    "hud_corner",
    "hud_scale",
    "announce",
];

impl Settings {
    fn read(get: impl Fn(&str) -> Option<String>) -> Self {
        let flag = |k: &str, d: bool| get(k).map_or(d, |v| v == "true");
        let num = |k: &str, d: f32, lo: f32, hi: f32| {
            get(k)
                .and_then(|v| v.parse::<f32>().ok())
                .filter(|v| v.is_finite())
                .map_or(d, |v| v.clamp(lo, hi))
        };
        let name = |k: &str| get(k).filter(|v| !v.is_empty());
        Self {
            voice_activation: flag("voice_activation", cfg!(target_os = "android")),
            rnnoise: flag("rnnoise", true),
            volume: num("volume", 2.0, 0.0, 2.0),
            mic_gain: num("mic_gain", 1.0, 0.0, 4.0),
            threshold_db: num("threshold_db", -45.0, -60.0, 0.0),
            microphone: name("microphone").filter(|n| voicechat_engine::is_valid_input_device(n)),
            speakers: name("speakers").filter(|n| voicechat_engine::is_valid_output_device(n)),
            colour: get("colour")
                .filter(|c| voicechat_engine::icons::COLOURS.contains(&c.as_str()))
                .unwrap_or_else(|| "blue".into()),
            look: {
                let mut look = menu::Prefs::default();
                for k in LOOK_KEYS {
                    if let Some(v) = get(k) {
                        look.set(k, &v);
                    }
                }
                look
            },
        }
    }
    fn look_value(&self, key: &str) -> String {
        let l = &self.look;
        match key {
            "theme" => l.theme.clone(),
            "opacity" => l.opacity.to_string(),
            "size" => l.size.to_string(),
            "animations" => l.animations.to_string(),
            "show_hud" => l.show_hud.to_string(),
            "hud_style" => l.hud_style.to_string(),
            "hud_corner" => l.hud_corner.to_string(),
            "hud_scale" => l.hud_scale.to_string(),
            "announce" => l.announce.to_string(),
            _ => String::new(),
        }
    }
    pub fn labels(&self) -> Vec<String> {
        let on_off = |b: bool| if b { "ON" } else { "OFF" };
        let pct = |v: f32| (v * 100.0).round() as i32;
        vec![
            format!(
                "Microphone: {}",
                self.microphone.as_deref().unwrap_or("Default")
            ),
            format!(
                "Speakers: {}",
                self.speakers.as_deref().unwrap_or("Default")
            ),
            format!(
                "Talk with: {}",
                if self.voice_activation {
                    "voice activation"
                } else {
                    "push to talk"
                }
            ),
            format!("Voice volume: {}%", pct(self.volume)),
            format!("Microphone boost: {}%", pct(self.mic_gain)),
            format!(
                "Voice activation level: {} dB",
                self.threshold_db.round() as i32
            ),
            format!("On-screen status: {}", on_off(self.look.show_hud)),
            format!("Messages in chat: {}", on_off(self.look.announce)),
            format!("Mic colour: {}", self.colour),
            format!("Menu theme: {}", self.look.theme),
            format!(
                "On-screen status style: {}",
                menu::HUD_STYLES[self.look.hud_style as usize % menu::HUD_STYLES.len()]
            ),
            format!(
                "On-screen status corner: {}",
                menu::HUD_CORNERS[self.look.hud_corner as usize % menu::HUD_CORNERS.len()]
            ),
            format!("Menu animations: {}", on_off(self.look.animations)),
            "More in the V menu > Customize".to_owned(),
        ]
    }
    pub fn click(&mut self, i: usize) {
        match i {
            0 => self.microphone = next_device(&voicechat_engine::microphones(), &self.microphone),
            1 => self.speakers = next_device(&voicechat_engine::speakers(), &self.speakers),
            2 => self.voice_activation = !self.voice_activation,
            3 => self.volume = next(&VOLUMES, self.volume),
            4 => self.mic_gain = next(&GAINS, self.mic_gain),
            5 => self.threshold_db = next(&THRESHOLDS, self.threshold_db),
            6 => self.look.show_hud = !self.look.show_hud,
            7 => self.look.announce = !self.look.announce,
            8 => {
                let all = voicechat_engine::icons::COLOURS;
                let i = all
                    .iter()
                    .position(|c| *c == self.colour)
                    .map_or(0, |i| (i + 1) % all.len());
                self.colour = all[i].into();
            }
            9 => {
                let i = menu::THEMES
                    .iter()
                    .position(|t| *t == self.look.theme)
                    .map_or(0, |i| (i + 1) % menu::THEMES.len());
                self.look.theme = menu::THEMES[i].into();
            }
            10 => self.look.hud_style = (self.look.hud_style + 1) % menu::HUD_STYLES.len() as u8,
            11 => self.look.hud_corner = (self.look.hud_corner + 1) % menu::HUD_CORNERS.len() as u8,
            12 => self.look.animations = !self.look.animations,
            _ => {}
        }
    }
    pub fn stored(&self) -> Vec<(&'static str, String)> {
        let mut out = vec![
            ("voice_activation", self.voice_activation.to_string()),
            ("rnnoise", self.rnnoise.to_string()),
            ("volume", self.volume.to_string()),
            ("mic_gain", self.mic_gain.to_string()),
            ("threshold_db", self.threshold_db.to_string()),
            ("microphone", self.microphone.clone().unwrap_or_default()),
            ("speakers", self.speakers.clone().unwrap_or_default()),
            ("colour", self.colour.clone()),
        ];
        out.extend(LOOK_KEYS.iter().map(|k| (*k, self.look_value(k))));
        out
    }
    pub fn engine(&self) -> voicechat_engine::Settings {
        voicechat_engine::Settings {
            voice_activation: self.voice_activation,
            rnnoise: self.rnnoise && horizon_client::android_runtime::experimental_svc_rnnoise(),
            volume: self.volume,
            mic_gain: self.mic_gain,
            threshold_db: self.threshold_db,
            input_device: self.microphone.clone(),
            output_device: self.speakers.clone(),
        }
    }
}

/// The icon for where voice chat is now.
pub fn state_icon(s: &voicechat_engine::Status) -> voicechat_engine::icons::Icon {
    use voicechat_engine::{State, icons::Icon};
    match s.state {
        _ if s.disabled => Icon::Disabled,
        State::Off | State::Waiting | State::Connecting | State::Lost => Icon::Connecting,
        State::Connected if s.muted => Icon::Muted,
        State::Connected if s.transmitting => Icon::Talking,
        State::Connected => Icon::Idle,
    }
}

/// An icon's runs, empty if it's missing.
pub fn icon_runs(colour: &str, icon: voicechat_engine::icons::Icon) -> Vec<(u32, u32, u32, u32)> {
    voicechat_engine::icons::icon(colour, icon)
        .map(voicechat_engine::icons::runs)
        .unwrap_or_default()
}

/// A key's name as written on a keyboard: `key.keyboard.caps.lock` -> CAPS LOCK.
pub fn key_text(key: &str) -> String {
    match key.strip_prefix("key.keyboard.") {
        Some(k) => k.replace('.', " ").to_uppercase(),
        None => key.to_owned(),
    }
}

/// The device lists the menu shows, found again when its pickers open.
pub fn devices(refresh: bool) -> (Vec<String>, Vec<String>) {
    static LISTS: Mutex<Option<(Vec<String>, Vec<String>)>> = Mutex::new(None);
    let mut guard = LISTS.lock().unwrap_or_else(|e| e.into_inner());
    if refresh || guard.is_none() {
        *guard = Some((
            voicechat_engine::microphones(),
            voicechat_engine::speakers(),
        ));
    }
    guard.clone().unwrap_or_default()
}

#[derive(Default)]
struct RuntimeState {
    settings: Option<Settings>,
    last_exp_rnnoise: Option<bool>,
    /// The server and profile voice chat joined, while on a server.
    joined: Option<(String, u128)>,
    /// The last problem said in chat.
    told: Option<String>,
    told_connected: bool,
    /// Touch HUD button rectangles `[x, y, w, h]` in GUI-scaled coordinates: `[menu, mute, ptt]`.
    touch_buttons: [Option<[f32; 4]>; 3],
    /// Active touch ID holding the on-screen PTT button.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    ptt_touch_id: Option<u64>,
    /// Counter for periodic Android AudioManager volume/routing synchronization.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    audio_sync_ticks: u32,
}

fn ensure_platform_hooks() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        horizon_client::audio::set_output_tap(voicechat_engine::push_game_audio_reference);
        #[cfg(target_os = "android")]
        voicechat_engine::set_platform_hooks(
            crate::platform::ensure_microphone_permission,
            crate::platform::on_voice_mic_open,
            crate::platform::on_voice_mic_close,
        );
    });
}

fn check_mic_permission() -> bool {
    #[cfg(target_os = "android")]
    {
        return crate::platform::ensure_microphone_permission();
    }
    #[cfg(not(target_os = "android"))]
    {
        true
    }
}

fn with_state<T>(f: impl FnOnce(&mut RuntimeState) -> T) -> T {
    static STATE: Mutex<Option<RuntimeState>> = Mutex::new(None);
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(RuntimeState::default))
}

/// The settings, read once from the mod's data folder.
pub fn settings(get: impl Fn(&str) -> Option<String>) -> Settings {
    let exp_rnnoise = horizon_client::android_runtime::experimental_svc_rnnoise();
    let (s, fresh) = with_state(|st| match &st.settings {
        Some(s) => {
            let changed = st.last_exp_rnnoise != Some(exp_rnnoise);
            st.last_exp_rnnoise = Some(exp_rnnoise);
            (s.clone(), changed)
        }
        None => {
            let s = Settings::read(get);
            st.settings = Some(s.clone());
            st.last_exp_rnnoise = Some(exp_rnnoise);
            (s, true)
        }
    });
    if fresh {
        voicechat_engine::set_settings(s.engine());
    }
    s
}

pub fn set_settings(s: Settings) {
    voicechat_engine::set_settings(s.engine());
    with_state(|st| st.settings = Some(s));
}

/// What the tick should do about joining: Some(true) join, Some(false) leave.
pub fn session_change(now: Option<(String, u128)>) -> Option<bool> {
    with_state(|st| {
        if st.joined == now {
            return None;
        }
        let was = st.joined.is_some();
        st.joined = now.clone();
        st.told = None;
        st.told_connected = false;
        match (was, now.is_some()) {
            (_, true) => Some(true),
            (true, false) => Some(false),
            (false, false) => None,
        }
    })
}

/// A chat line for a status change worth telling, once.
pub fn news(status: &voicechat_engine::Status) -> Option<String> {
    with_state(|st| {
        if status.state == voicechat_engine::State::Connected && !st.told_connected {
            st.told_connected = true;
            return Some(format!(
                "Complex Voice Chat connected. {}",
                if status.voice_activation {
                    "Voice activation is on; V opens the menu."
                } else {
                    "Press V for the menu."
                }
            ));
        }
        match &status.problem {
            Some(p) if st.told.as_deref() != Some(p) => {
                st.told = Some(p.clone());
                Some(format!("Complex Voice Chat: {p}"))
            }
            _ => None,
        }
    })
}

// HUD colours (ARGB).
pub const PANEL: u32 = 0x9010_1018;
pub const GREEN: u32 = 0x55FF55;
pub const RED: u32 = 0xFF5555;
pub const GREY: u32 = 0xAAAAAA;
pub const YELLOW: u32 = 0xFFDD55;
pub const WHITE: u32 = 0xFFFFFF;
pub const PAD: f32 = 3.0;
pub const LINE: f32 = 10.0;

/// Applies the menu's clicks, then gives it the state to show.
pub(super) fn sync_menu(game: &mut crate::game::Game<'_>) {
    use horizon_client::options_ui::voicechat_menu::{Action, GroupRow, PlayerRow, View};
    ensure_platform_hooks();
    let mut s = {
        let game = &*game;
        settings(|k| game.native_setting(ID, k))
    };
    let before = s.clone();
    let mut refresh = false;
    let mut out = Vec::new();
    for action in menu::take_actions() {
        match action {
            Action::Mute => {
                let _ = check_mic_permission();
                voicechat_engine::toggle_mute();
            }
            Action::Disable => out.extend(voicechat_engine::toggle_disabled().1),
            Action::VoiceActivation(on) => {
                if on {
                    let _ = check_mic_permission();
                }
                s.voice_activation = on;
            }
            Action::Rnnoise(on) => s.rnnoise = on,
            Action::Volume(v) => s.volume = v,
            Action::MicGain(v) => s.mic_gain = v,
            Action::Threshold(v) => s.threshold_db = v,
            Action::Microphone(name) => s.microphone = name,
            Action::Speakers(name) => s.speakers = name,
            Action::Test(on) => {
                if on && !check_mic_permission() {
                    continue;
                }
                voicechat_engine::set_testing(on);
            }
            Action::PlayerVolume(uuid, v) => voicechat_engine::set_player_volume(uuid, v),
            Action::RefreshDevices => refresh = true,
            Action::Colour(c) => s.colour = c,
            Action::CreateGroup {
                name,
                password,
                kind,
            } => {
                let kind = voicechat_engine::GroupKind::ALL[(kind as usize).min(2)];
                out.extend(voicechat_engine::create_group(
                    &name,
                    password.as_deref(),
                    kind,
                ));
            }
            Action::JoinGroup(id, password) => {
                out.extend(voicechat_engine::join_group(id, password.as_deref()));
            }
            Action::LeaveGroup => out.extend(voicechat_engine::leave_group()),
            Action::Set(key, value) => {
                s.look.set(key, &value);
            }
        }
    }
    for (channel, data) in out {
        game.send(horizon_protocol::packets::play::Serverbound::CustomPayload(
            horizon_protocol::packets::configuration::CustomPayload { channel, data },
        ));
    }
    if s != before {
        set_settings(s.clone());
        for (key, value) in s.stored() {
            game.native_set_setting(ID, key, &value);
        }
    }
    let (microphones, speaker_list) = devices(refresh);
    let st = voicechat_engine::status();
    use voicechat_engine::State;
    let (state, state_good) = match st.state {
        State::Off => ("Not on a voice chat server".to_owned(), false),
        State::Waiting => ("Asking the server...".to_owned(), false),
        State::Connecting => ("Connecting...".to_owned(), false),
        State::Connected => ("Connected".to_owned(), true),
        State::Lost => ("Connection lost".to_owned(), false),
    };
    let group_name = |id: Option<u128>| {
        id.and_then(|id| st.groups.iter().find(|g| g.id == id))
            .map(|g| g.name.clone())
    };
    let players = st
        .players
        .iter()
        .map(|p| PlayerRow {
            uuid: p.uuid,
            name: p.name.clone(),
            // Talking shows as the speaker icon and the green name: no words.
            status: if p.disabled {
                "voice chat off".into()
            } else if p.disconnected {
                "not connected".into()
            } else {
                String::new()
            },
            talking: p.talking,
            volume: p.volume,
            group: group_name(p.group),
        })
        .collect();
    let talking_ids: Vec<u128> = st
        .players
        .iter()
        .filter(|p| p.talking)
        .map(|p| p.uuid)
        .collect();
    let groups = st
        .groups
        .iter()
        .map(|g| GroupRow {
            id: g.id,
            name: g.name.clone(),
            locked: g.has_password,
            kind: match g.kind {
                voicechat_engine::GroupKind::Normal => 0,
                voicechat_engine::GroupKind::Open => 1,
                voicechat_engine::GroupKind::Isolated => 2,
            },
            persistent: g.persistent,
            members: g
                .members
                .iter()
                .map(|(id, n)| {
                    let me = *id == st.own;
                    (
                        if me { format!("{n} (you)") } else { n.clone() },
                        if me {
                            st.transmitting
                        } else {
                            talking_ids.contains(id)
                        },
                    )
                })
                .collect(),
        })
        .collect();
    menu::set_view(View {
        state,
        state_good,
        connected: st.state == State::Connected,
        muted: st.muted,
        disabled: st.disabled,
        transmitting: st.transmitting,
        voice_activation: s.voice_activation,
        rnnoise: s.rnnoise,
        talk_key: key_text(&game.native_mod_key(ID, PUSH_TO_TALK[0], PUSH_TO_TALK[2])),
        menu_key: key_text(&game.native_mod_key(ID, MENU[0], MENU[2])),
        mute_key: key_text(&game.native_mod_key(ID, MUTE[0], MUTE[2])),
        off_key: key_text(&game.native_mod_key(ID, DISABLE[0], DISABLE[2])),
        volume: s.volume,
        mic_gain: s.mic_gain,
        threshold_db: s.threshold_db,
        level_db: st.level_db,
        testing: st.testing,
        microphone: s.microphone.clone(),
        speakers: s.speakers.clone(),
        microphones,
        speaker_list,
        players,
        icon: icon_runs(&s.colour, state_icon(&st)),
        speaker_icon: icon_runs(&s.colour, voicechat_engine::icons::Icon::Speaker),
        group_icon: icon_runs(&s.colour, voicechat_engine::icons::Icon::Group),
        swatches: voicechat_engine::icons::COLOURS
            .iter()
            .map(|c| {
                use voicechat_engine::icons::Icon;
                (
                    c.to_string(),
                    icon_runs(c, Icon::Idle),
                    icon_runs(c, Icon::Talking),
                )
            })
            .collect(),
        colour: s.colour.clone(),
        problem: st.problem,
        group: st.group,
        own_group: st.own_group,
        groups,
        groups_enabled: st.groups_enabled,
        group_problem: st.group_problem,
        prefs: s.look.clone(),
    });
}

fn voicechat_enabled(game: &crate::game::Game<'_>) -> bool {
    game.native_on(ID) && horizon_client::android_runtime::experimental_svc_enabled()
}

/// Each tick: join or leave voice chat with the server, hand the server's
/// voice chat messages to the engine and send its replies, tell the engine
/// where the listener and the talking players are, and say news in chat.
pub(super) fn tick(game: &mut crate::game::Game<'_>) {
    if !game.native_window() {
        return;
    }
    ensure_platform_hooks();
    let on = voicechat_enabled(game);
    let settings = {
        let game = &*game;
        settings(|k| game.native_setting(ID, k))
    };
    let server = match &game.remote {
        Some(r) if on && r.session.address != "local" => {
            Some((r.session.address.clone(), r.profile()))
        }
        _ => None,
    };
    let mut out: Vec<voicechat_engine::Payload> = Vec::new();
    match session_change(server.clone()) {
        Some(true) => {
            let _ = check_mic_permission();
            let address = server.map(|s| s.0).unwrap_or_default();
            out.extend(voicechat_engine::join(move || {
                horizon_protocol::servers::resolve_address_ip(&address)
            }));
        }
        Some(false) => voicechat_engine::leave(),
        None => {}
    }
    if on && game.remote.is_some() {
        out.extend(voicechat_engine::poll_handshake_retry());
        #[cfg(target_os = "android")]
        {
            let should_sync = with_state(|st| {
                st.audio_sync_ticks = st.audio_sync_ticks.wrapping_add(1);
                st.audio_sync_ticks % 40 == 0
            });
            if should_sync {
                crate::platform::sync_voice_audio_state();
            }
        }
    }
    for (channel, data) in take_payloads() {
        if on && game.remote.is_some() {
            out.extend(voicechat_engine::payload(&channel, &data));
        }
    }
    for (channel, data) in out {
        game.send(horizon_protocol::packets::play::Serverbound::CustomPayload(
            horizon_protocol::packets::configuration::CustomPayload { channel, data },
        ));
    }
    let status = voicechat_engine::status();
    if let Some(p) = game.native_player() {
        let remote = game.remote.as_ref();
        voicechat_engine::update_listener([p.x, p.y, p.z], p.yaw, |uuid| {
            let r = remote?;
            if let Some(id) = r.player_entity(uuid)
                && let Some((_, at)) = r.tracked(id as i32)
            {
                return Some(at);
            }
            // Stage 2: Offline-mode / proxy fallback by player name when the voice UUID
            // differs from the entity's UUID in the world.
            let name = status
                .players
                .iter()
                .find(|pl| pl.uuid == uuid)
                .map(|pl| pl.name.as_str())?;
            let world_uuid = r
                .tab_players
                .iter()
                .find_map(|(&u, tp)| tp.name.eq_ignore_ascii_case(name).then_some(u))
                .or_else(|| {
                    r.players
                        .iter()
                        .find_map(|(&u, pname)| pname.eq_ignore_ascii_case(name).then_some(u))
                })?;
            let id = r.player_entity(world_uuid)?;
            r.tracked(id as i32).map(|(_, at)| at)
        });
    }
    if on && settings.look.announce {
        if let Some(line) = news(&status) {
            game.native_chat(&line);
        }
    }
}

/// Push to talk (held), mute and voice chat off/on.
pub(super) fn key(game: &mut crate::game::Game<'_>, key: &str, pressed: bool) {
    if !voicechat_enabled(game) {
        return;
    }
    if key == game.native_mod_key(ID, PUSH_TO_TALK[0], PUSH_TO_TALK[2]) {
        // Released anywhere; pressed only while playing (not typing in chat).
        let active = pressed && game.native_playing();
        if active && !check_mic_permission() {
            return;
        }
        voicechat_engine::push_to_talk(active);
        return;
    }
    if pressed && game.native_window() && key == game.native_mod_key(ID, MENU[0], MENU[2]) {
        use horizon_client::options_ui::Page;
        if game.native_playing() {
            // Over the world, as Mod Menu's panel: closing goes back to playing.
            voicechat_engine::push_to_talk(false);
            game.capture(false);
            game.open_options_at(true, Page::Credits);
            game.ui.options.in_world = true;
            menu::open();
            sync_menu(game);
        } else if game.screen == crate::game::Screen::Options
            && menu::shows(&game.ui.options)
            && !menu::typing()
        {
            let _ = game.options_key(winit::keyboard::KeyCode::Escape, None);
        }
        return;
    }
    if !pressed || !game.native_playing() {
        return;
    }
    let announce = {
        let game = &*game;
        settings(|k| game.native_setting(ID, k)).look.announce
    };
    if key == game.native_mod_key(ID, MUTE[0], MUTE[2]) {
        let _ = check_mic_permission();
        let muted = voicechat_engine::toggle_mute();
        if announce {
            game.native_chat(if muted {
                "Complex Voice Chat: microphone muted"
            } else {
                "Complex Voice Chat: microphone on"
            });
        }
    } else if key == game.native_mod_key(ID, DISABLE[0], DISABLE[2]) {
        let (off, out) = voicechat_engine::toggle_disabled();
        for (channel, data) in out {
            game.send(horizon_protocol::packets::play::Serverbound::CustomPayload(
                horizon_protocol::packets::configuration::CustomPayload { channel, data },
            ));
        }
        if announce {
            game.native_chat(if off {
                "Complex Voice Chat: OFF (you hear nobody and nobody hears you)"
            } else {
                "Complex Voice Chat: ON"
            });
        }
    }
}

fn touch_hud_visible() -> bool {
    #[cfg(target_os = "android")]
    {
        return crate::touch_visible();
    }
    #[cfg(not(target_os = "android"))]
    {
        false
    }
}

/// The on-screen status, in the corner and at the size picked in the V menu's
/// Customize page.
pub(super) fn hud(
    game: &mut crate::game::Game<'_>,
    batch: &mut horizon_client::hud::HudBatch,
    corners: &mut horizon_client::extras_hud::Corners,
) {
    use horizon_client::extras::Corner;
    use voicechat_engine::State;
    with_state(|st| st.touch_buttons = [None; 3]);
    if !voicechat_enabled(game) || game.remote.is_none() {
        return;
    }
    let settings = {
        let game = &*game;
        settings(|k| game.native_setting(ID, k))
    };
    let look = &settings.look;
    let corner = match look.hud_corner {
        1 => Corner::BottomRight,
        2 => Corner::TopLeft,
        3 => Corner::TopRight,
        _ => Corner::BottomLeft,
    };
    if !look.show_hud || !game.native_corner_shown(corner) {
        return;
    }
    let s = voicechat_engine::status();
    let key = key_text(&game.native_mod_key(ID, PUSH_TO_TALK[0], PUSH_TO_TALK[2]));
    let head: (String, u32) = match s.state {
        State::Off | State::Waiting => return,
        State::Connecting => ("Connecting...".into(), YELLOW),
        State::Lost => ("Connection lost".into(), RED),
        State::Connected if s.disabled => ("Voice chat off".into(), GREY),
        State::Connected if s.muted => ("Muted".into(), RED),
        State::Connected if s.transmitting => ("Talking".into(), GREEN),
        State::Connected if s.voice_activation => ("Ready".into(), WHITE),
        State::Connected => (format!("Hold {key}"), WHITE),
    };
    let scale = look.hud_scale.clamp(1.0, 3.0);
    const ICON: f32 = 16.0;
    let mic = icon_runs(&settings.colour, state_icon(&s));
    let group_icon = icon_runs(&settings.colour, voicechat_engine::icons::Icon::Group);
    let speaker = icon_runs(&settings.colour, voicechat_engine::icons::Icon::Speaker);
    let draw = |game: &crate::game::Game<'_>,
                batch: &mut horizon_client::hud::HudBatch,
                runs: &[(u32, u32, u32, u32)],
                at: [f32; 2],
                px: f32| {
        for &(rx, ry, len, argb) in runs {
            game.native_fill(
                batch,
                [
                    at[0] + rx as f32 * px,
                    at[1] + ry as f32 * px,
                    len as f32 * px,
                    px,
                ],
                argb,
            );
        }
    };
    let start = batch.vertices.len();
    let (x, y) = if look.hud_style == 0 {
        let w = if s.group.is_some() {
            ICON * 2.0 + 2.0
        } else {
            ICON
        };
        let (x, y) = corners.place(corner, w * scale, ICON * scale);
        draw(game, batch, &mic, [x, y], 1.0);
        if s.group.is_some() {
            draw(game, batch, &group_icon, [x + ICON + 2.0, y], 1.0);
        }
        (x, y)
    } else {
        let mut lines: Vec<(String, u32, bool)> = vec![(head.0, head.1, false)];
        if let Some(g) = &s.group {
            lines.push((g.clone(), YELLOW, true));
        }
        if look.hud_style >= 2 {
            for name in s.talking.iter().take(6) {
                lines.push((name.clone(), GREEN, true));
            }
        }
        let text_x = ICON + 4.0;
        let w = lines
            .iter()
            .map(|(t, _, talker)| game.native_text_width(t) + if *talker { 10.0 } else { 0.0 })
            .fold(0.0, f32::max)
            + text_x
            + 2.0 * PAD;
        let h = (lines.len() as f32 * LINE - 1.0).max(ICON) + 2.0 * PAD;
        let (x, y) = corners.place(corner, w * scale, h * scale);
        game.native_fill(batch, [x, y, w, h], PANEL);
        draw(game, batch, &mic, [x + PAD, y + PAD], 1.0);
        let mut ty = y + PAD;
        for (k, (text, rgb, talker)) in lines.iter().enumerate() {
            let mut tx = x + PAD + text_x;
            if *talker {
                let icon = if k == 1 && s.group.is_some() {
                    &group_icon
                } else {
                    &speaker
                };
                draw(game, batch, icon, [tx, ty], 0.5);
                tx += 10.0;
            }
            game.native_text(batch, text, [tx, ty], *rgb, true);
            ty += LINE;
        }
        (x, y)
    };
    if scale != 1.0 {
        batch.scale_since(start, x, y, scale, scale);
    }

    if touch_hud_visible() && game.native_playing() {
        let btn_h = 18.0;
        let gap = 3.0;
        let show_ptt = s.state == State::Connected && !s.voice_activation;
        let mute_lbl = if s.muted { "Unmute" } else { "Mute" };
        let ptt_lbl = if s.transmitting { "Talking" } else { "PTT" };
        let w_menu = game.native_text_width("V") + 10.0;
        let w_mute = game.native_text_width(mute_lbl) + 10.0;
        let w_ptt = if show_ptt {
            game.native_text_width(ptt_lbl) + 10.0
        } else {
            0.0
        };
        let total_w = w_menu + gap + w_mute + if show_ptt { gap + w_ptt } else { 0.0 };
        let (bx, by) = corners.place(corner, total_w, btn_h);
        let draw_touch_btn = |game: &crate::game::Game<'_>,
                              batch: &mut horizon_client::hud::HudBatch,
                              rect: [f32; 4],
                              label: &str,
                              fg: u32| {
            game.native_fill(batch, rect, PANEL);
            let tw = game.native_text_width(label);
            game.native_text(
                batch,
                label,
                [rect[0] + (rect[2] - tw) * 0.5, rect[1] + 5.0],
                fg,
                true,
            );
        };
        let r_menu = [bx, by, w_menu, btn_h];
        let r_mute = [bx + w_menu + gap, by, w_mute, btn_h];
        draw_touch_btn(game, batch, r_menu, "V", WHITE);
        draw_touch_btn(
            game,
            batch,
            r_mute,
            mute_lbl,
            if s.muted { RED } else { GREEN },
        );
        let r_ptt = if show_ptt {
            let r = [r_mute[0] + w_mute + gap, by, w_ptt, btn_h];
            draw_touch_btn(
                game,
                batch,
                r,
                ptt_lbl,
                if s.transmitting { GREEN } else { YELLOW },
            );
            Some(r)
        } else {
            None
        };
        with_state(|st| {
            st.touch_buttons = [Some(r_menu), Some(r_mute), r_ptt];
        });
    }
}

/// Handles touch input on the on-screen voice chat HUD buttons (`[V]`, `[Mute]`, `[PTT]`).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(super) fn touch_hud(
    game: &mut crate::game::Game<'_>,
    id: u64,
    point: [f32; 2],
    started: bool,
    ended: bool,
) -> bool {
    let (btns, ptt_id) = with_state(|st| {
        if ended && st.ptt_touch_id == Some(id) {
            st.ptt_touch_id = None;
            return (st.touch_buttons, Some(id));
        }
        (st.touch_buttons, st.ptt_touch_id)
    });
    if ended && ptt_id == Some(id) {
        voicechat_engine::push_to_talk(false);
        return true;
    }
    if ptt_id == Some(id) {
        return true;
    }
    if !started || !voicechat_enabled(game) || !game.native_playing() {
        return false;
    }
    let inside = |r: [f32; 4]| {
        point[0] >= r[0] && point[0] <= r[0] + r[2] && point[1] >= r[1] && point[1] <= r[1] + r[3]
    };
    if let Some(r) = btns[0]
        && inside(r)
    {
        let menu_key = game.native_mod_key(ID, MENU[0], MENU[2]);
        key(game, &menu_key, true);
        return true;
    }
    if let Some(r) = btns[1]
        && inside(r)
    {
        let mute_key = game.native_mod_key(ID, MUTE[0], MUTE[2]);
        key(game, &mute_key, true);
        return true;
    }
    if let Some(r) = btns[2]
        && inside(r)
    {
        if check_mic_permission() {
            with_state(|st| st.ptt_touch_id = Some(id));
            voicechat_engine::push_to_talk(true);
        }
        return true;
    }
    false
}

/// Each frame: tear down voice session if we left a server to the title screen,
/// and while the voice chat menu is open, apply what was clicked and hand the
/// menu the latest state.
pub(super) fn frame(game: &mut crate::game::Game<'_>) {
    if game.remote.is_none() && session_change(None) == Some(false) {
        voicechat_engine::leave();
    }
    if !menu::is_open() {
        return;
    }
    if game.screen != crate::game::Screen::Options || !menu::shows(&game.ui.options) {
        menu::closed();
        voicechat_engine::set_testing(false);
        if game.screen == crate::game::Screen::Playing {
            game.capture(true);
        }
        return;
    }
    if menu::take_done() {
        let _ = game.options_key(winit::keyboard::KeyCode::Escape, None);
        return;
    }
    sync_menu(game);
}

/// The mod's row in Options > Mods, with its keys.
pub(super) fn listed(mods: &mut Vec<crate::game::native_hooks::NativeMod>) {
    mods.push(crate::game::native_hooks::NativeMod {
        id: ID.into(),
        title: format!("{NAME} {VERSION}"),
        description: DESCRIPTION.into(),
        keys: [MENU, PUSH_TO_TALK, MUTE, DISABLE]
            .iter()
            .map(|k| k.map(String::from))
            .collect(),
    });
}

/// The mod's settings page.
pub(super) fn page(game: &mut crate::game::Game<'_>, mod_id: &str, buttons: &mut Vec<String>) {
    if mod_id != ID {
        return;
    }
    let game = &*game;
    buttons.extend(settings(|k| game.native_setting(ID, k)).labels());
}

/// A click on the settings page: change it, keep it, apply it.
pub(super) fn click(game: &mut crate::game::Game<'_>, mod_id: &str, button: usize) {
    if mod_id != ID {
        return;
    }
    let mut s = {
        let game = &*game;
        settings(|k| game.native_setting(ID, k))
    };
    s.click(button);
    set_settings(s.clone());
    for (key, value) in s.stored() {
        game.native_set_setting(ID, key, &value);
    }
}
