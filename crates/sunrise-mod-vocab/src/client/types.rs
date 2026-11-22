// Types that lived in the API root before the domain split.
use crate::*;

/// The family of a client notification entering the toast queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ToastKind {
    /// An advancement completed by the local player.
    Advancement,
    /// Newly unlocked recipe displays.
    Recipe,
    /// A tutorial hint, including its progress indicator.
    Tutorial,
    /// A system notice, such as narrator status, backups or pack downloads.
    System,
    /// A toast without one of the above families, including now-playing music.
    Other,
}

/// What a key did in an `input_key` event (GLFW's `action`: release 0, press 1, repeat 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum KeyAction {
    /// The key was let go.
    Release,
    /// The key went down.
    Press,
    /// The key is held and the operating system repeats it.
    Repeat,
}

/// A snapshot of the client window's modifier keys.
///
/// The bits use GLFW's input-event encoding: shift 1, control 2, alt 4 and super 8.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientModifiers {
    /// Modifier bits in GLFW's input-event encoding.
    pub bits: u32,
}

impl ClientModifiers {
    /// Construct a snapshot from GLFW modifier bits.
    pub fn new(bits: u32) -> Self {
        Self { bits }
    }
    /// Whether shift is held.
    pub fn shift(self) -> bool {
        self.bits & 1 != 0
    }
    /// Whether control is held.
    pub fn control(self) -> bool {
        self.bits & 2 != 0
    }
    /// Whether alt is held.
    pub fn alt(self) -> bool {
        self.bits & 4 != 0
    }
    /// Whether super is held.
    pub fn super_key(self) -> bool {
        self.bits & 8 != 0
    }
}

/// A resolved vanilla key mapping for one running client.
///
/// Resolve it with [`vanilla_key_mapping`](crate::prelude::ClientInputClientHost::vanilla_key_mapping)
/// in `client_setup`. Keep it across rebinding, options replacement, and screen or world changes.
/// A different client refuses it with [`MappingError::Stale`]; resolve the saved mapping name
/// against that client. Persist the name, not this handle.
// no wire: process-local client mapping handle
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct VanillaKeyMappingRef {
    pub(super) client_generation: std::num::NonZeroU64,
    pub(super) slot: usize,
}

/// A resolved mod key mapping in the current client's registration roster.
///
/// Resolve it with [`mod_key_mapping`](crate::prelude::ClientInputClientHost::mod_key_mapping)
/// after registration, usually in `client_setup`. Rebinding, resetting and changing screens do not
/// invalidate it. If the client or its ordered registration roster is replaced, discard the stale
/// handle and resolve again. Persist the owner and id, not this handle.
// no wire: process-local registered key mapping handle
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ModKeyMappingRef {
    pub(super) registry_generation: std::num::NonZeroU64,
    pub(super) slot: usize,
}

/// Why resolving or using a vanilla key mapping, or reading a translated mod key label, was refused.
// no wire: refusal of a process-local client mapping operation
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MappingError {
    /// Resolution found no vanilla mapping with the given name.
    Unknown {
        /// The name asked for.
        key: String,
    },
    /// The handle does not belong to the current client or no longer identifies a mapping.
    Stale,
    /// No client mapping backend is available in this host or phase.
    Unavailable,
    /// The translated binding label could not be represented as API text.
    Label(TextFormError),
}

impl NoBackend for MappingError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

impl From<TextFormError> for MappingError {
    fn from(error: TextFormError) -> Self {
        Self::Label(error)
    }
}

impl std::fmt::Display for MappingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown { key } => write!(f, "unknown vanilla mapping {key:?}"),
            Self::Stale => f.write_str("stale vanilla mapping handle"),
            Self::Unavailable => f.write_str("client mappings are unavailable"),
            Self::Label(error) => write!(f, "invalid mapping label: {error}"),
        }
    }
}

impl std::error::Error for MappingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Label(error) => Some(error),
            _ => None,
        }
    }
}

/// The metadata of a server play packet observed by `Hooks::client_packet`.
///
/// Fields beyond the type id are exposed only for custom payloads. This is
/// plain mod API data, not the game's decoded protocol enum.
// no wire: this is a local envelope for already-encoded vanilla packet bytes.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ClientPacket {
    /// The protocol type, such as `minecraft:ping` or `minecraft:custom_payload`.
    /// An id outside the protocol's packet table is reported as `sunrise:unknown`.
    pub id: Identifier,
    /// The channel and bytes of a custom payload; `None` for other packet types.
    pub custom_payload: Option<ClientPayload>,
}

impl ClientPacket {
    /// Construct packet metadata with an optional custom payload.
    pub fn new(id: Identifier, custom_payload: Option<ClientPayload>) -> Self {
        Self { id, custom_payload }
    }
}

/// The channel and uninterpreted bytes of a server custom payload.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientPayload {
    /// The namespaced plugin channel, exactly as received.
    pub channel: String,
    /// The payload bytes, without the packet id or channel prefix.
    pub data: Vec<u8>,
}

impl ClientPayload {
    /// Construct a custom payload without interpreting its bytes.
    pub fn new(channel: String, data: Vec<u8>) -> Self {
        Self { channel, data }
    }
}

/// Camera values from the most recently extracted world frame.
#[derive(Clone, Debug)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct Camera {
    /// The camera position in blocks (x, y, z).
    pub position: [f64; 3],
    /// The eye height above the position, in blocks.
    pub eye_height: f32,
    /// The fraction of the current tick elapsed, 0 to 1.
    pub partial_tick: f32,
    /// The horizontal rotation in degrees.
    pub yaw: f32,
    /// The vertical rotation in degrees; negative looks up.
    pub pitch: f32,
    /// The vertical field of view in degrees.
    pub fov_degrees: f32,
    /// The near clip distance in blocks.
    pub near: f32,
    /// The far clip distance in blocks.
    pub far: f32,
    /// Camera-relative view matrix in column-major order.
    pub view: [f32; 16],
    /// Reversed-Z projection matrix in column-major order.
    pub projection: [f32; 16],
    /// The id of the dimension drawn, such as `minecraft:overworld`.
    pub dimension: String,
}

