// Complex Voice Chat's menu (V in a world): the Options screen's Credits
// page drawn as a panel of its own while the menu is open.
//
// A sidebar on the left picks the page, and always has the two switches
// that matter most (microphone and voice chat on/off):
//   Players    everyone with voice chat: who's talking, their group, a
//              volume slider and a mute button each, and a search box
//   Groups     your group and its members, every group on the server
//              (join, with a password if it has one), make a new group
//   Audio      microphone, speakers, voice volume, microphone boost
//   Talking    push to talk or voice activation, the activation level with
//              a live meter, the microphone test, the keys
//   Customize  the menu's theme, see-through-ness, size and animations, the
//              mic icon's colour, the on-screen status (style, corner, size)
//              and chat messages
//
// This module only draws and takes clicks and typing: the game hands it a
// `View` each frame and applies the `Action`s it queues (the voice engine
// lives in the game crate).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use super::{OptionsUi, Page};
use crate::hud::{HudAtlas, HudBatch, rgba};
use crate::text::Text;

type Rect = [f32; 4];
type Rgba = (u8, u8, u8, u8);

const EDGE: Rgba = (255, 255, 255, 28);
const BUTTON: Rgba = (255, 255, 255, 16);
const BUTTON_HOVER: Rgba = (255, 255, 255, 36);
const TRACK: Rgba = (0, 0, 0, 140);
const TRACK_OFF: Rgba = (62, 66, 76, 255);
const FIELD: Rgba = (0, 0, 0, 150);
const GOOD: Rgba = (92, 230, 138, 255);
const BAD: Rgba = (255, 96, 96, 255);
const WARN: Rgba = (255, 206, 92, 255);
const WHITE: Rgba = (255, 255, 255, 255);
const DIM: Rgba = (165, 172, 186, 255);
const FAINT: Rgba = (100, 107, 120, 255);
const SHADOW: Rgba = (0, 0, 0, 90);

/// The panel's height of header, sidebar width, a row, the gaps.
const TOP: f32 = 28.0;
const SIDE: f32 = 92.0;
const ROW: f32 = 20.0;
const GAP: f32 = 4.0;
const PAD: f32 = 8.0;
/// Small, normal and large panels (at most; smaller screens shrink them).
const SIZES_PX: [(f32, f32); 3] = [(360.0, 214.0), (430.0, 262.0), (510.0, 312.0)];

pub const THEMES: [&str; 6] = ["blue", "grey", "black", "white", "red", "rainbow"];
pub const SIZES: [&str; 3] = ["Small", "Normal", "Large"];
pub const HUD_STYLES: [&str; 3] = ["Icon only", "Icon + status", "Full"];
pub const HUD_CORNERS: [&str; 4] = ["Bottom left", "Bottom right", "Top left", "Top right"];
pub const GROUP_KINDS: [(&str, &str); 3] = [
    ("Normal", "Members hear each other and players nearby."),
    ("Open", "Players nearby hear the group as well."),
    ("Isolated", "Members hear only each other."),
];

/// An icon as runs of one colour: (x, y, length, 0xAARRGGBB).
pub type Runs = Vec<(u32, u32, u32, u32)>;

/// How the player wants things to look; the game keeps them.
#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    /// One of `THEMES`.
    pub theme: String,
    /// The panel's opacity, 0.5 to 1.
    pub opacity: f32,
    /// Index in `SIZES`.
    pub size: u8,
    pub animations: bool,
    pub show_hud: bool,
    /// Index in `HUD_STYLES`, `HUD_CORNERS`.
    pub hud_style: u8,
    pub hud_corner: u8,
    /// 1 to 3.
    pub hud_scale: f32,
    pub announce: bool,
}
impl Default for Prefs {
    fn default() -> Self {
        Self {
            theme: "blue".into(),
            opacity: 0.96,
            size: 1,
            animations: true,
            show_hud: true,
            hud_style: 0,
            hud_corner: 0,
            hud_scale: 1.0,
            announce: true,
        }
    }
}
impl Prefs {
    /// Apply `key = value` (as `Action::Set` carries it); false if unknown.
    pub fn set(&mut self, key: &str, value: &str) -> bool {
        let flag = || value == "true";
        let num = |lo: f32, hi: f32, d: f32| {
            value
                .parse::<f32>()
                .ok()
                .filter(|v| v.is_finite())
                .map_or(d, |v| v.clamp(lo, hi))
        };
        let index = |n: usize| {
            value
                .parse::<u8>()
                .ok()
                .filter(|v| (*v as usize) < n)
                .unwrap_or(0)
        };
        match key {
            "theme" if THEMES.contains(&value) => self.theme = value.to_owned(),
            "opacity" => self.opacity = num(0.5, 1.0, 0.96),
            "size" => self.size = index(SIZES.len()),
            "animations" => self.animations = flag(),
            "show_hud" => self.show_hud = flag(),
            "hud_style" => self.hud_style = index(HUD_STYLES.len()),
            "hud_corner" => self.hud_corner = index(HUD_CORNERS.len()),
            "hud_scale" => self.hud_scale = num(1.0, 3.0, 1.0),
            "announce" => self.announce = flag(),
            _ => return false,
        }
        true
    }
}

/// What the game shows in the menu, refreshed each frame.
#[derive(Clone, Debug, Default)]
pub struct View {
    /// "Connected", "Not on a voice chat server", ...
    pub state: String,
    pub state_good: bool,
    pub connected: bool,
    pub muted: bool,
    pub disabled: bool,
    pub transmitting: bool,
    pub voice_activation: bool,
    pub rnnoise: bool,
    /// The keys, readable (`CAPS LOCK`, `V`).
    pub talk_key: String,
    pub menu_key: String,
    pub mute_key: String,
    pub off_key: String,
    pub volume: f32,
    pub mic_gain: f32,
    pub threshold_db: f32,
    pub level_db: f32,
    pub testing: bool,
    /// The chosen devices (None = default) and what's on offer.
    pub microphone: Option<String>,
    pub speakers: Option<String>,
    pub microphones: Vec<String>,
    pub speaker_list: Vec<String>,
    pub players: Vec<PlayerRow>,
    pub problem: Option<String>,
    pub group: Option<String>,
    pub own_group: Option<u128>,
    pub groups: Vec<GroupRow>,
    pub groups_enabled: bool,
    pub group_problem: Option<String>,
    /// The mic colour chosen, its icon for the state now, the icon shown
    /// next to talking players, the group icon, and each colour's idle and
    /// talking icons.
    pub colour: String,
    pub icon: Runs,
    pub speaker_icon: Runs,
    pub group_icon: Runs,
    pub swatches: Vec<(String, Runs, Runs)>,
    pub prefs: Prefs,
}