/// A snapshot of the client's input preferences.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientInputOptions {
    /// Mouse sensitivity, normally between 0 and 1.
    pub sensitivity: f32,
    /// Whether horizontal mouse movement is inverted.
    pub invert_x: bool,
    /// Whether vertical mouse movement is inverted.
    pub invert_y: bool,
    /// Multiplier applied to mouse wheel movement.
    pub mouse_wheel_sensitivity: f32,
    /// Whether scroll input is reduced to discrete steps.
    pub discrete_scroll: bool,
    /// Whether crouching toggles instead of requiring a held key.
    pub toggle_crouch: bool,
    /// Whether sprinting toggles instead of requiring a held key.
    pub toggle_sprint: bool,
    /// Whether automatic jumping is enabled.
    pub auto_jump: bool,
}

/// A snapshot of the client's graphics preferences.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientGraphicsOptions {
    /// Field of view in degrees, 30 to 110.
    pub fov: f32,
    /// Render distance in chunks.
    pub render_distance: i32,
    /// Simulation distance in chunks.
    pub simulation_distance: i32,
    /// Brightness, 0 to 1.
    pub brightness: f32,
    /// Frame rate limit; 260 means unlimited.
    pub max_fps: u32,
    /// Whether vertical sync is on.
    pub vsync: bool,
    /// Whether the window is fullscreen.
    pub fullscreen: bool,
    /// Whether the view bobs while walking.
    pub view_bobbing: bool,
    /// Whether smooth lighting is on.
    pub smooth_lighting: bool,
    /// Particle setting: 0 all, 1 decreased, 2 minimal.
    pub particles: u8,
    /// Entity distance scale.
    pub entity_distance: f32,
    /// Biome blend radius.
    pub biome_blend: i32,
}

/// A snapshot of the client's GUI and accessibility preferences.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientGuiOptions {
    /// Requested GUI scale; 0 means automatic.
    pub gui_scale: u32,
    /// Attack indicator: 0 off, 1 crosshair, 2 hotbar.
    pub attack_indicator: u8,
    /// Whether the main hand is the left one.
    pub main_hand_left: bool,
    /// Whether the debug screen shows reduced information.
    pub reduced_debug_info: bool,
    /// Current `advancedItemTooltips` preference, toggled with F3+H.
    /// This is the user's preference, independent of a tooltip request's advanced flag.
    pub advanced_item_tooltips: bool,
    /// Whether the high contrast resource pack is on.
    pub high_contrast: bool,
    /// Whether subtitles are shown.
    pub show_subtitles: bool,
    /// Multiplier for how long subtitles and toasts stay.
    pub notification_display_time: f32,
    /// Opacity of the background behind text, 0 to 1.
    pub text_background_opacity: f32,
    /// Menu background blur radius.
    pub menu_blur: i32,
}

/// A snapshot of the client's chat preferences.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientChatOptions {
    /// Which chat is shown: 0 all, 1 commands only, 2 hidden.
    pub visibility: u8,
    /// Whether colours in chat show.
    pub colors: bool,
    /// Whether links in chat are clickable.
    pub links: bool,
    /// Raw `chatOpacity` preference, 0 to 1; text opacity is `value * 0.9 + 0.1`.
    /// Background opacity is [`ClientGuiOptions::text_background_opacity`].
    pub opacity: f32,
    /// Chat scale.
    pub scale: f32,
    /// Chat width, 0 to 1 of its range.
    pub width: f32,
    /// Chat height while the chat is open, 0 to 1 of its range.
    pub height_focused: f32,
    /// Chat height while the chat is closed, 0 to 1 of its range.
    pub height_unfocused: f32,
    /// Chat line spacing.
    pub line_spacing: f32,
    /// Seconds between player chat messages showing.
    pub delay: f64,
}

/// A snapshot of the client's audio preferences.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientAudioOptions {
    /// Master volume, 0 to 1.
    pub master_volume: f32,
}

// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// Borrowed humanoid inputs at arm-pose extraction, shared by players and mobs.
/// Poses are in main/off-hand order, before listeners answer; item declarations
/// are already included. Model-specific animation may subsequently adjust the arms.
// exhaustive: the engine builds this read-only view
pub struct HumanoidRenderState<'a> {
    /// Stack held in the main hand; empty stacks are absent.
    pub main_hand: Option<&'a ItemStack>,
    /// Stack held in the off hand; empty stacks are absent.
    pub off_hand: Option<&'a ItemStack>,
    /// Whether the entity's main arm is its left arm.
    pub main_arm_left: bool,
    /// Whether the entity is currently using an item.
    pub is_using_item: bool,
    /// Hand currently using an item, absent when no item is in use.
    pub use_hand: Option<Hand>,
    /// Elapsed item-use ticks at extraction.
    pub ticks_using_item: i32,
    /// Animation of the item currently being used.
    pub use_animation: UseAnimation,
    /// Selected main-hand pose before any hook answers.
    pub main_arm_pose: ArmPose,
    /// Selected off-hand pose before any hook answers.
    pub off_arm_pose: ArmPose,
}

// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// Context and adjustable pose for a first-person hand render.
///
/// Translation is in blocks and rotation is in degrees (pitch, yaw, roll).
/// The renderer applies the transform before the ordinary hand and item poses.
// exhaustive: the game builds it with a literal; mods only read it
#[derive(Clone)]
pub struct HandRender<'a> {
    /// The UUID of the player whose hand is drawn.
    pub player: u128,
    /// The network id of that player.
    pub player_entity_id: i32,
    /// Which hand is drawn.
    pub hand: Hand,
    /// The stack shown in the hand; `None` for the empty main-hand arm.
    pub stack: Option<&'a ItemStack>,
    /// The fraction of the current tick elapsed, 0 to 1.
    pub partial: f32,
    /// The player pitch interpolated for this frame, in degrees.
    pub interpolated_pitch: f32,
    /// The swing progress, 0 to 1.
    pub swing: f32,
    /// The inverse equip height, including the swap animation scale.
    pub equip: f32,
    /// Vanilla packed light at the eyes (`LightCoordsUtil.pack`: block << 4 | sky << 20).
    pub packed_light: u32,
    /// A mod-set offset in blocks; applied before the hand is drawn.
    pub translation: [f32; 3],
    /// A mod-set rotation as pitch, yaw and roll in degrees.
    pub rotation: [f32; 3],
    /// A mod-set uniform scale; 1 leaves the size alone.
    pub scale: f32,
}

/// Context for a player tuning a note block.
#[derive(Clone, Copy, Debug)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct NoteBlockChange {
    /// The level the note block is in.
    pub dimension: LevelDimension,
    /// The note block's position.
    pub pos: BlockPos,
    /// The block's instrument before tuning.
    pub instrument: NoteInstrument,
    /// The note before tuning, in the range 0 through 24.
    pub old_note: i32,
}

/// A note block's instrument (`NoteBlockInstrument`), for `note_block_play`,
/// in the `instrument` property's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum NoteInstrument {
    /// The harp, over most blocks.
    Harp,
    /// The bass drum, over stone.
    Basedrum,
    /// The snare drum, over sand and gravel.
    Snare,
    /// The clicks, over glass.
    Hat,
    /// The double bass, over wood.
    Bass,
    /// The flute, over clay.
    Flute,
    /// The bell, over a gold block.
    Bell,
    /// The guitar, over wool.
    Guitar,
    /// The chime, over packed ice.
    Chime,
    /// The xylophone, over a bone block.
    Xylophone,
    /// The iron xylophone, over an iron block.
    IronXylophone,
    /// The cow bell, over soul sand.
    CowBell,
    /// The didgeridoo, over a pumpkin.
    Didgeridoo,
    /// The bit, over an emerald block.
    Bit,
    /// The banjo, over hay.
    Banjo,
    /// The pling, over glowstone.
    Pling,
    /// The trumpet, over a copper block.
    Trumpet,
    /// The trumpet, over an exposed copper block.
    TrumpetExposed,
    /// The trumpet, over an oxidized copper block.
    TrumpetOxidized,
    /// The trumpet, over a weathered copper block.
    TrumpetWeathered,
    /// A zombie's sound, over a zombie head.
    Zombie,
    /// A skeleton's sound, over a skeleton skull.
    Skeleton,
    /// A creeper's sound, over a creeper head.
    Creeper,
    /// The ender dragon's sound, over a dragon head.
    Dragon,
    /// A wither skeleton's sound, over its skull.
    WitherSkeleton,
    /// A piglin's sound, over a piglin head.
    Piglin,
    /// The sound set by a player head's own note block sound.
    CustomHead,
}

impl NoteInstrument {
    /// Every instrument, in the `instrument` property's order.
    pub const ALL: [Self; 27] = {
        use NoteInstrument::*;
        [
            Harp,
            Basedrum,
            Snare,
            Hat,
            Bass,
            Flute,
            Bell,
            Guitar,
            Chime,
            Xylophone,
            IronXylophone,
            CowBell,
            Didgeridoo,
            Bit,
            Banjo,
            Pling,
            Trumpet,
            TrumpetExposed,
            TrumpetOxidized,
            TrumpetWeathered,
            Zombie,
            Skeleton,
            Creeper,
            Dragon,
            WitherSkeleton,
            Piglin,
            CustomHead,
        ]
    };
    /// The `instrument` property's value (`harp`, `cow_bell`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Harp => "harp",
            Self::Basedrum => "basedrum",
            Self::Snare => "snare",
            Self::Hat => "hat",
            Self::Bass => "bass",
            Self::Flute => "flute",
            Self::Bell => "bell",
            Self::Guitar => "guitar",
            Self::Chime => "chime",
            Self::Xylophone => "xylophone",
            Self::IronXylophone => "iron_xylophone",
            Self::CowBell => "cow_bell",
            Self::Didgeridoo => "didgeridoo",
            Self::Bit => "bit",
            Self::Banjo => "banjo",
            Self::Pling => "pling",
            Self::Trumpet => "trumpet",
            Self::TrumpetExposed => "trumpet_exposed",
            Self::TrumpetOxidized => "trumpet_oxidized",
            Self::TrumpetWeathered => "trumpet_weathered",
            Self::Zombie => "zombie",
            Self::Skeleton => "skeleton",
            Self::Creeper => "creeper",
            Self::Dragon => "dragon",
            Self::WitherSkeleton => "wither_skeleton",
            Self::Piglin => "piglin",
            Self::CustomHead => "custom_head",
        }
    }
}

/// The mixer group a sound plays on (`SoundSource`), in vanilla's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum SoundSource {
    /// The master volume, which scales every other source.
    Master,
    /// Music discs aside, the game's music.
    Music,
    /// Jukeboxes and music discs.
    Records,
    /// Rain and thunder.
    Weather,
    /// Sounds blocks make.
    Blocks,
    /// Hostile creatures.
    Hostile,
    /// Friendly and neutral creatures.
    Neutral,
    /// Players.
    Players,
    /// Ambient and cave sounds.
    Ambient,
    /// Voice sounds.
    Voice,
    /// Interface sounds, such as a button click.
    Ui,
}

impl SoundSource {
    /// Its position in vanilla's `SoundSource` order, as the network sends it.
    pub fn index(self) -> usize {
        self as usize
    }
    /// Every source, in vanilla's order (`SoundSource.values()`).
    pub const ALL: [SoundSource; 11] = [
        Self::Master,
        Self::Music,
        Self::Records,
        Self::Weather,
        Self::Blocks,
        Self::Hostile,
        Self::Neutral,
        Self::Players,
        Self::Ambient,
        Self::Voice,
        Self::Ui,
    ];
}

/// How a sound fades with distance (`SoundInstance.Attenuation`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum SoundAttenuation {
    /// Heard at full volume everywhere.
    None,
    /// Fades linearly to silence at the sound's range.
    Linear,
}