#[derive(Clone, Debug, Default)]
pub struct PlayerRow {
    pub uuid: u128,
    pub name: String,
    /// "voice chat off", "not connected" or "".
    pub status: String,
    pub talking: bool,
    pub volume: f32,
    /// Their group's name.
    pub group: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct GroupRow {
    pub id: u128,
    pub name: String,
    pub locked: bool,
    /// Index in `GROUP_KINDS`.
    pub kind: u8,
    pub persistent: bool,
    /// Members' names, and whether each is talking.
    pub members: Vec<(String, bool)>,
}

/// What the player did in the menu, for the game to apply.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Mute,
    Disable,
    VoiceActivation(bool),
    Rnnoise(bool),
    Volume(f32),
    MicGain(f32),
    Threshold(f32),
    Microphone(Option<String>),
    Speakers(Option<String>),
    Test(bool),
    PlayerVolume(u128, f32),
    /// The device pickers opened: list the devices again.
    RefreshDevices,
    Colour(String),
    CreateGroup {
        name: String,
        password: Option<String>,
        kind: u8,
    },
    JoinGroup(u128, Option<String>),
    LeaveGroup,
    /// A look preference (`Prefs::set`).
    Set(&'static str, String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Players,
    Groups,
    Audio,
    Talking,
    Look,
    Microphones,
    Speakers,
    Create,
    /// The password for group `.0`.
    Password(u128),
}

const SIDEBAR: [(Tab, &str); 5] = [
    (Tab::Players, "Players"),
    (Tab::Groups, "Groups"),
    (Tab::Audio, "Audio"),
    (Tab::Talking, "Talking"),
    (Tab::Look, "Customize"),
];

impl Tab {
    /// The sidebar page this one is part of.
    fn home(self) -> Tab {
        match self {
            Tab::Microphones | Tab::Speakers => Tab::Audio,
            Tab::Create | Tab::Password(_) => Tab::Groups,
            t => t,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Field {
    Search,
    Name,
    Password,
    JoinPassword,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Slider {
    Volume,
    Gain,
    Threshold,
    Opacity,
    HudScale,
    Player(u128),
}

#[derive(Clone, Debug, PartialEq)]
enum Act {
    Do(Action),
    Go(Tab),
    /// Pick a device (None = default) and go back to Audio.
    Pick(bool, Option<String>),
    Focus(Field),
    /// The kind of group to make.
    Kind(u8),
    /// A group's row: join it (asking for its password if `.1`).
    Join(u128, bool),
    /// Create the group / join with the password typed.
    Submit,
}

#[derive(Clone, Debug)]
enum Kind {
    Button {
        label: String,
        lit: Option<bool>,
        act: Act,
        primary: bool,
    },
    Toggle {
        label: String,
        on: bool,
        act: Act,
    },
    Slider {
        id: Slider,
        label: String,
        value: f32,
        min: f32,
        max: f32,
        text: String,
        meter: Option<f32>,
    },
    Text {
        text: String,
        color: Rgba,
    },
    Heading {
        text: String,
    },
    Player {
        row: PlayerRow,
    },
    Group {
        row: GroupRow,
        joined: bool,
    },
    Input {
        field: Field,
        text: String,
        placeholder: String,
        secret: bool,
        focused: bool,
    },
    Swatch {
        name: String,
        idle: Runs,
        talking: Runs,
        lit: bool,
    },
    Theme {
        name: String,
        lit: bool,
    },
    Tab {
        label: String,
        lit: bool,
        tab: Tab,
    },
}

#[derive(Clone, Debug)]
struct Widget {
    /// The same widget from one frame to the next (its glow, its knob).
    id: u64,
    rect: Rect,
    kind: Kind,
}

impl Widget {
    fn act(&self) -> Option<&Act> {
        match &self.kind {
            Kind::Button { act, .. } | Kind::Toggle { act, .. } => Some(act),
            _ => None,
        }
    }
}

struct Ripple {
    at: Instant,
    x: f32,
    y: f32,
    clip: Rect,
}

#[derive(Default)]
struct State {
    open: bool,
    tab: Tab,
    view: View,
    actions: Vec<Action>,
    dragging: Option<Slider>,
    scrollbar_dragging: bool,
    scroll: f32,
    scroll_target: f32,
    content: f32,
    opened: Option<Instant>,
    /// When the page last changed (it slides in).
    page_at: Option<Instant>,
    drawn: Option<Instant>,
    /// Done was clicked: the game closes the screen.
    done: bool,
    /// The text boxes and the one typed into.
    fields: HashMap<Field, String>,
    focus: Option<Field>,
    create_kind: u8,
    /// The sidebar highlight's place (tab units), gliding.
    side_at: Option<f32>,
    glow: HashMap<u64, f32>,
    knobs: HashMap<u64, f32>,
    ripples: Vec<Ripple>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

/// The menu opened (the game opened the Options screen on Credits for it).
pub fn open() {
    with_state(|s| {
        s.open = true;
        s.scroll = 0.0;
        s.scroll_target = 0.0;
        s.dragging = None;
        s.scrollbar_dragging = false;
        s.opened = Some(Instant::now());
        s.page_at = None;
        s.drawn = None;
        s.done = false;
        s.focus = None;
        s.side_at = None;
        s.ripples.clear();
        // Back on the page it was left on, unless that was a sub-page.
        s.tab = s.tab.home();
        s.actions.push(Action::RefreshDevices);
    });
}

/// The Options screen closed: the menu with it.
pub fn closed() {
    with_state(|s| {
        s.open = false;
        s.dragging = None;
        s.scrollbar_dragging = false;
        s.focus = None;
    });
}

/// The mouse button was released.
pub fn release() {
    with_state(|s| {
        s.dragging = None;
        s.scrollbar_dragging = false;
    });
}

pub fn is_open() -> bool {
    with_state(|s| s.open)
}

/// Whether a text box is being typed into (keys are letters, not commands).
pub fn typing() -> bool {
    with_state(|s| s.open && s.focus.is_some())
}

/// Whether the menu stands in for the page `options` shows.
pub fn shows(options: &OptionsUi) -> bool {
    is_open() && options.page() == Page::Credits
}

pub fn set_view(view: View) {
    with_state(|s| s.view = view);
}

pub fn take_actions() -> Vec<Action> {
    with_state(|s| std::mem::take(&mut s.actions))
}

/// Whether "Done" was clicked (the game closes the screen), forgetting it.
pub fn take_done() -> bool {
    with_state(|s| std::mem::take(&mut s.done))
}

// ---------- colours ----------

fn hsv(h: f32, s: f32, v: f32) -> Rgba {
    let h = h.rem_euclid(1.0) * 6.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let byte = |f: f32| ((f + m) * 255.0).round() as u8;
    (byte(r), byte(g), byte(b), 255)
}

/// A theme's accent and panel colours at time `t` (rainbow cycles).
fn theme(name: &str, t: f32) -> (Rgba, Rgba) {
    match name {
        "grey" => ((178, 186, 198, 255), (28, 30, 35, 255)),
        "black" => ((235, 235, 240, 255), (5, 5, 7, 255)),
        "white" => ((250, 250, 255, 255), (36, 38, 46, 255)),
        "red" => ((255, 92, 92, 255), (26, 13, 15, 255)),
        "rainbow" => (hsv(t * 0.08, 0.6, 1.0), (17, 18, 26, 255)),
        _ => ((86, 204, 242, 255), (16, 19, 28, 255)),
    }
}

fn alpha(c: Rgba, a: f32) -> Rgba {
    (
        c.0,
        c.1,
        c.2,
        (f32::from(c.3) * a.clamp(0.0, 1.0)).round() as u8,
    )
}

fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2), m(a.3, b.3))
}

fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

fn ease_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c = 1.70158;
    1.0 + (c + 1.0) * (t - 1.0).powi(3) + c * (t - 1.0).powi(2)
}

/// One frame's step of `value` towards `target` (frame-rate independent).
fn approach(value: &mut f32, target: f32, rate: f32, dt: f32) {
    *value += (target - *value) * (1.0 - (-rate * dt).exp());
    if (target - *value).abs() < 0.005 {
        *value = target;
    }
}

// ---------- layout ----------

struct Frame {
    panel: Rect,
    side: Rect,
    content: Rect,
}

fn frame(w: f32, h: f32, size: u8) -> Frame {
    let (mw, mh) = SIZES_PX[(size as usize).min(SIZES_PX.len() - 1)];
    let pw = mw.min(w - 12.0).max(260.0);
    let ph = mh.min(h - 12.0).max(160.0);
    let panel = [((w - pw) / 2.0).floor(), ((h - ph) / 2.0).floor(), pw, ph];
    let side = [
        panel[0] + 4.0,
        panel[1] + TOP + 4.0,
        SIDE - 6.0,
        ph - TOP - 8.0,
    ];
    let content = [
        panel[0] + SIDE + 4.0,
        panel[1] + TOP + 6.0,
        pw - SIDE - 4.0 - PAD,
        ph - TOP - 12.0,
    ];
    Frame {
        panel,
        side,
        content,
    }
}

fn pct(v: f32) -> String {
    format!("{}%", (v * 100.0).round() as i32)
}

fn title_case(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// Lays widgets out down a column.
struct Column {
    x: f32,
    w: f32,
    y: f32,
    salt: u64,
    out: Vec<Widget>,
}

impl Column {
    fn id(&self) -> u64 {
        self.salt
            .wrapping_mul(1000)
            .wrapping_add(self.out.len() as u64)
    }
    fn put(&mut self, h: f32, kind: Kind) {
        let id = self.id();
        self.out.push(Widget {
            id,
            rect: [self.x, self.y, self.w, h],
            kind,
        });
        self.y += h + GAP;
    }
    /// Several widgets side by side, sharing the width.
    fn row(&mut self, h: f32, kinds: Vec<Kind>) {
        let n = kinds.len().max(1) as f32;
        let cw = ((self.w - GAP * (n - 1.0)) / n).floor();
        for (k, kind) in kinds.into_iter().enumerate() {
            let id = self.id();
            let x = self.x + k as f32 * (cw + GAP);
            let w = if k as f32 == n - 1.0 {
                self.x + self.w - x
            } else {
                cw
            };
            self.out.push(Widget {
                id,
                rect: [x, self.y, w, h],
                kind,
            });
        }
        self.y += h + GAP;
    }
    fn heading(&mut self, text: &str) {
        self.y += 2.0;
        self.put(
            10.0,
            Kind::Heading {
                text: text.to_owned(),
            },
        );
    }
    fn text(&mut self, text: impl Into<String>, color: Rgba) {
        self.put(
            9.0,
            Kind::Text {
                text: text.into(),
                color,
            },
        );
    }
}

fn button(label: impl Into<String>, act: Act) -> Kind {
    Kind::Button {
        label: label.into(),
        lit: None,
        act,
        primary: false,
    }
}

fn choice(label: impl Into<String>, lit: bool, act: Act) -> Kind {
    Kind::Button {
        label: label.into(),
        lit: Some(lit),
        act,
        primary: false,
    }
}

fn set(key: &'static str, value: impl ToString) -> Act {
    Act::Do(Action::Set(key, value.to_string()))
}

fn field_text(s: &State, f: Field) -> String {
    s.fields.get(&f).cloned().unwrap_or_default()
}

fn input(s: &State, field: Field, placeholder: &str, secret: bool) -> Kind {
    Kind::Input {
        field,
        text: field_text(s, field),
        placeholder: placeholder.to_owned(),
        secret,
        focused: s.focus == Some(field),
    }
}

/// The sidebar: the pages, then the two switches at its foot.
fn sidebar(s: &State, f: &Frame) -> Vec<Widget> {
    let v = &s.view;
    let r = f.side;
    let mut out = Vec::new();
    let home = s.tab.home();
    for (k, (tab, label)) in SIDEBAR.iter().enumerate() {
        out.push(Widget {
            id: 900 + k as u64,
            rect: [r[0], r[1] + k as f32 * (ROW + 2.0), r[2], ROW],
            kind: Kind::Tab {
                label: (*label).to_owned(),
                lit: home == *tab,
                tab: *tab,
            },
        });
    }
    let foot = r[1] + r[3] - 2.0 * (ROW + 2.0) - 10.0;
    out.push(Widget {
        id: 950,
        rect: [r[0], foot, r[2], ROW],
        kind: Kind::Toggle {
            label: "Microphone".into(),
            on: !v.muted,
            act: Act::Do(Action::Mute),
        },
    });
    out.push(Widget {
        id: 951,
        rect: [r[0], foot + ROW + 2.0, r[2], ROW],
        kind: Kind::Toggle {
            label: "Voice chat".into(),
            on: !v.disabled,
            act: Act::Do(Action::Disable),
        },
    });
    out
}

/// The page's widgets at scroll `scroll`, and how tall the page is.
fn widgets(s: &State, f: &Frame, scroll: f32) -> (Vec<Widget>, f32) {
    let v = &s.view;
    let c = f.content;
    let top = c[1] - scroll;
    let salt = match s.tab {
        Tab::Players => 1,
        Tab::Groups => 2,
        Tab::Audio => 3,
        Tab::Talking => 4,
        Tab::Look => 5,
        Tab::Microphones => 6,
        Tab::Speakers => 7,
        Tab::Create => 8,
        Tab::Password(_) => 9,
    };
    let mut col = Column {
        x: c[0],
        w: c[2] - 4.0,
        y: top,
        salt,
        out: Vec::new(),
    };
    let full = col.w;
    match s.tab {
        Tab::Players => {
            col.put(15.0, input(s, Field::Search, "Search players...", false));
            if let Some(p) = &v.problem {
                col.text(p.clone(), BAD);
            }
            let search = field_text(s, Field::Search).trim().to_lowercase();
            let shown: Vec<&PlayerRow> = v
                .players
                .iter()
                .filter(|p| search.is_empty() || p.name.to_lowercase().contains(&search))
                .collect();
            let talking = v.players.iter().filter(|p| p.talking).count();
            col.heading(&if talking > 0 {
                format!("PLAYERS ({})  -  {talking} TALKING", v.players.len())
            } else {
                format!("PLAYERS ({})", v.players.len())
            });
            if shown.is_empty() {
                let text = if !v.connected {
                    "Join a server with voice chat to talk."
                } else if v.players.is_empty() {
                    "Nobody else here has voice chat right now."
                } else {
                    "Nobody matches your search."
                };
                col.text(text, FAINT);
            }
            for p in shown {
                let y = col.y;
                col.put(ROW + 2.0, Kind::Player { row: p.clone() });
                // A volume slider and a mute button over the row's right side.
                let mw = 34.0;
                let sw = 84.0f32.min(full * 0.36);
                let muted = p.volume <= 0.001;
                let id = col.id();
                col.out.push(Widget {
                    id,
                    rect: [c[0] + full - mw, y + 3.0, mw, ROW - 4.0],
                    kind: Kind::Button {
                        label: if muted {
                            "Unmute".into()
                        } else {
                            "Mute".into()
                        },
                        lit: Some(muted),
                        act: Act::Do(Action::PlayerVolume(p.uuid, if muted { 1.0 } else { 0.0 })),
                        primary: false,
                    },
                });
                let id = col.id();
                col.out.push(Widget {
                    id,
                    rect: [c[0] + full - mw - GAP - sw, y + 5.0, sw, ROW - 8.0],
                    kind: Kind::Slider {
                        id: Slider::Player(p.uuid),
                        label: String::new(),
                        value: p.volume,
                        min: 0.0,
                        max: 2.0,
                        text: pct(p.volume),
                        meter: None,
                    },
                });
            }
        }
        Tab::Groups => {
            if !v.connected {
                col.text("Join a server with voice chat to use groups.", FAINT);
            } else if !v.groups_enabled {
                col.text("This server has groups turned off.", FAINT);
            } else {
                if let Some(p) = &v.group_problem {
                    col.text(p.clone(), BAD);
                }
                let own = v
                    .own_group
                    .and_then(|id| v.groups.iter().find(|g| g.id == id));
                if let Some(g) = own {
                    col.heading("YOUR GROUP");
                    col.put(
                        ROW + 6.0,
                        Kind::Group {
                            row: g.clone(),
                            joined: true,
                        },
                    );
                    let names: Vec<String> = g.members.iter().map(|(n, _)| n.clone()).collect();
                    let line = if names.is_empty() {
                        "Just you".to_owned()
                    } else {
                        names.join(", ")
                    };
                    col.text(format!("Members: {line}"), DIM);
                    col.text(GROUP_KINDS[(g.kind as usize).min(2)].1, FAINT);
                    col.put(
                        ROW,
                        Kind::Button {
                            label: "Leave group".into(),
                            lit: None,
                            act: Act::Do(Action::LeaveGroup),
                            primary: false,
                        },
                    );
                } else if v.own_group.is_some() {
                    col.heading("YOUR GROUP");
                    col.text(v.group.clone().unwrap_or_default(), WHITE);
                    col.put(ROW, button("Leave group", Act::Do(Action::LeaveGroup)));
                } else {
                    col.put(
                        ROW,
                        Kind::Button {
                            label: "+  Create a group".into(),
                            lit: None,
                            act: Act::Go(Tab::Create),
                            primary: true,
                        },
                    );
                }
                let others: Vec<&GroupRow> = v
                    .groups
                    .iter()
                    .filter(|g| Some(g.id) != v.own_group)
                    .collect();
                col.heading(&format!("GROUPS ON THIS SERVER ({})", others.len()));
                if others.is_empty() {
                    col.text(
                        "No other groups yet. Make one and invite your friends!",
                        FAINT,
                    );
                }
                for g in others {
                    col.put(
                        ROW + 6.0,
                        Kind::Group {
                            row: g.clone(),
                            joined: false,
                        },
                    );
                }
            }
        }
        Tab::Create => {
            col.heading("NEW GROUP");
            col.text("Name", DIM);
            col.put(15.0, input(s, Field::Name, "Type a name...", false));
            col.text("Password (leave empty for none)", DIM);
            col.put(15.0, input(s, Field::Password, "No password", true));
            col.text("Type", DIM);
            col.row(
                ROW - 2.0,
                GROUP_KINDS
                    .iter()
                    .enumerate()
                    .map(|(k, (name, _))| {
                        choice(*name, s.create_kind as usize == k, Act::Kind(k as u8))
                    })
                    .collect(),
            );
            col.text(GROUP_KINDS[(s.create_kind as usize).min(2)].1, FAINT);
            col.y += 4.0;
            let ready = !field_text(s, Field::Name).trim().is_empty();
            col.row(
                ROW,
                vec![
                    button("Cancel", Act::Go(Tab::Groups)),
                    Kind::Button {
                        label: "Create".into(),
                        lit: Some(ready),
                        act: Act::Submit,
                        primary: true,
                    },
                ],
            );
        }
        Tab::Password(id) => {
            let name = v
                .groups
                .iter()
                .find(|g| g.id == id)
                .map_or("this group".to_owned(), |g| g.name.clone());
            col.heading("PASSWORD NEEDED");
            col.text(format!("\"{name}\" has a password."), DIM);
            col.put(
                15.0,
                input(s, Field::JoinPassword, "Type the password...", true),
            );
            col.y += 4.0;
            col.row(
                ROW,
                vec![
                    button("Cancel", Act::Go(Tab::Groups)),
                    Kind::Button {
                        label: "Join".into(),
                        lit: Some(true),
                        act: Act::Submit,
                        primary: true,
                    },
                ],
            );
        }
        Tab::Audio => {
            col.heading("DEVICES");
            col.put(
                ROW,
                button(
                    format!(
                        "Microphone: {}",
                        v.microphone.as_deref().unwrap_or("Default")
                    ),
                    Act::Go(Tab::Microphones),
                ),
            );
            col.put(
                ROW,
                button(
                    format!("Speakers: {}", v.speakers.as_deref().unwrap_or("Default")),
                    Act::Go(Tab::Speakers),
                ),
            );
            if let Some(p) = &v.problem {
                col.text(p.clone(), BAD);
            }
            col.heading("VOLUME");
            col.put(
                ROW,
                Kind::Slider {
                    id: Slider::Volume,
                    label: "Everyone".into(),
                    value: v.volume,
                    min: 0.0,
                    max: 2.0,
                    text: pct(v.volume),
                    meter: None,
                },
            );
            col.put(
                ROW,
                Kind::Slider {
                    id: Slider::Gain,
                    label: "Mic boost".into(),
                    value: v.mic_gain,
                    min: 0.0,
                    max: 4.0,
                    text: pct(v.mic_gain),
                    meter: None,
                },
            );
        }
        Tab::Talking => {
            col.heading("HOW YOU TALK");
            col.row(
                ROW,
                vec![
                    choice(
                        format!("Hold {}", v.talk_key),
                        !v.voice_activation,
                        Act::Do(Action::VoiceActivation(false)),
                    ),
                    choice(
                        "Voice activation",
                        v.voice_activation,
                        Act::Do(Action::VoiceActivation(true)),
                    ),
                ],
            );
            let meter = ((v.level_db + 60.0) / 60.0).clamp(0.0, 1.0);
            col.put(
                ROW,
                Kind::Slider {
                    id: Slider::Threshold,
                    label: "Activation level".into(),
                    value: v.threshold_db,
                    min: -60.0,
                    max: 0.0,
                    text: format!("{} dB", v.threshold_db.round() as i32),
                    meter: Some(meter),
                },
            );
            col.text(
                if v.voice_activation {
                    "You talk when the green bar passes the marker."
                } else {
                    "The bar shows your microphone's level."
                },
                FAINT,
            );
            col.put(
                ROW,
                Kind::Toggle {
                    label: "RNNoise & echo filter".into(),
                    on: v.rnnoise,
                    act: Act::Do(Action::Rnnoise(!v.rnnoise)),
                },
            );
            col.put(
                ROW,
                Kind::Toggle {
                    label: "Test microphone (hear yourself)".into(),
                    on: v.testing,
                    act: Act::Do(Action::Test(!v.testing)),
                },
            );
            col.heading("KEYS");
            col.text(
                format!(
                    "Talk: {}    Mute: {}    Voice off: {}    Menu: {}",
                    v.talk_key, v.mute_key, v.off_key, v.menu_key
                ),
                DIM,
            );
            col.text(
                "Change them in Mod Menu > Complex Voice Chat > Change keys.",
                FAINT,
            );
        }
        Tab::Look => {
            let p = &v.prefs;
            col.heading("MENU THEME");
            col.row(
                26.0,
                THEMES
                    .iter()
                    .map(|t| Kind::Theme {
                        name: (*t).to_owned(),
                        lit: p.theme == *t,
                    })
                    .collect(),
            );
            col.put(
                ROW,
                Kind::Slider {
                    id: Slider::Opacity,
                    label: "Opacity".into(),
                    value: p.opacity,
                    min: 0.5,
                    max: 1.0,
                    text: pct(p.opacity),
                    meter: None,
                },
            );
            col.row(
                ROW - 2.0,
                SIZES
                    .iter()
                    .enumerate()
                    .map(|(k, n)| choice(*n, p.size as usize == k, set("size", k)))
                    .collect(),
            );
            col.put(
                ROW,
                Kind::Toggle {
                    label: "Animations".into(),
                    on: p.animations,
                    act: set("animations", !p.animations),
                },
            );
            col.heading("MIC ICON COLOUR");
            let n = v.swatches.len().max(1);
            col.row(
                30.0,
                v.swatches
                    .iter()
                    .take(n)
                    .map(|(name, idle, talking)| Kind::Swatch {
                        name: name.clone(),
                        idle: idle.clone(),
                        talking: talking.clone(),
                        lit: *name == v.colour,
                    })
                    .collect(),
            );
            col.heading("ON-SCREEN STATUS");
            col.put(
                ROW,
                Kind::Toggle {
                    label: "Show it".into(),
                    on: p.show_hud,
                    act: set("show_hud", !p.show_hud),
                },
            );
            col.row(
                ROW - 2.0,
                HUD_STYLES
                    .iter()
                    .enumerate()
                    .map(|(k, n)| choice(*n, p.hud_style as usize == k, set("hud_style", k)))
                    .collect(),
            );
            col.row(
                ROW - 2.0,
                HUD_CORNERS
                    .iter()
                    .enumerate()
                    .map(|(k, n)| choice(*n, p.hud_corner as usize == k, set("hud_corner", k)))
                    .collect(),
            );
            col.put(
                ROW,
                Kind::Slider {
                    id: Slider::HudScale,
                    label: "Size".into(),
                    value: p.hud_scale,
                    min: 1.0,
                    max: 3.0,
                    text: format!("{:.2}x", p.hud_scale),
                    meter: None,
                },
            );
            col.heading("CHAT");
            col.put(
                ROW,
                Kind::Toggle {
                    label: "Messages in chat (connected, mute...)".into(),
                    on: p.announce,
                    act: set("announce", !p.announce),
                },
            );
        }
        Tab::Microphones | Tab::Speakers => {
            let input = s.tab == Tab::Microphones;
            let (chosen, list) = if input {
                (&v.microphone, &v.microphones)
            } else {
                (&v.speakers, &v.speaker_list)
            };
            col.heading(if input {
                "CHOOSE A MICROPHONE"
            } else {
                "CHOOSE SPEAKERS"
            });
            col.put(
                ROW,
                choice(
                    "Default (Windows' choice)",
                    chosen.is_none(),
                    Act::Pick(input, None),
                ),
            );
            for n in list {
                col.put(
                    ROW,
                    choice(
                        n.clone(),
                        chosen.as_ref() == Some(n),
                        Act::Pick(input, Some(n.clone())),
                    ),
                );
            }
            if list.is_empty() {
                col.text("No devices found.", FAINT);
            }
            col.y += 2.0;
            col.put(ROW, button("Back", Act::Go(Tab::Audio)));
        }
    }
    let height = col.y - top;
    (col.out, height)
}

// ---------- drawing ----------

fn color(c: Rgba) -> u32 {
    rgba(c.0, c.1, c.2, c.3)
}

fn fill(batch: &mut HudBatch, atlas: &HudAtlas, r: Rect, c: Rgba) {
    if r[2] > 0.0 && r[3] > 0.0 && c.3 > 0 {
        batch.fill(atlas, r[0], r[1], r[2], r[3], color(c));
    }
}

fn rounded(batch: &mut HudBatch, atlas: &HudAtlas, r: Rect, c: Rgba) {
    let [x, y, w, h] = r;
    if w < 4.0 || h < 4.0 {
        fill(batch, atlas, r, c);
        return;
    }
    fill(batch, atlas, [x + 2.0, y, w - 4.0, 1.0], c);
    fill(batch, atlas, [x + 1.0, y + 1.0, w - 2.0, 1.0], c);
    fill(batch, atlas, [x, y + 2.0, w, h - 4.0], c);
    fill(batch, atlas, [x + 1.0, y + h - 2.0, w - 2.0, 1.0], c);
    fill(batch, atlas, [x + 2.0, y + h - 1.0, w - 4.0, 1.0], c);
}

fn framed(batch: &mut HudBatch, atlas: &HudAtlas, r: Rect, inner: Rgba, edge: Rgba) {
    rounded(batch, atlas, r, edge);
    rounded(
        batch,
        atlas,
        [r[0] + 1.0, r[1] + 1.0, r[2] - 2.0, r[3] - 2.0],
        inner,
    );
}

fn disc(batch: &mut HudBatch, atlas: &HudAtlas, cx: f32, cy: f32, radius: f32, c: Rgba) {
    let rows = radius.ceil() as i32;
    for k in -rows..rows {
        let dy = k as f32 + 0.5;
        let half = (radius * radius - dy * dy).max(0.0).sqrt();
        if half > 0.0 {
            fill(batch, atlas, [cx - half, cy + k as f32, half * 2.0, 1.0], c);
        }
    }
}

fn label(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    text: &str,
    x: f32,
    y: f32,
    c: Rgba,
    shadow: bool,
) {
    batch.text_styled(
        atlas,
        &Text::literal(text),
        x.floor(),
        y.floor(),
        color(c),
        shadow,
    );
}

fn small(batch: &mut HudBatch, atlas: &HudAtlas, text: &str, x: f32, y: f32, c: Rgba) {
    let start = batch.vertices.len();
    label(batch, atlas, text, x, y, c, false);
    batch.scale_since(start, x.floor(), y.floor(), 0.75, 0.75);
}

/// `text` cut to `width` pixels with "..." when it's longer.
fn fit(atlas: &HudAtlas, text: &str, width: f32) -> String {
    if atlas.font.width(text) <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let next = format!("{out}{ch}...");
        if atlas.font.width(&next) > width {
            break;
        }
        out.push(ch);
    }
    format!("{out}...")
}

fn centred(batch: &mut HudBatch, atlas: &HudAtlas, text: &str, r: Rect, c: Rgba) {
    let text = fit(atlas, text, r[2] - 6.0);
    let x = r[0] + (r[2] - atlas.font.width(&text)) / 2.0;
    label(batch, atlas, &text, x, r[1] + (r[3] - 8.0) / 2.0, c, true);
}

/// An icon's runs at `(x, y)`, `scale` GUI pixels per icon pixel.
fn icon(batch: &mut HudBatch, atlas: &HudAtlas, runs: &Runs, x: f32, y: f32, scale: f32) {
    for &(rx, ry, len, argb) in runs {
        let [a, r, g, b] = argb.to_be_bytes();
        fill(
            batch,
            atlas,
            [
                x + rx as f32 * scale,
                y + ry as f32 * scale,
                len as f32 * scale,
                scale,
            ],
            (r, g, b, a),
        );
    }
}

/// A padlock, 7 by 8.
fn lock(batch: &mut HudBatch, atlas: &HudAtlas, x: f32, y: f32, c: Rgba) {
    fill(batch, atlas, [x + 2.0, y, 3.0, 1.0], c);
    fill(batch, atlas, [x + 1.0, y + 1.0, 1.0, 2.0], c);
    fill(batch, atlas, [x + 5.0, y + 1.0, 1.0, 2.0], c);
    fill(batch, atlas, [x, y + 3.0, 7.0, 5.0], c);
}

fn inside(r: Rect, (x, y): (f32, f32)) -> bool {
    x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3]
}

/// Every vertex since `start` with its alpha times `a`.
fn fade_since(batch: &mut HudBatch, start: usize, a: f32) {
    if a >= 1.0 {
        return;
    }
    let a = a.max(0.0);
    for v in &mut batch.vertices[start..] {
        let old = (v.color >> 24) as f32;
        v.color = (v.color & 0x00FF_FFFF) | (((old * a).round() as u32).min(255) << 24);
    }
}

fn shift_since(batch: &mut HudBatch, start: usize, dx: f32, dy: f32) {
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    for v in &mut batch.vertices[start..] {
        v.pos[0] += dx;
        v.pos[1] += dy;
    }
}

/// What a frame's drawing needs besides the widget.
struct Look {
    accent: Rgba,
    clock: f32,
    dt: f32,
    animations: bool,
}

fn draw_widget(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    s: &mut State,
    w: &Widget,
    cursor: (f32, f32),
    look: &Look,
) {
    let hover = inside(w.rect, cursor);
    let glow = {
        let g = s.glow.entry(w.id).or_insert(0.0);
        if look.animations {
            approach(g, if hover { 1.0 } else { 0.0 }, 16.0, look.dt);
        } else {
            *g = if hover { 1.0 } else { 0.0 };
        }
        *g
    };
    let accent = look.accent;
    let r = w.rect;
    match &w.kind {
        Kind::Button {
            label: text,
            lit,
            primary,
            ..
        } => {
            let on = *lit == Some(true);
            let base = if *primary && on {
                alpha(accent, 0.35)
            } else if *primary {
                alpha(accent, 0.18)
            } else if on {
                alpha(accent, 0.28)
            } else {
                BUTTON
            };
            let lift = -glow;
            let r = [r[0], r[1] + lift, r[2], r[3]];
            framed(
                batch,
                atlas,
                r,
                mix(base, mix(base, WHITE, 0.25), glow),
                mix(
                    EDGE,
                    alpha(accent, 0.8),
                    glow.max(if on { 0.5 } else { 0.0 }),
                ),
            );
            if on {
                fill(
                    batch,
                    atlas,
                    [r[0] + 4.0, r[1] + r[3] - 3.0, r[2] - 8.0, 1.0],
                    accent,
                );
            }
            centred(
                batch,
                atlas,
                text,
                r,
                if on || *primary {
                    WHITE
                } else {
                    mix(DIM, WHITE, 0.5 + glow * 0.5)
                },
            );
        }
        Kind::Toggle {
            label: text, on, ..
        } => {
            let k = {
                let knob = s.knobs.entry(w.id).or_insert(if *on { 1.0 } else { 0.0 });
                if look.animations {
                    approach(knob, if *on { 1.0 } else { 0.0 }, 18.0, look.dt);
                } else {
                    *knob = if *on { 1.0 } else { 0.0 };
                }
                *knob
            };
            framed(
                batch,
                atlas,
                r,
                mix(BUTTON, BUTTON_HOVER, glow),
                mix(EDGE, alpha(accent, 0.6), glow),
            );
            let room = r[2] - 40.0;
            label(
                batch,
                atlas,
                &fit(atlas, text, room),
                r[0] + 6.0,
                r[1] + (r[3] - 8.0) / 2.0,
                mix(DIM, WHITE, k),
                true,
            );
            let track = [r[0] + r[2] - 28.0, r[1] + (r[3] - 10.0) / 2.0, 22.0, 10.0];
            if k > 0.0 {
                rounded(
                    batch,
                    atlas,
                    [
                        track[0] - 2.0,
                        track[1] - 2.0,
                        track[2] + 4.0,
                        track[3] + 4.0,
                    ],
                    alpha(accent, 0.2 * k),
                );
            }
            rounded(batch, atlas, track, mix(TRACK_OFF, accent, k));
            // The knob squashes a little mid-slide.
            let squash = 1.0 + (k * std::f32::consts::PI).sin() * 0.35;
            let kw = 8.0 * squash;
            let kx = track[0] + 1.0 + k * (track[2] - 2.0 - kw);
            rounded(batch, atlas, [kx, track[1] + 2.0, kw, 8.0], SHADOW);
            rounded(batch, atlas, [kx, track[1] + 1.0, kw, 8.0], WHITE);
        }
        Kind::Slider {
            id,
            label: text,
            value,
            min,
            max,
            text: shown,
            meter,
        } => {
            let (track, caption) = if text.is_empty() {
                (r, None)
            } else {
                let lw = (r[2] * 0.36).floor().min(96.0);
                ([r[0] + lw, r[1] + 4.0, r[2] - lw, r[3] - 8.0], Some(lw))
            };
            if let Some(lw) = caption {
                label(
                    batch,
                    atlas,
                    &fit(atlas, text, lw - 4.0),
                    r[0],
                    r[1] + (r[3] - 8.0) / 2.0,
                    DIM,
                    true,
                );
            }
            let active = hover || s.dragging == Some(*id);
            rounded(batch, atlas, track, TRACK);
            let t = ((value - min) / (max - min)).clamp(0.0, 1.0);
            if let Some(m) = meter {
                // The microphone's level, live; green past the activation level.
                let c = if *m >= t { GOOD } else { alpha(GOOD, 0.45) };
                fill(
                    batch,
                    atlas,
                    [
                        track[0] + 1.0,
                        track[1] + 1.0,
                        (track[2] - 2.0) * m,
                        track[3] - 2.0,
                    ],
                    c,
                );
            } else {
                let inner = [
                    track[0] + 1.0,
                    track[1] + 1.0,
                    (track[2] - 2.0) * t,
                    track[3] - 2.0,
                ];
                fill(batch, atlas, inner, alpha(accent, 0.45 + 0.2 * glow));
            }
            let kx = track[0] + (track[2] - 4.0) * t;
            if active {
                rounded(
                    batch,
                    atlas,
                    [kx - 2.0, track[1] - 3.0, 8.0, track[3] + 6.0],
                    alpha(accent, 0.3),
                );
            }
            fill(
                batch,
                atlas,
                [kx, track[1] - 1.0, 4.0, track[3] + 2.0],
                if active { WHITE } else { accent },
            );
            centred(batch, atlas, shown, track, WHITE);
        }
        Kind::Text { text, color: c } => {
            label(batch, atlas, &fit(atlas, text, r[2]), r[0], r[1], *c, true);
        }
        Kind::Heading { text } => {
            fill(batch, atlas, [r[0], r[1] + 1.0, 2.0, 7.0], accent);
            small(batch, atlas, text, r[0] + 5.0, r[1] + 2.0, FAINT);
            let tw = atlas.font.width(text) * 0.75 + 10.0;
            fill(
                batch,
                atlas,
                [r[0] + tw, r[1] + 5.0, (r[2] - tw).max(0.0), 1.0],
                alpha(WHITE, 0.06),
            );
        }
        Kind::Player { row } => {
            if hover {
                rounded(batch, atlas, r, alpha(WHITE, 0.04 + 0.04 * glow));
            }
            if row.talking {
                // A green bar on the left, breathing while they talk.
                let pulse = 0.6 + 0.4 * (look.clock * 7.0).sin().abs();
                rounded(batch, atlas, r, alpha(GOOD, 0.08 * pulse));
                fill(
                    batch,
                    atlas,
                    [r[0], r[1] + 3.0, 2.0, r[3] - 6.0],
                    alpha(GOOD, pulse),
                );
            }
            let dot = if row.talking {
                GOOD
            } else if row.status.is_empty() {
                FAINT
            } else {
                BAD
            };
            if row.talking && !s.view.speaker_icon.is_empty() {
                icon(
                    batch,
                    atlas,
                    &s.view.speaker_icon.clone(),
                    r[0] + 4.0,
                    r[1] + r[3] / 2.0 - 5.0,
                    0.625,
                );
            } else {
                disc(batch, atlas, r[0] + 9.0, r[1] + r[3] / 2.0, 2.0, dot);
            }
            let room = r[2] * 0.45;
            let name_c = if row.talking { GOOD } else { WHITE };
            let has_sub = !row.status.is_empty() || row.group.is_some();
            let ny = if has_sub {
                r[1] + 3.0
            } else {
                r[1] + (r[3] - 8.0) / 2.0
            };
            label(
                batch,
                atlas,
                &fit(atlas, &row.name, room),
                r[0] + 17.0,
                ny,
                name_c,
                true,
            );
            let sub = match (&row.group, row.status.is_empty()) {
                (Some(g), true) => format!("in {g}"),
                (Some(g), false) => format!("{} - in {g}", row.status),
                (None, _) => row.status.clone(),
            };
            if !sub.is_empty() {
                small(
                    batch,
                    atlas,
                    &fit(atlas, &sub, room / 0.75),
                    r[0] + 17.0,
                    r[1] + 13.0,
                    FAINT,
                );
            }
        }
        Kind::Group { row, joined } => {
            let base = if *joined {
                alpha(accent, 0.18)
            } else {
                mix(BUTTON, BUTTON_HOVER, glow)
            };
            framed(
                batch,
                atlas,
                r,
                base,
                mix(EDGE, alpha(accent, 0.8), if *joined { 0.7 } else { glow }),
            );
            let gi = s.view.group_icon.clone();
            if !gi.is_empty() {
                icon(
                    batch,
                    atlas,
                    &gi,
                    r[0] + 4.0,
                    r[1] + (r[3] - 16.0) / 2.0,
                    1.0,
                );
            }
            let x = r[0] + 24.0;
            let name = fit(atlas, &row.name, r[2] - 110.0);
            label(batch, atlas, &name, x, r[1] + 4.0, WHITE, true);
            if row.locked {
                lock(
                    batch,
                    atlas,
                    x + atlas.font.width(&name) + 4.0,
                    r[1] + 4.0,
                    WARN,
                );
            }
            let kind = GROUP_KINDS[(row.kind as usize).min(2)].0;
            let n = row.members.len();
            let talking = row.members.iter().filter(|m| m.1).count();
            let mut sub = format!("{kind} - {n} {}", if n == 1 { "member" } else { "members" });
            if talking > 0 {
                sub.push_str(&format!(", {talking} talking"));
            }
            small(
                batch,
                atlas,
                &sub,
                x,
                r[1] + 15.0,
                if talking > 0 { GOOD } else { FAINT },
            );
            let tag = if *joined {
                "JOINED"
            } else if row.locked {
                "JOIN (password)"
            } else {
                "JOIN"
            };
            let tw = atlas.font.width(tag) * 0.75 + 10.0;
            let tr = [r[0] + r[2] - tw - 6.0, r[1] + (r[3] - 11.0) / 2.0, tw, 11.0];
            framed(
                batch,
                atlas,
                tr,
                alpha(if *joined { GOOD } else { accent }, 0.15 + 0.2 * glow),
                alpha(if *joined { GOOD } else { accent }, 0.6),
            );
            small(batch, atlas, tag, tr[0] + 5.0, tr[1] + 3.0, WHITE);
        }
        Kind::Input {
            text,
            placeholder,
            secret,
            focused,
            ..
        } => {
            let f = if *focused { 1.0 } else { glow * 0.5 };
            if *focused {
                rounded(
                    batch,
                    atlas,
                    [r[0] - 1.0, r[1] - 1.0, r[2] + 2.0, r[3] + 2.0],
                    alpha(accent, 0.25),
                );
            }
            framed(batch, atlas, r, FIELD, mix(EDGE, accent, f));
            let shown = if *secret {
                "*".repeat(text.chars().count())
            } else {
                text.clone()
            };
            let room = r[2] - 10.0;
            if shown.is_empty() && !*focused {
                label(
                    batch,
                    atlas,
                    &fit(atlas, placeholder, room),
                    r[0] + 4.0,
                    r[1] + (r[3] - 8.0) / 2.0,
                    FAINT,
                    false,
                );
            } else {
                let mut cut = shown.clone();
                while atlas.font.width(&cut) > room && !cut.is_empty() {
                    cut.remove(0);
                }
                label(
                    batch,
                    atlas,
                    &cut,
                    r[0] + 4.0,
                    r[1] + (r[3] - 8.0) / 2.0,
                    WHITE,
                    false,
                );
                if *focused {
                    let caret = 0.5 + 0.5 * (look.clock * 6.0).cos();
                    let cx = r[0] + 5.0 + atlas.font.width(&cut);
                    fill(
                        batch,
                        atlas,
                        [cx, r[1] + 3.0, 1.0, r[3] - 6.0],
                        alpha(accent, caret),
                    );
                }
            }
        }
        Kind::Swatch {
            name,
            idle,
            talking,
            lit,
        } => {
            framed(
                batch,
                atlas,
                r,
                if *lit {
                    alpha(accent, 0.25)
                } else {
                    mix(BUTTON, BUTTON_HOVER, glow)
                },
                if *lit {
                    accent
                } else {
                    mix(EDGE, alpha(accent, 0.6), glow)
                },
            );
            let both = 34.0;
            let x = r[0] + (r[2] - both).max(0.0) / 2.0;
            let bob = if hover && look.animations {
                (look.clock * 8.0).sin() * 1.0
            } else {
                0.0
            };
            if r[2] >= both + 4.0 {
                icon(batch, atlas, idle, x, r[1] + 2.0 + bob, 1.0);
                icon(batch, atlas, talking, x + 18.0, r[1] + 2.0 - bob, 1.0);
            } else {
                icon(
                    batch,
                    atlas,
                    idle,
                    r[0] + (r[2] - 16.0) / 2.0,
                    r[1] + 2.0 + bob,
                    1.0,
                );
            }
            let t = fit(atlas, &title_case(name), r[2] / 0.75 - 4.0);
            let tw = atlas.font.width(&t) * 0.75;
            small(
                batch,
                atlas,
                &t,
                r[0] + (r[2] - tw) / 2.0,
                r[1] + 21.0,
                if *lit { WHITE } else { DIM },
            );
        }
        Kind::Theme { name, lit } => {
            let (a, panel) = theme(name, look.clock);
            framed(
                batch,
                atlas,
                r,
                panel,
                if *lit { WHITE } else { mix(EDGE, a, glow) },
            );
            let dot = 4.0 + glow * 1.0 + if *lit { 1.0 } else { 0.0 };
            disc(batch, atlas, r[0] + r[2] / 2.0, r[1] + 9.0, dot, a);
            let t = fit(atlas, &title_case(name), r[2] / 0.75 - 4.0);
            let tw = atlas.font.width(&t) * 0.75;
            small(
                batch,
                atlas,
                &t,
                r[0] + (r[2] - tw) / 2.0,
                r[1] + 17.0,
                if *lit { WHITE } else { DIM },
            );
        }
        Kind::Tab {
            label: text, lit, ..
        } => {
            if !*lit && glow > 0.0 {
                rounded(batch, atlas, r, alpha(WHITE, 0.06 * glow));
            }
            let x = r[0] + 8.0 + glow * 2.0;
            label(
                batch,
                atlas,
                &fit(atlas, text, r[2] - 12.0),
                x,
                r[1] + (r[3] - 8.0) / 2.0,
                if *lit { WHITE } else { mix(DIM, WHITE, glow) },
                true,
            );
        }
    }
}

pub fn draw(batch: &mut HudBatch, atlas: &HudAtlas, cursor: (f32, f32)) {
    let (w, h) = (batch.width, batch.height);
    let now = Instant::now();
    with_state(|s| {
        let prefs = s.view.prefs.clone();
        let anim = prefs.animations;
        let dt = s
            .drawn
            .map_or(0.0, |d| now.duration_since(d).as_secs_f32().min(0.1));
        s.drawn = Some(now);
        let clock = s
            .opened
            .map_or(0.0, |o| now.duration_since(o).as_secs_f32());
        let (accent, panel_c) = theme(&prefs.theme, clock);
        let look = Look {
            accent,
            clock,
            dt,
            animations: anim,
        };
        let f = frame(w, h, prefs.size);
        let st = s.scroll_target;
        if anim {
            approach(&mut s.scroll, st, 18.0, dt);
        } else {
            s.scroll = st;
        }
        let (_, content) = widgets(s, &f, 0.0);
        s.content = content;
        let most = (content - f.content[3]).max(0.0);
        s.scroll_target = s.scroll_target.min(most);
        s.scroll = s.scroll.min(most);
        let (ws, _) = widgets(s, &f, s.scroll);
        let side = sidebar(s, &f);

        let open = if anim { ease_out(clock / 0.22) } else { 1.0 };
        fill(
            batch,
            atlas,
            [0.0, 0.0, w, h],
            (4, 6, 12, (140.0 * open) as u8),
        );
        let p = f.panel;
        let start_panel = batch.vertices.len();
        for k in 1..=3 {
            let g = k as f32 * 2.0;
            rounded(
                batch,
                atlas,
                [p[0] - g, p[1] - g + 3.0, p[2] + g * 2.0, p[3] + g * 2.0],
                (0, 0, 0, 24),
            );
        }
        let pa = prefs.opacity.clamp(0.5, 1.0);
        framed(
            batch,
            atlas,
            p,
            alpha(panel_c, pa),
            alpha(mix(EDGE, accent, 0.25), 1.0),
        );
        // The accent line along the top, a shine sweeping across it.
        let bar = [p[0] + 2.0, p[1] + 1.0, p[2] - 4.0, 2.0];
        fill(batch, atlas, bar, accent);
        if anim {
            let sweep = (clock * 0.4).fract();
            let sx = bar[0] - 30.0 + (bar[2] + 60.0) * sweep;
            for k in 0..6 {
                let a = 1.0 - (k as f32 - 2.5).abs() / 3.0;
                let x0 = (sx + k as f32 * 5.0).max(bar[0]);
                let x1 = (sx + k as f32 * 5.0 + 5.0).min(bar[0] + bar[2]);
                fill(
                    batch,
                    atlas,
                    [x0, bar[1], x1 - x0, 2.0],
                    alpha(WHITE, a * 0.55),
                );
            }
        }
        fill(
            batch,
            atlas,
            [p[0] + 1.0, p[1] + TOP, p[2] - 2.0, 1.0],
            EDGE,
        );
        // The sidebar's panel.
        fill(
            batch,
            atlas,
            [p[0] + 1.0, p[1] + TOP + 1.0, SIDE - 2.0, p[3] - TOP - 2.0],
            (0, 0, 0, (70.0 * pa) as u8),
        );

        // The header: the mic icon (breathing while talking), the title, the connection.
        let (ix, iy) = (p[0] + 7.0, p[1] + 7.0);
        if s.view.transmitting && anim {
            let b = 0.5 + 0.5 * (clock * 8.0).sin();
            disc(
                batch,
                atlas,
                ix + 8.0,
                iy + 8.0,
                9.0 + b * 2.0,
                alpha(GOOD, 0.15 + 0.1 * b),
            );
        }
        if !s.view.icon.is_empty() {
            icon(batch, atlas, &s.view.icon.clone(), ix, iy, 1.0);
        }
        label(
            batch,
            atlas,
            "Complex Voice Chat",
            ix + 21.0,
            p[1] + 7.0,
            WHITE,
            true,
        );
        let sub = match s.tab {
            Tab::Players => "Players",
            Tab::Groups => "Groups",
            Tab::Create => "Groups > New group",
            Tab::Password(_) => "Groups > Password",
            Tab::Audio => "Audio",
            Tab::Microphones => "Audio > Microphone",
            Tab::Speakers => "Audio > Speakers",
            Tab::Talking => "Talking",
            Tab::Look => "Customize",
        };
        small(batch, atlas, sub, ix + 21.0, p[1] + 17.0, FAINT);
        // The close button and the connection chip.
        let close = [p[0] + p[2] - 18.0, p[1] + 7.0, 13.0, 13.0];
        let ch = inside(close, cursor);
        rounded(
            batch,
            atlas,
            close,
            if ch { alpha(BAD, 0.6) } else { BUTTON },
        );
        centred(
            batch,
            atlas,
            "x",
            [close[0], close[1] - 1.0, close[2], close[3]],
            WHITE,
        );
        let chip = fit(atlas, &s.view.state, p[2] * 0.4);
        let cw = atlas.font.width(&chip) + 14.0;
        let cr = [close[0] - cw - 5.0, p[1] + 7.0, cw, 13.0];
        let good = s.view.state_good;
        let pulse = if !good && anim {
            0.6 + 0.4 * (clock * 4.0).sin().abs()
        } else {
            1.0
        };
        rounded(
            batch,
            atlas,
            cr,
            if good {
                (92, 230, 138, 36)
            } else {
                (255, 206, 92, 30)
            },
        );
        disc(
            batch,
            atlas,
            cr[0] + 6.0,
            cr[1] + 6.5,
            2.0,
            alpha(if good { GOOD } else { WARN }, pulse),
        );
        label(
            batch,
            atlas,
            &chip,
            cr[0] + 10.0,
            cr[1] + 3.0,
            if good { GOOD } else { WARN },
            true,
        );

        // The sidebar: the highlight glides to the page picked.
        let home = s.tab.home();
        let target = SIDEBAR.iter().position(|(t, _)| *t == home).unwrap_or(0) as f32;
        let at = *s.side_at.get_or_insert(target);
        let mut at2 = at;
        if anim {
            approach(&mut at2, target, 18.0, dt);
        } else {
            at2 = target;
        }
        s.side_at = Some(at2);
        let hr = [f.side[0], f.side[1] + at2 * (ROW + 2.0), f.side[2], ROW];
        rounded(batch, atlas, hr, alpha(accent, 0.2));
        fill(batch, atlas, [hr[0], hr[1] + 4.0, 2.0, ROW - 8.0], accent);
        for wd in &side {
            draw_widget(batch, atlas, s, wd, cursor, &look);
        }
        let hint = format!("{} / Esc closes", s.view.menu_key);
        small(
            batch,
            atlas,
            &fit(atlas, &hint, (f.side[2] - 4.0) / 0.75),
            f.side[0] + 2.0,
            f.side[1] + f.side[3] - 6.0,
            FAINT,
        );

        // The page, sliding in when it changes.
        let start_page = batch.vertices.len();
        let area = f.content;
        for wd in &ws {
            if wd.rect[1] + wd.rect[3] < area[1] - 2.0 || wd.rect[1] > area[1] + area[3] + 2.0 {
                continue;
            }
            draw_widget(batch, atlas, s, wd, cursor, &look);
        }
        if anim {
            let e = ease_out(
                s.page_at
                    .map_or(1.0, |t| now.duration_since(t).as_secs_f32() / 0.2),
            );
            shift_since(batch, start_page, (1.0 - e) * 14.0, 0.0);
            fade_since(batch, start_page, e);
        }
        batch.clip_since(
            start_page,
            area[0] - 2.0,
            area[1] - 1.0,
            area[0] + area[2] + 2.0,
            area[1] + area[3],
        );
        if most > 0.0 {
            let bar_h = (area[3] * area[3] / content).max(12.0);
            let by = area[1] + (area[3] - bar_h) * (s.scroll / most);
            rounded(
                batch,
                atlas,
                [area[0] + area[2] + 2.0, area[1], 2.0, area[3]],
                alpha(WHITE, 0.05),
            );
            rounded(
                batch,
                atlas,
                [area[0] + area[2] + 2.0, by, 2.0, bar_h],
                alpha(accent, 0.8),
            );
        }

        // Ripples where the last clicks landed.
        s.ripples
            .retain(|r| now.duration_since(r.at).as_secs_f32() < 0.45);
        if anim {
            for r in &s.ripples {
                let t = now.duration_since(r.at).as_secs_f32() / 0.45;
                let rs = batch.vertices.len();
                disc(
                    batch,
                    atlas,
                    r.x,
                    r.y,
                    3.0 + ease_out(t) * 50.0,
                    alpha(WHITE, (1.0 - t) * 0.2),
                );
                batch.clip_since(
                    rs,
                    r.clip[0],
                    r.clip[1],
                    r.clip[0] + r.clip[2],
                    r.clip[1] + r.clip[3],
                );
            }
        }

        // Opening: the panel pops up from a little smaller and fades in.
        if anim && clock < 0.35 {
            let t = (clock / 0.3).min(1.0);
            let grow = 0.9 + 0.1 * ease_back(t);
            batch.scale_since(
                start_panel,
                p[0] + p[2] / 2.0,
                p[1] + p[3] / 2.0,
                grow,
                grow,
            );
            fade_since(batch, start_panel, ease_out(t * 1.3));
        }
    });
}

fn apply_slider(s: &mut State, id: Slider, value: f32) {
    let v = &mut s.view;
    let action = match id {
        Slider::Volume => {
            v.volume = value;
            Action::Volume(value)
        }
        Slider::Gain => {
            v.mic_gain = value;
            Action::MicGain(value)
        }
        Slider::Threshold => {
            v.threshold_db = value;
            Action::Threshold(value)
        }
        Slider::Opacity => {
            v.prefs.opacity = value;
            Action::Set("opacity", value.to_string())
        }
        Slider::HudScale => {
            v.prefs.hud_scale = value;
            Action::Set("hud_scale", value.to_string())
        }
        Slider::Player(u) => {
            if let Some(p) = v.players.iter_mut().find(|p| p.uuid == u) {
                p.volume = value;
            }
            Action::PlayerVolume(u, value)
        }
    };
    // Only the latest value of a drag matters.
    s.actions.retain(|a| !same_slot(a, &action));
    s.actions.push(action);
}

fn same_slot(a: &Action, b: &Action) -> bool {
    match (a, b) {
        (Action::PlayerVolume(x, _), Action::PlayerVolume(y, _)) => x == y,
        (Action::Set(x, _), Action::Set(y, _)) => x == y,
        _ => std::mem::discriminant(a) == std::mem::discriminant(b),
    }
}

fn slider_value(w: &Widget, x: f32) -> Option<(Slider, f32)> {
    let Kind::Slider {
        id,
        label: text,
        min,
        max,
        ..
    } = &w.kind
    else {
        return None;
    };
    let r = w.rect;
    let track = if text.is_empty() {
        r
    } else {
        let lw = (r[2] * 0.36).floor().min(96.0);
        [r[0] + lw, r[1], r[2] - lw, r[3]]
    };
    let t = ((x - track[0] - 2.0) / (track[2] - 4.0)).clamp(0.0, 1.0);
    let mut value = min + (max - min) * t;
    // Snap: 1 dB for the level, a quarter for the status size, else 5%.
    value = match id {
        Slider::Threshold => value.round(),
        Slider::HudScale => (value * 4.0).round() / 4.0,
        _ => (value * 20.0).round() / 20.0,
    };
    Some((*id, value))
}

fn go(s: &mut State, tab: Tab) {
    if matches!(tab, Tab::Microphones | Tab::Speakers) {
        s.actions.push(Action::RefreshDevices);
    }
    if tab == Tab::Create {
        s.fields.insert(Field::Name, String::new());
        s.fields.insert(Field::Password, String::new());
        s.create_kind = 0;
        s.focus = Some(Field::Name);
    } else if let Tab::Password(_) = tab {
        s.fields.insert(Field::JoinPassword, String::new());
        s.focus = Some(Field::JoinPassword);
    } else {
        s.focus = None;
    }
    if s.tab != tab {
        s.page_at = Some(Instant::now());
    }
    s.tab = tab;
    s.scroll = 0.0;
    s.scroll_target = 0.0;
}

/// Create the group / join with the password, from the page's boxes.
fn submit(s: &mut State) {
    match s.tab {
        Tab::Create => {
            let name = field_text(s, Field::Name).trim().to_owned();
            if name.is_empty() {
                s.focus = Some(Field::Name);
                return;
            }
            let pw = field_text(s, Field::Password).trim().to_owned();
            s.actions.push(Action::CreateGroup {
                name,
                password: (!pw.is_empty()).then_some(pw),
                kind: s.create_kind,
            });
            go(s, Tab::Groups);
        }
        Tab::Password(id) => {
            let pw = field_text(s, Field::JoinPassword);
            s.actions.push(Action::JoinGroup(id, Some(pw)));
            go(s, Tab::Groups);
        }
        _ => {}
    }
}

/// A click (or a drag, with the button held) at `(x, y)`.
pub fn click(x: f32, y: f32, size: (f32, f32), drag: bool) {
    with_state(|s| {
        let f = frame(size.0, size.1, s.view.prefs.size);
        let area = f.content;
        let most = (s.content - area[3]).max(0.0);
        let bar_h = if s.content > 0.0 {
            (area[3] * area[3] / s.content).max(12.0)
        } else {
            12.0
        };
        if drag {
            if s.scrollbar_dragging && most > 0.0 {
                let travel = area[3] - bar_h;
                let ratio = if travel > 0.0 {
                    ((y - area[1] - bar_h / 2.0) / travel).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                s.scroll_target = (ratio * most).clamp(0.0, (s.content - 40.0).max(0.0));
                return;
            }
            let (ws, _) = widgets(s, &f, s.scroll);
            let Some(id) = s.dragging else { return };
            // The slider being dragged, wherever the pointer went.
            if let Some(w) = ws
                .iter()
                .find(|w| matches!(&w.kind, Kind::Slider { id: i, .. } if *i == id))
                && let Some((id, value)) = slider_value(w, x)
            {
                apply_slider(s, id, value);
            }
            return;
        }
        s.dragging = None;
        s.scrollbar_dragging = false;
        if most > 0.0 && inside([area[0] + area[2] - 4.0, area[1], 16.0, area[3]], (x, y)) {
            s.scrollbar_dragging = true;
            let travel = area[3] - bar_h;
            let ratio = if travel > 0.0 {
                ((y - area[1] - bar_h / 2.0) / travel).clamp(0.0, 1.0)
            } else {
                0.0
            };
            s.scroll_target = (ratio * most).clamp(0.0, (s.content - 40.0).max(0.0));
            return;
        }
        let (ws, _) = widgets(s, &f, s.scroll);
        let p = f.panel;
        if inside([p[0] + p[2] - 18.0, p[1] + 7.0, 13.0, 13.0], (x, y)) {
            s.done = true;
            return;
        }
        let side = sidebar(s, &f);
        // Sliders and buttons sit on player rows: test them first.
        let mut order: Vec<&Widget> = side.iter().collect();
        let content: Vec<&Widget> = if inside(f.content, (x, y)) {
            ws.iter().collect()
        } else {
            Vec::new()
        };
        order.extend(
            content
                .iter()
                .filter(|w| !matches!(w.kind, Kind::Player { .. })),
        );
        order.extend(
            content
                .iter()
                .filter(|w| matches!(w.kind, Kind::Player { .. })),
        );
        let Some(w) = order.into_iter().find(|w| inside(w.rect, (x, y))).cloned() else {
            s.focus = None;
            return;
        };
        if !matches!(
            w.kind,
            Kind::Slider { .. } | Kind::Text { .. } | Kind::Heading { .. } | Kind::Player { .. }
        ) {
            let clip = if inside(f.side, (x, y)) {
                f.side
            } else {
                w.rect
            };
            s.ripples.push(Ripple {
                at: Instant::now(),
                x,
                y,
                clip,
            });
        }
        if !matches!(w.kind, Kind::Input { .. }) {
            s.focus = None;
        }
        let act = match &w.kind {
            Kind::Slider { .. } => {
                if let Some((id, value)) = slider_value(&w, x) {
                    s.dragging = Some(id);
                    apply_slider(s, id, value);
                }
                return;
            }
            Kind::Tab { tab, .. } => Act::Go(*tab),
            Kind::Input { field, .. } => Act::Focus(*field),
            Kind::Swatch { name, .. } => Act::Do(Action::Colour(name.clone())),
            Kind::Theme { name, .. } => set("theme", name),
            Kind::Group { row, joined: false } => Act::Join(row.id, row.locked),
            _ => match w.act() {
                Some(a) => a.clone(),
                None => return,
            },
        };
        match act {
            Act::Do(a) => {
                // Show the change at once; the game's next view confirms it.
                match &a {
                    Action::Mute => s.view.muted = !s.view.muted,
                    Action::Disable => s.view.disabled = !s.view.disabled,
                    Action::VoiceActivation(b) => s.view.voice_activation = *b,
                    Action::Test(b) => s.view.testing = *b,
                    Action::Colour(c) => s.view.colour = c.clone(),
                    Action::PlayerVolume(u, v) => {
                        if let Some(p) = s.view.players.iter_mut().find(|p| p.uuid == *u) {
                            p.volume = *v;
                        }
                    }
                    Action::Set(k, v) => {
                        s.view.prefs.set(k, v);
                    }
                    Action::LeaveGroup => s.view.own_group = None,
                    _ => {}
                }
                s.actions.retain(|old| {
                    !same_slot(old, &a) || matches!(a, Action::Mute | Action::Disable)
                });
                s.actions.push(a);
            }
            Act::Go(tab) => go(s, tab),
            Act::Pick(input, name) => {
                if input {
                    s.view.microphone = name.clone();
                    s.actions.push(Action::Microphone(name));
                } else {
                    s.view.speakers = name.clone();
                    s.actions.push(Action::Speakers(name));
                }
                go(s, Tab::Audio);
            }
            Act::Focus(field) => s.focus = Some(field),
            Act::Kind(k) => s.create_kind = k,
            Act::Join(id, locked) => {
                if locked {
                    go(s, Tab::Password(id));
                } else {
                    s.actions.push(Action::JoinGroup(id, None));
                }
            }
            Act::Submit => submit(s),
        }
    });
}

/// The wheel scrolls the page.
pub fn wheel(amount: f32) {
    with_state(|s| {
        s.scroll_target = (s.scroll_target - amount * 18.0).clamp(0.0, (s.content - 40.0).max(0.0));
    });
}

/// A key on the menu; whether it used it. Escape that isn't used closes
/// the menu (the page's own handling).
pub fn key(key: winit::keyboard::KeyCode) -> bool {
    use winit::keyboard::KeyCode as K;
    with_state(|s| {
        s.dragging = None;
        s.scrollbar_dragging = false;
        if let Some(field) = s.focus {
            match key {
                K::Escape => s.focus = None,
                K::Backspace => {
                    s.fields.entry(field).or_default().pop();
                }
                K::Enter | K::NumpadEnter => {
                    if field == Field::Search {
                        s.focus = None;
                    } else {
                        submit(s);
                    }
                }
                K::Tab => {
                    s.focus = match field {
                        Field::Name => Some(Field::Password),
                        Field::Password => Some(Field::Name),
                        f => Some(f),
                    }
                }
                _ => {}
            }
            return true;
        }
        match key {
            K::Escape => match s.tab {
                Tab::Microphones | Tab::Speakers => {
                    go(s, Tab::Audio);
                    true
                }
                Tab::Create | Tab::Password(_) => {
                    go(s, Tab::Groups);
                    true
                }
                _ => false,
            },
            K::Enter | K::NumpadEnter => {
                submit(s);
                true
            }
            K::ArrowUp | K::ArrowDown => {
                // Up and down move through the sidebar's pages.
                let at = SIDEBAR
                    .iter()
                    .position(|(t, _)| *t == s.tab.home())
                    .unwrap_or(0);
                let next = if key == K::ArrowUp {
                    at.saturating_sub(1)
                } else {
                    (at + 1).min(SIDEBAR.len() - 1)
                };
                go(s, SIDEBAR[next].0);
                true
            }
            _ => true,
        }
    })
}

/// Typed text goes into the box being typed into.
pub fn type_text(text: &str) {
    with_state(|s| {
        let Some(field) = s.focus else { return };
        let most = if field == Field::Search { 32 } else { 24 };
        let entry = s.fields.entry(field).or_default();
        for ch in text.chars().filter(|c| !c.is_control()) {
            if entry.chars().count() < most {
                entry.push(ch);
            }
        }
        if field == Field::Search {
            s.scroll_target = 0.0;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reset() {
        *STATE.lock().unwrap() = None;
    }

    const SIZE: (f32, f32) = (480.0, 270.0);

    /// Where to click to hit the first widget `pick` matches (sidebar or page).
    fn find(pick: impl Fn(&Kind) -> bool) -> (f32, f32) {
        with_state(|s| {
            let f = frame(SIZE.0, SIZE.1, s.view.prefs.size);
            let (ws, _) = widgets(s, &f, s.scroll);
            let mut all = sidebar(s, &f);
            all.extend(ws);
            all.iter()
                .find(|w| pick(&w.kind))
                .map(|w| (w.rect[0] + 3.0, w.rect[1] + 3.0))
                .expect("widget")
        })
    }

    fn button_named(label: &'static str) -> impl Fn(&Kind) -> bool {
        move |k| match k {
            Kind::Button { label: l, .. } | Kind::Tab { label: l, .. } => l.starts_with(label),
            _ => false,
        }
    }

    fn press(pick: impl Fn(&Kind) -> bool) {
        let (x, y) = find(pick);
        click(x, y, SIZE, false);
    }

    /// Clicks land on what's drawn: Audio, the microphone picker, a device.
    #[test]
    fn menu_pages_groups_and_typing() {
        reset();
        open();
        set_view(View {
            microphones: vec!["Mic A".into(), "Mic B".into()],
            ..View::default()
        });
        press(button_named("Audio"));
        press(button_named("Microphone:"));
        press(button_named("Mic B"));
        let actions = take_actions();
        assert!(
            actions.contains(&Action::Microphone(Some("Mic B".into()))),
            "{actions:?}"
        );
        assert!(with_state(|s| s.tab == Tab::Audio));
        press(button_named("Microphone:"));
        assert!(
            key(winit::keyboard::KeyCode::Escape),
            "Escape goes back a page"
        );
        assert!(
            !key(winit::keyboard::KeyCode::Escape),
            "Escape on a sidebar page closes the menu"
        );

        // Groups: a locked one asks for the password; Enter joins with it.
        set_view(View {
            connected: true,
            groups_enabled: true,
            groups: vec![GroupRow {
                id: 42,
                name: "Base".into(),
                locked: true,
                ..GroupRow::default()
            }],
            ..View::default()
        });
        press(button_named("Groups"));
        press(|k| matches!(k, Kind::Group { row, .. } if row.id == 42));
        assert!(with_state(|s| s.tab == Tab::Password(42)));
        assert!(typing());
        type_text("pw");
        key(winit::keyboard::KeyCode::Enter);
        assert!(take_actions().contains(&Action::JoinGroup(42, Some("pw".into()))));
        assert!(!typing());

        // Making a group: name, password, the Isolated kind, Create.
        press(button_named("+  Create"));
        type_text("Friends");
        key(winit::keyboard::KeyCode::Tab);
        type_text("123");
        press(button_named("Isolated"));
        press(button_named("Create"));
        let actions = take_actions();
        assert!(
            actions.contains(&Action::CreateGroup {
                name: "Friends".into(),
                password: Some("123".into()),
                kind: 2
            }),
            "{actions:?}"
        );

        // Customize: the red theme and the large size apply at once.
        press(button_named("Customize"));
        press(|k| matches!(k, Kind::Theme { name, .. } if name == "red"));
        press(button_named("Large"));
        let actions = take_actions();
        assert!(
            actions.contains(&Action::Set("theme", "red".into())),
            "{actions:?}"
        );
        assert!(with_state(
            |s| s.view.prefs.theme == "red" && s.view.prefs.size == 2
        ));
    }

    #[test]
    fn prefs_parse_and_clamp() {
        let mut p = Prefs::default();
        assert!(p.set("opacity", "0.1"));
        assert_eq!(p.opacity, 0.5);
        assert!(p.set("hud_corner", "3"));
        assert_eq!(p.hud_corner, 3);
        assert!(p.set("hud_corner", "9"));
        assert_eq!(p.hud_corner, 0);
        assert!(!p.set("theme", "purple") && p.theme == "blue");
        assert!(!p.set("nope", "1"));
    }
}