/// A sound about to play, for `play_sound` (`SoundInstance`'s own settings:
/// vanilla's `getIdentifier`, `getSource`, `getVolume`, `getPitch`, `getX/Y/Z`,
/// `isLooping`, `getDelay`, `getAttenuation` and `isRelative`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct SoundPlay {
    /// The sound event (`minecraft:entity.pig.ambient`).
    pub identifier: String,
    /// The mixer group it plays on.
    pub source: SoundSource,
    /// Its own volume, before the source's and the master's.
    pub volume: f32,
    /// Its pitch, 1.0 unchanged.
    pub pitch: f32,
    /// Where it plays, in blocks; meaningless when `relative`.
    pub position: [f64; 3],
    /// It restarts when it ends.
    pub looping: bool,
    /// Ticks between a looping sound's repeats (`SoundInstance.getDelay`): 0 loops it
    /// seamlessly, more plays it again that many ticks after it ends.
    pub delay: i32,
    /// How it fades with distance.
    pub attenuation: SoundAttenuation,
    /// It plays at the listener, wherever they move.
    pub relative: bool,
    /// How the mixer shapes it: its own gain, filters and reverb sends ([`SoundEffects`], with
    /// every field's range). [`SoundEffects::NONE`] by default, which changes nothing.
    ///
    /// Set at the play (`play_local_sound`, or a `play_sound` edit with `starting` true), and
    /// changed afterwards only on the live refresh of a ticking sound (a `play_sound` edit with
    /// `starting` false, once a tick). A sound that doesn't tick keeps the effects it started
    /// with until it ends: nothing changes them after it starts. A value out of range refuses
    /// the play or the edit ([`SoundEffects::valid`]).
    pub effects: SoundEffects,
}

/// How the mixer shapes one sound ([`SoundPlay::effects`]): a gain, a low-pass and a high-pass on
/// the sound heard directly, and how much of it goes to each of three shared reverbs. A muffled
/// sound behind a wall takes a low `lowpass` and some `direct` away; a sound in a cave sends to
/// `cavern`. Every change glides over one output buffer, so moving them each tick never clicks.
/// [`SoundEffects::NONE`] changes nothing, and a sound with it plays exactly as without effects.
///
/// Every field's valid range, inclusive at both ends:
///
/// | field | range | off / open |
/// | --- | --- | --- |
/// | `gain` | 0 to 1 | 1 |
/// | `direct` | 0 to 1 | 1 |
/// | `lowpass` | 20 to 20000 Hz | 20000 (open) |
/// | `highpass` | 0 to 20000 Hz | 0 (off) |
/// | `room`, `hall`, `cavern` | 0 to 1 | 0 (no send) |
/// | `reverb_lowpass` | 20 to 20000 Hz | 20000 (open) |
///
/// The cutoffs are in hertz whatever the output's sample rate, so the same value is valid on
/// every device. NaN, infinities and anything outside a range are refused, never clamped
/// ([`SoundEffects::valid`]): the `play_sound` edit at the start, the live refresh of a ticking
/// sound and `play_local_sound` all run that one check, and a refused value plays nothing, as a
/// bad volume does. Inside the mixer, a direct-sound cutoff above the output's Nyquist frequency
/// (half its sample rate; the mixer stops at 0.49 of the rate) works as that limit: on a 32 kHz
/// output, `lowpass` 18000 filters as 15680 would. That limit depends on the device, not on the
/// value, and is not a refusal.
///
/// A plain value with no sound id in it, so other audio may take the same type later.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct SoundEffects {
    /// The whole sound's level, direct and sent, 0 to 1, after its volume and distance. It
    /// doesn't change how far the sound carries.
    pub gain: f32,
    /// The level of the sound heard directly, 0 to 1; the sends keep theirs. Lower it for a
    /// sound heard only around a corner or through a wall.
    pub direct: f32,
    /// The direct sound's low-pass cutoff in hertz, 20 to 20000, independent of the output
    /// rate: at 20000 it is open (no filter). A two-pole Butterworth (12 dB per octave).
    pub lowpass: f32,
    /// The direct sound's high-pass cutoff in hertz, 0 to 20000, independent of the output
    /// rate: at 0 it is off (no filter). A two-pole Butterworth (12 dB per octave).
    pub highpass: f32,
    /// The send to the small room's reverb, 0 to 1: 0.6 s to fall 60 dB, damped above 7000 Hz
    /// (a house, a corridor).
    pub room: f32,
    /// The send to the hall's reverb, 0 to 1: 1.8 s to fall 60 dB, damped above 4500 Hz (a
    /// large build, a ravine).
    pub hall: f32,
    /// The send to the cavern's reverb, 0 to 1: 4.0 s to fall 60 dB, damped above 3000 Hz (a
    /// deep cave).
    pub cavern: f32,
    /// The sends' low-pass cutoff in hertz, 20 to 20000, independent of the output rate: at
    /// 20000 it is open. A one-pole low-pass (6 dB per octave) on what goes to the reverbs.
    pub reverb_lowpass: f32,
}

impl SoundEffects {
    /// No effects: full level, filters open, no reverb.
    pub const NONE: Self = Self {
        gain: 1.0,
        direct: 1.0,
        lowpass: 20_000.0,
        highpass: 0.0,
        room: 0.0,
        hall: 0.0,
        cavern: 0.0,
        reverb_lowpass: 20_000.0,
    };

    /// Every value is a finite number in its range: `gain`, `direct`, `room`, `hall` and
    /// `cavern` 0 to 1; `lowpass` and `reverb_lowpass` 20 to 20000 Hz; `highpass` 0 to 20000 Hz
    /// (all inclusive). False for NaN, an infinity or anything outside; nothing is clamped. The
    /// one check behind the `play_sound` edit, the live refresh of a ticking sound and
    /// `play_local_sound`. The ranges don't depend on the output's sample rate.
    pub fn valid(&self) -> bool {
        let within = |v: f32, lo: f32, hi: f32| v.is_finite() && (lo..=hi).contains(&v);
        [self.gain, self.direct, self.room, self.hall, self.cavern]
            .iter()
            .all(|&v| within(v, 0.0, 1.0))
            && within(self.lowpass, 20.0, 20_000.0)
            && within(self.highpass, 0.0, 20_000.0)
            && within(self.reverb_lowpass, 20.0, 20_000.0)
    }
}

impl Default for SoundEffects {
    fn default() -> Self {
        Self::NONE
    }
}

#[cfg(test)]
#[path = "sound_effects_tests.rs"]
mod sound_effects_tests;

impl SoundPlay {
    /// Sound event `identifier` at the listener, not fading with distance, on the interface
    /// source at volume and pitch 1 (`SimpleSoundInstance.forUI`).
    pub fn ui(identifier: &str) -> Self {
        Self {
            identifier: identifier.to_owned(),
            source: SoundSource::Ui,
            volume: 1.0,
            pitch: 1.0,
            position: [0.0; 3],
            looping: false,
            delay: 0,
            attenuation: SoundAttenuation::None,
            relative: true,
            effects: SoundEffects::NONE,
        }
    }
    /// Sound event `identifier` at `position` on `source`, fading with distance, at volume
    /// and pitch 1 (`SimpleSoundInstance`'s positioned constructor).
    pub fn at(identifier: &str, source: SoundSource, position: [f64; 3]) -> Self {
        Self {
            identifier: identifier.to_owned(),
            source,
            volume: 1.0,
            pitch: 1.0,
            position,
            looping: false,
            delay: 0,
            attenuation: SoundAttenuation::Linear,
            relative: false,
            effects: SoundEffects::NONE,
        }
    }
    /// The same sound at `volume`.
    pub fn with_volume(self, volume: f32) -> Self {
        Self { volume, ..self }
    }
    /// The same sound at `pitch`.
    pub fn with_pitch(self, pitch: f32) -> Self {
        Self { pitch, ..self }
    }
    /// The same sound shaped by `effects`.
    pub fn with_effects(self, effects: SoundEffects) -> Self {
        Self { effects, ..self }
    }
    /// The same sound looping, `delay` ticks between its repeats.
    pub fn looping(self, delay: i32) -> Self {
        Self {
            looping: true,
            delay,
            ..self
        }
    }
}

/// A sound a client mod started with `play_local_sound`, to follow and control: `state`, `stop`
/// and `fade`. Each handle names one play and is never reused. Once the sound has ended, been
/// stopped or been cleared (leaving or changing levels stops every sound), the handle is stale: it
/// reads [`SoundState::Stopped`] and its writes do nothing. It stays valid across a change of
/// output device, which keeps the sounds playing.
///
/// Dropping or overwriting the handle doesn't stop the sound: keep a looping sound's handle while
/// it plays (stop it before starting another in its place), or nothing can stop it until the
/// player leaves the level.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its field is private, so mods cannot build or destructure it
pub struct SoundRef {
    pub(crate) id: u64,
}

/// Where a mod's sound is ([`SoundRef`]'s `state`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum SoundState {
    /// Not started yet: queued for the next audio update, waiting out `play_local_sound`'s
    /// `after` ticks, or between a looping sound's repeats (`SoundPlay::looping`'s `delay`).
    Waiting,
    /// Playing (`SoundEngine.isActive`).
    Playing,
    /// Over: it ended, was stopped, faded to 0 or was cleared. It never plays again.
    Stopped,
}

// no wire: dispatch result retains a static owning-mod identity; not an owned transport record.
/// A replace hook's answer, or a collect hook's entry, with the mod that gave it: the dispatch
/// fills in `mod_id` (the answering mod's `[mod] id`), so no mod can answer for another.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Answered<T> {
    /// The replace hook's answer, or the collect hook's entry.
    pub answer: T,
    /// The `[mod] id` of the mod that answered.
    pub mod_id: &'static str,
}

/// What a mod answers to `screen_opening` (`ScreenEvent.Opening`): the screen
/// that was about to open does not. The first mod to answer wins.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ScreenOpening {
    /// `Opening.setCanceled(true)`: the current screen stays open.
    Cancel,
    /// `Opening.setNewScreen(..)`: the answering mod's own screen
    /// (`Host::mod_screen_open`) opens instead, if it can open there (over the
    /// world or the title's screens, not over a book, a prompt or while a
    /// world loads; the screen that was about to open does then). Over the
    /// title, closing it returns to the title screen.
    ModScreen,
    /// `Opening.setNewScreen(..)` with a vanilla screen: `target` opens
    /// instead, as it opens on its own. A target that can't open here (a
    /// title screen in a world, a world's screen on the title or while the
    /// world loads) keeps the screen that was about to open, and the game
    /// logs it.
    Vanilla(ScreenTarget),
}

/// A vanilla screen a mod's `screen_opening` answer opens in place of the one
/// asked for (`ScreenOpening::Vanilla`): the screens that open with no
/// arguments, named after the `ScreenView.id` they open with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ScreenTarget {
    /// In a world: the pause menu (`PauseScreen`), `paused`.
    Pause,
    /// In a world: the options (`OptionsScreen`), `options`.
    Options,
    /// In a world: the player's inventory (`InventoryScreen`; the creative
    /// inventory in creative mode), `inventory` or `creative`.
    Inventory,
    /// In a world: the advancements (`AdvancementsScreen`), `advancements`.
    Advancements,
    /// In a world: the statistics (`StatsScreen`), `stats`.
    Stats,
    /// In a world: an empty chat box (`ChatScreen`), `chat`.
    Chat,
    /// On the title: the title screen itself (`TitleScreen`), `title`.
    Title,
    /// On the title: the world list (`SelectWorldScreen`; Create New World
    /// when there is no world), `world_select`.
    WorldSelect,
    /// On the title: Create New World (`CreateWorldScreen`), `world_create`.
    WorldCreate,
    /// On the title: the server list (`JoinMultiplayerScreen`), `multiplayer`.
    Multiplayer,
    /// On the title: Add Server (`EditServerScreen` for a new server),
    /// `add_server`; it returns to the server list.
    AddServer,
    /// On the title: Direct Connection (`DirectJoinServerScreen`),
    /// `direct_connect`; it returns to the server list.
    DirectConnect,
}

// no wire: toggle geometry is chosen by the client screen factory.
/// Where a mod menu's recipe-book toggle sits: `x` GUI pixels right of the menu's left edge and
/// `y` below the line `from` names. Vanilla screens place theirs either way:
/// `leftPos + 5, topPos + 34` is `x: 5.0, y: 34.0, from: Menu`, and
/// `leftPos + 5, height / 2 - 49` is `x: 5.0, y: -49.0, from: ScreenCentre`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
// exhaustive: mods build it with a literal and the game reads every field
pub struct RecipeBookToggle {
    /// Pixels right of the menu's left edge.
    pub x: f32,
    /// Pixels below the line `from` names; negative above it.
    pub y: f32,
    /// What `y` counts from.
    pub from: ToggleOrigin,
}

// no wire: the toggle's coordinate origin is used only by client layout.
/// The line a recipe-book toggle's `y` counts from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// exhaustive: the two lines vanilla's screens measure from, matched without a wildcard
pub enum ToggleOrigin {
    /// The menu's top edge.
    #[default]
    Menu,
    /// The window's vertical centre, as vanilla's `height / 2` (integer division, so an odd
    /// window height rounds down).
    ScreenCentre,
}

/// The stable identifier reported by screen hooks for an observed screen.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ScreenId {
    /// The same identifier as [`ScreenView::id`], such as `inventory` or `paused`.
    pub id: String,
}

/// The sign the sign editor is on (`ScreensClientHost::sign_editor`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct SignEditView {
    /// The block position of the sign being edited.
    pub pos: BlockPos,
    /// Whether the side being edited is the sign's front (`true`) or back (`false`).
    pub front: bool,
}

/// What a [`ScreenRegion`] is: one of the parts vanilla 26.3 draws outside a container screen's panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ScreenRegionKind {
    /// The open recipe book's panel (`RecipeBookComponent`, 147 x 166, left of the container's window).
    RecipeBook,
    /// The open recipe book's column of category tabs (`RecipeBookTabButton`s at `xPosTab`, 35 x 27 each, 27 apart),
    /// left of its panel; as tall as the tabs the book shows.
    RecipeBookTabs,
    /// One tab of the creative inventory (`CreativeModeInventoryScreen.extractTabButton`, 26 x 32),
    /// above or below its panel: one region per tab drawn.
    CreativeTab,
    /// The active effects beside the inventory (`EffectsInInventory`), as wide and tall as they draw (compact or
    /// full rows), after `render_inventory_mob_effects` moved or resized them.
    Effects,
}

/// A region of the open screen that the game itself draws outside its container panel ([`ScreenView::rect`]), in
/// GUI pixels from the window's top left, as `[x, y, width, height]`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ScreenRegion {
    /// What the game draws there.
    pub kind: ScreenRegionKind,
    /// Where: `[x, y, width, height]` in GUI pixels.
    pub rect: [f32; 4],
}

impl ScreenRegion {
    /// A region of `kind` at `rect` (`[x, y, width, height]` in GUI pixels).
    pub const fn new(kind: ScreenRegionKind, rect: [f32; 4]) -> Self {
        Self { kind, rect }
    }
}

// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// The open screen, for the screen hooks (`screen_render_post`, the
/// mouse ones `screen_mouse_button_pressed_pre`, `..._released_pre` and
/// `screen_mouse_scrolled_pre`, and the key ones
/// `screen_key_pressed_pre` and `screen_char_typed_pre`): what
/// it is, its menu and title, where its container panel is, how many slots its
/// menu has, and where the mouse and the frame's partial tick are.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ScreenView<'a> {
    /// One id per `Screen` (`Screen::id`): `inventory`, `creative`, `paused`,
    /// `stats`, `options`, `win`… Stable across releases, so a mod can match
    /// on it. Every container, station and mod window draws on the inventory
    /// screen and so reports `inventory`: tell them apart by `menu` and
    /// `title` (both None on a window the client opened itself) and by
    /// `slots`. The title's own screens report theirs: `title`,
    /// `world_select`, `world_create`, `world_create_flat`, `world_edit`,
    /// `multiplayer`, `add_server`, `direct_connect`…, with no menu, no title
    /// and no slots.
    pub id: &'static str,
    /// The type id of the menu the server opened for this screen
    /// (`minecraft:generic_9x3`, `minecraft:furnace`, a mod's own id; a mount's
    /// inventory is `sunrise:mount<n>`). None on a screen without one (the
    /// client's own inventory, creative, the pause menu, stats…) and for a
    /// menu type the client does not know.
    pub menu: Option<&'a str>,
    /// The title the server gave the open menu, if it has one: the component
    /// itself, borrowed as an owned API component tree.
    /// None when the server gave none, on a screen the client opened, and
    /// while playing.
    pub title: Option<&'a Text>,
    /// The screen's container panel in GUI pixels, `[x, y, width, height]`;
    /// the whole window for a screen with none. The panel is where the game
    /// draws the window: 176 wide for most, wider for the merchant and the
    /// beacon, a mod screen's own size for a mod's.
    pub rect: [f32; 4],
    /// The slots the open menu has, as `Game::slots` counts them: the player
    /// inventory's 36, then the container's, the station's, the mod screen's
    /// or the crafting grid's (a screen with no menu of its own still counts
    /// the inventory menu's). Counted, not listed.
    pub slots: usize,
    /// The pointer in GUI pixels (`Screen.mouseX`, `mouseY`, from the
    /// window's top left), as `cursor` gives it: for hover highlights.
    pub mouse: [f32; 2],
    /// This frame's partial tick (`getGameTimeDeltaPartialTick(false)`, 0 to
    /// 1): what the screen draws animation against.
    pub partial_tick: f32,
    /// What the game draws outside `rect`: the book panel and tabs, creative tabs,
    /// and inventory effects. Render.Post sees the current draw, or an empty slice if
    /// Pre canceled it. Input and the earlier render phases see the last published draw
    /// for the screen. Opening, closing, init, screen room and the inventory-effects
    /// layout hook see an empty slice. Hidden regions and areas drawn by other mods
    /// are not included.
    pub regions: &'a [ScreenRegion],
}

/// A camera-local visual light, with no gameplay light or shadow writes.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ColoredLight {
    /// The light's position in blocks.
    pub at: [f64; 3],
    /// The colour as `0xRRGGBB`.
    pub rgb: u32,
    /// How far it reaches, in blocks.
    pub radius: f32,
    /// The light's strength.
    pub intensity: f32,
}

impl ColoredLight {
    /// A light at `at` in `rgb`, reaching `radius` blocks at strength `intensity`.
    pub const fn new(at: [f64; 3], rgb: u32, radius: f32, intensity: f32) -> Self {
        Self {
            at,
            rgb,
            radius,
            intensity,
        }
    }
}

// no wire: screen geometry is constructed and consumed in the client window.
/// Layout for a window's [`crate::MenuScreen`]: its size, where
/// each of the container's slots is (`Slot.x`, `Slot.y`), and the player
/// inventory's first row (vanilla's is 84; the hotbar is 58 below it) and
/// left column, from its top left. The screen controls client presentation
/// and per-slot active/visibility state (`Slot.isActive()`); server-side
/// placement and pickup rules belong to the menu and its `MenuPart`.
#[derive(Clone, Debug, Default, PartialEq)]
// exhaustive: mods build it with a literal and the game reads every field
pub struct MenuScreenLayout {
    /// Positive finite width in GUI pixels. Vanilla's container screens are 176 wide.
    pub width: f32,
    /// Positive finite height in GUI pixels; vanilla's is 166.
    pub height: f32,
    /// Slot geometry at its original backing index. `Some(slot)` shows the slot
    /// initially; `None` (including omitted trailing entries) marks an initially
    /// inactive/hidden slot (`Slot.isActive()`) that can be activated later with
    /// [`crate::prelude::ScreensClientHost::set_screen_slot`]. Don't compact this
    /// list; there are no entries past the backing.
    pub slots: Vec<Option<ScreenSlot>>,
    /// The y of the player inventory's first row; vanilla's is 84.
    pub inventory_y: f32,
    /// The inventory's left column (vanilla's is 8).
    pub inventory_x: f32,
    /// Title label position in GUI pixels from the screen's top-left corner.
    /// `None` uses vanilla's `[8, 6]`. Ignored when `own_labels` is true.
    pub title_label: Option<[f32; 2]>,
    /// Inventory label position in GUI pixels from the screen's top-left corner.
    /// `None` uses `[8, height - 94]`, independently of inventory slot positions.
    /// Ignored when `own_labels` is true.
    pub inventory_label: Option<[f32; 2]>,
    /// Sunrise leaves the title and inventory label for the screen's draw callback.
    pub own_labels: bool,
    /// Leave the background to the screen's draw callback, omitting the
    /// default panel and slot frames.
    pub own_background: bool,
    /// The registered recipe book hosted beside this menu. Its declarations must
    /// target this menu; no panel is enabled until authoritative settings arrive.
    /// If the menu declares a book with `MenusRegistrar::menu_recipe_book`, this
    /// field must name that book. Otherwise the book is optional and display-only.
    pub recipe_book: Option<crate::RecipeBookTypeKey>,
    /// Optional 20 × 18 recipe-book toggle position, from this menu's left and, per
    /// [`RecipeBookToggle::from`], its top or the window's centre.
    /// The screen author reserves this area, as vanilla screen templates do.
    /// Without an anchor the engine uses an external gutter; if none fits,
    /// the book is refused with a diagnostic while the menu remains usable.
    pub recipe_book_toggle: Option<RecipeBookToggle>,
    /// The recipe book's result ghost gets a 24 × 24 frame, vanilla's
    /// `AbstractRecipeBookScreen.isBiggerResultSlot` for a crafting-table-sized result. Off by
    /// default.
    pub recipe_book_big_result: bool,
}

// no wire: a slot's coordinates and empty icon are local screen presentation.
/// One visible container slot, including vanilla's `Slot.getNoItemIcon`.
/// Its index in [`MenuScreenLayout::slots`] is the menu's backing index.
#[derive(Clone, Debug, Default, PartialEq)]
// exhaustive: mods declare both geometry and the optional empty-slot sprite
pub struct ScreenSlot {
    /// Top-left in GUI pixels relative to the screen.
    pub position: [f32; 2],
    /// GUI sprite identifier, such as `minecraft:container/slot/ingot`.
    /// Drawn at 16 × 16 only when the displayed stack is empty, including
    /// quick-craft previews. Unknown sprites draw the missing texture,
    /// matching vanilla's `Slot.getNoItemIcon`.
    pub no_item_icon: Option<String>,
}

/// An item the panel shows (`panel_items`): its id
/// (`minecraft:diamond`), a count (1 draws none) and its components beyond
/// the item's own, in vanilla's item syntax, or "".
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct Stack {
    /// The item identity, resolved once at load.
    pub item: ItemKey,
    /// How many are in the stack.
    pub count: u32,
    /// The stack's data components, as the game's JSON form.
    pub components: String,
}

/// How a mod's item panel (`panel_items`) looks (`panel_style`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum PanelStyle {
    /// The recipe book's page arrows and a plain search box (the default).
    #[default]
    RecipeBook,
    /// A recipe viewer's: square page buttons on a bar over the items and
    /// the viewer's search box, as Sunrise's Recipes panel draws them.
    Viewer,
    /// JEI's look on Minecraft 1.12: the items flush with the screen's right
    /// edge under a row of square page buttons that always shows (they wrap
    /// around), the viewer's search box at the foot of the screen with a
    /// settings button beside it that opens the mod's page in Options > Mods,
    /// and a whole slot lit under the pointer.
    Classic {
        /// The search box under the window when there is room below it (JEI's
        /// "center search bar"), else under the items.
        centred_search: bool,
        /// Lay the panel out beside a window at least this wide, centred on
        /// the screen (a recipe screen the mod draws over the container);
        /// 0 for the container's own.
        window_width: u16,
    },
}

/// What the player did on a native mod's panel (`panel_event`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum PanelEvent {
    /// A click on an item of the panel, by its index in
    /// `panel_items`' list.
    Clicked {
        /// The item's index in the panel's list.
        index: u32,
        /// Whether it was a right click.
        right: bool,
    },
    /// The search box's text, each time it changes: the player's edit, or
    /// the owner's own `panel_search` once its box draws. The same event for
    /// both, so the filter changes in one place.
    Search(String),
    /// A physical key (`key.keyboard.r`) went down
    /// while the pointer was on an item: a slot of the screen's, or the
    /// panel's.
    Key {
        /// The key's name, such as `key.keyboard.r`.
        key: String,
        /// The item the pointer was on.
        stack: Stack,
    },
    /// A click on a slot of a recipe the panel shows (`panel_recipes`): the
    /// recipe's index in the page's `recipes` and the slot's in it (the results
    /// come after the slots), and the stack it showed then.
    SlotClicked {
        /// The recipe's index in the panel's list.
        recipe: u32,
        /// The slot's index in the recipe; results come after the slots.
        slot: u32,
        /// The stack the slot showed then.
        stack: Stack,
        /// Whether it was a right click.
        right: bool,
    },
    /// The action button of the recipe at this index of the `panel_recipes`
    /// page.
    Action(u32),
    /// The player turned the `panel_recipes` page: the index of the first
    /// recipe now shown.
    Page(u32),
    /// The `panel_recipes` page's back button; the mod decides what follows
    /// (`panel_recipes(mod_id, None)` shows the items again).
    Back,
    /// The player picked the tab at this index of the `panel_recipes` page's
    /// `tabs`.
    Tab(u32),
}

/// The zoom a native mod asks for (`zoom`): the factor (1 is none), eased through the
/// field of view like the spyglass when `smooth`, and the scale of mouse turning.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct Zoom {
    /// The zoom as a multiple of the normal view; 1 is no zoom.
    pub factor: f32,
    /// Whether the camera eases into the zoom.
    pub smooth: bool,
    /// Scale applied to mouse turning while zoomed.
    pub turn: f32,
}

/// `ParticleStatus`: how many particles a client shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ParticleStatus {
    /// All particles.
    All,
    /// Fewer particles.
    Decreased,
    /// Only the essential particles.
    Minimal,
}

/// The client's ordinary crosshair hit (`Minecraft.hitResult`), within the
/// current interaction ranges. Entity handles belong to the current client
/// level's epoch, including dragon parts; a replacement level invalidates them.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum CrosshairTarget {
    /// The crosshair is on a block.
    Block {
        /// The block's position.
        pos: BlockPos,
        /// The block's state.
        state: BlockState,
        /// Where on the block the ray hit.
        hit: BlockHit,
    },
    /// The crosshair is on an entity.
    Entity {
        /// The entity under the crosshair.
        entity: EntityRef,
        /// The hit point in blocks.
        point: [f64; 3],
    },
}

/// A custom boss bar (vanilla's `CustomBossEvent`, the `/bossbar` ones) of an
/// open server world. It holds the world's epoch and the bar's UUID, so a bar
/// that was removed (even if its id was added again) or a handle from an
/// earlier open reads as `None` and changes nothing. Mob and raid bars are not
/// custom boss events and have no handle.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct BossBarRef {
    pub(crate) epoch: WorldEpoch,
    pub(crate) uuid: u128,
}

/// `BossEvent.BossBarColor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum BossBarColor {
    /// A pink bar.
    Pink,
    /// A blue bar.
    Blue,
    /// A red bar.
    Red,
    /// A green bar.
    Green,
    /// A yellow bar.
    Yellow,
    /// A purple bar.
    Purple,
    /// A white bar.
    White,
}

impl BossBarColor {
    /// Every color, in ordinal order.
    pub const ALL: [Self; 7] = [
        Self::Pink,
        Self::Blue,
        Self::Red,
        Self::Green,
        Self::Yellow,
        Self::Purple,
        Self::White,
    ];
    /// `BossBarColor.getName`: `pink`, `blue`, `red`, `green`, `yellow`,
    /// `purple` or `white`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Pink => "pink",
            Self::Blue => "blue",
            Self::Red => "red",
            Self::Green => "green",
            Self::Yellow => "yellow",
            Self::Purple => "purple",
            Self::White => "white",
        }
    }
    /// The color with that name, or None for one vanilla doesn't have.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|color| color.name() == name)
    }
}

/// `BossEvent.BossBarOverlay`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum BossBarOverlay {
    /// Progress.
    Progress,
    /// Notched6.
    Notched6,
    /// Notched10.
    Notched10,
    /// Notched12.
    Notched12,
    /// Notched20.
    Notched20,
}

impl BossBarOverlay {
    /// Every overlay, in ordinal order.
    pub const ALL: [Self; 5] = [
        Self::Progress,
        Self::Notched6,
        Self::Notched10,
        Self::Notched12,
        Self::Notched20,
    ];
    /// `BossBarOverlay.getName`: `progress`, `notched_6`, `notched_10`,
    /// `notched_12` or `notched_20`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Progress => "progress",
            Self::Notched6 => "notched_6",
            Self::Notched10 => "notched_10",
            Self::Notched12 => "notched_12",
            Self::Notched20 => "notched_20",
        }
    }
    /// The overlay with that name, or None for one vanilla doesn't have.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|overlay| overlay.name() == name)
    }
}

/// One boss bar the client shows (vanilla's `LerpingBossEvent`), as
/// `customize_boss_event_progress` reads it. Any bar the server sends: mob,
/// raid and `/bossbar` ones alike.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientBossBar {
    /// The bar's UUID, as the server's boss event packets name it.
    pub uuid: u128,
    /// The title drawn above the bar.
    pub name: Text,
    /// The fill drawn this frame, 0 to 1 (`getProgress`, eased toward the
    /// server's value).
    pub progress: f32,
    /// The bar's color.
    pub color: BossBarColor,
    /// The notches drawn over it.
    pub overlay: BossBarOverlay,
    /// Whether it darkens the sky (`shouldDarkenScreen`).
    pub darken_screen: bool,
    /// Whether it plays the boss music (`shouldPlayBossMusic`).
    pub play_boss_music: bool,
    /// Whether it thickens the fog (`shouldCreateWorldFog`).
    pub create_world_fog: bool,
}

/// One player's open menu, invalidated on close, replacement or reconnect.
/// Client menu handles are also invalidated on respawn.
///
/// Clicks require a mutable client host, not a server or a read-only hook:
/// ```compile_fail
/// use mod_api::{MenuRef, ServerHost};
/// fn wrong(menu: MenuRef, host: &mut dyn ServerHost) {
///     menu.click(host, &[]);
/// }
/// ```
/// ```compile_fail
/// use mod_api::{MenuRef, ClientHost};
/// fn wrong(menu: MenuRef, host: &dyn ClientHost) {
///     menu.click(host, &[]);
/// }
/// ```
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct MenuRef {
    pub(crate) player: PlayerRef,
    pub(crate) window: i32,
    pub(crate) generation: u64,
}
