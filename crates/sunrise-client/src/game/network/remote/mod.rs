//! Multiplayer: the game driven by a server connection instead of the local
//! simulation. Mirrors vanilla's ClientPacketListener (chunks, entities,
//! inventory, health, time, weather, chat, respawn and dimension changes),
//! LocalPlayer.sendPosition (movement packets) and MultiPlayerGameMode
//! (block breaking with sequence numbers, use-item, attacks, container
//! clicks). The player still moves with the local physics, as the vanilla
//! client predicts its own movement; the server stays authoritative for
//! everything else.
mod selector_metadata;
use crate::game::collision_source::Mover;
use selector_metadata::hurt_sound_for;
mod attack_target;
mod break_prediction;
use break_prediction::Prediction;
mod connection;
mod fall_sound;
pub(in crate::game) mod general;
pub(in crate::game) use general::RemoteGeneral;
mod interaction_actions;
mod interaction_prediction;
use inventory::{
    CREATIVE_SYNCED, Predicted, changed_slots, creative_sync_slot, creative_sync_stacks,
    mark_synced, menu_items, predict_clicks, predict_clicks_with,
};
mod level_format;
mod raw_ingress;
mod recipe_properties;
mod use_item;

mod block_entity_data;
#[cfg(test)]
mod block_entity_data_tests;
#[cfg(test)]
mod claimed_item_use_tests;
use super::raw_backlog::Received;
use super::remote_chunks::{ChunkDecoder, ChunkFormat, Next};
use super::remote_vehicles::PassengerSeat;
use super::render_tick::{mob_tracking_body, player_tracking_body};
use super::{Dim, Game, Hit, Mining, Screen, item_use::ticks_left, level_sounds::LevelSound};
use crate::game::api_values::TryIntoApi;
use mod_api::InteractionKey;
use movement::MovePacket;
#[cfg(test)]
// The move-packet tests reach it through `super::super`.
use movement::stepped_move;
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
use sunrise_client::first_person::{Hand, SwingAnimation};
<<<<<<< sync-sunrise-1
use sunrise_client::gamepad::ServerStick;
use sunrise_client::input_queue::InputQueue;
=======
>>>>>>> sunrise/main
use sunrise_client::{
    Result,
    blocks::Fluid,
    chunk::Chunk,
    containers::{BlockEntity, Furnace, FurnaceKind},
    entities::{Experience, ExperienceOrb, ItemEntity},
    hud::HudBatch,
    items::ItemStack,
    join_seq::JoinSeq,
    lan::data::{BIOME_REGISTRY, client_biomes},
    menu::Input,
    mobs::{Mob, MobType, Who},
    motion::{MotionWorld, PlayerMotion},
    net::{Event, Session},
    particles::TrackingTarget,
    random_source,
    save::SaveDir,
    screen_effects,
    sprint::Keys,
    streaming::{ChunkGenerator, Proto, Streaming},
    tab_list::SPECTATOR,
    text::Text,
};
use sunrise_core::{geometry::Vec3, random::RandomSource};
use sunrise_protocol::{
    entity_data::{ITEM_ENTITY_ITEM, Value},
    item::{Hashed, Stack},
    packets::{
        Empty,
        play::{self, Clientbound as C, Serverbound as S},
    },
    registry::{Builtin, DimensionType, Dynamic, SyncedNames, ceil_log2},
    rsa::RsaPublicKey,
    secure_chat::{self, Signature},
    text::{Language, plain},
};
use {
    sunrise_core::nbt::Tag,
    sunrise_mod_runtime::attachments::{Holder, ResidentAttachments},
    sunrise_world::{
        level::InteractionResult,
        registry::BlockStateId,
        stepped_interpolation::{Path, PosRot, Start, SteppedInterpolation},
    },
};

mod abilities;
mod chat;
mod chat_received;
mod commands;
pub(in crate::game) use chat::{ChatTrust, Decoration};
use chat_received::QueuedChat;
pub(in crate::game) mod client_level_tick;
mod effect;
mod entity_section_steps;
mod inventory;
mod item_use;
mod kept_columns;
mod menu_interactions;
mod minecarts;
use minecarts::CartLerp;
mod motion;
mod movement;
mod packets_entity;
mod packets_object;
mod packets_player;
mod packets_world;
mod remote_attributes;
mod remote_state;
mod session;
mod skin;
mod status;
mod sync;
mod tab_entries;
mod tables;
mod tick;
mod tracking;
mod ui;
#[path = "use.rs"]
mod r#use;

pub(super) use super::villager_client::builtin_registries;

mod chat_queue;

<<<<<<< sync-sunrise-1
/// `ChatType`'s two decorations: how a message shows in the chat, and what the narrator says of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Decoration {
    Chat,
    Narration,
}

/// `ChatTypeDecoration.decorate` of a chat type's `chat` or `narration`
/// entry: its translation, with the sender, the target or the message as its
/// parameters, in its style. A chat type the registry lacks decorates as
/// `minecraft:chat` (`chat.type.text`, narrated `chat.type.text.narrate`).
fn chat_type_decorate(
    chat_type: Option<&Tag>,
    decoration: Decoration,
    bound: &play::ChatTypeBound,
    content: &serde_json::Value,
) -> serde_json::Value {
    use sunrise_client::scoreboard::tag_to_json;
    let (field, default_key) = match decoration {
        Decoration::Chat => ("chat", "chat.type.text"),
        Decoration::Narration => ("narration", "chat.type.text.narrate"),
    };
    let decoration = chat_type.and_then(|t| t.get(field));
    let key = match decoration.and_then(|d| d.get("translation_key")) {
        Some(Tag::String(s)) => s.to_string_lossy(),
        _ => default_key.into(),
    };
    let parameter = |name: &str| match name {
        "sender" => tag_to_json(&bound.name),
        "target" => bound
            .target
            .as_ref()
            .map_or_else(|| serde_json::Value::from(""), tag_to_json),
        _ => content.clone(),
    };
    let with: Vec<serde_json::Value> = match decoration.and_then(|d| d.get("parameters")) {
        Some(Tag::List(list)) => list
            .iter()
            .map(|p| match p {
                Tag::String(s) => parameter(&s.to_string_lossy()),
                _ => serde_json::Value::from(""),
            })
            .collect(),
        _ => vec![parameter("sender"), parameter("content")],
    };
    let mut component = match decoration.and_then(|d| d.get("style")) {
        Some(style @ Tag::Compound(_)) => tag_to_json(style),
        _ => serde_json::json!({}),
    };
    component["translate"] = key.into();
    component["with"] = with.into();
    component
}

/// A text component the server sent, for the HUD and the chat. Legacy `§` codes in its strings
/// (which plugins still send) style the text as the font's StringDecomposer does.
=======
/// A text component the server sent, for the HUD.
>>>>>>> sunrise/main
pub(super) fn remote_text(tag: &Tag) -> sunrise_client::text::Text {
    use sunrise_client::scoreboard::{comp_text, tag_to_json};
    comp_text(&tag_to_json(tag)).resolve_legacy()
}

/// `Player.getVoicePitch`: a constant 1.0
/// (`javap -p -c -constants net.minecraft.world.entity.player.Player`:
/// `0: fconst_1  1: freturn`), so a player's own hurt and death sounds are
/// neither raised for a baby nor jittered - `LivingEntity.getVoicePitch`, which
/// the mobs port, does both. It takes no random, so it draws nothing from the
/// level random either.
pub(super) fn player_voice_pitch() -> f32 {
    1.0
}

/// The live entity a bound sound names; absence means that object was removed.
fn entity_sound_state(
    id: i32,
    player: Option<(i32, [f64; 3], bool)>,
    mob: Option<sunrise_client::sound_engine::SoundEntityState>,
    tracked: &HashMap<i32, Tracked>,
    dragon: Option<(i32, [f64; 3])>,
) -> Option<sunrise_client::sound_engine::SoundEntityState> {
    use sunrise_client::sound_engine::SoundEntityState;
    if let Some((player_id, position, silent)) = player
        && player_id == id
    {
        return Some(SoundEntityState {
            position,
            silent,
            removed: false,
        });
    }
    let mut state = mob.or_else(|| {
        tracked.get(&id).map(|t| SoundEntityState {
            position: t.position,
            silent: t.silent,
            removed: false,
        })
    })?;
    // The dragon's Mob is metadata storage; only Dragon follows its movement.
    if let Some((dragon_id, position)) = dragon
        && dragon_id == id
    {
        state.position = position;
    }
    Some(state)
}

/// `ClientboundOpenScreenPacket.title` as a container screen draws it: the
/// component's styles, and the `§` codes a literal holds (`StringDecomposer`).
fn menu_title(tag: &Tag) -> sunrise_client::text::Text {
    remote_text(tag).resolve_legacy()
}

/// Streaming needs a generator; a server world never generates.
struct NoTerrain;
impl ChunkGenerator for NoTerrain {
    type Worker = ();
    fn worker(&self) {}
    fn proto(&self, _: &mut (), _: i32, _: i32) -> Proto {
        Box::new(())
    }
    fn finish(&self, _: Proto) -> Chunk {
        Chunk::empty(0, 0)
    }
}

/// A server entity the client tracks (position as last sent).
pub(super) struct Tracked {
    pub(super) kind: String,
    pub(super) position: [f64; 3],
    /// `Entity.getPositionCodec`'s base: the position the server last
    /// reported (AddEntity, MoveEntity, EntityPositionSync), which a move
    /// packet's delta counts from. A TeleportEntity moves the entity but not
    /// this base, so the server's next delta still lands where it means.
    codec_base: [f64; 3],
    on_fire: bool,
    mod_fire_immune: bool,
    silent: bool,
    /// `Entity.vehicle`: the entity it rides (`SetPassengers`), whatever either one's type.
    pub(in crate::game) vehicle: Option<u32>,
}

/// An item has joined the client level, but has no stack to draw yet.
pub(super) struct PendingItem {
    join: JoinSeq,
    position: [f64; 3],
    velocity: Vec3,
}

/// `Entity.startRiding` / `removePassenger` for a `SetPassengers` packet: the
/// listed entities ride `vehicle`, the ones that rode it and are no longer
/// listed got off. Every tracked entity counts, whatever its type, so a
/// server's item display or marker on a player hides that player's name tag
/// as vanilla's `isVehicle` does (#5666).
fn seat_passengers(entities: &mut HashMap<i32, Tracked>, vehicle: u32, passengers: &[i32]) {
    for (id, t) in entities.iter_mut() {
        if passengers.contains(id) {
            t.vehicle = Some(vehicle);
        } else if t.vehicle == Some(vehicle) {
            t.vehicle = None;
        }
    }
}
/// Smooth an entity toward a new position over three ticks (InterpolationHandler).
#[derive(Clone, Copy)]
/// `LinearInterpolationHandler`'s target: a position and rotation (yRot,
/// xRot) reached in `steps` ticks.
pub(super) struct Lerp {
    pub(super) target: [f64; 3],
    rotation: Option<(f32, f32)>,
    pub(super) steps: u32,
}

/// `SteppedInterpolationHandler.doInterpolate`'s `setPos` and `setRot` on a
/// mob: its yaw kept continuous (the same turn as vanilla's, without the
/// wrap that would spin it in the frame between ticks), its pitch clamped.
pub(super) fn place_interpolated(m: &mut Mob, at: PosRot) {
    m.position = Vec3::new(at.position[0], at.position[1], at.position[2]);
    m.yaw += wrap_degrees(at.yaw - m.yaw);
    m.head_pitch = (at.pitch % 360.0).clamp(-90.0, 90.0);
}

/// Where a mob is, as its `SteppedInterpolationHandler` sees it.
fn mob_pos_rot(m: &Mob) -> PosRot {
    PosRot {
        position: [m.position.x, m.position.y, m.position.z],
        yaw: m.yaw,
        pitch: m.head_pitch,
    }
}

/// `Minecraft.services().profileKeySignatureValidator()`: the services' profile-key keys, which a
/// player's chat key must be signed by. Asked for once per connection to a server, off the game
/// thread.
pub(super) enum ServicesKeys {
    Pending,
    Ready(Vec<RsaPublicKey>),
    /// `canValidateProfileKeys()` is false: none could be had.
    Missing,
}
/// The signed-in account's `/minecraft/profile` fetch after joining.
enum OwnProfile {
    Idle,
    Fetching(std::sync::mpsc::Receiver<Option<sunrise_client::skins::Textures>>),
    Done(Option<sunrise_client::skins::Textures>),
}

pub(in crate::game) struct PendingCauldron {
    hand: Hand,
    hit: Hit,
    attempt: mod_api::InteractionAttempt,
    selected: usize,
    main: Option<ItemStack>,
    off: Option<ItemStack>,
    aim: ItemUseAim,
    swing: [bool; 2],
    sequence: i32,
    item_after_block: bool,
}

#[derive(PartialEq)]
struct ItemUseAim {
    // UseItem carries these angles but not the server's ray origin.
    rotation: (f32, f32),
    eye: Option<[f64; 3]>,
    // The local ray can also come from a tracked input device.
    ray: Option<([f64; 3], [f64; 3])>,
    reach: f64,
}

/// The packet route that starts a remote level entry.
pub(in crate::game) enum RemoteDimensionEntry {
    Login,
    Respawn,
    DeathRespawn,
}

pub(super) struct Remote {
    pub(super) professions: Arc<sunrise_client::mobs::villager::ResolvedProfessions>,
    pub session: Session,
    /// Display-only address computed at connection setup. Never holds the raw friend invite.
    pub(super) display_address: Option<String>,
    pub brand: String,
    /// `ClientPacketListener.random` draws packet spread, not provider RNG.
    pub(super) particle_packet_random: sunrise_core::random::LegacyRandom,
    pub(super) recipe_viewer: crate::game::recipe_book::ViewerChannels,
    /// This connection's configured feature flags, carried through reconfiguration.
    pub(super) features: std::sync::Arc<[String]>,
    /// The server announced it runs mods' quiet commands.
    pub quiet_commands: bool,
    /// Starling: the server announced it carries the old mod messages (old_mod_net.rs).
    pub mod_net: bool,
    /// The server advertised private custom recipe-book synchronization.
    pub custom_books: bool,
    /// The synced attachments the server sent, by holder.
    pub attachments: HashMap<Holder, ResidentAttachments>,
    /// The colliders the server sent, by entity id (`sunrise:collider_sync`).
    pub colliders: HashMap<u32, sunrise_client::colliders::ColliderState>,
    /// The colliders at the last client tick, before any packets handled between ticks.
    pub(super) collider_shove_start:
        Option<(Dim, sunrise_client_render::moving_colliders::Prepared)>,
    /// How old the copy of the collider carrying the player is (`sunrise:carry_age`).
    pub carry_age: sunrise_client::colliders::CarryAge,
    /// The server samples far terrain for us, this far at most (`lod_net`).
    pub far_terrain: Option<sunrise_client::lod_net::Offer>,
    pub(super) builtin: Arc<Builtin>,
    lang: Arc<Language>,
    pub(in crate::game) registries: Arc<Dynamic>,
    /// The block, item and fluid tags of `registries` by name, as native mods read them
    /// (`install_remote_tags`).
    pub(super) tags: Option<sunrise_client::items::Items>,
    /// The game's enchantments and the server's that it lacks (a data
    /// pack's), from their synced data; `None` when the server has none.
    pub(super) enchantments: Option<Arc<sunrise_client::enchant::Enchantments>>,
    pub(super) joined: bool,
    /// `ServerReconfigScreen` is up since this tick.
    pub(super) reconfiguring: Option<u64>,
    /// `handleLogin` ran: vanilla's `Minecraft.player` exists from then on,
    /// while here the player is made at the first teleport.
    pub(super) logged_in: bool,
    status: String,
    pub(super) entity_id: i32,
    local_entity_silent: bool,
    /// `Minecraft.cameraEntity` when the server put the view on another
    /// entity (ClientboundSetCameraPacket); None while it is the player.
    pub(super) camera: Option<i32>,
    /// The item the player's use began with (`LivingEntity.useItem`).
    pub(super) use_item: Option<Arc<str>>,
    pub(super) dimension: DimensionType,
    /// The level's key (`minecraft:overworld`): where its far terrain is kept.
    pub(in crate::game) level: String,
    /// `level`'s key, as mods' handles name the level.
    pub(super) level_key: mod_api::DimensionKey,
    /// The level's facts from the dimension type this connection's server sent (`None` when it
    /// sent none the game takes): owned by the connection, so entering a level touches no other
    /// level's facts, the integrated server's included.
    pub(in crate::game) level_facts: Option<Arc<sunrise_world::dimension_type::DimensionFacts>>,
    /// The sky, fog and cloud attributes (and timelines) of the level from the server's type over
    /// the client's own environment of that type: what gameplay reads. `None` when the type is
    /// refused, or has no data at all: neither the server sent it nor the client's resources hold
    /// it (`fill_known_entries` fills a known pack's entries from the client's own files).
    pub(in crate::game) level_environment: Option<Arc<sunrise_client::environment::Environment>>,
    /// The same over the packs' Polytone stack of that type, what the frame draws (`None` without
    /// pack modifiers, or for a type the packs have no stack of: any but the three vanilla IDs).
    pub(in crate::game) level_pack_environment:
        Option<Arc<sunrise_client::environment::Environment>>,
    /// The level's synchronized dimension type, which both environments are built from again
    /// when the packs change.
    synced_type: Option<(String, serde_json::Value)>,
    /// Both environments by synchronized type name, with what they were built from, so that a
    /// level of an unchanged type is entered again without reading the biome files.
    level_environments: HashMap<String, level_format::LevelEnvironments>,
    /// Connection-owned facts, cached by level and the full named type identity.
    facts_cache: HashMap<
        String,
        (
            String,
            serde_json::Value,
            Arc<sunrise_world::dimension_type::DimensionFacts>,
        ),
    >,
    /// The levels this connection added to the process's table of levels: the only ones it drops.
    added_levels: std::collections::HashSet<String>,
    /// `ClientPlayerNetworkEvent.LoggingOut` was fired for this connection: every way out
    /// of a world reports it once.
    logged_out: bool,
    /// The other levels' worlds the server opened on `sunrise:levels` (`viewed_worlds`).
    pub(in crate::game) viewed: super::viewed_worlds::ViewedWorlds,
    /// The level's hashed seed (login, respawn): the folder of its kept chunks
    /// (`server_terrain::level_dir`).
    pub(super) hashed_seed: i64,
    /// What chunk packets decode with (the dimension's height and sky, the
    /// biome registry); replaced whenever one of them changes.
    pub(in crate::game) chunk_format: Arc<ChunkFormat>,
    /// Decodes chunk packets as they arrive, off the frame.
    decoder: ChunkDecoder,
    /// Options.serverRenderDistance: the server's view distance from the
    /// login or the last chunk-cache-radius packet.
    pub(super) view_distance: i32,
    /// `ClientLevel.serverSimulationDistance`: from the login or the last
    /// simulation-distance packet.
    pub(super) simulation_distance: i32,
    /// `BlockStatePredictionHandler.currentSequenceNr`: the level's, so a new level (a login,
    /// another dimension) counts from 0 again.
    sequence: i32,
    /// `BlockStatePredictionHandler.lastTeleportSequence`: the sequence of the last
    /// teleport, so a prediction made before it keeps its block but not its snap.
    last_teleport_sequence: i32,
    pub(super) position_sender: sunrise_client::player_sync::PositionSender,
    /// When the last tick ended to the server, and the last movement packet went: an old
    /// server behind ViaVersion (its anticheat) wants one every tick, so gaps are logged.
    tick_end_at: Option<Instant>,
    moved_at: Option<Instant>,
    /// When the play phase began (login or the end of a reconfiguration).
    play_began: Option<Instant>,
    /// This second's movement packets and ticks ended, from when: a second over 20 of
    /// either is logged (an old server's anticheat counts them against the time passed).
    rate: (Option<Instant>, u32, u32),
    /// The last movement packets sent: position, on ground, input flags and when. The
    /// first setback logs them, the path an anticheat refused.
    trail: std::collections::VecDeque<([f64; 3], bool, u8, Instant)>,
    /// Free stick direction (experimental, `stick_steer`): this tick's rotation the server sees
    /// in place of the view's, and when the player last hit, used or dropped something.
    steer_yaw: Option<f32>,
    interacted_at: std::cell::Cell<Option<Instant>>,
    /// The look a teleport's acknowledgement told, for `PositionSender::told_rotation`.
    told_rotation: std::cell::Cell<Option<(f32, f32)>>,
    /// Hits and uses waiting for a move with the view's look (`send`).
    held_back: std::cell::RefCell<Vec<S>>,
    /// The stick's mode here this tick ([`ServerStick`]): this session's choice, else the one
    /// remembered for the server (read once), else the setting.
    stick_mode: sunrise_client::gamepad::ServerStick,
    stick_choice: Option<sunrise_client::gamepad::ServerStick>,
    stick_remembered: Option<Option<sunrise_client::gamepad::ServerStick>>,
    /// The setting as last seen here: changed while playing, it becomes the mode here.
    stick_setting: Option<sunrise_client::gamepad::ServerStick>,
    /// Setbacks while the stick moved the player, for asking once a session how it should.
    stick_setbacks: std::collections::VecDeque<Instant>,
    stick_used_at: Option<Instant>,
    stick_asked: bool,
    /// key.swapOffhand presses since the last tick, which sends them (`remote_handle_input`).
    swap_presses: u32,
    last_input: u8,
    was_sprinting: bool,
    /// The slot the server last heard (`MultiPlayerGameMode.carriedIndex`): a cell, so `send` can
    /// send a new one first (`ensureHasSentCarriedItem`).
    carried: std::cell::Cell<Option<usize>>,
    was_shared_flags: u8,
    was_living_flags: u8,
    last_recorded_pose: Option<crate::pose::Pose>,
    /// The last ClientboundPlayerAbilitiesPacket. Vanilla's LocalPlayer
    /// holds its abilities from the login on; here the player is made at
<<<<<<< sync-sunrise-1
    /// the first teleport, which takes them then. None after a respawn
    /// until the server sends them: the game mode's then
    /// (`GameType.updatePlayerAbilities`).
    pub(super) abilities: Option<play::PlayerAbilities>,
    loaded_sent: bool,
    /// Ticks the player has waited for the chunk under it (logged when long: the player
    /// cannot move meanwhile).
    waiting_ticks: u32,
    /// Server teleports of the player that took it back a short way (an anti-cheat's or the
    /// server's correction), logged so a player who "cannot walk" shows why.
    setbacks: u32,
    /// The player's attributes as the server last sent them (ClientboundUpdateAttributesPacket):
    /// movement speed, jump strength, gravity, step height... kept for a LocalPlayer made
    /// after the packet came, and across respawns.
    player_attributes: Vec<play::AttributeSnapshot>,
=======
    /// the first teleport, which takes them then.
    abilities: Option<play::PlayerAbilities>,
    /// The old player's motion, from a Respawn with KEEP_ENTITY_DATA
    /// (`handleRespawn`: `newPlayer.setDeltaMovement(oldPlayer.getDeltaMovement())`),
    /// for the player made at the next teleport: a portal's teleport adds to it.
    respawn_velocity: Option<Vec3>,
    /// ServerboundPlayerLoadedPacket went out: the tick's keybinds act from then on.
    pub(in crate::game) loaded_sent: bool,
>>>>>>> sunrise/main
    /// The dimension of the client level the mods heard made
    /// (`native_client_level_load`), to be told dropped once.
    pub(super) heard_level: Option<String>,
    /// `ClientPacketListener.levelLoadTracker`: the loading screen after a login
    /// or respawn, until the player's section is shown.
    pub(super) level_load: sunrise_client::level_loading::LevelLoadTracker,
    /// The ClientInformation last sent (`Options.broadcastOptions` sends it
    /// again when a view distance, Chat, skin or Main Hand option changes).
    sent_info: sunrise_protocol::packets::configuration::ClientInformation,
    pub(in crate::game) group_count: sunrise_client::chunk_map::ConnectionGroupCount,
    /// BlockStatePredictionHandler: blocks changed ahead of the server (a predicted break),
    /// with what it remembers of each of them. Until the server acknowledges the
    /// sequence its block updates only replace the remembered state; then the block
    /// takes it.
    predictions: HashMap<[i32; 3], Prediction>,
    /// The entities the level held when its `LocalPlayer` was added
    /// (`ClientLevel.addEntity` on login or respawn): `ClientLevel.tickEntities`
    /// ticks in the order entities were added, so these tick before the
    /// player and every later one after it.
    ticked_before_player: std::collections::HashSet<u32>,
    /// Open container: (window id, menu name).
    pub(super) menu: ClientMenu,
    /// The crafting result slot of the open menu as the server last set it
    /// (`CraftingMenu.slotChangedCraftingGrid`): the client never computes it.
    pub(super) craft_result: Option<ItemStack>,
    /// The open menu's title as the server named it, with its styles.
    menu_title: Option<sunrise_client::text::Text>,
    menu_title_snapshot: Option<Arc<mod_api::Text>>,
    pub(in crate::game) state_id: i32,
    /// Window 0's state and carried stack while another menu is active.
    inventory_state_id: i32,
    inventory_carried: Option<ItemStack>,
    inventory_craft_grid: [Option<ItemStack>; 4],
    inventory_craft_result: Option<ItemStack>,
    use_target: Option<[i32; 3]>,
    pub(super) cauldron_entries: Option<sunrise_client::cauldron::interactions::Dispatcher<()>>,
    /// Receipts are connection-scoped; unlike block prediction sequences, they
    /// are never reset on a level change.
    cauldron_receipt: i64,
    pub(in crate::game) pending_cauldrons: std::cell::RefCell<HashMap<i64, PendingCauldron>>,
    /// A mod menu's extra data, kept for the `OpenScreen` of its window that
    /// follows (`sunrise:menu_extra`).
    pub(in crate::game) menu_extra: Option<sunrise_protocol::menu_extra::Message>,
    pub(super) entities: HashMap<i32, Tracked>,
    /// Per-tick remote entity scratch retained between frames.
    tick_fluids: Vec<Option<sunrise_client::mobs::living::Fluids>>,
    tick_rain: Vec<bool>,
    tick_standings: Vec<Option<String>>,
    tick_water_fx: Vec<sunrise_client::mobs::MobEvent>,
    /// Complete network riding graph, including invisible plugin seat entities.
    pub(super) passenger_vehicles: HashMap<u32, PassengerSeat>,
    pub(super) lerps: HashMap<i32, Lerp>,
    /// A native mod's general rows the server shows the window (`general.rs`).
    pub(super) general: general::RemoteGenerals,
    /// The minecarts the server moves with `ClientboundMoveMinecartPacket`
    /// (`NewMinecartBehavior`), which have no interpolation handler.
    pub(super) cart_lerps: HashMap<i32, CartLerp>,
    /// Living entities' `SteppedInterpolationHandler`s.
    pub(super) stepped: HashMap<i32, SteppedInterpolation>,
    /// `LivingEntity.lerpHeadTo`: the head yaw and the ticks left to reach it.
    pub(super) head_lerps: HashMap<i32, (f32, u32)>,
    /// The happy ghast the player just got off, until the server's teleport
    /// puts the player on top of it (`place_ghast_under`).
    pub(super) ghast_dismount: Option<u32>,
    /// Whether the player is the first passenger of its vehicle, the one
    /// `getControllingPassenger` names.
    pub(super) first_passenger: bool,
    /// The player's place in its vehicle's `getPassengers`.
    pub(super) passenger_index: usize,
    /// Item entities waiting for their item data: where they are and their motion.
    pub(super) pending_items: HashMap<i32, PendingItem>,
    health: (f32, i32, f32),
    /// `LocalPlayer.flashOnSetHealth`: set by the first health the server
    /// sends; from then on a drop starts the hurt animation.
    flash_on_set_health: bool,
    game_mode: i32,
    pub chat: Vec<(String, Instant)>,
    /// Lines received since joining, for the control socket's events.
    pub chat_received: u64,
    /// Entity.DATA_CUSTOM_NAME and DATA_CUSTOM_NAME_VISIBLE as the server set them, by entity
    /// (holograms are armor stands with a visible name).
    pub(super) names: HashMap<i32, (Option<sunrise_client::text::Text>, bool)>,
    /// `ChatListener.delayedMessageQueue`: player chat held back by Chat
    /// Delay, with the signature a deletion names it by and what the
    /// narrator says once it shows.
    chat_queue: VecDeque<QueuedChat>,
    /// `ClientPacketListener.messageSignatureCache`: the signatures packed
    /// ones index, as the server's copy for this connection holds them.
    pub(super) signatures: sunrise_protocol::secure_chat::SignatureCache,
    /// `ChatListener.previousMessageTime`: when a message last showed, if one has.
    chat_previous: Option<Instant>,
    /// `ClientPacketListener.nextChatIndex`: the global index the next chat packet must carry.
    next_chat_index: i32,
    /// `serverEnforcesSecureChat`, from the login packet.
    enforces_secure_chat: bool,
    services_keys: Arc<std::sync::Mutex<ServicesKeys>>,
    /// The `RemoteChatSession.Data` each player's list entry last carried (INITIALIZE_CHAT).
    chat_sessions: HashMap<u128, Option<(u128, i64, Vec<u8>, Vec<u8>)>>,
    /// `PlayerInfo.messageValidator` of the players whose chat has been read.
    chat_validators: HashMap<u128, secure_chat::MessageValidator>,
    /// `ConnectScreen.lastNarration`: when "Joining the world" was last said.
    joining_narrated: Option<Instant>,
    /// The tree the chat colours, completes and parses with: the server's
    /// (ClientboundCommandsPacket) once it has sent one, else the game's own,
    /// plus the commands native mods own (`chat::chat_tree_for`). Rebuilt where
    /// that base changes, not at each keystroke.
    pub chat_tree: sunrise_client::command::Dispatcher,
    selector_metadata_supported: bool,
    /// Whether a `ClientboundCommandsPacket` has supplied the server's tree.
    /// The local fallback is for UI completion only; vanilla signs command
    /// arguments against an empty tree until the server sends one.
    pub(super) chat_tree_received: bool,
    /// The chat session, the last-seen window and the signature cache.
    pub(super) secure_chat: super::remote_chat::SecureChat,
    /// `ClientSuggestionProvider.customCompletionSuggestions`: the extra chat
    /// completions this server sent (ClientboundCustomChatCompletionsPacket).
    custom_completions: std::collections::BTreeSet<String>,
    players: HashMap<u128, String>,
    /// `ClientPacketListener.playerInfoMap` as player-list entries, and which of them are listed.
    tab_players: HashMap<u128, sunrise_client::tab_list::TabPlayer>,
    /// Each listed profile's unpacked `textures` property.
    skin_textures: HashMap<u128, sunrise_client::skins::Textures>,
    /// Active skin and cape from the services profile when the server sent none.
    own_profile: OwnProfile,
    /// The skin texture each profile's players show, once its download is ready.
    skins: HashMap<u128, String>,
    skins_revision: u64,
    published_skins: Option<(usize, u128, u64, u64)>,
    listed: std::collections::HashSet<u128>,
    /// `ClientboundLoginPacket.onlineMode`: the player list shows heads.
    pub(super) online_mode: bool,
    /// `ClientboundLoginPacket.levels`: the server's level keys, which
    /// `ClientPacketListener.levels()` hands to command suggestions.
    pub(super) levels: Vec<String>,
    /// Player entities in the level: entity id to profile UUID.
    pub(super) level_players: HashMap<i32, u128>,
    /// The profile name each player entity was created with (`RemotePlayer.getGameProfile`): it outlives the
    /// player-list entry, which servers remove to hide their NPCs from the list.
    level_player_names: HashMap<i32, String>,
    /// The `textures` each player entity found in the player list first
    /// (`AbstractClientPlayer.getPlayerInfo` keeps that `PlayerInfo`): its skin
    /// outlives the entry, which NPC plugins remove a moment after spawning.
    level_player_textures: HashMap<i32, sunrise_client::skins::Textures>,
    /// Entities with shared flag 6 (`Entity.isCurrentlyGlowing` on a client); mobs keep
    /// theirs in `Mob::glowing`.
    pub(super) glowing: std::collections::HashSet<i32>,
    /// The player's own shared flag 5 (`Entity.isInvisible`) as the server last set it.
    pub(super) own_invisible: bool,
    /// The player's own shared flag 0 (`Entity.isOnFire`) as the server last
    /// set it: its fire (blaze fireballs, fire aspect, lightning) never
    /// reaches the window's own `fire_ticks` otherwise.
    own_on_fire: bool,
    /// `LivingEntity.DATA_ARROW_COUNT_ID` and `DATA_STINGER_COUNT_ID` of the
    /// own player, as the server synced them.
    pub(super) own_arrows: i32,
    pub(super) own_stingers: i32,
    /// `Player.DATA_SHOULDER_PARROT_LEFT` and `_RIGHT` of the own player.
    pub(super) own_shoulders: [Option<i32>; 2],
    /// The local player's profile UUID (always in the level).
    pub(super) profile: u128,
    /// The local player's profile name: its score holder and team member.
    pub(super) profile_name: String,
    /// `MultiPlayerGameMode.destroyBlockPos`, `.destroyProgress` and the face last sent: the block
    /// the break targets, which outlives the break itself.
    destroying: Option<([i32; 3], u8, f32)>,
    /// `MultiPlayerGameMode.destroyingItem`: the main-hand stack the break began with.
    destroying_item: Option<ItemStack>,
    /// `MultiPlayerGameMode.isDestroying`: a break is running. `destroying` keeps the target after
    /// one ends (`destroyBlockPos` is written only when a break starts), so a dig resumed on the
    /// same block and item sends no new START and the server keeps its clock (#4124).
    is_destroying: bool,
    /// Per-hand swing policy, restored when the current input attempt ends.
    pub(in crate::game) interaction_swing: [bool; 2],
    destroy_delay: u32,
    /// Whether the main and off hand still wait for the server's answer to
    /// a SWAP_ITEM_WITH_OFFHAND (`remote_hand_slot_set`).
    swap_answer: [bool; 2],
    stop_mining: bool,
    /// `CommonPlayerSpawnInfo.isFlat`: the level is a superflat one.
    pub(super) flat: bool,
    death_message: Option<sunrise_client::text::Text>,
    /// `LocalPlayer.showDeathScreen`: off with `immediate_respawn`.
    show_death_screen: bool,
    /// `Player.isReducedDebugInfo`: the server's `reduced_debug_info` rule.
    pub(super) reduced_debug_info: bool,
    /// `LocalPlayer.doLimitedCrafting`: whether limited crafting is enabled.
    pub(super) do_limited_crafting: bool,
    player_type: Option<Arc<MobType>>,
    /// Inventory, off hand and armour as last synchronised, for creative-mode
    /// slot updates (`creative_sync_stacks`).
    synced: Vec<Option<ItemStack>>,
    /// While a creative screen is open: the inventory menu's crafting grid, armour and off hand
    /// (slots 1 to 8 and 45) as last sent or received, which its clicks send when they change.
    creative_menu: Option<[Option<ItemStack>; 9]>,
    decode_errors: u32,
    /// Connection-owned `ClientClockManager` state, retained across dimensions.
    clocks: HashMap<i32, play::ClockState>,
    default_clock: Option<i32>,
    overworld_clock: Option<i32>,
    clock_game_time: i64,
    /// `ClientChunkCache.Storage`'s view centre (ClientboundSetChunkCacheCenterPacket).
    chunk_center: (i32, i32),
    /// Received events a frame left for the next one, in order, and when
    /// the oldest of them first waited (`poll_remote`).
    backlog: VecDeque<Received>,
    backlog_since: Option<Instant>,
    /// `ClientCommonPacketListenerImpl.serverLinks`: the links "Server
    /// Links..." in the pause menu lists (only `http`/`https` URIs kept).
    pub(super) server_links: Vec<sunrise_protocol::packets::configuration::ServerLink>,
    /// `ClientPacketListener.getDebugQueryHandler`: the F3+I query waiting for the server's data.
    pub(super) debug_query: super::debug_keys::DebugQuery,
}

/// The time a frame spends on chunk packets before it leaves the rest of
/// the received events for the next frame. A batch of 25 to 60 columns
/// (0.1 ms each on an M4 Pro) all in one frame was a 3 to 8 ms frame every
/// time the player crossed a chunk border at 32 chunks; they decode on the
/// chunk decoder's thread now, and the frame only inserts them.
const CHUNK_PACKETS_PER_FRAME: Duration = Duration::from_millis(1);
/// Events that have waited this long are all handled in the next frame,
/// whatever inserting their columns costs, up to a column still decoding:
/// the packets after a slow machine's chunks don't wait on its frames.
const BACKLOG_LIMIT: Duration = Duration::from_millis(50);
<<<<<<< sync-sunrise-1
/// What a frame spends on an overdue backlog's chunk packets on a phone: there one frame
/// taking the whole backlog after a teleport was a visible hitch (a 214 ms frame on the POCO
/// F6 Pro); four times a frame's usual share still drains it in a few frames.
const OVERDUE_CHUNK_PACKETS_PER_FRAME: Duration = Duration::from_millis(4);
/// A gap in the ticks or movement sent to the server worth a log line (an old server's
/// anticheat behind ViaVersion counts on one each tick, and a reminder every 20).
const GAP_LOGGED: Duration = Duration::from_millis(1100);

/// A movement packet in the log, when it came late or was the first. Apart from
/// `remote_send_state`: horizon-mixins refuses to hook a function that calls `log::info!`
/// (E_UNSUPPORTED_MACRO).
fn log_movement_gap(moved_at: &mut Option<Instant>, play_began: Option<Instant>) {
    let now = Instant::now();
    match moved_at.replace(now).map(|at| now - at) {
        Some(gap) if gap >= GAP_LOGGED => log::info!(
            "Sunrise: {} ms between two movement packets",
            gap.as_millis()
        ),
        Some(_) => {}
        None => log::info!(
            "Sunrise: the first movement packet {} ms after the play phase began",
            play_began.map_or(0, |at| (now - at).as_millis())
        ),
    }
}
/// How many movement packets the first setback's log line shows.
const TRAIL: usize = 12;
/// Free stick direction rests this long after a hit, a use or a drop: those go with the view's
/// rotation, and turning away at once after a hit looks like an aimbot to an anticheat.
const STEER_REST: Duration = Duration::from_secs(1);
=======
/// How long a tick about to run waits for one chunk column still decoding;
/// past it the column, and what came after it, wait for the next frame.
const TICK_DECODE_WAIT: Duration = Duration::from_millis(250);
>>>>>>> sunrise/main
/// The open menu name of a server's horse inventory, then its chest's
/// columns (the packet names no `MenuType`).
const MOUNT_MENU: &str = "sunrise:mount";
/// ServerboundPlayerInputPacket's shift bit.
const SHIFT_INPUT: u8 = 1 << 5;

impl Remote {
    /// Count movement packets and ticks ended; a second with more than 20 of either is logged.
    fn count_rate(&mut self, moves: u32, ticks: u32) {
        let now = Instant::now();
        let (since, m, t) = &mut self.rate;
        let start = *since.get_or_insert(now);
        if now - start >= Duration::from_secs(1) {
            if *m > 21 || *t > 21 {
                log::info!(
                    "Sunrise: {m} movement packets and {t} ticks ended in {} ms",
                    (now - start).as_millis()
                );
            }
            *since = Some(now);
            *m = 0;
            *t = 0;
        }
        *m += moves;
        *t += ticks;
    }
    /// A new ClientPacketListener after a reconfiguration: what the last one tracked is gone.
    fn reset_listener(&mut self) {
        self.joined = false;
        self.entity_id = 0;
        self.position_sender = Default::default();
        self.tick_end_at = None;
        self.moved_at = None;
        self.play_began = None;
        self.rate = (None, 0, 0);
        self.trail.clear();
        self.steer_yaw = None;
        self.last_input = 0;
        self.was_sprinting = false;
        self.carried.set(None);
        self.loaded_sent = false;
        self.predictions = Default::default();
        self.menu = ClientMenu::default();
        self.menu_title = None;
        self.state_id = 0;
        self.use_target = None;
        self.entities.clear();
        self.lerps.clear();
        self.stepped.clear();
        self.head_lerps.clear();
        self.pending_items.clear();
        self.flash_on_set_health = false;
        self.names.clear();
        self.players.clear();
        self.tab_players.clear();
        self.skin_textures.clear();
        self.skins.clear();
        self.listed.clear();
        self.level_players.clear();
        self.destroying = None;
        self.destroy_delay = 0;
        self.input = Default::default();
        self.stop_mining = false;
        self.placing = None;
        self.death_message = None;
        self.synced = vec![None; CREATIVE_SYNCED];
        self.clocks.clear();
        self.clock_game_time = 0;
        self.chunk_center = (0, 0);
    }
}

fn face_direction(face: [i32; 3]) -> u8 {
    match face {
        [0, -1, 0] => 0,
        [0, 1, 0] => 1,
        [0, 0, -1] => 2,
        [0, 0, 1] => 3,
        [-1, 0, 0] => 4,
        [1, 0, 0] => 5,
        _ => 1,
    }
}

/// A protocol number format as the scoreboard's.
fn number_format(f: &play::NumberFormat) -> sunrise_client::scoreboard::NumberFormat {
    use sunrise_client::scoreboard::{NumberFormat, tag_to_json};
    match f {
        play::NumberFormat::Blank => NumberFormat::Blank,
        play::NumberFormat::Styled(t) => NumberFormat::Styled(tag_to_json(t)),
        play::NumberFormat::Fixed(t) => NumberFormat::Fixed(tag_to_json(t)),
    }
}

impl Game<'_> {
<<<<<<< sync-sunrise-1
    #[cfg(target_os = "android")]
    pub(super) fn touch_mining_progress(&self) -> f32 {
        self.remote
            .as_ref()
            .and_then(|r| r.destroying.map(|d| d.2))
            .or_else(|| self.mining.as_ref().map(|m| m.progress))
            .unwrap_or(0.)
    }

    /// A `Holder<SoundEvent>` off the wire: an inline event's location, or
    /// the name of a registered one.
    pub(super) fn remote_sound_name(
        &self,
        sound: &(i32, Option<(String, Option<f32>)>),
    ) -> Option<String> {
        match &sound.1 {
            Some((location, _)) => Some(location.clone()),
            None => self.remote.as_ref().and_then(|r| {
                r.builtin
                    .get("minecraft:sound_event")?
                    .name(sound.0)
                    .map(str::to_owned)
            }),
        }
    }
    pub(super) fn remote_sound_entity_state(
        &mut self,
        id: i32,
    ) -> Option<sunrise_client::sound_engine::SoundEntityState> {
        let r = self.remote.as_ref()?;
        let player = self.player.as_ref().map(|p| {
            (
                r.entity_id,
                [p.position.x, p.position.y, p.position.z],
                r.local_entity_silent,
            )
        });
        // The local player is not in mobs, so avoid a guaranteed index miss.
        if player.is_some_and(|p| p.0 == id) {
            return entity_sound_state(id, player, None, &r.entities, None);
        }
        let mob = self.mob_index.get(&self.mobs, id as u32).map(|m| {
            sunrise_client::sound_engine::SoundEntityState {
                position: [m.position.x, m.position.y, m.position.z],
                silent: m.silent,
                removed: m.removed,
            }
        });
        let dragon = self
            .dragon
            .as_ref()
            .map(|d| (d.id as i32, [d.position.x, d.position.y, d.position.z]));
        let mut state = entity_sound_state(id, player, mob, &r.entities, dragon)?;
        // Mob states already use the live position. In particular,
        // level_entity would find the dragon's stale Mob before its Dragon.
        if mob.is_some() || dragon.is_some_and(|d| d.0 == id) {
            return Some(state);
        }
        // Vehicles, projectiles and other simulated objects may have moved
        // since the last tracking packet.
        if let Some(position) = self
            .level_entity(id)
            .and_then(|target| self.target_position(target))
        {
            state.position = position;
        }
        Some(state)
=======
    /// One vanilla active cache: `(2 * (max(2, client_view_distance) + 3) + 1)^2` columns
    /// (`ClientChunkCache.calculateStorageRange`), times the active dimension's sections,
    /// times 4,096 cells. Viewed levels share that footprint; a legitimate server need not
    /// group more cells than one full active world.
    /// The server's view-radius packet and per-view column caps cannot grow the ceiling.
    pub(in crate::game) fn connection_group_limit(&self) -> usize {
        self.remote.as_ref().map_or(0, |remote| {
            let radius = remote.sent_info.view_distance.max(2) as usize + 3;
            let width = 2 * radius + 1;
            (width * width).saturating_mul(self.world.shape.section_count() * 4096)
        })
>>>>>>> sunrise/main
    }

    // remote_sound_name: remote/sync.rs

    /// The hurt sound the hurt player's own client plays from
    /// `ClientboundDamageEventPacket` (`LivingEntity.handleDamageEvent`):
    /// `Player.getHurtSound` is the damage type's effects, with the codec's
    /// `hurt` default when the type is unknown.
    pub(super) fn remote_hurt_sound(&self, source_type: i32) -> &'static str {
        match self.remote.as_ref() {
            Some(r) => hurt_sound_for(&r.registries, &self.health_resources.damage, source_type),
            None => sunrise_client::health::hurt_sound("hurt"),
        }
    }
<<<<<<< sync-sunrise-1
    /// Leave the title for a server (ConnectScreen): an empty world fed by the connection.
    pub(super) fn connect_server(&mut self, address: &str, name: &str) -> Result<()> {
        self.connect_server_with_cookies(address, name, None)
    }
    /// `connect_server` carrying a transfer's stored server cookies
    /// (`TransferState`); None on an ordinary join.
    pub(super) fn connect_server_with_cookies(
        &mut self,
        address: &str,
        name: &str,
        cookies: Option<sunrise_client::net::Cookies>,
    ) -> Result<()> {
        self.connect_server_with_options(address, name, cookies, false, false)
    }
    /// `ConnectScreen.startConnecting`: keep LAN discovery's server kind through loading.
    pub(super) fn connect_server_with_options(
        &mut self,
        address: &str,
        name: &str,
        cookies: Option<sunrise_client::net::Cookies>,
        lan: bool,
        realm: bool,
    ) -> Result<()> {
        super::world_loading::preload_server_loot(
            &self.options.resources,
            &self.res.core.item_data,
        );
        if self.defer_world_entry(super::PendingWorldEntry::Server {
            address: address.to_owned(),
            name: name.to_owned(),
            cookies: cookies.clone(),
            lan,
            realm,
        })? {
            return Ok(());
        }
        self.join_world_rest()?;
        self.client_logging_out();
        if let Err(error) = self.join_server(address, name, None, cookies, lan, realm) {
            log::warn!("Sunrise: could not join server: {error}");
            let skip = self.settings.file.bool("skipMultiplayerWarning");
            match &mut self.title {
                Some(title) => {
                    title.skip_multiplayer_warning = skip;
                    title.failed(error);
                }
                None => {
                    let mut title = self
                        .new_title(std::env::current_dir()?.join("saves"))
                        .with_user(|| self.options.player_name());
                    title.skip_multiplayer_warning = skip;
                    title.failed(error);
                    self.show_title(title);
                }
            }
            self.capture(false);
        }
        Ok(())
    }
    pub(super) fn connect_remote_session(
        &mut self,
        address: &str,
        name: &str,
        local: Option<sunrise_protocol::connection::Pipe>,
        cookies: Option<sunrise_client::net::Cookies>,
        lan: bool,
        realm: bool,
    ) -> Result<()> {
        self.realm_session = realm;
        self.reset_tutorial();
        self.perf.world_opening();
        // A server's own biomes come over the connection; the last local
        // world's packed table must not name its ids. The window's own
        // server thread keeps the one it just set.
        if local.is_none() {
            self.world_biomes = None;
            self.world_biomes_base = None;
        }
        let root = self.options.resources.clone();
        let item_data = Arc::clone(&self.res.core.item_data);
        let builtin = builtin_registries(&root).ok_or("Resources are incomplete: registries.json is missing or invalid. Click \"Prepare again\" in the launcher (or run sunrise-prepare), then try again.")?;
        let lang = Arc::new(sunrise_client::net::load_language(&root));
        // Options.createInformation: what the server learns about this player,
        // sent again when one of those options changes.
        let info = self
            .settings
            .client_information(client_view_distance(self.settings.render_distance));
        if local.is_none() && !self.options.split_guest {
            mod_api::content::crash::set_detail(
                "Server",
                sunrise_client::net::shown_address(address),
            );
        }
        // A signed-in account always plays under its own name on servers (an online-mode one
        // takes no other); single player's own server, the name its world opened with
        // (integrated.rs: a name chosen in Skin Customization goes first); a splitscreen player
        // signs in with its own account when it picked one.
        let auth = self.auth();
        let name = match (&auth, &local) {
            (Some(a), None) => a.name.as_str(),
            _ => name,
        };
        let user = auth.as_ref().map_or_else(
            || {
                (
                    name.to_owned(),
                    sunrise_protocol::crypto::offline_uuid(name),
                )
            },
            |a| (a.name.clone(), a.uuid),
        );
        self.server_packs_connect(local.is_none().then_some(address), user);
        let session = match local {
            Some(pipe) => Session::connect_local(
                pipe,
                name,
                auth.clone(),
                info.clone(),
                lang.clone(),
                item_data.clone(),
            ),
            None => {
                self.extras_connecting(address, name, lan);
                match cookies {
                    Some(cookies) => Session::connect_transferred(
                        address,
                        name,
                        auth.clone(),
                        info.clone(),
                        lang.clone(),
                        cookies,
                        item_data.clone(),
                    ),
                    None => {
                        let connect = if lan {
                            Session::connect_lan
                        } else {
                            Session::connect
                        };
                        connect(
                            address,
                            name,
                            auth.clone(),
                            info.clone(),
                            lang.clone(),
                            item_data.clone(),
                        )
                    }
                }
            }
        };
        let player_type = super::perspective::player_type(&root);
        let mut streaming = Streaming::new(
            Arc::new(NoTerrain),
            self.worker_context(),
            self.res.threads(),
        );
        streaming.remote = true;
        streaming.radius = self.settings.render_distance;
        self.streaming = streaming;
        self.world = Game::world_for_mods(self.res.facts().air, 0);
        self.save = Arc::new(SaveDir::new(
            std::env::temp_dir().join("sunrise-remote-unused"),
            sunrise_client::save::LevelShape::OVERWORLD,
        ));
        self.inventory = vec![None; 36];
        self.offhand = None;
        self.armor = [None, None, None, None];
        self.craft_grid = vec![None; 9];
        self.craft_size = 2;
        self.remove_local_player_body();
        self.spawn = None;
        self.mobs.clear();
        self.clear_remote_objects();
        self.clear_remote_projectiles();
        self.creatures.projectiles.clear();
        self.dropped.clear();
        self.clear_client_particles();
        self.falling.clear();
        self.mining = None;
        self.container = None;
        self.carried = None;
        self.using = None;
        self.end_mod_use();
        self.clicks.miss_time = 0;
        // A new LocalPlayer has no effects: the server sends what it has
        // (ClientboundUpdateMobEffectPacket on placement).
        self.potions = Default::default();
        self.set_mode_id(0);
        self.experience = Experience {
            level: 0,
            progress: 0.0,
            total: 0,
        };
        // The connected client never opens a server world (single player also
        // joins here), so load the registries its argument parsers and local
        // suggestions need before receiving the server's command tree.
        self.ui.registries = Default::default();
        self.build_registries();
        self.ui.registries.permission = 0;
        self.ui.chat.clear();
        self.set_screen(Screen::Playing);
        self.hide_title();
        self.dim = Dim::Overworld;
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections(self.world.layout());
        }
        self.apply_settings();
        self.clear_waypoints();
        self.boss_overlay.reset();
        // The chat's tree until the server sends its own, or forever when it
        // sends none: the game's own, plus the commands native mods own.
        let chat_tree = self.chat_tree_for(None);
        // Minecraft.services() has the keys wherever the game is played, single player's own
        // server included: its guests' chat keys are checked against them.
        let services_keys = Arc::new(std::sync::Mutex::new(ServicesKeys::Pending));
        let slot = services_keys.clone();
        let _ = std::thread::Builder::new()
            .name("Services keys".into())
            .spawn(move || {
                use sunrise_server_files::profiles;
                let keys = profiles::fetch_profile_key_keys(&profiles::discovery_url());
                *slot.lock().unwrap_or_else(|e| e.into_inner()) = match keys {
                    Ok(keys) if !keys.is_empty() => ServicesKeys::Ready(keys),
                    Ok(_) => ServicesKeys::Missing,
                    Err(why) => {
                        log::error!("Sunrise: {why}");
                        ServicesKeys::Missing
                    }
                };
            });
        self.ui.registries.selector_tokens.clear();
        self.remote = Some(Remote {
            selector_metadata_supported: true,
            professions: self.client_professions()?,
            session,
            brand: "unknown".into(),
            recipe_viewer: Default::default(),
            features: ["minecraft:vanilla".to_string()].into(),
            quiet_commands: false,
            mod_net: false,
            custom_books: false,
            logged_out: false,
            attachments: HashMap::new(),
            far_terrain: None,
            builtin,
            status: lang
                .get("connect.connecting")
                .cloned()
                .unwrap_or_else(|| "Connecting to the server...".into()),
            lang,
            registries: Arc::new(Dynamic::default()),
            tags: None,
            enchantments: None,
            joined: false,
            reconfiguring: None,
            logged_in: false,
            entity_id: 0,
            local_entity_silent: false,
            camera: None,
            use_item: None,
            dimension: DimensionType::vanilla("minecraft:overworld"),
            level: "minecraft:overworld".into(),
            level_key: mod_api::DimensionKey::OVERWORLD,
            level_facts: None,
            level_environment: None,
            facts_cache: Default::default(),
            added_levels: Default::default(),
            viewed: Default::default(),
            hashed_seed: 0,
            chunk_format: Arc::new(ChunkFormat::new(
                &DimensionType::vanilla("minecraft:overworld"),
                Vec::new(),
                7,
            )),
            decoder: ChunkDecoder::new(),
            view_distance: self.settings.render_distance,
            simulation_distance: self.settings.simulation_distance,
            sequence: 0,
            last_teleport_sequence: -1,
            position_sender: Default::default(),
            tick_end_at: None,
            moved_at: None,
            play_began: None,
            rate: (None, 0, 0),
            trail: Default::default(),
            steer_yaw: None,
            interacted_at: Default::default(),
            told_rotation: Default::default(),
            held_back: Default::default(),
            stick_mode: Default::default(),
            stick_choice: None,
            stick_remembered: None,
            stick_setting: None,
            stick_setbacks: Default::default(),
            stick_used_at: None,
            stick_asked: false,
            swap_presses: 0,
            last_input: 0,
            was_sprinting: false,
            carried: std::cell::Cell::new(None),
            was_shared_flags: 0,
            was_living_flags: 0,
            last_recorded_pose: None,
            abilities: None,
            loaded_sent: false,
            waiting_ticks: 0,
            setbacks: 0,
            player_attributes: Vec::new(),
            heard_level: None,
            level_load: sunrise_client::level_loading::LevelLoadTracker::new(0),
            sent_info: info,
            predictions: HashMap::new(),
            menu: ClientMenu::default(),
            ticked_before_player: std::collections::HashSet::new(),
            craft_result: None,
            menu_title: None,
            menu_title_snapshot: None,
            state_id: 0,
            use_target: None,
            menu_extra: None,
            entities: HashMap::new(),
            tick_fluids: Vec::new(),
            tick_rain: Vec::new(),
            tick_standings: Vec::new(),
            tick_water_fx: Vec::new(),
            passenger_vehicles: HashMap::new(),
            lerps: HashMap::new(),
            cart_lerps: HashMap::new(),
            stepped: HashMap::new(),
            head_lerps: HashMap::new(),
            ghast_dismount: None,
            first_passenger: false,
            passenger_index: 0,
            pending_items: HashMap::new(),
            health: (20.0, 20, 5.0),
            flash_on_set_health: false,
            game_mode: 0,
            chat: Vec::new(),
            chat_received: 0,
            names: HashMap::new(),
            chat_queue: VecDeque::new(),
            signatures: Default::default(),
            chat_previous: None,
            next_chat_index: 0,
            enforces_secure_chat: false,
            services_keys,
            chat_sessions: HashMap::new(),
            chat_validators: HashMap::new(),
            joining_narrated: None,
            chat_tree,
            chat_tree_received: false,
            secure_chat: Default::default(),
            custom_completions: Default::default(),
            players: HashMap::new(),
            tab_players: HashMap::new(),
            skin_textures: HashMap::new(),
            own_profile: OwnProfile::Idle,
            skins: HashMap::new(),
            skins_revision: 0,
            published_skins: None,
            listed: Default::default(),
            online_mode: false,
            levels: Vec::new(),
            level_players: HashMap::new(),
            level_player_names: HashMap::new(),
            level_player_textures: HashMap::new(),
            glowing: Default::default(),
            own_invisible: false,
            own_on_fire: false,
            own_arrows: 0,
            own_stingers: 0,
            own_shoulders: [None; 2],
            profile: 0,
            profile_name: String::new(),
            destroying: None,
            destroying_item: None,
            is_destroying: false,
            interaction_swing: [true; 2],
            destroy_delay: 0,
            input: Default::default(),
            pending_keys: InputQueue::new(queued_keys::PENDING_KEYS),
            swap_answer: [false; 2],
            stop_mining: false,
            placing: None,
            flat: false,
            death_message: None,
            show_death_screen: true,
            reduced_debug_info: false,
            do_limited_crafting: false,
            player_type,
            synced: vec![None; CREATIVE_SYNCED],
            creative_menu: None,
            decode_errors: 0,
            clocks: HashMap::new(),
            default_clock: None,
            overworld_clock: None,
            clock_game_time: 0,
            chunk_center: (0, 0),
            backlog: VecDeque::new(),
            backlog_since: None,
            server_links: Vec::new(),
            debug_query: Default::default(),
            transfer: None,
        });
        // The level's TickRateManager follows the server's ticking packets.
        self.clock = sunrise_client::clock::FixedStep::client();
        // The player's advancements and statistics are the server's.
        self.adv = Default::default();
        self.stats = Default::default();
        Ok(())
    }
=======
    // connect_remote_session: remote/session.rs
>>>>>>> sunrise/main

    // init_remote_for_replay: remote/session.rs

<<<<<<< sync-sunrise-1
        self.set_mode_id(0);
        self.experience = Experience {
            level: 0,
            progress: 0.0,
            total: 0,
        };
        self.ui.registries = Default::default();
        self.build_registries();
        self.ui.registries.permission = 0;
        self.ui.chat.clear();
        self.set_screen(Screen::Playing);
        self.title = None;
        self.dim = Dim::Overworld;
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections(self.world.layout());
        }
        self.apply_settings();
        self.clear_waypoints();
        self.boss_overlay.reset();

        let chat_tree = self.chat_tree_for(None);
        let services_keys = Arc::new(std::sync::Mutex::new(ServicesKeys::Ready(Vec::new())));

        self.ui.registries.selector_tokens.clear();
        self.remote = Some(Remote {
            names: HashMap::new(),
            tick_end_at: None,
            moved_at: None,
            play_began: None,
            rate: (None, 0, 0),
            trail: Default::default(),
            steer_yaw: None,
            interacted_at: Default::default(),
            told_rotation: Default::default(),
            held_back: Default::default(),
            stick_mode: Default::default(),
            stick_choice: None,
            stick_remembered: None,
            stick_setting: None,
            stick_setbacks: Default::default(),
            stick_used_at: None,
            stick_asked: false,
            swap_presses: 0,
            waiting_ticks: 0,
            setbacks: 0,
            player_attributes: Vec::new(),
            creative_menu: None,
            selector_metadata_supported: true,
            professions: self.client_professions()?,
            session,
            brand: "sunrise-studio".into(),
            recipe_viewer: Default::default(),
            features: ["minecraft:vanilla".to_string()].into(),
            quiet_commands: false,
            mod_net: false,
            custom_books: false,
            attachments: HashMap::new(),
            far_terrain: None,
            builtin,
            status: "Playing Studio Replay".into(),
            lang,
            registries: Arc::new(Dynamic::default()),
            tags: None,
            enchantments: None,
            joined: false,
            reconfiguring: None,
            logged_in: true,
            logged_out: false,
            local_entity_silent: false,
            entity_id: 0,
            camera: None,
            use_item: None,
            dimension: DimensionType::vanilla("minecraft:overworld"),
            level: "minecraft:overworld".into(),
            level_key: mod_api::DimensionKey::OVERWORLD,
            level_facts: None,
            level_environment: None,
            facts_cache: Default::default(),
            added_levels: Default::default(),
            viewed: Default::default(),
            hashed_seed: 0,
            chunk_format: Arc::new(ChunkFormat::new(
                &DimensionType::vanilla("minecraft:overworld"),
                Vec::new(),
                7,
            )),
            decoder: ChunkDecoder::new(),
            view_distance: self.settings.render_distance,
            simulation_distance: self.settings.simulation_distance,
            sequence: 0,
            last_teleport_sequence: -1,
            position_sender: Default::default(),
            last_input: 0,
            was_sprinting: false,
            was_shared_flags: 0,
            was_living_flags: 0,
            last_recorded_pose: None,
            carried: std::cell::Cell::new(None),
            abilities: None,
            loaded_sent: false,
            heard_level: None,
            level_load: sunrise_client::level_loading::LevelLoadTracker::new(0),
            sent_info: info,
            predictions: HashMap::new(),
            menu: ClientMenu::default(),
            ticked_before_player: std::collections::HashSet::new(),
            craft_result: None,
            menu_title: None,
            menu_title_snapshot: None,
            state_id: 0,
            use_target: None,
            menu_extra: None,
            entities: HashMap::new(),
            tick_fluids: Vec::new(),
            tick_rain: Vec::new(),
            tick_standings: Vec::new(),
            tick_water_fx: Vec::new(),
            passenger_vehicles: HashMap::new(),
            lerps: HashMap::new(),
            cart_lerps: HashMap::new(),
            stepped: HashMap::new(),
            head_lerps: HashMap::new(),
            ghast_dismount: None,
            first_passenger: false,
            passenger_index: 0,
            pending_items: HashMap::new(),
            health: (20.0, 20, 5.0),
            flash_on_set_health: false,
            game_mode: 0,
            chat: Vec::new(),
            chat_received: 0,
            chat_queue: VecDeque::new(),
            signatures: Default::default(),
            chat_previous: None,
            next_chat_index: 0,
            enforces_secure_chat: false,
            services_keys,
            chat_sessions: HashMap::new(),
            chat_validators: HashMap::new(),
            joining_narrated: None,
            chat_tree,
            chat_tree_received: false,
            secure_chat: Default::default(),
            custom_completions: Default::default(),
            players: HashMap::new(),
            tab_players: HashMap::new(),
            skin_textures: HashMap::new(),
            own_profile: OwnProfile::Idle,
            skins: HashMap::new(),
            skins_revision: 0,
            published_skins: None,
            listed: Default::default(),
            online_mode: false,
            levels: Vec::new(),
            level_players: HashMap::new(),
            level_player_names: HashMap::new(),
            level_player_textures: HashMap::new(),
            glowing: Default::default(),
            own_invisible: false,
            own_on_fire: false,
            own_arrows: 0,
            own_stingers: 0,
            own_shoulders: [None; 2],
            profile: 0,
            profile_name: String::new(),
            destroying: None,
            destroying_item: None,
            is_destroying: false,
            interaction_swing: [true; 2],
            destroy_delay: 0,
            input: Default::default(),
            pending_keys: InputQueue::new(queued_keys::PENDING_KEYS),
            swap_answer: [false; 2],
            stop_mining: false,
            placing: None,
            flat: false,
            death_message: None,
            show_death_screen: true,
            reduced_debug_info: false,
            do_limited_crafting: false,
            player_type,
            synced: vec![None; CREATIVE_SYNCED],
            decode_errors: 0,
            clocks: HashMap::new(),
            default_clock: None,
            overworld_clock: None,
            clock_game_time: 0,
            chunk_center: (0, 0),
            backlog: VecDeque::new(),
            backlog_since: None,
            server_links: Vec::new(),
            debug_query: Default::default(),
            transfer: None,
        });
        self.clock = sunrise_client::clock::FixedStep::client();
        self.adv = Default::default();
        self.stats = Default::default();
        Ok(())
    }

    /// Disconnect and return to the multiplayer screen (DisconnectedScreen).
    pub(super) fn leave_server(&mut self, reason: Option<String>) -> Result<()> {
        if reason.is_none() && self.integrated.as_ref().is_some_and(|s| !s.stopped()) {
            return self.begin_closing_save(false);
        }
        self.client_logging_out();
        self.finish_perf();
        if reason.is_none() {
            // Leaving on purpose: nothing for auto-reconnect to retry.
            self.extras_left();
        }
        self.clear_waypoints();
        self.hud_disconnected();
        // Vanilla's Hud also ticks on the title and connect screens, so the
        // action bar has run out by the next world; Sunrise's ticks in one.
        self.titles.overlay_time = 0;
        // GameRenderer.resetData and narrator.clear (Minecraft.disconnect).
        self.item_activation.reset();
        // Kept mod block entity geometry belongs to the world leaving.
        self.kept_block_entities.get_mut().clear();
        self.mod_view_distances.take();
        self.clear_narrator();
        // updateLevelInEngines(null): every sound stops and the music manager
        // lets go of its track. Leaving a server comes straight here, not
        // through finish_close_world, so the biome's ambient loop would
        // otherwise play on over the title screen until the next world.
        self.ambience.clear(&mut self.audio);
        self.music.stop_playing(None, &mut self.audio);
        self.audio.stop_all();
        self.mod_sounds.clear();
        // A world is leaving either way: its icon attempt goes with it.
        self.world_icon = None;
        // Minecraft.disconnect: the client level goes.
        if self.remote.is_some() {
            self.forget_all_remote_columns();
        }
        if let Some(old) = self.remote.as_mut().and_then(|r| r.heard_level.take()) {
            self.native_client_level_unload(&old);
        }
        if self.remote.is_some() {
            self.native_client_disconnecting();
        }
        if let Some(mut r) = self.remote.take() {
            r.session.disconnect();
        }
        super::lan::sync::forget_sync_data();
        self.gui.atlas.clear_object_skins();
        self.server_packs_disconnect();
        self.code_of_conduct = None;
        self.reset_tutorial();
        self.close_dialogs();
        // Gui.setScreen -> KeyMapping.releaseAll: the title or disconnect
        // screen releases every held key. Its release goes to that screen,
        // so a key kept here would still be down in the next world (#3246).
        self.keys.clear();
        self.left = false;
        self.right = false;
        // The shared world is closing: its invite code ends with it.
        self.friends_lan_closed();
        // Single player's server thread saves the world before the title shows.
        let single_player = match self.integrated.take() {
            Some(server) => {
                server.stop()?;
                true
            }
            None => false,
        };
        // A splitscreen player leaving says nothing of where P1 plays.
        if !self.options.split_guest {
            mod_api::content::crash::clear_detail("Server");
        }
        self.clock = Default::default();
        self.clear_remote_world();
        let saves = std::env::current_dir()?.join("saves");
        let mut title = self
            .new_title(saves.clone())
            .with_user(|| self.options.player_name());
        title.last_server = self.settings.file.string("lastServer");
        title.skip_multiplayer_warning = self.settings.file.bool("skipMultiplayerWarning");
        if single_player && reason.is_none() {
            self.show_title(title);
            self.capture(false);
            return Ok(());
        }
        title.mode = sunrise_client::title::Mode::Multiplayer(match reason {
            Some(reason) => sunrise_client::multiplayer::Screen::Disconnected {
                reason,
                retry: None,
                lines: std::cell::Cell::new(1),
            },
            None => sunrise_client::multiplayer::Screen::list(&saves),
        });
        self.show_title(title);
        self.capture(false);
        Ok(())
    }

    /// What the connection's level and listener held (leaving, or reconfiguring).
    fn clear_remote_world(&mut self) {
        self.adv = Default::default();
        self.stats = Default::default();
        self.score.board = Default::default();
        self.streaming = Streaming::new(Arc::new(NoTerrain), self.worker_context(), 1);
        self.streaming.paused = true;
        self.reset_client_world(0);
        self.remove_local_player_body();
        // The vehicle went with the level: an id kept here would seat the
        // next login on an entity that is not there.
        self.riding = None;
        // Out of the server's bed: InBedChatScreen goes with the level.
        self.sleeping = None;
        self.sleep_timer = 0;
        self.clear_entities();
        self.score.board = Default::default();
        self.tab_list = Default::default();
        self.border = Default::default();
        self.set_screen(Screen::Playing);
        self.container = None;
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections(self.world.layout());
        }
    }
    /// `Hud.onDisconnected`, from `Minecraft.disconnect` and `clearClientLevel`
    /// (a proxy's reconfiguration too), but the chat: joining clears it and a
    /// reconfiguration keeps it (`ChatComponent.storeState`).
    fn hud_disconnected(&mut self) {
        self.tab_list.reset();
        self.boss_overlay.reset();
        self.adv.toasts.clear();
        self.debug_charts.reset();
        self.titles.clear();
        self.titles.reset_times();
    }
=======
    // leave_server: remote/session.rs
>>>>>>> sunrise/main

    pub(super) fn send(&self, packet: S) {
        if let Some(r) = &self.remote {
            let interaction = is_interaction(&packet);
            // MultiPlayerGameMode's attack, interact, useItem, useItemOn, releaseUsingItem, dropItem and
            // piercingAttack run ensureHasSentCarriedItem before their packet: a slot the number keys
            // chose this tick goes out first. A miss, a dig's start or a swap waits for the next tick.
            if let S::Attack(_) | S::Interact(_) | S::UseItem(_) | S::UseItemOn(_) = &packet {
                self.remote_send_carried();
            } else if let S::PlayerAction(a) = &packet
                && matches!(a.action, 4 | 5 | 6 | 8)
            {
                self.remote_send_carried();
            }
            if (interaction || matches!(packet, S::Punch(_))) && r.stick_mode == ServerStick::Free {
                // A hit, a use, a block placed or an item dropped goes where the player looks:
                // a server places stairs, throws and aims by the rotation it was last sent. While
                // that is the stick's steering (last tick's move), it waits for the next tick,
                // after this tick's move with the view's look (`stick_steer` rests after it): a
                // look of its own would be a tick more to an anticheat (Grim's TickTimer), the
                // move after it the same look twice (AimDuplicateLook). The swings after it wait
                // too; a block being broken does not (`waits_for_the_view`).
                if interaction {
                    r.interacted_at.set(Some(Instant::now()));
                }
                let mut held = r.held_back.borrow_mut();
                if waits_for_the_view(&packet, r.steer_yaw.is_some(), !held.is_empty()) {
                    held.push(packet);
                    return;
                }
            }
            self.send_play(packet);
        }
    }
    fn send_action(&self, action: i32) {
        self.send(S::PlayerAction(play::PlayerAction {
            action,
            pos: [0; 3],
            direction: 0,
            sequence: 0,
        }));
    }
<<<<<<< sync-sunrise-1
    /// The hits and uses held back while the stick steered (`send`), the tick after.
    fn send_held_back(&self) {
        if let Some(r) = &self.remote {
            for packet in r.held_back.take() {
                self.send_play(packet);
            }
        }
    }
    /// The stick's mode on the server this tick, when on one.
    pub(super) fn remote_stick_mode(&self) -> Option<ServerStick> {
        self.remote.as_ref().map(|r| r.stick_mode)
    }
    /// The stick's mode on this connection: Analog in single player and for a LAN host.
    fn update_stick_mode(&mut self) -> ServerStick {
        let setting = self.settings.controller.server_stick;
        let dir = Self::options_path()
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default();
        let Some(r) = self.remote.as_mut() else {
            return ServerStick::Analog;
        };
        if r.session.address == "local" {
            r.stick_mode = ServerStick::Analog;
            return r.stick_mode;
        }
        let mut remembered = *r
            .stick_remembered
            .get_or_insert_with(|| sunrise_client::stick_memory::get(&dir, &r.session.address));
        // The setting changed while playing here: it is this server's mode now, the one
        // remembered for it included.
        if r.stick_setting
            .replace(setting)
            .is_some_and(|seen| seen != setting)
        {
            r.stick_choice = Some(setting);
            if remembered.is_some() {
                sunrise_client::stick_memory::set(&dir, &r.session.address, setting);
                remembered = Some(setting);
                r.stick_remembered = Some(remembered);
            }
            log::info!(
                "Sunrise: the stick moves as {} on this server, from the setting",
                setting.name()
            );
        }
        r.stick_mode = r.stick_choice.or(remembered).unwrap_or(setting);
        r.stick_mode
    }
    /// A setback while the stick moved the player: four in six seconds, in a mode the server
    /// may refuse, and the player is asked once a session how the stick should move here.
    fn stick_setback(&mut self) {
        let now = Instant::now();
        let ask = self.settings.controller.stick_prompt;
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        if !r
            .stick_used_at
            .is_some_and(|at| now - at < Duration::from_secs(1))
        {
            return;
        }
        r.stick_setbacks.push_back(now);
        while r
            .stick_setbacks
            .front()
            .is_some_and(|at| now - *at > Duration::from_secs(6))
        {
            r.stick_setbacks.pop_front();
        }
        if r.stick_setbacks.len() < 4
            || r.stick_asked
            || !ask
            || r.stick_mode == ServerStick::Keys
            || r.stick_choice.is_some()
            || r.stick_remembered.flatten().is_some()
        {
            return;
        }
        r.stick_asked = true;
        log::info!("Sunrise: the server keeps pulling the stick's moves back; asking the player");
        self.ask_stick_mode();
    }
    /// The question: which mode, remembered for this server or not, asked again or not.
    fn ask_stick_mode(&mut self) {
        let t = |key: &str| Tag::String(sunrise_client::lang::get_or(key, key).as_str().into());
        let choice = |key: &str, mode: ServerStick| {
            Tag::compound([
                ("label", t(key)),
                ("width", Tag::Int(300)),
                (
                    "action",
                    Tag::compound([
                        ("type", Tag::String("minecraft:dynamic/custom".into())),
                        ("id", Tag::String("starling:server_stick".into())),
                        (
                            "additions",
                            Tag::compound([("mode", Tag::String(mode.name().into()))]),
                        ),
                    ]),
                ),
            ])
        };
        let toggle = |key: &str, label: &str, initial: bool| {
            Tag::compound([
                ("type", Tag::String("minecraft:boolean".into())),
                ("key", Tag::String(key.into())),
                ("label", t(label)),
                ("initial", Tag::Byte(i8::from(initial))),
            ])
        };
        let dialog = Tag::compound([
            ("type", Tag::String("minecraft:multi_action".into())),
            ("title", t("starling.stick_prompt.title")),
            ("pause", Tag::Byte(0)),
            ("columns", Tag::Int(1)),
            (
                "body",
                Tag::compound([
                    ("type", Tag::String("minecraft:plain_message".into())),
                    ("contents", t("starling.stick_prompt.body")),
                    ("width", Tag::Int(300)),
                ]),
            ),
            (
                "inputs",
                Tag::List(vec![
                    toggle("remember", "starling.stick_prompt.remember", true),
                    toggle("dont_ask", "starling.stick_prompt.dont_ask", false),
                ]),
            ),
            (
                "actions",
                Tag::List(vec![
                    choice("starling.stick_prompt.free", ServerStick::Free),
                    choice("starling.stick_prompt.keys", ServerStick::Keys),
                    choice("starling.stick_prompt.analog", ServerStick::Analog),
                ]),
            ),
        ]);
        self.show_local_dialog(&dialog);
    }
    /// A click in one of the game's own dialogs (`starling:` actions).
    pub(super) fn starling_dialog_click(&mut self, id: &str, payload: Option<&Tag>) {
        // The link confirmation's Open (`confirm_link`).
        if id == "starling:open_link" {
            if let Some(Tag::String(url)) = payload.and_then(|p| p.get("url")) {
                self.open_link(&url.to_string_lossy());
            }
            return;
        }
        if id != "starling:server_stick" {
            log::warn!("Sunrise: no such dialog action {id}");
            return;
        }
        let field = |key: &str| payload.and_then(|p| p.get(key));
        let mode = match field("mode") {
            Some(Tag::String(s)) => ServerStick::parse(&s.to_string_lossy()),
            _ => return,
        };
        let on = |key: &str| matches!(field(key), Some(Tag::Byte(b)) if *b != 0);
        let path = Self::options_path();
        let dir = path
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default();
        if let Some(r) = self.remote.as_mut() {
            r.stick_choice = Some(mode);
            if on("remember") {
                sunrise_client::stick_memory::set(&dir, &r.session.address, mode);
                r.stick_remembered = Some(Some(mode));
            }
        }
        if on("dont_ask") {
            self.settings.controller.stick_prompt = false;
            self.settings.save(&path);
        }
        log::info!(
            "Sunrise: the stick moves as {} on this server{}",
            mode.name(),
            if on("remember") { ", remembered" } else { "" }
        );
    }
    /// Free stick direction (the controller setting, experimental): on a server the stick
    /// moves where it points by turning the rotation the server sees toward it and holding
    /// forward, a move the keys make. Not with keys held, nor while mining, using, riding or
    /// gliding, nor right after a hit or a use, which go with the view's rotation.
    fn stick_steer(&self, stick: Option<[f32; 2]>) -> Option<f32> {
        let r = self.remote.as_ref()?;
        let [left, forward] = stick?;
        let keys = [Action::Forward, Action::Left, Action::Back, Action::Right];
        let p = self.player.as_ref()?;
        if left.hypot(forward) <= 0.3
            || keys.iter().any(|k| self.keys.held(*k))
            || self.mining.is_some()
            || self.using.is_some()
            || self.riding.is_some()
            || p.fall_flying
            || r.interacted_at
                .get()
                .is_some_and(|at| at.elapsed() < STEER_REST)
        {
            return None;
        }
        // Moving forward at yaw - a is moving at angle a (to the left) of yaw: Entity.moveRelative.
        Some(self.yaw - left.atan2(forward).to_degrees())
    }
=======

>>>>>>> sunrise/main
    /// `LocalPlayer.isShiftKeyDown` (and so `isSecondaryUseActive`): the input as of the player's
    /// last tick (`input.keyPresses`, what the last ServerboundPlayerInputPacket said), not the key
    /// now: a sneak pressed in the frame of a click acts from the next tick, as on the server.
    fn remote_shift_down(&self) -> bool {
        self.remote
            .as_ref()
            .is_some_and(|r| r.last_input & SHIFT_INPUT != 0)
    }
    /// Whether secondary use skips a block's default interaction on this client.
    pub(super) fn remote_secondary_use_skips_block(&self) -> bool {
        self.remote_shift_down()
            && (self.stack_in(self.selected).is_some() || self.offhand.is_some())
    }
    fn next_sequence(&mut self) -> i32 {
        let r = self.remote.as_mut().expect("remote");
        r.sequence += 1;
        r.sequence
    }
    fn item_name(&self, id: i32) -> Arc<str> {
        let r = self.remote.as_ref().expect("remote");
        Arc::from(
            r.builtin
                .get("minecraft:item")
                .and_then(|i| i.name(id))
                .unwrap_or("minecraft:air"),
        )
    }

    // remote_effect_network_id: remote/sync.rs
    fn item_id(&self, name: &str) -> i32 {
        let r = self.remote.as_ref().expect("remote");
        r.builtin
            .get("minecraft:item")
            .and_then(|i| i.id(name))
            .unwrap_or(0)
    }

    /// A stack from the server, decoded as the saved and command item NBT
    /// are, so tooltips and models see every field.
    pub(super) fn to_stack(&self, s: &Option<Stack>) -> Option<ItemStack> {
        self.decode_remote_stack(s.as_ref()?)
    }

    fn decode_remote_stack(&self, stack: &Stack) -> Option<ItemStack> {
        let r = self.remote.as_ref()?;
        if !sunrise_protocol::item::custom::permitted_with_names(
            stack,
            r.registries.custom_components,
            &*r.registries,
        )
        .unwrap_or(false)
        {
            log::warn!("Dropping a stack with unnegotiated custom components");
            return None;
        }
        Some(sunrise_client::item_sync::from_network(
            stack,
            &r.builtin,
            &r.registries,
        ))
    }

    pub(super) fn to_network_projecting(
        &self,
        s: &Option<ItemStack>,
        projected: &dyn Fn(&sunrise_world::component_value::consume_effects::Admitted),
    ) -> Option<Stack> {
        let r = self.remote.as_ref()?;
        // A stack as the server sent it, only its count changed: its own
        // components go back, as a vanilla client keeps them.
        let effect_mapping = r
            .registries
            .consume_effect_codecs
            .as_ref()
            .map(|snapshot| snapshot.identity());
        let source = s.as_ref()?;
        if let Some(patch) = source.wire_patch_for(effect_mapping) {
            let mut out = Stack::new(self.item_id(&source.item), source.count as i32);
            let safe = r.registries.custom_components
                || sunrise_protocol::item::custom::entries().next().is_none()
                || sunrise_protocol::item::WirePatch::decode_reader(
                    sunrise_protocol::buf::Reader::new(&patch).with_names(&*r.registries),
                )
                .and_then(|(added, removed)| {
                    let decoded = Stack {
                        added,
                        removed,
                        ..Stack::new(out.item, out.count)
                    };
                    sunrise_protocol::item::custom::permitted_with_names(
                        &decoded,
                        false,
                        &*r.registries,
                    )
                })
                .unwrap_or(false);
            if safe {
                out.wire = sunrise_protocol::item::WirePatch(Some(patch));
                return Some(out);
            }
            log::warn!("Cannot send cached custom components to this peer");
        }
        let mut wire = sunrise_client::lan::guest::stack_to_network_projecting(
            |name| self.item_id(name),
            // Built-in first, as a hub's `SyncData::id` does: potions are one.
            |registry, name| {
                if registry == sunrise_protocol::item::consume_effects::REGISTRY {
                    return r.registries.consume_effect_id(name);
                }
                r.builtin
                    .get(registry)
                    .and_then(|ids| ids.id(name))
                    .or_else(|| r.registries.id(registry, name))
                    .or_else(|| {
                        (r.registries.custom_components
                            && registry == "minecraft:data_component_type")
                            .then(|| sunrise_protocol::item::custom::id(name))
                            .flatten()
                    })
            },
            s,
            projected,
        )?;
        sunrise_client::lan::guest::canonicalize_stack_patch(
            &mut wire,
            source,
            self.items(),
            &r.builtin,
        );
        Some(wire)
    }

    /// `HashedStack.create` with the connection's registries (the client's
    /// `decoratedHashOpsGenerator`).
    fn hashed_stack(&self, s: &Option<ItemStack>) -> sunrise_protocol::Result<Option<Hashed>> {
        if s.is_none() {
            return Ok(None);
        }
        let r = self
            .remote
            .as_ref()
            .ok_or_else(|| sunrise_protocol::Error::new("no live menu connection"))?;
        let names = SyncedNames {
            builtin: &r.builtin,
            dynamic: &r.registries,
        };
<<<<<<< sync-sunrise-1
        self.to_network(s).map(|s| Hashed::create(&s, &names))
    }
    /// A line in the connection's chat, with its styles, click and hover
    /// events (ChatComponent.addMessage). The lines the Chat option hides
    /// are dropped there (its `visibleMessageFilter`).
    pub(super) fn add_chat_line(
        &mut self,
        content: sunrise_client::text::Text,
        source: sunrise_client::chat::ChatSource,
    ) -> bool {
        self.signed_chat_line(content, source, None, ChatTrust::Secure)
    }
    /// `chat_line` of a player message, keeping its signature for a
    /// deletion (`GuiMessage.signature`).
    fn signed_chat_line(
        &mut self,
        content: sunrise_client::text::Text,
        source: sunrise_client::chat::ChatSource,
        signature: Option<Signature>,
        trust: ChatTrust,
    ) -> bool {
        if !self.ui.chat.shows(source) {
            return false;
        }
        let plain = content.plain();
        let focused = self.screen == Screen::Chat;
        if source == sunrise_client::chat::ChatSource::Player {
            self.ui.chat.add_signed(
                &self.gui.atlas,
                content,
                signature,
                trust.tag(),
                self.visual_ticks(),
                focused,
            );
        } else {
            // addServerSystemMessage and addClientSystemMessage both carry
            // GuiMessageTag.systemSinglePlayer: the grey bar at the line's left.
            let indicator = Some(sunrise_client::chat::SYSTEM_INDICATOR);
            self.ui.chat.add(
                &self.gui.atlas,
                content,
                indicator,
                source,
                self.visual_ticks(),
                focused,
            );
        }
        if let Some(r) = self.remote.as_mut() {
            r.chat.push((plain, Instant::now()));
            r.chat_received += 1;
            if r.chat.len() > 100 {
                r.chat.remove(0);
            }
        }
        true
    }

    /// `ChatListener.handleMessage` for player and disguised chat: held in the
    /// queue while Chat Delay is set, shown at once otherwise. `narration` is
    /// the chat type's narration of it (`decorateNarration`).
    fn handle_player_chat(&mut self, chat: QueuedChat) {
        if self.chat_delay_ms() > 0 {
            if let Some(r) = self.remote.as_mut() {
                r.chat_queue.push_back(chat);
            }
        } else {
            self.accept_chat(chat);
        }
    }

    /// `ChatListener.handleChatMessageError`: a message that fails validation shows as the red,
    /// italic `chat.validation_error` in its chat type (queued under Chat Delay as any
    /// message). Its signature counts as processed but not shown, for the acknowledgement the
    /// client sends the server. Vanilla also
    /// skips a blocked sender and one restricted to friends (not kept here either) and
    /// narrates the line.
    fn handle_chat_message_error(
        &mut self,
        chat_type: &play::ChatTypeBound,
        signature: Option<&Signature>,
    ) {
        if let Some(signature) = signature {
            self.mark_message_processed(signature, false);
        }
        let error = serde_json::json!({
            "translate": "chat.validation_error",
            "color": "red",
            "italic": true,
        });
        let line = self.decorate_chat(chat_type, &error, Decoration::Chat);
        // saySystemChatQueued(boundChatType.decorateNarration(CHAT_VALIDATION_ERROR)).
        let narration = self
            .decorate_chat(chat_type, &error, Decoration::Narration)
            .plain();
        self.handle_player_chat(QueuedChat {
            line,
            signature: None,
            trust: ChatTrust::Error,
            narration,
            sender: 0,
            chat_type: chat_type.chat_type,
            player: None,
        });
    }

    /// `ChatTrustLevel.evaluate` of a player message, with
    /// `ChatListener.evaluateTrustLevel`: the player's own message on their
    /// own server is secure. `decorated` is the line shown, `unsigned` the
    /// server's replacement for the signed content.
    fn chat_trust(
        &self,
        sender: u128,
        signed: bool,
        timestamp: i64,
        content: &str,
        unsigned: Option<&Tag>,
        decorated: &sunrise_client::text::Text,
    ) -> ChatTrust {
        use sunrise_client::scoreboard::{comp_text, tag_to_json};
        if self.integrated.is_some() && self.remote.as_ref().is_some_and(|r| r.profile == sender) {
            return ChatTrust::Secure;
        }
        // PlayerChatMessage.hasExpiredClient: MESSAGE_EXPIRES_AFTER_CLIENT is
        // the server's five minutes and two more.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let expired = now
            > timestamp.saturating_add(
                sunrise_protocol::secure_chat::EXPIRES_AFTER_SERVER + 2 * 60 * 1000,
            );
        if !signed || expired {
            return ChatTrust::NotSecure;
        }
        // ChatTrustLevel.isModified: the signed content is not in the line, or
        // the replacement uses a font other than the default.
        let font_changed = unsigned.is_some_and(|tag| {
            comp_text(&tag_to_json(tag)).spans.iter().any(|span| {
                span.style
                    .font
                    .as_deref()
                    .is_some_and(|font| font != "minecraft:default")
=======
        let projected = std::cell::RefCell::new(Vec::new());
        let stack = self
            .to_network_projecting(s, &|effect| projected.borrow_mut().push(effect.clone()))
            .ok_or_else(|| sunrise_protocol::Error::new("cannot encode a nonempty stack"))?;
        let projected = projected.into_inner();
        let hash = if let Some(data) = s.as_ref().and_then(ItemStack::data) {
            Hashed::create_with_admitted(&stack, &names, &|name, bytes| {
                let matches =
                    |effect: &sunrise_world::component_value::consume_effects::Admitted| {
                        &effect.name == name && effect.wire.as_ref() == bytes
                    };
                if stack.wire.0.is_some() {
                    data.wire
                        .admitted_effects()
                        .iter()
                        .find(|effect| matches(effect))
                        .cloned()
                } else {
                    data.find_admitted_consume_effect(&matches)
                        .or_else(|| projected.iter().find(|effect| matches(effect)).cloned())
                }
>>>>>>> sunrise/main
            })
        } else {
            Hashed::create(&stack, &names)
        };
        hash.map(Some)
            .map_err(|e| sunrise_protocol::Error::new(e.to_string()))
    }

    /// The minecarts the server sent, for the control socket's `status`
    /// (their velocity is not tracked here).
    pub(super) fn remote_carts(&self) -> Option<Vec<serde_json::Value>> {
        let r = self.remote.as_ref()?;
        let mut carts: Vec<&Tracked> = r
            .entities
            .values()
            .filter(|t| t.kind.ends_with("minecart"))
            .collect();
        carts.sort_by(|a, b| {
            a.position
                .partial_cmp(&b.position)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Some(
            carts
                .into_iter()
                .map(|t| serde_json::json!({"kind": t.kind, "position": t.position, "velocity": [0.0, 0.0, 0.0]}))
                .collect(),
        )
    }

    /// The account playing this game.
    pub(super) fn auth(&self) -> Option<std::sync::Arc<sunrise_client::auth::AuthSession>> {
        sunrise_client::auth::current()
    }

    /// `ClientSuggestionProvider.customCompletionSuggestions` of the server
    /// connection (none without one).
    pub(super) fn remote_custom_completions(&self) -> std::collections::BTreeSet<String> {
        self.remote
            .as_ref()
            .map(|r| r.custom_completions.clone())
            .unwrap_or_default()
    }

    /// The names in the server's player list, sorted.
    pub(super) fn remote_online_names(&self) -> Option<Vec<String>> {
        let r = self.remote.as_ref()?;
        let mut names: Vec<String> = r.players.values().cloned().collect();
        names.sort();
        Some(names)
    }

    /// Received events left for the next frame (`poll_remote`): until they
    /// are handled, the terrain the server sent is not all there.
    pub(super) fn remote_backlog(&self) -> usize {
        self.remote.as_ref().map_or(0, |r| r.backlog.len())
    }

    /// The attack clicks queued for the next tick (`KeyMapping.click`, not yet handled).
    pub(in crate::game) fn remote_attack_clicks_queued(&self) -> u32 {
        self.pending_mapping_clicks("key.attack")
    }

    /// Process what the network thread received (once per frame, like the
    /// client's packet processor), in order. Chunk packets get
    /// `CHUNK_PACKETS_PER_FRAME`: once a frame has spent it, the events from
    /// the next chunk packet on wait for the next frame, unless they have
    /// already waited `BACKLOG_LIMIT` (then all of them go, or on a phone
    /// `OVERDUE_CHUNK_PACKETS_PER_FRAME` of them). A chunk packet still
    /// decoding holds back what follows it.
    pub(super) fn poll_remote(&mut self) -> Result<()> {
        self.poll_remote_with(false)
    }

    /// `Minecraft.runTick` handles every packet that came (`runAllTasks`)
    /// before the frame's ticks. What this frame's poll left for later (chunks
    /// past its budget or still decoding, and everything after them) is taken
    /// when a tick is about to run, so the tick moves through those chunks, as
    /// vanilla's does. A batch's `ChunkBatchReceived` is not among them: the
    /// network thread answers it (`net::receive_play`).
    pub(in crate::game) fn poll_remote_before_tick(&mut self) -> Result<()> {
        if self.remote.as_ref().is_none_or(|r| r.backlog.is_empty()) {
            return Ok(());
        }
        self.poll_remote_with(true)
    }
    fn poll_remote_with(&mut self, before_tick: bool) -> Result<()> {
        let polled = self.poll_remote_events(before_tick);
        // The block events and block changes of a piston clock start and end
        // thousands of moves at once: the collision view and the renderer
        // take them all in one sync, before the tick or frame reads them.
        if std::mem::take(&mut self.moving_pistons_stale) {
            self.sync_moving_pistons();
        }
        polled
    }
<<<<<<< sync-sunrise-1
    fn poll_remote_events(&mut self) -> Result<()> {
        let Some(r) = self.remote.as_mut() else {
            return Ok(());
        };
        let mut events = std::mem::take(&mut r.backlog);
        r.receive_credited_backlog(&mut events);
        let overdue = r
            .backlog_since
            .is_some_and(|since| since.elapsed() >= BACKLOG_LIMIT);
        let budget = match overdue {
            false => CHUNK_PACKETS_PER_FRAME,
            true if cfg!(target_os = "android") => OVERDUE_CHUNK_PACKETS_PER_FRAME,
            true => Duration::MAX,
        };
        let mut chunks = Duration::ZERO;
        // A run of ForgetLevelChunk (thousands when the server's view moves far: a
        // teleport at 32 chunks) is forgotten at once, before whatever follows it.
        let mut forgotten = sunrise_client::fast_hash::FxHashSet::default();
        let mut forgotten_credits = Vec::new();
        while let Some(item) = events.pop_front() {
            if let Received::Event(Event::Packet(packet), _) = &item
                && let C::ForgetLevelChunk(p) = &**packet
            {
                self.observe_client_packet(packet);
                forgotten.insert((p.pos[0], p.pos[1]));
                forgotten_credits.push(item.into_event_credit());
                continue;
            }
            sunrise_client::spans::time("pr_forget", || {
                self.forget_remote_columns(&mut forgotten, true)
            });
            forgotten_credits.clear();
            let chunk = matches!(item, Received::Chunk(_, _));
            if chunk && chunks >= budget {
                events.push_front(item);
                break;
            }
            let started = chunk.then(Instant::now);
            let (event, _credit) = match item {
                Received::Event(event, credit) => (event, credit),
                Received::Chunk(format, credit) => {
                    // A column still decoding waits for the next frame, with
                    // what came after it, overdue or not.
                    let Some(next) = self.remote.as_ref().map(|r| r.decoder.next()) else {
                        break;
                    };
                    let (packet, column) = match next {
                        Next::Ready(decoded) => decoded,
                        Next::Decoding => {
                            events.push_front(Received::Chunk(format, credit));
                            break;
                        }
                        Next::Lost => {
                            log::warn!("Sunrise: the chunk decoder stopped; a column is missing");
                            continue;
                        }
                    };
                    self.observe_client_packet(&packet);
                    if let C::LevelChunkWithLight(p) = *packet {
                        sunrise_client::spans::time("pr_chunk", || {
                            self.remote_chunk(&p, Some((&format, column)))
                        });
                    }
                    chunks += started.map_or(Duration::ZERO, |t| t.elapsed());
                    continue;
                }
            };
            match event {
                Event::Status(s) => {
                    if let Some(r) = self.remote.as_mut() {
                        r.status = s;
                    }
                }
                Event::Joined {
                    registries,
                    name,
                    uuid,
                } => {
                    self.set_registries(registries);
                    self.extras_joined();
                    self.close_dialogs();
                    if let Some(r) = self.remote.as_mut() {
                        r.joined = true;
                        r.play_began = Some(Instant::now());
                        // handleLogin: startWaitingForNewLevel with Reason.OTHER.
                        r.profile = uuid;
                        r.profile_name = name.clone();
                        r.status = sunrise_client::lang::get_or(
                            "multiplayer.downloadingTerrain",
                            "Loading terrain...",
                        );
                    }
                    self.begin_book_negotiation();
                }
                Event::Features(features) => {
                    if let Some(remote) = &mut self.remote {
                        remote.features = features.into();
                    }
                }
                Event::Brand(brand) => {
                    if let Some(remote) = &mut self.remote {
                        remote.brand = brand;
                    }
                }
                Event::QuietCommands => {
                    if let Some(remote) = &mut self.remote {
                        remote.quiet_commands = true;
                    }
                }
                Event::ModNet => {
                    if let Some(remote) = &mut self.remote {
                        remote.mod_net = true;
                    }
                }
                Event::CustomBooks => {
                    if let Some(remote) = &mut self.remote {
                        remote.custom_books = true;
                    }
                }
                Event::FarTerrain(offer) => {
                    if let Some(remote) = &mut self.remote {
                        remote.far_terrain = Some(offer);
                    }
                }
                Event::Reconfiguring => {
                    log::info!("Sunrise: the server is reconfiguring the connection");
                    self.reset_book_view();
                    // handleConfigurationStart: the held-back chat shows, then
                    // clearClientLevel opens ServerReconfigScreen over no level.
                    self.flush_chat_queue();
                    self.send_chat_acknowledgement();
                    self.close_dialogs();
                    // The level goes, and a new listener starts from nothing when the
                    // configuration ends; the server configuring us now (a proxy's next one)
                    // offers its own far terrain, or none, and announces its
                    // own features again.
                    self.clear_remote_world();
                    if let Some(remote) = &mut self.remote {
                        remote.viewed.clear();
                        remote.far_terrain = None;
                        remote.quiet_commands = false;
                        remote.mod_net = false;
                        remote.custom_books = false;
                    }
                    if let Some(remote) = &self.remote {
                        remote.session.reset_far_terrain();
                    }
                    self.remove_local_player_body();
                    self.set_screen(Screen::Playing);
                    self.capture(false);
                    // clearClientLevel drops the client level now, so its
                    // unload comes before the configuration screen, not
                    // after the next login's load.
                    self.forget_all_remote_columns();
                    if let Some(old) = self.remote.as_mut().and_then(|r| r.heard_level.take()) {
                        self.native_client_level_unload(&old);
                    }
                    self.native_lifecycle.begin_close();
                    self.native_lifecycle_world_closed();
                    let ticks = self.ticks;
                    if let Some(r) = self.remote.as_mut() {
                        r.reset_listener();
                        r.reconfiguring = Some(ticks);
                        r.chat_tree_received = false;
                    }
                }
                Event::Reconfigured(registries) => {
                    // handleConfigurationStart → clearClientLevel, and the next
                    // ClientPacketListener starts with no players or scoreboard,
                    // no waypoints (its own ClientWaypointManager: a proxy's next
                    // server never untracks the last one's), and its new
                    // ClientSuggestionProvider with no custom chat completions.
                    // clearClientLevel clears the narrator.
                    self.clear_narrator();
                    self.hud_disconnected();
                    self.score.board = Default::default();
                    self.clear_waypoints();
                    if let Some(r) = self.remote.as_mut() {
                        r.attachments.clear();
                        r.players.clear();
                        r.tab_players.clear();
                        r.listed.clear();
                        r.signatures = Default::default();
                        r.custom_completions.clear();
                    }
                    self.ui.registries.custom_completions.clear();
                    self.set_registries(registries);
                    if let Some(r) = self.remote.as_mut() {
                        r.joined = true;
                        r.play_began = Some(Instant::now());
                        r.status = sunrise_client::lang::get_or(
                            "multiplayer.downloadingTerrain",
                            "Loading terrain...",
                        );
                    }
                }
                // handleTransfer: "Cannot transfer to server from
                // singleplayer" (no ServerData), so the world just closes.
                Event::Transfer { host, port } => {
                    if let Some(r) = self.remote.as_mut()
                        && r.session.address != "local"
                    {
                        r.transfer = Some(format!("{host}:{port}"));
                    }
                }
                Event::PackPush(p) => self.handle_pack_push(p),
                Event::PackPop(id) => self.handle_pack_pop(id),
                Event::ServerLinks(links) => {
                    if let Some(r) = self.remote.as_mut() {
                        r.server_links = super::server_links::trusted(links);
                    }
                }
                Event::ShowDialog(dialog) => self.show_dialog(&dialog),
                Event::ClearDialog => self.clear_dialog(),
                Event::CodeOfConduct(text) => self.handle_code_of_conduct(text),
                Event::DecodeError(e) => {
                    if let Some(r) = self.remote.as_mut() {
                        r.decode_errors += 1;
                        if r.decode_errors <= 20 {
                            log::warn!("Sunrise: skipped undecodable packet {e}");
                        }
                    }
                }
                Event::Disconnected {
                    reason,
                    key,
                    from_server,
                } => {
                    self.reset_book_view();
                    log::info!("Sunrise: disconnected: {reason}");
                    // ClientboundTransferPacket left a target: connect there
                    // instead of showing the disconnect, the stored cookies
                    // carried over (ClientCommonPacketListenerImpl.
                    // handleTransfer → ConnectScreen.startConnecting).
                    let transfer = self.remote.as_mut().and_then(|r| {
                        r.transfer.take().map(|address| {
                            (address, r.session.name.clone(), r.session.cookies.clone())
                        })
                    });
                    if let Some((address, name, cookies)) = transfer {
                        self.leave_server(None)?;
                        self.connect_server_with_cookies(&address, &name, Some(cookies))?;
                        return Ok(());
                    }
                    let retry = self.reconnect_after(&reason, key.as_deref(), from_server);
                    self.leave_server(Some(reason))?;
                    self.offer_reconnect(retry);
                    return Ok(());
                }
                Event::RawPayload(incoming) => self.receive_client_payload(*incoming)?,
                Event::Packet(packet) => match *packet {
                    C::CustomPayload(p) if p.channel == sunrise_client::perf_net::CHANNEL => {
                        self.remote_perf_reply(&p.data)
                    }
                    packet => self.handle_packet(packet)?,
                },
            }
            chunks += started.map_or(Duration::ZERO, |t| t.elapsed());
            if self.remote.is_none() {
                break;
            }
        }
        if self.remote.is_some() {
            sunrise_client::spans::time("pr_forget", || {
                self.forget_remote_columns(&mut forgotten, true)
            });
        }
        if let Some(r) = self.remote.as_mut() {
            r.backlog_since = if events.is_empty() {
                None
            } else {
                Some(r.backlog_since.unwrap_or_else(Instant::now))
            };
            r.backlog = events;
        }
        self.send_suggestion_request();
        Ok(())
    }
=======

>>>>>>> sunrise/main
    /// ClientSuggestionProvider.customSuggestion: the chat's completion
    /// request for an `ask_server` node goes to the server.
    fn send_suggestion_request(&mut self) {
        if self.remote.is_none() {
            return;
        }
        let Some((id, command)) = self
            .ui
            .chat_screen
            .as_mut()
            .and_then(|s| s.take_request())
            .or_else(|| self.command_block_suggestion_request())
        else {
            return;
        };
        let mut w = ::sunrise_core::vanilla_bytes::Writer::new();
        w.varint(id);
        w.string(&command);
        self.send(S::Unknown(
            sunrise_protocol::ids::play::serverbound::COMMAND_SUGGESTION,
            w.data,
        ));
    }
<<<<<<< sync-sunrise-1
    /// Entity.DATA_CUSTOM_NAME (2) and DATA_CUSTOM_NAME_VISIBLE (3) of any entity. Displays
    /// (text, item and block) are `text_displays.rs`'s (`remote_objects.rs` feeds them).
    fn remote_names_and_displays(&mut self, id: i32, values: &[(u8, i32, Value)]) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        for (index, _, value) in values {
            match (index, value) {
                (2, Value::OptText(t)) => {
                    r.names.entry(id).or_insert((None, false)).0 = t.as_ref().map(remote_text);
                }
                (3, Value::Bool(v)) => r.names.entry(id).or_insert((None, false)).1 = *v,
                _ => {}
            }
        }
    }
    /// ClientboundCommandSuggestionsPacket: id, range start and length, then
    /// each suggestion's text and optional tooltip.
    fn handle_command_suggestions(&mut self, data: &[u8]) {
        let lang = self
            .remote
            .as_ref()
            .map(|r| r.lang.clone())
            .unwrap_or_default();
        let mut r = sunrise_protocol::buf::Reader::new(data);
        let decoded = (|| {
            let id = r.varint()?;
            let start = r.varint()?;
            let length = r.varint()?;
            let n = r.count(usize::MAX)?;
            let mut list = Vec::with_capacity(n.min(1000));
            for _ in 0..n {
                let text = r.string(32767)?;
                let tooltip = if r.bool()? {
                    Some(plain(&r.nbt()?, &lang))
                } else {
                    None
                };
                list.push((text, tooltip));
            }
            sunrise_protocol::Result::Ok((id, start, length, list))
        })();
        if let Ok((id, start, length, list)) = decoded {
            let range = (start.max(0) as usize, length.max(0) as usize);
            self.chat_server_suggestions(id, range, list.clone());
            self.command_block_server_suggestions(id, range, list);
        }
    }
    fn set_registries(&mut self, registries: Arc<Dynamic>) {
        let entries: Vec<(&str, serde_json::Value)> = registries
            .registries
            .get("minecraft:enchantment")
            .into_iter()
            .flatten()
            .filter_map(|(name, data)| {
                Some((
                    name.as_str(),
                    sunrise_client::scoreboard::tag_to_json(data.as_ref()?),
                ))
            })
            .collect();
        let enchantments = match self
            .stations()
            .enchantments
            .with_entries(entries.iter().map(|(n, v)| (*n, v)))
        {
            Ok(table) => table.map(Arc::new),
            Err(error) => {
                log::warn!("Sunrise: refused enchantment registry: {error}");
                return;
            }
        };
        // The server's biomes the game lacks (a world data pack's) join the
        // game's table, from the data it sent: colours, meshing and the
        // chunk format all go through that table. The window's own server
        // thread set the world's table already.
        if self.integrated.is_none() {
            let table = registries
                .registries
                .get(BIOME_REGISTRY)
                .and_then(|list| client_biomes(self.res.biomes(), list))
                .map(Arc::new);
            if table.is_some() || self.world_biomes.is_some() {
                self.world_biomes_base = table.clone();
                self.world_biomes = table;
                let context = self.worker_context();
                self.streaming.reload_resources(&mut self.world, context);
            }
        }
        let biomes: Vec<u16> = registries
            .registries
            .get(BIOME_REGISTRY)
            .map(|list| {
                list.iter()
                    .map(|(name, _)| world_biomes!(self).id(name))
                    .collect()
            })
            .unwrap_or_default();
        self.server_items = self.server_items(&registries);
        if let Some(r) = self.remote.as_mut() {
            r.enchantments = enchantments;
            let bits = ceil_log2(biomes.len());
            let column = r.chunk_format.layout();
            r.chunk_format =
                Arc::new(ChunkFormat::new(&r.dimension, biomes, bits).in_column(column));
            r.registries = registries;
            r.resolve_clocks();
        }
    }
    /// Switch to a (possibly new) dimension: a fresh client level.
    pub(in crate::game) fn enter_remote_dimension(
        &mut self,
        spawn: &play::SpawnInfo,
        keep_world: bool,
        reason: sunrise_client::level_loading::Reason,
    ) {
        // Every level has the sea level its server says (`CommonPlayerSpawnInfo.seaLevel`): the
        // table may still hold a local world's (a superflat Overworld's).
        if let Ok(key) = sunrise_core::LevelKey::parse(&spawn.dimension) {
            sunrise_world::level::register_sea_level(key, spawn.sea_level);
        }
        self.track_remote_levels(&spawn.dimension);
        // The replacement player starts with its inventory menu, even if the
        // server switched levels without closing the previous container.
        self.remote_close_local();
        self.set_screen(Screen::Playing);
        let Some(r) = self.remote.as_ref() else {
            return;
        };
        // handleRespawn keeps the ClientLevel only for the same level key:
        // two levels sharing a dimension type are still two levels.
        let same = keep_world && r.heard_level.as_deref() == Some(spawn.dimension.as_str());
        let transition_screen = sunrise_client_render::dimension_transition::select(
            mod_api::__private::dimension_key(&spawn.dimension),
            r.heard_level
                .as_deref()
                .map(mod_api::__private::dimension_key),
        );
        if !same {
            self.forget_all_remote_columns();
        }
        let r = self.remote.as_mut().unwrap();
        // A Sunrise server's bulk edit sends whole columns with the light
        // from before its relight; any other server's packet light is final.
        let relight_resent = r.quiet_commands;
        let format = r.level_format(spawn, self.res.environment());
        let (dimension, shape) = (format.dimension, format.shape);
        r.chunk_format = format.chunk_format;
        r.level_facts = format.facts;
        r.level_environment = format.environment;
        r.dimension = dimension;
        r.resolve_clocks();
        self.world.day_time = r.clock_time(r.default_clock);
        r.level = spawn.dimension.clone();
        r.level_key = mod_api::__private::dimension_key(&spawn.dimension);
        r.hashed_seed = spawn.seed;
        r.game_mode = spawn.game_type;
        r.flat = spawn.flat;
        r.loaded_sent = false;
        r.abilities = None;
        // A replacement LevelLoadingScreen starts its bar from nothing (the
        // first one carries on from the server's start).
        if self.player.is_some() {
            self.loading_progress = 0.0;
        }
        // startWaitingForNewLevel: `LevelLoadTracker.startClientLoad` and the
        // `LevelLoadingScreen`.
        r.level_load.start_client_load(client_millis(), reason);
        r.level_load.set_transition_screen(transition_screen);
        // A replacement LocalPlayer has a fresh ambient mood and sendPosition baseline.
        self.ambience.moodiness = 0.0;
        r.position_sender = Default::default();
        r.predictions.clear();
        self.set_mode_id(spawn.game_type);
        self.dim = Dim::from_id(&spawn.dimension);
        self.audio.set_world(&spawn.dimension);
        // ClientPacketListener.handleLogin / handleRespawn: the server's
        // Player.getLastDeathLocation, for the recovery compass.
        self.level.last_death = spawn
            .last_death
            .as_ref()
            .map(|d| (d.dimension.clone(), d.pos));
        if !same {
            // A new ClientLevel: Minecraft.setLevel stops every sound.
            self.stop_sounds_for_new_level();
            let mut streaming = Streaming::new(
                Arc::new(NoTerrain),
                self.worker_context(),
                self.res.threads(),
            );
            streaming.remote = true;
            streaming.radius = self.streaming.radius;
            self.streaming = streaming;
            let (time, day, weather) = (
                self.world.game_time,
                self.world.day_time,
                self.world.weather,
            );
            self.reset_client_world(spawn.seed);
            // Entity IDs belong to this ClientLevel, not the connection. A
            // same-level respawn keeps it; a replacement invalidates handles.
            self.native_lifecycle_open_started();
            self.native_lifecycle_world_ready();
            self.world.shape = shape;
            self.world.game_time = time;
            self.world.day_time = day;
            self.world.weather = weather;
            self.mobs.clear();
            self.clear_remote_objects();
            self.clear_remote_projectiles();
            self.dropped.clear();
            self.clear_client_particles();
            if let Some(r) = self.remote.as_mut() {
                r.entities.clear();
                r.passenger_vehicles.clear();
                r.lerps.clear();
                r.cart_lerps.clear();
                r.stepped.clear();
                r.head_lerps.clear();
                r.pending_items.clear();
                // The entity ids are the old world's; the new dimension's
                // player entities re-map to their profiles on AddEntity.
                r.level_players.clear();
                r.level_player_names.clear();
                r.level_player_textures.clear();
                // A new ClientLevel: a new chunk storage, centred on 0, 0.
                r.chunk_center = (0, 0);
            }
            self.clear_lod();
            self.pending_uploads.clear();
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.clear_sections(self.world.layout());
            }
            // The new ClientLevel is told before setLevel drops the old one.
            let old = self
                .remote
                .as_mut()
                .and_then(|r| r.heard_level.replace(spawn.dimension.clone()));
            self.native_client_level_load(&spawn.dimension);
            if let Some(old) = old {
                self.native_client_level_unload(&old);
            }
        }
        self.streaming.relight_resent = relight_resent;
        let has_sky = self
            .remote
            .as_ref()
            .is_none_or(|r| r.dimension.has_skylight);
        self.streaming.sky_light = has_sky;
        self.streaming.nether_shading = self.nether_lighting();
        // The seed in SpawnInfo is already the hashed one the client's BiomeManager holds.
        self.streaming.biome_zoom = sunrise_core::biome::BiomeZoom::new(spawn.seed);
        let player = self.player.as_ref();
        self.particles
            .detach_tracking_target(TrackingTarget::LocalPlayer, || {
                player.map(player_tracking_body)
            });
        self.drop_remote_player();
        // handleLogin / handleRespawn: LocalPlayer.setPortalCooldown, so a
        // player who arrives standing in a portal gets no overlay from it.
        self.portals.player.cooldown = spawn.portal_cooldown;
        if let Some(profile) = self.remote.as_ref().map(|remote| remote.profile) {
            self.initialize_entity(Holder::Player(profile), "minecraft:player");
        }
        self.finish_remote_level();
    }
=======
>>>>>>> sunrise/main

    /// `handleServerData`: the server list entry of the server joined keeps
    /// the icon the server sends (`ServerList.saveSingleServer`). Single
    /// player has no entry.
    fn remote_server_data(&mut self, icon: Option<&[u8]>) {
        let (Some(r), Some(png)) = (self.remote.as_ref(), icon) else {
            return;
        };
        if self.integrated.is_some() {
            return;
        }
        if let Ok(dir) = std::env::current_dir() {
            sunrise_client::multiplayer::save_server_icon(
                &dir.join("saves"),
                &r.session.address,
                png,
            );
        }
    }

    /// The player's position, for `retainKnownServerState`'s `player.position()`.
    fn player_position(&self) -> [f64; 3] {
        let at = self
            .player
            .as_ref()
            .map(|p| p.position)
            .unwrap_or(Vec3::ZERO);
        [at.x, at.y, at.z]
    }

    /// `Entity.absSnapTo`: the remembered point, clamped to the level's edge.
    /// The velocity is left alone, and the player has no separate previous
    /// position to reset (its box is drawn from `position`).
    fn snap_player_to(&mut self, at: [f64; 3]) {
        let Some(player) = self.player.as_mut() else {
            return;
        };
        let x = at[0].clamp(-3.0e7, 3.0e7);
        let z = at[2].clamp(-3.0e7, 3.0e7);
        player.position = Vec3::new(x, at[1], z);
    }

    /// `Entity.isVehicle` on a client: the entities a tracked entity rides.
    pub(super) fn remote_vehicle_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.remote
            .iter()
            .flat_map(|r| r.entities.values().filter_map(|t| t.vehicle))
    }

    /// The `SetPassengers` bookkeeping of the tracked entities.
    pub(super) fn remote_seat_passengers(&mut self, vehicle: u32, passengers: &[i32]) {
        if let Some(r) = self.remote.as_mut() {
            seat_passengers(&mut r.entities, vehicle, passengers);
        }
    }

<<<<<<< sync-sunrise-1
    pub(super) fn spawn_entity(
        &mut self,
        id: i32,
        kind: &str,
        position: [f64; 3],
        rotation: [f32; 3],
    ) {
        self.detach_remote_tracking_entity(id);
        let [yaw, pitch, head] = rotation;
        let at = Vec3::new(position[0], position[1], position[2]);
        let mob_type = if kind == "minecraft:player" {
            self.remote.as_ref().and_then(|r| r.player_type.clone())
        } else {
            self.res.mob_types().get(kind).cloned()
        };
        if kind == "minecraft:ender_dragon" {
            self.spawn_remote_dragon(id, position, yaw);
        }
        if kind == "minecraft:experience_orb" {
            // Its value (ExperienceOrb.DATA_VALUE) follows in SetEntityData.
            self.orbs.retain(|o| o.id != id as u32);
            self.orbs
                .push(ExperienceOrb::new(id as u32, at, Vec3::ZERO, 1));
        }
        if let Some(t) = mob_type {
            // The window's copy of the server's mob: no mod part goals —
            // their `register_goals` ran on the authoritative side (#1077).
            let mut mob = Mob::new_remote(id as u32, t, at, head);
            super::remote_mob_data::client_defaults(&mut mob);
            (mob.yaw, mob.previous_yaw) = (yaw, yaw);
            (mob.head_pitch, mob.previous_head_pitch) = (pitch, pitch);
            mob.persistent = true;
            self.mobs.retain(|m| m.id != id as u32);
            self.mobs.push(mob);
        }
        if let Some(r) = self.remote.as_mut() {
            // A new entity has a new interpolation handler (a mob's or an
            // armor stand's), even under an id the server used before.
            r.stepped.remove(&id);
            if kind == "minecraft:item" {
                r.pending_items.insert(id, (position, Vec3::ZERO));
            }
            r.entities.insert(
                id,
                Tracked {
                    kind: kind.to_owned(),
                    position,
                    codec_base: position,
                    silent: false,
                    vehicle: None,
                },
            );
        }
    }
    /// A move packet for entity `id`: to the end of `path` (None: a
    /// rotation alone), turned to `rotation` if it has one. `packet` says
    /// whether a living entity more than 64 blocks away is put there at once
    /// (its `snaps`).
    fn move_entity(
        &mut self,
        id: i32,
        path: Option<Path>,
        rotation: Option<(f32, f32)>,
        on_ground: bool,
        packet: MovePacket,
    ) {
        // The dragon is drawn from its own state (game/mobs/dragon.rs), which
        // tick_remote_dragon eases to the server's moves through `lerps`. Its
        // copy in the mob list draws nothing: given the move, it kept the
        // dragon where it spawned (#2083).
        let dragon = self.dragon.as_ref().is_some_and(|d| d.id as i32 == id);
        let driven = self.driven_mount().is_some_and(|m| m.id as i32 == id);
        let mob = self
            .mob_index
            .get_mut(&mut self.mobs, id as u32)
            .filter(|_| !dragon);
        let tracked = self
            .remote
            .as_ref()
            .and_then(|r| r.entities.get(&id))
            .map(|t| t.position);
        let position = path
            .as_ref()
            .map(Path::end)
            .or(tracked)
            .or_else(|| {
                mob.as_ref()
                    .map(|m| [m.position.x, m.position.y, m.position.z])
            })
            .unwrap_or([0.0; 3]);
        if let Some(r) = self.remote.as_mut() {
            if let Some(t) = r.entities.get_mut(&id) {
                t.position = position;
            }
            if driven {
                // handleMoveEntity, handleEntityPositionSync: the mount this
                // client moves (isLocalInstanceAuthoritative) keeps its own
                // position, and no steps pile up for after the ride.
                r.stepped.remove(&id);
                return;
            }
            if let Some(m) = mob {
                // Shulker.createInterpolationHandler is NO_OP, so a move packet
                // is setPos at once. The 6-tick slide starts from the block
                // the shulker occupied before this packet.
                if m.species() == sunrise_client::mobs::Species::Shulker {
                    r.stepped.remove(&id);
                    if path.is_some() {
                        sunrise_client::mobs::shulker::client_set_pos(
                            m,
                            position[0],
                            position[1],
                            position[2],
                        );
                    }
                    if let Some((yaw, pitch)) = rotation {
                        m.yaw += wrap_degrees(yaw - m.yaw);
                        m.head_pitch = (pitch % 360.0).clamp(-90.0, 90.0);
                    }
                    m.on_ground = on_ground;
                    return;
                }
                // A living entity: LivingEntity.createInterpolationHandler's
                // SteppedInterpolationHandler, over its type's update interval.
                let entity = PosRot {
                    position: [m.position.x, m.position.y, m.position.z],
                    yaw: m.yaw,
                    pitch: m.head_pitch,
                };
                let handler = r.stepped.entry(id).or_insert_with(|| {
                    let interval = sunrise_client::lan::tracking::settings(&m.kind.id).1;
                    SteppedInterpolation::new(interval as i32)
                });
                if let Some(at) = stepped_move(handler, packet, entity, position, path, rotation) {
                    // snapTo: setOldPosAndRot too.
                    place_interpolated(m, at);
                    m.previous = m.position;
                    m.previous_yaw = m.yaw;
                    m.previous_head_pitch = m.head_pitch;
                }
                m.on_ground = on_ground;
                return;
            }
            // An armor stand is a living entity too: the same handler, over
            // its type's update interval, stepped in `tick_remote_entities`.
            if let Some(a) = self.armor_stands.iter_mut().find(|a| a.id == id as u32) {
                let entity = PosRot {
                    position: [a.position.x, a.position.y, a.position.z],
                    yaw: a.yaw,
                    pitch: 0.0,
                };
                let handler = r.stepped.entry(id).or_insert_with(|| {
                    let interval =
                        sunrise_client::lan::tracking::settings("minecraft:armor_stand").1;
                    SteppedInterpolation::new(interval as i32)
                });
                if let Some(at) = stepped_move(handler, packet, entity, position, path, rotation) {
                    a.snap(
                        Vec3::new(at.position[0], at.position[1], at.position[2]),
                        at.yaw,
                    );
                }
                a.on_ground = on_ground;
                return;
            }
            // Interaction extends Entity and keeps its NO_OP interpolation
            // handler: the server's position replaces its own at once.
            if let Some(i) = self.interactions.get_mut(&id) {
                if path.is_some() {
                    i.position = Vec3::new(position[0], position[1], position[2]);
                }
                return;
            }
            // Projectiles keep Entity's NO_OP InterpolationHandler too: the
            // server's position replaces theirs at once and their own tick
            // flies them on (`tick_remote_projectiles`). Eased towards it
            // instead, they only moved when an update came, every 10 to 20
            // ticks.
            if let Some(i) = self
                .creatures
                .projectiles
                .iter()
                .position(|p| p.id == id as u32)
            {
                self.creatures.projectiles.update(i, |p| {
                    if path.is_some() {
                        p.position = Vec3::new(position[0], position[1], position[2]);
                    }
                    if let Some((yaw, pitch)) = rotation {
                        p.yaw = yaw;
                        p.pitch = pitch;
                    }
                });
                return;
            }
            if path.is_some()
                && (self.potions.move_remote_thrown(
                    id as u32,
                    Vec3::new(position[0], position[1], position[2]),
                ) || self.end.move_remote_eye(id as u32, position))
            {
                return;
            }
            // A fishing hook keeps a LinearInterpolationHandler: eased towards the
            // update over three ticks, on top of its own movement (`tick_remote_hooks`).
            if path.is_some() {
                if let Some(h) = self.hook.as_mut().filter(|h| h.id() == id as u32) {
                    h.move_to(position, on_ground);
                    return;
                }
                if let Some(h) = self.remote_hooks.iter_mut().find(|h| h.id() == id as u32) {
                    h.move_to(position, on_ground);
                    return;
                }
            }
            // A thrown trident is an arrow: placed at once, flown on by its tick.
            if let Some(t) = self.tridents.iter_mut().find(|t| t.id == id as u32) {
                if path.is_some() {
                    t.position = Vec3::new(position[0], position[1], position[2]);
                }
                if let Some((yaw, pitch)) = rotation {
                    t.yaw = yaw;
                    t.pitch = pitch;
                }
                return;
            }
            // So does a firework rocket: its own tick flies it on.
            if path.is_some()
                && let Some(f) = self.rockets.iter_mut().find(|f| f.id == id as u32)
            {
                f.position = Vec3::new(position[0], position[1], position[2]);
                return;
            }
            if let Some(p) = r.pending_items.get_mut(&id) {
                p.0 = position;
            }
            // ItemEntity, FallingBlockEntity and PrimedTnt keep Entity's
            // NO_OP InterpolationHandler: moveOrInterpolateTo puts them there
            // at once, and their own tick moves them on between the server's
            // updates (`tick_remote_entities`, `tick_remote_projectiles`).
            let at = Vec3::new(position[0], position[1], position[2]);
            if let Some(d) = self.dropped.iter_mut().find(|d| d.id == id as u32) {
                d.position = at;
                d.on_ground = on_ground;
                return;
            }
            if let Some(i) = self.falling.iter().rposition(|f| f.id == id as u32) {
                self.falling.update(i, |f| f.position = at);
                return;
            }
            if let Some(t) = self.tnts.iter_mut().rev().find(|t| t.id == id as u32) {
                t.position = at;
                t.on_ground = on_ground;
                return;
            }
            // ExperienceOrb.createInterpolationHandler is a
            // LinearInterpolationHandler: the correction is spread over 3
            // ticks (`tick_remote_entities`), while `ExperienceOrb.tick` keeps
            // moving it.
            if let Some(o) = self.orbs.iter_mut().find(|o| o.id == id as u32) {
                o.on_ground = on_ground;
                r.lerps.insert(
                    id,
                    Lerp {
                        target: position,
                        rotation: None,
                        steps: 3,
                    },
                );
                return;
            }
            if r.cart_lerps.contains_key(&id) {
                // A minecart on NewMinecartBehavior keeps Entity's NO_OP
                // InterpolationHandler: a teleport puts it there at once.
                if let Some(i) = self.carts.iter().position(|c| c.id == id as u32) {
                    self.carts.update(i, |c| {
                        if path.is_some() {
                            c.position = at;
                        }
                        if let Some((yaw, pitch)) = rotation {
                            c.yaw = yaw;
                            c.pitch = pitch;
                        }
                    });
                }
            } else {
                // AbstractInterpolationHandler.interpolateTo: without a
                // rotation, the one it was heading for.
                let rotation = rotation.or_else(|| r.lerps.get(&id).and_then(|l| l.rotation));
                r.lerps.insert(
                    id,
                    Lerp {
                        target: position,
                        rotation,
                        steps: 3,
                    },
                );
            }
        }
        self.turn_remote_object(id, rotation.map(|(yaw, _)| yaw), on_ground);
        self.move_remote_text_display(id, position, rotation);
        self.turn_remote_projectile(id, rotation);
    }
    pub(super) fn detach_remote_tracking_entity(&mut self, id: i32) {
        let mobs = &self.mobs;
        let index = &mut self.mob_index;
        self.particles
            .detach_tracking_target(TrackingTarget::Entity(id), || {
                index.get(mobs, id as u32).map(mob_tracking_body)
            });
    }

    /// Map a menu slot to Sunrise's inventory screen slot ids.
    // Starling: open to the game, for the old helper native mods call (`menu_slot_old`).
    pub(in crate::game) fn inventory_screen_slot(menu: &str, slot: i32) -> Option<usize> {
        let s = usize::try_from(slot).ok()?;
        // A container of `n` slots, then the inventory and the hotbar.
        let slots = |n: usize| -> Option<usize> {
            match s {
                _ if s < n => Some(300 + s),
                _ if s < n + 27 => Some(9 + s - n),
                _ if s < n + 36 => Some(s - n - 27),
                _ => None,
            }
        };
        let rows = |n: usize| slots(n * 9);
        match menu {
            "inventory" => match s {
                0 => Some(200),
                1..=4 => Some(100 + s - 1),
                5..=8 => Some(500 + s - 5),
                9..=35 => Some(s),
                36..=44 => Some(s - 36),
                // InventoryMenu.SHIELD_SLOT: the off hand.
                45 => Some(504),
                _ => None,
            },
            "minecraft:crafting" => match s {
                0 => Some(200),
                1..=9 => Some(100 + s - 1),
                10..=36 => Some(9 + s - 10),
                37..=45 => Some(s - 37),
                _ => None,
            },
            "minecraft:merchant" => match s {
                0 | 1 => Some(super::stations::INPUT + s),
                2 => Some(super::stations::RESULT),
                3..=29 => Some(9 + s - 3),
                30..=38 => Some(s - 30),
                _ => None,
            },
            "minecraft:furnace" | "minecraft:blast_furnace" | "minecraft:smoker" => match s {
                0..=2 => Some(300 + s),
                3..=29 => Some(9 + s - 3),
                30..=38 => Some(s - 30),
                _ => None,
            },
            "minecraft:generic_9x1" => rows(1),
            "minecraft:generic_9x2" => rows(2),
            "minecraft:generic_9x3" | "minecraft:shulker_box" => rows(3),
            "minecraft:generic_9x4" => rows(4),
            "minecraft:generic_9x5" => rows(5),
            "minecraft:generic_9x6" => rows(6),
            // HopperMenu; DispenserMenu (dispensers and droppers).
            "minecraft:hopper" => slots(5),
            "minecraft:generic_3x3" => slots(9),
            "minecraft:brewing_stand" => slots(5),
            // CrafterMenu: the grid, the inventory, then the result (the
            // client's own).
            "minecraft:crafter_3x3" => slots(9),
            // HorseInventoryMenu: the saddle, the body armour, the chest,
            // then the inventory.
            m if m.starts_with(MOUNT_MENU) => {
                let n = m[MOUNT_MENU.len()..].parse::<usize>().ok()? * 3;
                match s {
                    0 => Some(super::mount_menu::SADDLE),
                    1 => Some(super::mount_menu::ARMOR),
                    _ if s < 2 + n => Some(super::mount_menu::CHEST + s - 2),
                    _ if s < 2 + n + 27 => Some(9 + s - 2 - n),
                    _ if s < 2 + n + 36 => Some(s - 2 - n - 27),
                    _ => None,
                }
            }
            // A content mod's menu type: its container, then the inventory.
            // A workstation: its inputs, its result, then the inventory.
            m => {
                if let Some(t) = sunrise_client::containers::mod_menu_type(m) {
                    return slots(t.slots);
                }
                let kind = Self::remote_station_kind(m)?;
                let inputs = kind.inputs();
                let own = inputs + usize::from(kind.has_result());
                match s {
                    _ if s < inputs => Some(super::stations::INPUT + s),
                    _ if s < own => Some(super::stations::RESULT),
                    _ if s < own + 27 => Some(9 + s - own),
                    _ if s < own + 36 => Some(s - own - 27),
                    _ => None,
                }
            }
        }
    }
    /// Find a game slot in the menu's complete, contiguous slot mapping.
    pub(super) fn menu_slot_index(menu: &str, internal: usize) -> Option<usize> {
        (0..)
            .map_while(|slot| Self::inventory_screen_slot(menu, slot))
            .position(|slot| slot == internal)
    }
    /// The workstation screen of a menu type a server opens.
    pub(super) fn remote_station_kind(menu: &str) -> Option<super::stations::Kind> {
        use super::stations::Kind;
        Some(match menu {
            "minecraft:loom" => Kind::Loom,
            "minecraft:stonecutter" => Kind::Stonecutter,
            "minecraft:grindstone" => Kind::Grindstone,
            "minecraft:anvil" => Kind::Anvil,
            "minecraft:smithing" => Kind::Smithing,
            "minecraft:enchantment" => Kind::Enchanting,
            "minecraft:beacon" => Kind::Beacon,
            "minecraft:cartography_table" => Kind::Cartography,
            _ => return None,
        })
    }
    fn open_menu_name(&self, container: i32) -> String {
        match self.remote.as_ref().and_then(|r| r.menu.as_ref()) {
            Some((id, name)) if *id == container && container != 0 => name.clone(),
            _ => "inventory".into(),
        }
    }
    fn set_menu_slot(&mut self, container: i32, slot: i32, item: Option<ItemStack>) {
        let menu = self.open_menu_name(container);
        if menu == "minecraft:lectern" {
            if slot == 0 {
                self.remote_lectern_book(item);
            }
            return;
        }
        if container != 0 && menu == "inventory" {
            return;
        }
        let Some(internal) = Self::inventory_screen_slot(&menu, slot) else {
            return;
        };
        if internal == 200 {
            // ClientboundContainerSetSlotPacket for the open menu's slot 0: the
            // server's result of its own recipes (a world's data packs' too).
            if container == self.remote_menu().0
                && let Some(r) = self.remote.as_mut()
            {
                r.craft_result = item;
            }
            return;
        }
        let slot = self.slot_mut(internal);
        let changed = !super::hand::matches_for_swap(slot.as_ref(), item.as_ref());
        *slot = item;
        if internal < 36 {
            self.remote_hand_slot_set(internal, changed);
        } else if internal == 504 {
            self.remote_hand_slot_set(super::shield::OFFHAND_SLOT, changed);
        }
        if internal >= super::stations::INPUT && self.station.is_some() {
            self.remote_station_slot_changed(internal);
        }
    }
=======
    // move_entity: remote/motion.rs
>>>>>>> sunrise/main

    pub(in crate::game) fn apply_packet(&mut self, packet: C) -> Result<()> {
        self.observe_client_packet(&packet);
        match packet {
            // ClientCommonPacketListenerImpl.handlePing runs on the game
            // thread, after all preceding packets (especially teleports).
            C::Ping(p) => self.send(S::Pong(p)),
            C::CustomPayload(p) if p.channel == super::cauldron_prediction::ENTRIES_CHANNEL => {
                let entries = super::cauldron_prediction::decode_registrations(&p.data)?;
                if let Some(remote) = &mut self.remote {
                    remote.cauldron_entries = Some(entries);
                }
            }
            C::CustomPayload(p) if p.channel == super::cauldron_prediction::USE_CHANNEL => {
                self.receive_cauldron_use(&p.data)?;
            }
            // The server's next generation of mod registries: all of it or the connection.
            C::CustomPayload(p) if p.channel == sunrise_client::datapack_net::CHANNEL => {
                sunrise_client::datapack_net::accept_chunk(&p.data)?;
            }
            C::CustomPayload(p)
                if p.channel == sunrise_client::lan::commands::selector_sync::CHANNEL =>
            {
                self.receive_selector_tokens(&p.data)?;
            }
            // Another level's world: its open, its packets and its close (`viewed_worlds`).
            C::CustomPayload(p) if p.channel == sunrise_protocol::viewed_levels::CHANNEL => {
                self.viewed_levels_payload(&p.data)?;
            }
            // Private custom recipe-book settings and placement protocol.
            C::CustomPayload(p)
                if p.channel == sunrise_client::recipe_book::settings_sync::CHANNEL =>
            {
                self.remote_book_settings(&p.data);
            }
            // A native mod's general row the server starts showing the window.
            C::CustomPayload(p) if p.channel == sunrise_protocol::general_entity::CHANNEL => {
                self.remote_general_entity(&p.data);
            }
            // The collider boxes of an entity the server shows the window.
            C::CustomPayload(p) if p.channel == sunrise_protocol::collider_sync::CHANNEL => {
                self.remote_collider_sync(&p.data);
            }
            // Per-position connected-texture groups synced for the window's chunks.
            C::CustomPayload(p) if p.channel == sunrise_protocol::connection_groups::CHANNEL => {
                self.refuse_play_payload("connection groups: payload outside its bundle")?;
            }
            // Full-width data slots for the open blockless mod menu.
            C::CustomPayload(p) if p.channel == sunrise_protocol::menu_extra::DATA_CHANNEL => {
                use sunrise_protocol::menu_extra::{DataSlot, LIVE_DATA_REVISION};
                if self
                    .remote
                    .as_ref()
                    .is_some_and(|r| r.registries.menu_extra_revision >= LIVE_DATA_REVISION)
                    && let Ok(slot) = DataSlot::decode(&p.data)
                {
                    self.native_screen_data(slot.window, usize::from(slot.index), slot.value);
                }
            }
            // A mod menu type's opening context, just before its OpenScreen.
            C::CustomPayload(p) if p.channel == sunrise_protocol::menu_extra::CHANNEL => {
                self.receive_menu_extra(&p.data)?;
            }
            C::CustomPayload(p) if sunrise_client::lan::recipes::viewer_channel(&p.channel) => {
                self.remote_recipe_viewer(&p.channel, &p.data);
            }
            // An anvil cost the data-slot short cannot hold, after that short.
            C::CustomPayload(p) if p.channel == sunrise_protocol::anvil_cost::CHANNEL => {
                self.remote_anvil_wide_cost(&p.data);
            }
            // The server's recipes its native mods asked to send.
            C::CustomPayload(p) if p.channel == sunrise_client::lan::recipes::CONTENT_CHANNEL => {
                self.remote_recipe_content(&p.data);
            }
            // Starling: the old mod messages (old_mod_net.rs).
            C::CustomPayload(p) if p.channel == sunrise_protocol::mod_net::CHANNEL => {
                self.remote_old_mod_net(&p.data);
            }
            // The passages of the chunks the server shows the window.
            C::CustomPayload(p) if p.channel == sunrise_protocol::passage_sync::CHANNEL => {
                self.remote_passage_sync(&p.data);
            }
            // PingDebugMonitor.onPongReceived.
            C::PongResponse(p) => {
                let delay = sunrise_client::net::ping_millis().wrapping_sub(p.time);
                self.debug_charts.ping.log(delay);
            }
<<<<<<< sync-sunrise-1
            C::Login(p) => {
                // A client's parts, from the tags it plays with.
                if let Err(e) = self.resolve_parts() {
                    log::warn!("Sunrise: {e}");
                }
                // Configuration's tags, over the block ids the parts just bound.
                self.install_remote_tags();
                self.refresh_render_visibility();
                self.test_highlights.clear();
                self.forget_synced_recipes();
                self.level.hardcore = p.hardcore;
                log::info!(
                    "Sunrise: joined as entity {}, game mode {}",
                    p.player_id,
                    p.spawn.game_type
                );
                // handleLogin: startWaitingForNewLevel with Reason.OTHER, which
                // replaces ServerReconfigScreen.
                if let Some(r) = self.remote.as_mut() {
                    r.reconfiguring = None;
                    r.attachments.clear();
                    r.viewed.clear();
                    // A new ClientPacketListener owns a fresh clock manager.
                    r.clocks.clear();
                    r.clock_game_time = 0;
                    self.world.game_time = 0;
                }
                // handleLogin makes a new MultiPlayerGameMode (carried index 0) and a
                // new LocalPlayer (slot 0, no effects, riding nothing: the server
                // pairs a vehicle again with ClientboundSetPassengersPacket).
                self.selected = 0;
                self.potions.effects.clear();
                self.riding = None;
                if let Some(player) = self.player.as_mut() {
                    player.entity_id = p.player_id;
                }
                if let Some(r) = self.remote.as_mut() {
                    r.player_attributes.clear();
                    r.carried.set(Some(0));
                    // handleLogin makes a new ClientLevel: its predictions count from 0.
                    r.sequence = 0;
                    r.own_invisible = false;
                    r.own_on_fire = false;
                    r.online_mode = p.online_mode;
                    r.levels.clone_from(&p.levels);
                    r.enforces_secure_chat = p.enforces_secure_chat;
                    r.entity_id = p.player_id;
                    r.local_entity_silent = false;
                    // handleLogin: the new LocalPlayer is the camera.
                    r.camera = None;
                    r.logged_in = true;
                    r.view_distance = p.chunk_radius;
                    r.simulation_distance = p.simulation_distance;
                    r.show_death_screen = p.show_death_screen;
                    r.reduced_debug_info = p.reduced_debug_info;
                    r.do_limited_crafting = p.limited_crafting;
                }
                self.bind_local_player_residency();
                self.refresh_debug_renderers();
                // LocalPlayer.setReducedDebugInfo always rebuilds, also on a second login.
                self.debug_entries.invalidate();
                self.streaming.radius = self
                    .settings
                    .render_distance
                    .min(p.chunk_radius.saturating_add(1))
                    .max(2);

                // handleLogin: startWaitingForNewLevel(.., Reason.OTHER).
                self.enter_remote_dimension(
                    &p.spawn,
                    false,
                    sunrise_client::level_loading::Reason::Other,
                );
                // No ClientInformation here: the configuration phase sent it,
                // and ClientPacketListener.broadcastClientInformation sends it
                // again only when an option changes it.
                self.remote_chat_login();
                if let Some(remote) = &self.remote {
                    self.native_client_player_logging_in(p.player_id, remote.profile);
                }
            }
            C::Respawn(p) => {
                // The new LocalPlayer copies reduced debug info through its setter.
                self.debug_entries.invalidate();
                let old_player = self.remote.as_ref().map(|r| (r.entity_id, r.profile));
                if let Some((_, uuid)) = old_player {
                    self.forget_entity_initialization(Holder::Player(uuid));
                }
                if let Some(r) = self.remote.as_ref() {
                    self.audio.remove_sound_entity(r.entity_id as u32);
                }
                // A new player, without effects (the server sends them
                // again) and with no item in use.
                self.potions.effects.clear();
                // Every synced attachment but the player's is sent again; the viewed
                // levels go or stay with the level entered (`enter_respawn_level`).
                if let Some(r) = self.remote.as_mut() {
                    r.attachments
                        .retain(|holder, _| matches!(holder, Holder::Player(_)));
                }
                self.using = None;
                self.end_mod_use();
                // After a death (no KEEP_ENTITY_DATA) the new LocalPlayer
                // rides nothing: the server pairs a vehicle again with its
                // passengers. A dimension change keeps the vehicle it travels in.
                if p.keep & 2 == 0 {
                    self.riding = None;
                    // KEEP_ENTITY_DATA copies the old player's synched data
                    // (its shared flags) to the new one; without it they reset.
                    if let Some(r) = self.remote.as_mut() {
                        r.own_invisible = false;
                        r.own_on_fire = false;
                        r.local_entity_silent = false;
                    }
                }
                // handleRespawn compares the levels' keys: a mod's level is
                // its own dimension even where `Dim` has no name for it.
                let dimension_changed = self
                    .remote
                    .as_ref()
                    .is_some_and(|r| r.level != p.spawn.dimension);
                let died = self.screen == Screen::Dead
                    || self
                        .player
                        .as_ref()
                        .is_some_and(|m| m.health.health() <= 0.0);
                let reason = sunrise_client::level_loading::reason_for_respawn(
                    died,
                    &p.spawn.dimension,
                    self.dim.id(),
                );
                if dimension_changed {
                    sunrise_client::spans::startup::arm("dimension.change");
                }
                // handleRespawn makes a new ClientLevel for another dimension
                // key (a proxy's server switch too): its predictions count from 0.
                if let Some(r) = self.remote.as_mut()
                    && r.level != p.spawn.dimension
                {
                    r.sequence = 0;
                }
                self.portals.respawn_attributes.respawn(p.keep & 1 != 0);
                self.enter_respawn_level(&p, reason);
                // handleRespawn adds the new LocalPlayer after what the kept
                // level holds; a new level holds nothing yet.
                let before: std::collections::HashSet<u32> =
                    self.mobs.iter().map(|m| m.id).collect();
                if let Some(r) = self.remote.as_mut() {
                    r.ticked_before_player = before;
                }
                if dimension_changed {
                    // LevelExtractor.setLevel: a new level drops the highlights.
                    self.test_highlights.clear();
                    let (music, _) = self.situational_music();
                    self.music.stop_playing(music.as_ref(), &mut self.audio);
                }
                self.set_screen(Screen::Playing);
                if let Some(r) = self.remote.as_mut() {
                    r.death_message = None;
                    r.health = (20.0, 20, 5.0);
                    // handleRespawn: the new LocalPlayer is the camera.
                    r.camera = None;
                }
                self.capture(true);
                if let Some((old_id, uuid)) = old_player {
                    self.native_client_player_clone(
                        old_id,
                        self.own_entity_id(),
                        uuid,
                        died,
                        dimension_changed,
                    );
                }
            }
=======
            C::DebugSample(p) => self.debug_charts.log_remote_sample(&p.sample)?,
            C::Login(p) => self.apply_login_packet(p),
            C::Respawn(p) => self.apply_respawn_packet(p),
>>>>>>> sunrise/main
            C::PlayerPosition(p) => self.remote_teleport(&p)?,
            C::PlayerRotation(p) => self.remote_rotation(&p),
            C::PlayerLookAt(p) => self.remote_look_at(&p),
            // ClientCommonPacketListenerImpl.handlePostEffects.
            C::PostEffects(p) => self.post_effects = p.effects,
            C::SetCamera(p) => self.remote_set_camera(p.camera_id),
            C::LevelChunkWithLight(p) => self.remote_chunk(&p, None),
            C::LightUpdate(p) => self.remote_light_update(&p),
            C::ChunksBiomes(p) => self.remote_chunks_biomes(&p),
            C::ForgetLevelChunk(p) => self.apply_forget_level_chunk_packet(p),
            C::BlockUpdate(p) => self.server_block(p.pos, p.state, None),
            C::AddTransientBlock(p) => self.add_transient_block(p.pos, p.state),
            C::BlockDestruction(p) => self.apply_block_destruction_packet(p),
            C::SectionBlocksUpdate(p) => {
                self.apply_section_blocks_update_packet(&p, Default::default())
            }
            C::BlockChangedAck(p) => self.end_predictions(p.sequence)?,
            C::BlockEvent(p) => self.apply_block_event_packet(p)?,
            // ClientLevel.levelEvent: LevelEventHandler, as this client's own
            // level plays them (particles and sounds: a spawner's burst, bone
            // meal, waxing, a block broken by someone else).
            // ClientPacketListener.handleLevelEvent: a global one is heard
            // from anywhere (ClientLevel.globalLevelEvent).
            C::LevelEvent(p) if p.global => self.global_level_event_sound(p.kind, p.pos),
            C::LevelEvent(p) => self.level_event_at(p.kind, p.pos, p.data),
            C::LevelParticles(p) => self.remote_level_particles(&p),
            C::Explode(p) => self.remote_explode(&p),
            C::Sound(p) => self.apply_sound_packet(p),
            C::SoundEntity(p) => self.apply_sound_entity_packet(p),
            C::StopSound(p) => self
                .audio
                .stop_sounds(p.sound.as_deref(), p.source.map(|s| s as usize)),
            C::AddEntity(p) => self.apply_add_entity_packet(p),
            C::RemoveEntities(p) => self.apply_remove_entities_packet(p),
            C::MoveEntityPos(play::MoveEntityPos(m))
            | C::MoveEntityPosRot(play::MoveEntityPosRot(m))
            | C::MoveEntityRot(play::MoveEntityRot(m)) => {
                // handleMoveEntity: the delta counts from the position codec's
                // base, which then moves to the delta's end.
                let base = self
                    .remote
                    .as_ref()
                    .and_then(|r| r.entities.get(&m.id))
                    .map(|t| t.codec_base);
                let Some(base) = base else { return Ok(()) };
                // VecDeltaCodec.decode: each step's position from the last.
                let path = m.delta.as_ref().map(|d| match d {
                    play::VecDelta::Linear(_) => Path::Linear(d.end(base)),
                    play::VecDelta::Stepped(steps) => {
                        let mut at = base;
                        Path::Stepped(
                            steps
                                .iter()
                                .map(|(delta, ticks)| {
                                    at = play::VecDelta::apply(at, *delta);
                                    (at, *ticks)
                                })
                                .collect(),
                        )
                    }
                });
                let rotation = m.rotation.map(|(y, x)| {
                    (
                        sunrise_protocol::buf::unpack_degrees(y),
                        sunrise_protocol::buf::unpack_degrees(x),
                    )
                });
                if let (Some(path), Some(t)) = (
                    &path,
                    self.remote.as_mut().and_then(|r| r.entities.get_mut(&m.id)),
                ) {
                    t.codec_base = path.end();
                }
                self.remote_move_entity_and_file(m.id, path, rotation, m.on_ground);
            }
            C::EntityPositionSync(p) => self.apply_entity_position_sync_packet(p),
            C::TeleportEntity(p) => self.apply_teleport_entity_packet(p)?,
            C::SetPassengers(p) => self.remote_set_passengers(p.vehicle, p.passengers),
            C::SetEntityLink(p) => self.apply_set_entity_link_packet(p),
            C::MoveVehicle(p) => self.apply_move_vehicle_packet(p),
            C::MoveMinecartAlongTrack(p) => self.remote_move_minecart(p),
<<<<<<< sync-sunrise-1
            // ClientPacketListener.handleRotateMob: lerpHeadTo(yHeadRot, 3).
            C::RotateHead(p) => {
                if let Some(r) = self.remote.as_mut() {
                    let yaw = sunrise_protocol::buf::unpack_degrees(p.head_yaw);
                    r.head_lerps.insert(p.id, (yaw, 3));
                }
            }
            C::SetEntityData(p) => {
                // Any entity's custom name (armor stand holograms, objects).
                self.remote_names_and_displays(p.id, &p.values);
                if let Some(r) = self.remote.as_mut() {
                    for (index, _, value) in &p.values {
                        if let (4, Value::Bool(silent)) = (index, value) {
                            if p.id == r.entity_id {
                                r.local_entity_silent = *silent;
                            }
                            if let Some(t) = r.entities.get_mut(&p.id) {
                                t.silent = *silent;
                            }
                        }
                    }
                }
                // Entity.DATA_SHARED_FLAGS_ID, DATA_CUSTOM_NAME and
                // DATA_CUSTOM_NAME_VISIBLE: server NPCs and holograms rely on
                // these instead of saved NBT.
                if let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32) {
                    for (index, _, value) in &p.values {
                        match (index, value) {
                            (0, Value::Byte(flags)) => {
                                m.invisible = flags & 0x20 != 0;
                                m.glowing = flags & 0x40 != 0;
                            }
                            (2, Value::OptText(name)) => {
                                let text = name.as_ref().map(sunrise_client::entity_nbt::name_text);
                                m.custom_name =
                                    text.as_ref().map(sunrise_client::text::Text::plain);
                                m.custom_name_text = text;
                            }
                            (2, Value::Text(name)) => {
                                let text = sunrise_client::entity_nbt::name_text(name);
                                m.custom_name = Some(text.plain());
                                m.custom_name_text = Some(text);
                            }
                            (3, Value::Bool(visible)) => m.name_visible = *visible,
                            _ => {}
                        }
                    }
                }
                if let Some((_, _, Value::Byte(flags))) = p.values.iter().find(|v| v.0 == 0)
                    && let Some(r) = self.remote.as_mut()
                {
                    if flags & 0x40 != 0 {
                        r.glowing.insert(p.id);
                    } else {
                        r.glowing.remove(&p.id);
                    }
                }
                self.remote_mob_data(p.id, &p.values);
                if let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32)
                    && m.species() == sunrise_client::mobs::Species::Creeper
                {
                    for (index, _, value) in &p.values {
                        match (index, value) {
                            // Creeper.DATA_SWELL_DIR, DATA_IS_POWERED, DATA_IS_IGNITED.
                            (16, Value::Int(dir)) => m.extra.counter = *dir,
                            (17, Value::Bool(powered)) => m.extra.charged = *powered,
                            (18, Value::Bool(ignited)) => m.extra.flag = *ignited,
                            _ => {}
                        }
                    }
                }
                if let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32) {
                    for (index, _, value) in &p.values {
                        match (index, value) {
                            // Entity.DATA_TICKS_FROZEN: isFullyFrozen shakes it.
                            (7, Value::Int(ticks)) => m.ticks_frozen = *ticks,
                            // Strider.DATA_SUFFOCATING (after AgeableMob's two
                            // and DATA_BOOST_TIME): a cold strider shakes.
                            (19, Value::Bool(cold))
                                if m.species() == sunrise_client::mobs::Species::Strider =>
                            {
                                m.extra.flag = *cold;
                            }
                            _ => {}
                        }
                    }
                }
                // LivingEntity.DATA_LIVING_ENTITY_FLAGS: for another player
                // too, remote_mob_data's apply reads index 8 above.
                let mannequin = self
                    .mobs
                    .iter()
                    .any(|m| m.id == p.id as u32 && &*m.kind.id == "minecraft:mannequin");
                let other = mannequin
                    || self.remote.as_ref().is_some_and(|r| {
                        r.level_players.contains_key(&p.id) && r.entity_id != p.id
                    });
                // LivingEntity.onSyncedDataUpdated(SLEEPING_POS_ID): another
                // player lies down in their bed (drawn lying in it), or gets up.
                if other {
                    for (index, _, value) in &p.values {
                        if let (14, Value::OptPos(bed)) = (index, value) {
                            let code = bed.map_or(0, |b| self.bed_facing(b));
                            let at = bed.and_then(|head| self.sleeping_position(head));
                            if let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32) {
                                m.extra.ints[1] = code;
                                if bed.is_some() {
                                    // setPosToBed leaves the server's position alone
                                    // when the client's bed has no sleep shape.
                                    if let Some(at) = at {
                                        m.position = at;
                                        m.previous = at;
                                    }
                                    m.head_pitch = 0.0;
                                    m.previous_head_pitch = 0.0;
                                }
                            }
                        }
                        // Avatar.DATA_PLAYER_MAIN_HAND (HumanoidArm: 0 left, 1
                        // right): the arm a swing of the main hand moves.
                        if let (15, Value::Id(arm)) = (index, value)
                            && let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32)
                        {
                            m.extra.main_hand_left = *arm == 0;
                        }
                        // Avatar.DATA_PLAYER_MODE_CUSTOMISATION: the skin layers
                        // the player shows (PlayerModelPart bits).
                        if let (16, Value::Byte(bits)) = (index, value)
                            && let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32)
                        {
                            m.extra.hidden_model_parts = !(*bits as u8) & 0x7F;
                        }
                    }
                }
                // LivingEntity.onSyncedDataUpdated(SLEEPING_POS_ID): this
                // player lies down in the bed, or gets up.
                if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) {
                    for (index, _, value) in &p.values {
                        match (index, value) {
                            (14, Value::OptPos(bed)) => {
                                if bed.is_some() && self.sleeping.is_none() {
                                    self.sleep_timer = 0;
                                }
                                self.sleeping = *bed;
                                if let Some(head) = *bed {
                                    // LivingEntity.onSyncedDataUpdated(SLEEPING_POS_ID): setPosToBed.
                                    let at = self.sleeping_position(head);
                                    if let Some(pl) = self.player.as_mut() {
                                        if let Some(at) = at {
                                            pl.position = at;
                                        }
                                        pl.velocity = Vec3::ZERO;
                                        pl.pitch = 0.0;
                                    }
                                    if let Some(at) = at {
                                        self.previous_position = at;
                                    }
                                    self.pitch = 0.0;
                                    self.snap_player_model_pitch();
                                }
                            }
                            // Entity.DATA_SHARED_FLAGS_ID's sprinting bit: the
                            // server stops the sprint after a hit's extra
                            // knockback (the flag only, as assignValues sets
                            // it: the speed modifier and wasSprinting stay).
                            // Its invisible bit (Entity.isInvisible): the
                            // Invisibility effect's, or a plugin's that hides
                            // the real body under a pose (#3788).
                            (0, Value::Byte(flags)) => {
                                if let Some(pl) = self.player.as_mut() {
                                    pl.sprinting = flags & 0x08 != 0;
                                    // Entity.DATA_SHARED_FLAGS_ID's fall-flying
                                    // bit (0x80, `Entity.setSharedFlag(7, ...)`):
                                    // `LivingEntity.updateFallFlying` clears the
                                    // glide on the server only and `LocalPlayer`
                                    // never ends one itself, so this packet is
                                    // the glide's own end. Applied when it is
                                    // handled — the frame's poll, before its
                                    // ticks — as ClientPacketListener does, so a
                                    // clear that arrives mid-tick lets that tick
                                    // keep its glide travel (#7616).
                                    pl.fall_flying = flags & -128 != 0;
                                }
                                let on_fire = flags & 0x01 != 0;
                                if let Some(r) = self.remote.as_mut() {
                                    r.own_invisible = flags & 0x20 != 0;
                                    let was_on_fire =
                                        std::mem::replace(&mut r.own_on_fire, on_fire);
                                    // The server put the fire out: the
                                    // window's countdown ends with it.
                                    if was_on_fire
                                        && !on_fire
                                        && let Some(pl) = self.player.as_mut()
                                    {
                                        pl.fire_ticks = pl.fire_ticks.min(0);
                                    }
                                }
                            }
                            // Entity.DATA_AIR_SUPPLY_ID and
                            // Player.DATA_PLAYER_ABSORPTION_ID: the bubbles
                            // and golden hearts the server's player has.
                            (1, Value::Int(air)) => {
                                if let Some(pl) = self.player.as_mut() {
                                    pl.air = *air;
                                }
                            }
                            // Entity.DATA_TICKS_FROZEN: the server's count of the
                            // powder snow freeze, which the client's own copy reads
                            // for the frost overlay, the frozen hearts and the
                            // shiver of its third-person model (#11217).
                            (7, Value::Int(ticks)) => {
                                if let Some(pl) = self.player.as_mut() {
                                    pl.ticks_frozen = *ticks;
                                }
                            }
                            // LivingEntity.DATA_ARROW_COUNT_ID and DATA_STINGER_COUNT_ID.
                            (12, Value::Int(count)) => {
                                if let Some(r) = self.remote.as_mut() {
                                    r.own_arrows = *count;
                                }
                            }
                            (13, Value::Int(count)) => {
                                if let Some(r) = self.remote.as_mut() {
                                    r.own_stingers = *count;
                                }
                            }
                            (19 | 20, Value::Id(v)) => {
                                if let Some(r) = self.remote.as_mut() {
                                    r.own_shoulders[usize::from(*index == 20)] =
                                        (*v > 0).then(|| (*v - 1).clamp(0, 4));
                                }
                            }
                            (17, Value::Float(amount)) => {
                                if let Some(pl) = self.player.as_mut() {
                                    let _ = pl.internal_set_absorption(*amount);
                                }
                            }
                            // LivingEntity.DATA_LIVING_ENTITY_FLAGS: the server
                            // starts or stops this player's use, as an axe that
                            // disables the raised shield does (Player.disableShield's
                            // stopUsingItem). LocalPlayer.onSyncedDataUpdated.
                            (8, Value::Byte(flags)) => {
                                let selected = self.selected;
                                let using = self.using;
                                self.using = synced_local_use(using, *flags, selected, |slot| {
                                    self.stack_in(slot).is_some()
                                });
                                // onSyncedDataUpdated's useItemRemaining: getUseDuration,
                                // without startUsingItem's event.
                                if using.is_none()
                                    && let Some((slot, _)) = self.using
                                    && let Some(stack) = self.stack_in(slot).cloned()
                                {
                                    self.use_duration = self.use_duration(&stack);
                                    self.use_start = super::using_item::UseStart::of(stack);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                // Entity.DATA_SHARED_FLAGS_ID's crouching and sprinting bits:
                // another player's or a mannequin's crouch (their model and
                // Entity.isDiscrete's name tag), and the dust a runner raises.
                if (mannequin
                    || self
                        .remote
                        .as_ref()
                        .is_some_and(|r| r.level_players.contains_key(&p.id)))
                    && let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32)
                {
                    for (index, _, value) in &p.values {
                        if let (0, Value::Byte(flags)) = (index, value) {
                            m.extra.crouching = flags & 2 != 0;
                            m.extra.sprinting = flags & 8 != 0;
                        }
                    }
                }
                for (index, _, value) in &p.values {
                    if *index == ITEM_ENTITY_ITEM
                        && let Value::Item(stack) = value
                    {
                        self.remote_item_data(p.id, stack);
                    }
                    // ExperienceOrb.DATA_VALUE (the icon's size).
                    if *index == 8
                        && let Value::Int(v) = value
                        && let Some(o) = self.orbs.iter_mut().find(|o| o.id == p.id as u32)
                    {
                        o.value = *v;
                    }
                }
                self.remote_object_data(p.id, &p.values);
                self.remote_projectile_data(p.id, &p.values);
                self.remote_dragon_data(p.id, &p.values);
            }
            // ClientPacketListener.handleSetEquipment: what a mob or another
            // player holds and wears (EquipmentSlot ordinals).
            C::SetEquipment(p) => {
                self.remote_object_equipment(p.id, &p.slots);
                let items: Vec<(u8, Option<Arc<str>>, Option<ItemStack>)> = p
                    .slots
                    .iter()
                    .map(|(slot, stack)| {
                        (
                            *slot,
                            stack.as_ref().map(|s| self.item_name(s.item)),
                            self.to_stack(stack),
                        )
                    })
                    .collect();
                // CrossbowItem.isCharged: the main hand's `charged_projectiles`
                // (AvatarRenderer.getArmPose holds a loaded crossbow up).
                let charged = p.slots.iter().find(|(slot, _)| *slot == 0).map(|(_, s)| {
                    s.as_ref().is_some_and(|s| {
                        matches!(
                            s.component(sunrise_protocol::item::component::CHARGED_PROJECTILES),
                            Some(sunrise_protocol::item::Component::Stacks(list))
                                if !list.is_empty()
                        )
                    })
                });
                if let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32) {
                    if let Some(charged) = charged
                        && &*m.kind.id == "minecraft:player"
                    {
                        m.extra.charged = charged;
                    }
                    for (slot, item, stack) in items {
                        let stack = stack.or_else(|| item.map(|item| ItemStack::new(&item, 1)));
                        m.set_equipment_stack(slot as usize, stack);
                    }
                }
            }
            C::TakeItemEntity(p) => {
                // ClientPacketListener.handleTakeItemEntity: the pickup sound at
                // the orb, item or arrow, whoever collects it (the server sends
                // none); an entity this client doesn't track sounds at this player.
                let orb_at = self
                    .orbs
                    .iter()
                    .find(|o| o.id == p.item as u32)
                    .map(|o| o.position);
                let orb = orb_at.is_some();
                let item = self.dropped.iter().find(|d| d.id == p.item as u32);
                let shot = self
                    .creatures
                    .projectiles
                    .iter()
                    .find(|s| s.id == p.item as u32);
                let shot = shot.map(|s| s.position).or_else(|| {
                    self.tridents
                        .iter()
                        .find(|t| t.id == p.item as u32)
                        .map(|t| t.position)
                });
                let at = match (item, shot) {
                    (Some(d), _) => Some([d.position.x, d.position.y, d.position.z]),
                    (None, Some(s)) => Some([s.x, s.y, s.z]),
                    (None, None)
                        if self
                            .remote
                            .as_ref()
                            .is_some_and(|r| r.entity_id == p.player) =>
                    {
                        self.eye_position()
                    }
                    (None, None) => None,
                };
                if let Some(o) = orb_at {
                    let pitch = (self.world.random.next_float() - self.world.random.next_float())
                        * 0.35
                        + 0.9;
                    self.audio.play(
                        "minecraft:entity.experience_orb.pickup",
                        Some([o.x, o.y, o.z]),
                        0.1,
                        pitch,
                    );
                } else if let Some(at) = at {
                    let pitch = ((self.world.random.next_float() - self.world.random.next_float())
                        * 0.7
                        + 1.0)
                        * 2.0;
                    self.audio
                        .play("minecraft:entity.item.pickup", Some(at), 0.2, pitch);
                }
                // ItemPickupParticle toward the collector (this player, another one or a mob).
                let collector = if self
                    .remote
                    .as_ref()
                    .is_some_and(|r| r.entity_id == p.player)
                {
                    super::pickups::Collector::Player
                } else {
                    super::pickups::Collector::Mob(p.player as u32)
                };
                if let Some(item) = self.dropped.iter().find(|d| d.id == p.item as u32).cloned() {
                    self.add_pickup(super::pickups::Taken::Item(Box::new(item)), collector);
                } else if let Some(orb) = self.orbs.iter().find(|o| o.id == p.item as u32).cloned()
                {
                    self.add_pickup(super::pickups::Taken::Orb(Box::new(orb)), collector);
                } else if let Some(shot) = self
                    .creatures
                    .projectiles
                    .iter()
                    .find(|s| s.id == p.item as u32)
                    .cloned()
                {
                    // An arrow flies to its collector too, then goes.
                    self.add_pickup(super::pickups::Taken::Arrow(Box::new(shot)), collector);
                } else if let Some(i) = self.tridents.iter().position(|t| t.id == p.item as u32) {
                    // So does a trident: a server's client keeps them apart.
                    let trident = self.tridents.remove(i);
                    self.add_pickup(super::pickups::Taken::Trident(Box::new(trident)), collector);
                }
                // An orb stays: the server may still hold merged ones in it.
                if !orb {
                    self.remove_entity(p.item);
                }
            }
            // ClientPacketListener.handleSetEntityMotion: knockback on this player.
            C::SetEntityMotion(p) if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) => {
                if let Some(pl) = self.player.as_mut() {
                    pl.velocity = Vec3::new(p.velocity[0], p.velocity[1], p.velocity[2]);
                }
            }
            // The same packet for another entity: Entity.lerpMotion.
            C::SetEntityMotion(p) => self.remote_motion(p.id, p.velocity),
            // ClientPacketListener.handleProjectilePowerPacket: the power a
            // hurting projectile accelerates by, sent bundled with its motion.
            C::ProjectilePower(p) => {
                super::remote_projectiles::remote_projectile_power(
                    &mut self.creatures.projectiles,
                    p.id,
                    p.power,
                );
            }
            // ClientPacketListener.handleDamageEvent: LivingEntity.handleDamageEvent
            // jolts the walk animation and restarts the hurt flash, and the
            // hurt player's own client plays its hurt sound (`Player.getHurtSound`:
            // the damage type's effects) at `getSoundVolume` and
            // `Player.getVoicePitch`, which is 1.0 for every player. The
            // server's sound packet reaches every other player around, never
            // this one (#4149), and that copy is played at 1.0.
            // This player's own hurt tilt and flash come from the server too
            // (sent to the player itself): its own moves never hurt it.
            C::DamageEvent(p) => {
                let me = self.remote.as_ref().map(|r| r.entity_id);
                let own = me == Some(p.id);
                self.pvp_hurt(p.id, own, p.direct.map(|d| Some(d) == me));
                if own {
                    // For `events`: this client has no combat tracker of its own here.
                    let cause = self.remote.as_ref().and_then(|r| {
                        r.registries
                            .name("minecraft:damage_type", p.source_type)
                            .map(str::to_owned)
                    });
                    let by = p
                        .cause
                        .and_then(|id| u32::try_from(id).ok())
                        .and_then(|id| {
                            self.mob_index
                                .get(&self.mobs, id)
                                .map(|m| m.kind.id.to_string())
                        });
                    self.events
                        .note_hit(cause.unwrap_or_else(|| "unknown".into()), by);
                    if let Some(pl) = self.player.as_mut() {
                        pl.health.animate_hurt();
                        let sound = self.remote_hurt_sound(p.source_type);
                        let pitch = player_voice_pitch();
                        self.play_client_player_sound(sound, 1.0, pitch);
                    }
                } else if let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32) {
                    m.limb_speed = 1.5;
                    m.hurt_time = 10;
                } else {
                    self.remote_dragon_hurt(p.id);
                }
            }
            C::HurtAnimation(p) => {
                if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) {
                    // Player.animateHurt: the tilt turns around the yaw.
                    if let Some(pl) = self.player.as_mut() {
                        pl.health.animate_hurt_dir(p.yaw);
                    }
                } else if let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32) {
                    m.hurt_time = 10;
                } else {
                    self.remote_dragon_hurt(p.id);
                }
            }
            C::EntityEvent(p) => {
                if p.event == super::fireworks::FIREWORK_EXPLODE_EVENT {
                    self.remote_firework_explosion(p.id);
                }
                self.remote_fangs_event(p.id as u32, p.event);
                self.remote_boat_event(p.id, p.event);
                // MinecartTNT.handleEntityEvent 70: prime the fuse the window draws.
                self.remote_tnt_cart_event(p.id, p.event);
                // Arrow.handleEntityEvent 0: a tipped arrow's burst of particles.
                if p.event == 0 {
                    self.remote_arrow_puff(p.id as u32);
                }
                // LivingEntity.handleEntityEvent 2: a spear's hit felt.
                if p.event == 2 {
                    self.remote_kinetic_hit(p.id);
                }
                // LocalPlayer.setPermissions also refreshes an open World Options screen.
                if (24..=28).contains(&p.event)
                    && self.remote.as_ref().is_some_and(|r| r.entity_id == p.id)
                {
                    self.ui.registries.permission = (p.event - 24) as u8;
                    self.remote_world_options_permission(p.event >= 26);
                }
                // Player.handleEntityEvent 22 / 23: reduced debug info on / off.
                if (22..=23).contains(&p.event)
                    && let Some(r) = self.remote.as_mut().filter(|r| r.entity_id == p.id)
                {
                    r.reduced_debug_info = p.event == 22;
                    self.debug_entries.invalidate();
                }
                // Ravager.handleEntityEvent 69: the roar's puff and push.
                if p.event == 69 {
                    self.remote_ravager_roar(p.id);
                }
                // ClientPacketListener.handleEntityEvent 63: a sniffer's digging sound.
                if p.event == 63 {
                    self.start_sniffer_digging_sound(p.id as u32);
                }
                // ClientPacketListener.handleEntityEvent 21: a guardian's hum.
                if p.event == 21 {
                    self.start_guardian_sound(p.id as u32);
                }
                // ZombieVillager.handleEntityEvent 16: the cure starts.
                if p.event == 16 {
                    self.zombie_villager_cure_sound(p.id as u32);
                }
                // Witch.handleEntityEvent 15: the magic puffs over its hat.
                if p.event == 15 {
                    self.witch_magic_particles(p.id as u32);
                }
                // MinecartTNT.handleEntityEvent 70: primeFuse on the client.
                if p.event == 70 {
                    self.remote_cart_primed(p.id as u32);
                }
                // Fox.handleEntityEvent 45: the crumbs of the food in its mouth.
                if p.event == 45 {
                    self.fox_eat_particles(p.id as u32);
                }
                // Rabbit.handleEntityEvent(1): the hop clocks, and
                // Entity.spawnSprintParticle (one block-dust crumb).
                if p.event == 1 {
                    let hop = {
                        let m = self.mob_index.get_mut(&mut self.mobs, p.id as u32);
                        if let Some(m) = m
                            && m.species() == sunrise_client::mobs::Species::Rabbit
                        {
                            let sample =
                                (m.position, m.velocity, f64::from(m.width()), m.supporting);
                            sunrise_client::mobs::rabbit::hop_event(m);
                            Some(sample)
                        } else {
                            None
                        }
                    };
                    if let Some((position, movement, width, support)) = hop {
                        let id = p.id as u32;
                        self.sprint_particle(position, movement, support, width, |game| {
                            // this.random: the rabbit's own, x then z.
                            game.mob_index
                                .get_mut(&mut game.mobs, id)
                                .map(|m| (m.random.next_double(), m.random.next_double()))
                        });
                    }
                }
                if p.event == 19
                    && let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32)
                {
                    sunrise_client::mobs::aquatic::squid_cycle_event(m);
                }
                // LivingEntity.handleEntityEvent 55: another player swapped
                // its main and off hand (`swapHandItems`).
                if p.event == 55
                    && let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32)
                {
                    m.extra.swap_hands();
                }
                // LivingEntity.handleEntityEvent 47 to 52, 65 and 68: an
                // equipped item broke (onEquippedItemBroken).
                if let Some(slot) = sunrise_client::mobs::item_break::equipment_break_slot(p.event)
                {
                    self.remote_equipment_break(p.id, slot);
                }
                // EntityEvent.DEATH: start the death animation.
                if p.event == 3
                    && let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32)
                {
                    m.health = 0.0;
                }
                // LivingEntity.handleEntityEvent 3 on this player: its death
                // sound, which Player.playSound plays for this client alone
                // (the server's goes to everyone else).
                if p.event == 3
                    && self.remote.as_ref().is_some_and(|r| r.entity_id == p.id)
                    && self.player.is_some()
                {
                    let sound = "minecraft:entity.player.death";
                    let pitch = player_voice_pitch();
                    self.play_client_player_sound(sound, 1.0, pitch);
                }
                // The events that start a mob's animation (the warden's).
                if let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32) {
                    sunrise_client::mobs::sync::entity_event(m, p.event as u8);
                }
                // LivingEntity.handleEntityEvent 46: portal particles from the
                // old position to the new one (chorus fruit, an enderman).
                if p.event == 46 {
                    self.entity_teleport_particles(p.id);
                }
                // LivingEntity.handleEntityEvent 60: the poof as the dead body goes.
                if p.event == 60
                    && let Some(m) = self.mob_index.get(&self.mobs, p.id as u32)
                {
                    let (at, w, h) = (m.position, f64::from(m.width()), f64::from(m.height()));
                    self.death_poof([at.x, at.y, at.z], w, h);
                }
                if p.event == super::totem::TOTEM_EVENT {
                    self.remote_totem(p.id);
                }
                // Mob.handleEntityEvent 20: the spawn poof.
                if p.event == 20 && self.mobs.iter().any(|m| m.id == p.id as u32) {
                    self.poof_particles(p.id as u32);
                }
                // The particle bursts of Villager 12 to 14 and 42, TamableAnimal
                // and AbstractHorse 6 and 7, Animal and Allay 18, Ocelot 40 and
                // 41, Dolphin 38: the table in `entity_event_burst` is the one
                // list of them; any other event draws nothing here.
                self.entity_event_particles(p.id as u32, p.event);
                // Snowball and ThrownEgg.handleEntityEvent 3: the crumbs where
                // the projectile is (the server removes it right after).
                if p.event == 3 {
                    let crumbs = self
                        .creatures
                        .projectiles
                        .iter()
                        .find(|pr| pr.id == p.id as u32)
                        .and_then(|pr| {
                            let item = match pr.kind {
                                sunrise_client::projectiles::ProjectileKind::Snowball => {
                                    "minecraft:snowball"
                                }
                                sunrise_client::projectiles::ProjectileKind::Egg => "minecraft:egg",
                                sunrise_client::projectiles::ProjectileKind::Thrown(t) => t.item,
                                _ => return None,
                            };
                            Some(([pr.position.x, pr.position.y, pr.position.z], item))
                        });
                    if let Some((at, item)) = crumbs {
                        self.item_burst(at, item);
                    }
                }
                if p.event == super::combat::DROWN_EVENT {
                    self.remote_drown(p.id);
                }
                // Entity events 53 and 54: honey crumbs while sliding, and the
                // ten from a landing (`HoneyBlock.showSlideParticles` /
                // `showJumpParticles`).
                if sunrise_client::landing::honey_particle_count(p.event).is_some() {
                    let at = if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) {
                        self.player
                            .as_ref()
                            .map(|q| [q.position.x, q.position.y, q.position.z])
                    } else {
                        self.mobs
                            .iter()
                            .find(|m| m.id == p.id as u32)
                            .map(|m| [m.position.x, m.position.y, m.position.z])
                    };
                    if let Some(at) = at {
                        self.honey_entity_particles(at, p.event);
                    }
                }
                if p.event == super::consume::USE_ITEM_COMPLETE_EVENT
                    && self.remote.as_ref().is_some_and(|r| r.entity_id == p.id)
                {
                    self.remote_complete_using();
                }
            }
            C::Animate(p) => self.remote_animate(p.id, p.action),
            // ClientPacketListener.handleSwingAnimation: the entity swings
            // (LivingEntity.swing, which a swing in its first half ignores).
            C::SwingAnimation(p) => {
                let hand = if p.hand == 1 { Hand::Off } else { Hand::Main };
                let animation = sunrise_client::first_person::SwingAnimation {
                    kind: p.kind.clamp(0, 255) as u8,
                    duration: p.duration,
                };
                if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) {
                    self.swing_hand(hand, animation);
                } else if let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32) {
                    m.swing_hand_modified(hand == Hand::Off, animation);
                }
            }
            // The player's own instances: its maximum health (the hearts the
            // HUD draws and the health it may hold), reach, speed and so on.
            // Then the received health again, which the old maximum clamped.
            C::UpdateAttributes(p) if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) => {
                // Kept for a LocalPlayer made after the packet came, and across respawns.
                if let Some(r) = self.remote.as_mut() {
                    for snapshot in &p.values {
                        r.player_attributes
                            .retain(|s| s.attribute != snapshot.attribute);
                        r.player_attributes.push(snapshot.clone());
                    }
                }
                let nameplate = self.remote_nameplate_values(&p.values);
                self.apply_own_attributes(p.values);
                if let Some(m) = self.player_model.as_mut() {
                    if let Some(value) = nameplate.name_tag {
                        m.set_name_tag_distance(value);
                    }
                    if let Some(value) = nameplate.below_name {
                        m.set_below_name_distance(value);
                    }
                }
                self.apply_remote_health();
            }
            C::UpdateAttributes(p) => self.remote_mob_attributes(p.id, &p.values),
=======
            C::RotateHead(p) => self.apply_rotate_head_packet(p),
            C::SetEntityData(p) => self.apply_set_entity_data_packet(p),
            C::SetEquipment(p) => self.apply_set_equipment_packet(p),
            C::TakeItemEntity(p) => self.apply_take_item_entity_packet(p),
            C::SetEntityMotion(p) => self.apply_set_entity_motion_packet(p)?,
            C::ProjectilePower(p) => self.apply_projectile_power_packet(p),
            C::DamageEvent(p) => self.apply_damage_event_packet(p),
            C::HurtAnimation(p) => self.apply_hurt_animation_packet(p),
            C::EntityEvent(p) => self.apply_entity_event_packet(p)?,
            C::Animate(p) => self.remote_animate(p.id, p.action),
            C::SwingAnimation(p) => self.apply_swing_animation_packet(p),
            C::UpdateAttributes(p) => self.apply_update_attributes_packet(p)?,
>>>>>>> sunrise/main
            C::UpdateMobEffect(p) => self.remote_effect(&p),
            C::RemoveMobEffect(p) => self.apply_remove_mob_effect_packet(p),
            C::ChangeDifficulty(p) => {
                self.ui.difficulty = p.difficulty.clamp(0, 3) as u8;
                self.level.difficulty_locked = p.locked;
                self.remote_world_options_difficulty(self.ui.difficulty, p.locked);
            }
            C::GameRuleValues(p) => self.remote_world_options_rules(p.entries),
            C::SetHealth(p) => self.apply_set_health_packet(p),
            C::PlayerCombatKill(p) => self.apply_player_combat_kill_packet(p)?,
            C::TrackedWaypoint(p) => self.handle_waypoint(&p),
            C::SetExperience(p) => self.apply_set_experience_packet(p),
            C::SetTime(p) => self.apply_set_time_packet(p),
            C::GameEvent(p) => self.apply_game_event_packet(p),
            C::PlayerAbilities(p) => self.apply_player_abilities_packet(p),
            C::SetHeldSlot(p) => self.apply_set_held_slot_packet(p),
            C::ContainerSetContent(p) => self.apply_container_set_content_packet(p),
            C::ContainerSetSlot(p) => self.apply_container_set_slot_packet(p),
            C::SetCursorItem(p) => self.apply_set_cursor_item_packet(p),
            C::SetPlayerInventory(p) => self.apply_set_player_inventory_packet(p),
            C::ContainerSetData(p) => self.apply_container_set_data_packet(p)?,
            C::OpenScreen(p) => self.remote_open_screen(p),
            C::MountScreenOpen(p) => self.remote_open_mount(p),
            C::UpdateRecipes(p) => self.remote_recipe_properties(p),

            C::RecipeBookAdd(p) => self.remote_recipe_book_add(p),
            C::RecipeBookRemove(p) => self.remote_recipe_book_remove(p),
            C::RecipeBookSettings(p) => self.remote_recipe_book_settings(p),
            C::PlaceGhostRecipe(p) => self.remote_ghost_recipe(p),
            C::MerchantOffers(p) => self.remote_merchant_offers(p),
            C::ContainerClose(_) => {
                // `LocalPlayer.clientSideCloseContainer` sets the screen to
                // null, including dialogs and WaitingForResponseScreen. This
                // differs from clearDialog, which restores the parent screen.
                let dialog = self.dialog_open();
                if dialog {
                    self.close_dialogs();
                    self.ui.chat_screen = None;
                    self.book = None;
                    self.sign_edit = None;
                }
                if !self.remote_lectern_closed() && (self.screen == Screen::Inventory || dialog) {
                    self.remote_close_local();
                    self.set_screen(Screen::Playing);
                    self.capture(true);
                }
            }
            C::TagQuery(p) => self.answer_tag_query(p.transaction, p.tag),
            C::Unknown(id, data)
                if id == sunrise_protocol::ids::play::clientbound::COMMAND_SUGGESTIONS =>
            {
                self.handle_command_suggestions(&data);
            }
            C::CustomChatCompletions(p) => {
                // ClientPacketListener.handleCustomChatCompletions.
                if let Some(r) = self.remote.as_mut() {
                    sunrise_client::command::modify_custom_completions(
                        &mut r.custom_completions,
                        sunrise_client::command::CustomCompletionAction::from_id(p.action),
                        &p.entries,
                    );
                    self.ui.registries.custom_completions = r.custom_completions.clone();
                }
            }
            C::UpdateTags(p) => self.remote_update_tags(&p),
            C::Commands(p) => self.apply_commands_packet(p),
            C::Cooldown(p) => {
                // ClientPacketListener.handleItemCooldown: the server starts
                // (or confirms) a group's cooldown; 0 ends it.
                let group: Arc<str> = Arc::from(p.group.as_str());
                if p.duration == 0 {
                    self.cooldowns.remove(&group);
                } else {
                    self.cooldowns.add(group, p.duration);
                }
            }
            // ClientPacketListener.handleGameTestHighlightPos.
            C::GameTestHighlightPos(p) => self.highlight_test_pos(p.absolute, p.relative),
            // ClientPacketListener.handleTestInstanceBlockStatus.
            C::TestInstanceBlockStatus(p) => {
                self.block_editor_status(remote_text(&p.status), p.size);
            }
            C::SystemChat(p) => self.apply_system_chat_packet(p)?,
            C::PlayerChat(p) => self.apply_player_chat_packet(p)?,
            C::DisguisedChat(p) => self.apply_disguised_chat_packet(p),
            C::DeleteChat(p) => self.handle_delete_chat(&p.signature)?,
            C::PlayerInfoUpdate(p) => self.apply_player_info_update_packet(p),
            C::PlayerInfoRemove(p) => self.apply_player_info_remove_packet(p),
            C::TabList(p) => self.apply_tab_list_packet(p),
            C::InitializeBorder(p) => self.apply_initialize_border_packet(p),
            C::SetBorderCenter(p) => self.border.set_center([p.x, p.z]),
            C::SetBorderLerpSize(p) => self.apply_set_border_lerp_size_packet(p),
            C::SetBorderSize(p) => self.border.set_size(p.size),
            C::SetBorderWarningDelay(p) => self.border.warning_time = p.delay,
            C::SetBorderWarningDistance(p) => self.border.warning_distance = p.blocks,
            C::SetObjective(p) => self.remote_objective(p),
            C::SetDisplayObjective(p) => self.apply_set_display_objective_packet(p),
            C::SetScore(p) => self.apply_set_score_packet(p),
            C::ResetScore(p) => match &p.objective {
                Some(o) => self.score.board.reset_one(&p.owner, o),
                None => self.score.board.reset_all(&p.owner),
            },
            C::SetPlayerTeam(p) => self.remote_team(p),
            C::SetSimulationDistance(p) => {
                if let Some(r) = self.remote.as_mut() {
                    r.simulation_distance = p.distance;
                }
            }
            C::SetChunkCacheRadius(p) => {
                self.streaming.radius = self
                    .settings
                    .render_distance
                    .min(p.distance.saturating_add(1))
                    .max(2);
                if let Some(r) = self.remote.as_mut() {
                    r.view_distance = p.distance;
                    let center = r.chunk_center;
                    // ClientChunkCache.updateViewRadius: a new storage of that size.
                    self.drop_remote_chunks_out_of_range(center, p.distance);
                }
            }
            C::SetChunkCacheCenter(p) => {
                if let Some(r) = self.remote.as_mut() {
                    r.chunk_center = (p.x, p.z);
                    let distance = r.view_distance;
                    self.drop_remote_chunks_out_of_range((p.x, p.z), distance);
                }
            }
            C::BlockEntityData(p) => self.remote_block_entity_packet(p),
            C::OpenSignEditor(p) => self.remote_open_sign_editor(p),
            C::OpenBook(p) => self.remote_open_book(p.hand),
            C::SetDefaultSpawnPosition(p) => self.apply_set_default_spawn_position_packet(p),
            C::ServerData(p) => self.remote_server_data(p.icon.as_deref()),
            C::MapItemData(p) => self.remote_map_item_data(&p),
            // handleTickingState / handleTickingStep do nothing without a ClientLevel.
            C::TickingState(p) if self.remote.as_ref().is_some_and(Remote::has_level) => {
                self.clock.set_rate(p.rate);
                self.clock.set_frozen(p.frozen);
            }
            C::TickingStep(p) if self.remote.as_ref().is_some_and(Remote::has_level) => {
                self.clock.set_frozen_ticks_to_run(p.steps);
            }
            C::TickingState(_) | C::TickingStep(_) => {}
            C::UpdateAdvancements(p) => self.remote_advancements(p),
            C::AwardStats(p) => self.apply_award_stats_packet(p)?,
            // ClientPacketListener.setActionBarText, setTitleText,
            // setSubtitleText, setTitlesAnimation and handleTitlesClear.
            C::BossEvent(p) => self.apply_boss_event(&p),
            C::SetActionBarText(p) => self.titles.set_overlay(remote_text(&p.text), false),
            C::SetTitleText(p) => self.titles.set_title(remote_text(&p.text)),
            C::SetSubtitleText(p) => self.titles.set_subtitle(remote_text(&p.text)),
            C::SetTitlesAnimation(p) => self.titles.set_times(p.fade_in, p.stay, p.fade_out),
            // ClientCommonPacketListenerImpl.handleShowDialog / handleClearDialog.
            C::ShowDialog(p) => self.show_play_dialog(&p.dialog),
            C::ClearDialog(_) => self.clear_dialog(),
            // ClientPacketListener.handleLowDiskSpaceWarning.
            C::LowDiskSpaceWarning(_) => self.client_reports.report_low_disk_space(),
            C::ClearTitles(p) => self.apply_clear_titles_packet(p),
            _ => {}
        }
        Ok(())
    }

    // remote_objective: remote/sync.rs

<<<<<<< sync-sunrise-1
    /// Resolve inline `object:player` profiles through the same skin cache as
    /// player heads on items. The next frame uses the uploaded atlas texture.
    pub(super) fn resolve_inline_profiles(
        &mut self,
        profiles: &[std::sync::Arc<serde_json::Value>],
    ) {
        use sunrise_client::skins;
        for profile in profiles {
            let skin = skins::head_skin(
                profile,
                &mut self.skin_profiles.borrow_mut(),
                &mut self.skin_downloads.borrow_mut(),
                |location| self.gui.atlas.sprites.contains_key(location),
            );
            let id = skins::parse_profile(profile).id;
            let name = profile
                .as_str()
                .or_else(|| profile.get("name").and_then(serde_json::Value::as_str));
            // The player list's skin for the same profile is drawn too.
            let keys = [
                Some(format!("id:{id:032x}")),
                name.map(|n| format!("name:{}", n.to_ascii_lowercase())),
            ];
            for key in keys.iter().flatten() {
                if let Some(listed) = self.gui.atlas.object_skins.get(key)
                    && listed.starts_with("skins/")
                {
                    let listed = listed.clone();
                    self.again_skin(&listed);
                }
            }
            self.gui
                .atlas
                .set_object_skin(format!("id:{id:032x}"), skin.clone());
            if let Some(name) = name {
                self.gui
                    .atlas
                    .set_object_skin(format!("name:{}", name.to_ascii_lowercase()), skin);
            }
        }
    }
    fn poll_own_profile(&mut self) {
        let auth = self.auth();
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        match &mut r.own_profile {
            OwnProfile::Idle => {
                let Some(auth) = auth else {
                    r.own_profile = OwnProfile::Done(None);
                    return;
                };
                let (send, recv) = std::sync::mpsc::channel();
                std::thread::Builder::new()
                    .name("Own profile".into())
                    .spawn(move || {
                        let _ = send.send(sunrise_client::auth::own_textures(auth.as_ref()));
                    })
                    .ok();
                r.own_profile = OwnProfile::Fetching(recv);
            }
            OwnProfile::Fetching(recv) => {
                if let Ok(own) = recv.try_recv() {
                    r.own_profile = OwnProfile::Done(own);
                }
            }
            OwnProfile::Done(_) => {}
        }
    }
    /// `PlayerInfo.getSkin` for every player entity, including NPCs that are
    /// never in the tab list: resolve each profile's textures and give the
    /// level's player models their skin.
    /// The 3D skin layers model for a player (Options → Skin Customization,
    /// `threeDSkinLayers`): `kind`'s outer layer with one slab per painted
    /// texel, built once per (skin, slim) from the decoded image the skin
    /// downloader still holds (`sunrise_client::mobs::skin_layers`). `None`
    /// while the option is off or the image is not in memory yet.
    pub(in crate::game) fn skin_3d_model(
        &self,
        kind: &std::sync::Arc<sunrise_client::mobs::MobType>,
        skin: &str,
        slim: bool,
    ) -> Option<std::sync::Arc<sunrise_client::mobs::Part>> {
        use sunrise_client::mobs::skin_layers;
        if !self.settings.extras.three_d_skin_layers {
            return None;
        }
        let key = (skin.to_owned(), slim);
        if let Some(model) = self.skin_3d.borrow_mut().get(&key) {
            return Some(model);
        }
        let model_key = if slim { "player_slim#main" } else { "main" };
        let base = kind
            .layer(model_key)
            .cloned()
            .unwrap_or_else(|| kind.model.clone());
        let image = self.skin_downloads.borrow().image(skin)?.to_vec();
        let model = std::sync::Arc::new(skin_layers::extrude(&base, &image, skin_layers::SIZE)?);
        self.skin_3d.borrow_mut().insert(key, model.clone());
        Some(model)
    }

    pub(super) fn update_remote_skins(&mut self) {
        let local = self.remote.as_ref().map(|r| r.profile);
        self.poll_own_profile();
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let local = local.unwrap_or(r.profile);
        let mut entries = skin_entries(
            &r.level_players,
            &r.skin_textures,
            &mut r.level_player_textures,
        );
        let (online_mode, names): (bool, Vec<(u128, String)>) = (
            r.online_mode,
            r.players.iter().map(|(u, n)| (*u, n.clone())).collect(),
        );
        let own = match &r.own_profile {
            OwnProfile::Done(Some(t)) => Some(t.clone()),
            _ => None,
        };
        // Minecraft.profileFuture: the signed-in account's active skin and cape
        // from `/minecraft/profile` when the server sent none (single player) or
        // only a skin without the equipped cape.
        if let Some(own) = own {
            if let Some((_, t)) = entries.iter_mut().find(|(u, _)| *u == local) {
                if t.skin.is_none() {
                    t.skin = own.skin.clone();
                    if own.skin.is_some() {
                        t.slim = own.slim;
                    }
                }
                if t.cape.is_none() {
                    t.cape = own.cape.clone();
                }
            } else if own.skin.is_some() || own.cape.is_some() {
                entries.retain(|(u, _)| *u != local);
                entries.push((local, own));
            }
        }
        // Skin Customization's Ely.by Skins: on a server that sends no skin for a player (offline
        // mode), the one their name has on Ely.by, as offline launchers show it.
        let mut from_elyby = std::collections::HashSet::new();
        // An Ely.by account's own skin where the server sent none: Ely.by's, as Mojang's profile
        // is a Microsoft account's (`own_profile` asks Mojang nothing for it).
        if !entries.iter().any(|(u, t)| *u == local && t.skin.is_some())
            && let Some(textures) = self.own_elyby_textures()
        {
            entries.retain(|(u, _)| *u != local);
            entries.push((local, textures));
            from_elyby.insert(local);
        }
        if !online_mode {
            for (uuid, name) in names {
                if entries.iter().any(|(u, t)| *u == uuid && t.skin.is_some()) {
                    continue;
                }
                if let Some(textures) = self.elyby_textures(&name) {
                    entries.retain(|(u, _)| *u != uuid);
                    entries.push((uuid, textures));
                    from_elyby.insert(uuid);
                }
            }
        }
        let mut resolved: Vec<(u128, String)> = Vec::new();
        // PlayerSkin.model of each resolved skin: the textures' `slim`.
        let mut slim = std::collections::HashSet::new();
        // PlayerSkin.cape of each resolved skin.
        let mut capes: HashMap<u128, String> = HashMap::new();
        for (uuid, textures) in entries {
            // PlayerInfo.createSkinLookup: only the local player's own skin
            // may be unsigned; Ely.by's never are.
            let secure = uuid != local && !from_elyby.contains(&uuid);
            let skin = {
                let mut downloads = self.skin_downloads.borrow_mut();
                sunrise_client::skins::lookup(&textures, secure, |kind, url| {
                    downloads.state(kind, url)
                })
            };
            if let Some(skin) = skin {
                if textures.slim {
                    slim.insert(uuid);
                }
                if let Some(cape) = sunrise_client::skins::cape(&textures) {
                    capes.insert(uuid, cape);
                }
                resolved.push((uuid, skin));
            }
        }
        // Options > Skin Customization: the player's own skin, in this game only (others get
        // what the server sends of the profile).
        if let Some((own, own_slim)) = self.own_skin_sprite() {
            resolved.retain(|(u, _)| *u != local);
            resolved.push((local, own));
            if own_slim {
                slim.insert(local);
            } else {
                slim.remove(&local);
            }
        }
        // An offline name's skin from the launcher, where neither the server
        // nor an account gives the local player one.
        if !resolved.iter().any(|(u, _)| *u == local)
            && let Some((rgba, is_slim)) = sunrise_client::skins::offline_skin()
        {
            const OFFLINE: &str = "sunrise/offline_skin";
            if !self.skin_slots.contains(OFFLINE) || !self.gui.atlas.sprites.contains_key(OFFLINE) {
                self.store_skin(OFFLINE.into(), rgba.clone());
            }
            if *is_slim {
                slim.insert(local);
            }
            resolved.push((local, OFFLINE.into()));
        }
        // An offline name's cape from the launcher, where the profile has none.
        if !capes.contains_key(&local)
            && let Some(rgba) = sunrise_client::skins::offline_cape()
        {
            const OFFLINE: &str = "capes/offline";
            if !self.skin_slots.contains(OFFLINE) || !self.gui.atlas.sprites.contains_key(OFFLINE) {
                self.store_skin(OFFLINE.into(), rgba.to_vec());
            }
            capes.insert(local, OFFLINE.into());
        }
        // PlayerInfo.getSkin: the downloaded skin and its model, else the
        // profile's default skin (DefaultPlayerSkin.get), slim or wide.
        let skin_of = |skins: &HashMap<u128, String>, uuid: u128| match skins.get(&uuid) {
            Some(skin) => (skin.clone(), slim.contains(&uuid)),
            None => {
                let skin = sunrise_client::tab_list::default_skin(uuid);
                (skin.to_owned(), skin.contains("/slim/"))
            }
        };
        let skins: HashMap<_, _> = resolved.into_iter().collect();
        let Some((level, own)) = self.remote.as_ref().map(|r| {
            (
                r.level_players
                    .iter()
                    .map(|(id, uuid)| (*id as u32, *uuid))
                    .collect::<Vec<_>>(),
                r.profile,
            )
        }) else {
            return;
        };
        // Only the skins drawn hold atlas slots: the level's players, not
        // every entry of a busy server's player list.
        // Build once: resolving each player must not scan every level entity.
        let players: HashMap<_, _> = self
            .mobs
            .iter()
            .enumerate()
            .filter(|(_, m)| &*m.kind.id == "minecraft:player")
            .map(|(i, m)| (m.id, i))
            .collect();
        for (id, uuid) in level {
            let Some(&i) = players.get(&id) else {
                continue;
            };
            let (skin, is_slim) = self.drawn_skin(uuid, skin_of(&skins, uuid));
            let kind = self.mobs[i].kind.clone();
            let skin_3d = self.skin_3d_model(&kind, &skin, is_slim);
            let cape = capes.get(&uuid).filter(|c| self.again_skin(c)).cloned();
            let mob = &mut self.mobs[i];
            mob.skin = Some(skin);
            mob.slim = is_slim;
            mob.skin_3d = skin_3d;
            mob.cape = cape;
        }
        if self.player_model.is_some() {
            let (skin, is_slim) = self.drawn_skin(own, skin_of(&skins, own));
            // Avatar.isModelPartShown(CAPE): the Skin Customization option.
            let cape = capes
                .get(&own)
                .filter(|_| self.settings.file.bool("modelPart_cape"))
                .filter(|c| self.again_skin(c))
                .cloned();
            #[cfg(target_os = "android")]
            let cape = if self.settings.file.bool("modelPart_cape") {
                self.wardrobe_cape(cape)
            } else {
                None
            };
            let kind = self.player_model.as_ref().map(|m| m.kind.clone());
            let skin_3d = kind
                .as_ref()
                .and_then(|kind| self.skin_3d_model(kind, &skin, is_slim));
            if let Some(mob) = self.player_model.as_mut() {
                mob.skin = Some(skin);
                mob.slim = is_slim;
                mob.skin_3d = skin_3d;
                mob.cape = cape;
            }
        }
        if let Some(r) = self.remote.as_mut()
            && r.skins != skins
        {
            r.skins_revision = r.skins_revision.wrapping_add(1);
            r.skins = skins;
        }
        // ClientMannequin.updateSkin (PlayerSkinRenderCache.lookup): a
        // mannequin's DATA_PROFILE `textures` property resolves like a
        // player's, never required to be signed.
        let mannequins: Vec<(usize, sunrise_client::skins::Textures)> = self
            .mobs
            .iter()
            .enumerate()
            .filter(|(_, m)| &*m.kind.id == "minecraft:mannequin" && m.profile.is_some())
            .map(|(i, m)| {
                let profile = m.profile.as_ref();
                let t = profile.and_then(|p| p.properties.iter().find(|p| p.name == "textures"));
                let textures = if let Some(t) = t {
                    let signature = if t.signature.is_some() {
                        sunrise_client::skins::Signature::Signed
                    } else {
                        sunrise_client::skins::Signature::Unsigned
                    };
                    sunrise_client::skins::unpack(
                        &t.value,
                        signature,
                        sunrise_client::skins::texture_uris(),
                    )
                } else {
                    // `ResolvableProfile.resolveProfile`: no `textures` and
                    // only a name or an id resolves through the services.
                    let key = profile.and_then(|p| match (&p.name, p.uuid) {
                        (Some(name), None) => {
                            Some(sunrise_client::skins::NameOrId::Name(name.clone()))
                        }
                        (None, Some(id)) => Some(sunrise_client::skins::NameOrId::Id(id)),
                        _ => None,
                    });
                    key.and_then(|key| match self.skin_profiles.borrow_mut().state(&key) {
                        sunrise_client::skins::Resolve::Done(Some(profile)) => {
                            profile.textures().map(|(value, signature)| {
                                sunrise_client::skins::unpack(
                                    &value,
                                    signature,
                                    sunrise_client::skins::texture_uris(),
                                )
                            })
                        }
                        _ => None,
                    })
                    .unwrap_or_default()
                };
                (i, textures)
            })
            .collect();
        let mut resolved: Vec<(usize, Option<String>, bool)> = Vec::new();
        for (id, textures) in mannequins {
            let skin = {
                let mut downloads = self.skin_downloads.borrow_mut();
                sunrise_client::skins::lookup(&textures, false, |kind, url| {
                    downloads.state(kind, url)
                })
            };
            let slim = skin.is_some() && textures.slim;
            resolved.push((id, skin, slim));
        }
        for (i, skin, slim) in resolved {
            let skin = skin.filter(|s| !s.starts_with("skins/") || self.again_skin(s));
            // Resolution and atlas uploads do not insert or remove mobs.
            let mob = &mut self.mobs[i];
            mob.slim = slim && skin.is_some();
            mob.skin = skin;
        }
    }
    /// Puts a downloaded skin in an atlas skin slot (`SkinSlots::place`),
    /// unless every slot holds a skin drawn in the last frame.
    pub(super) fn store_skin(&mut self, location: String, rgba: Vec<u8>) {
        // 3D skin layers reads the pixels back when it builds the slabs.
        self.skin_downloads
            .borrow_mut()
            .remember_image(&location, &rgba);
        let Some((slot, evicted)) = self.skin_slots.place(location.clone()) else {
            return;
        };
        if let Some(old) = evicted {
            self.gui.atlas.sprites.remove(&old);
        }
        let Some(mut rect) = self
            .gui
            .atlas
            .sprites
            .get(&format!("sunrise/skin{slot}"))
            .copied()
        else {
            return;
        };
        let (w, h) = (self.gui.atlas.width as f32, self.gui.atlas.height as f32);
        self.map_patches.push(sunrise_client::font::Patch {
            x: (rect.u0 * w).round() as u32,
            y: (rect.v0 * h).round() as u32,
            width: 64,
            height: 64,
            rgba,
        });
        // A cape is a 64x32 texture in the top half of the slot (skins::process_cape).
        if location.starts_with("capes/") {
            rect.v1 = (rect.v0 + rect.v1) / 2.0;
        }
        self.gui.atlas.sprites.insert(location, rect);
    }
    /// Whether a downloaded skin or cape is in the atlas to draw, marking it drawn (`again_skin`
    /// without queueing it again: `own_skin_sprite` stores its own).
    pub(super) fn show_skin(&mut self, location: &str) -> bool {
        // A slot without its sprite (the atlas was rebuilt, or the upload
        // never happened) counts as missing, so the skin is stored again.
        let shown =
            self.skin_slots.contains(location) && self.gui.atlas.sprites.contains_key(location);
        if shown {
            self.skin_downloads.borrow_mut().drawn(location);
        }
        shown
    }
    /// Whether a downloaded skin or cape is in the atlas to draw, marking it
    /// drawn; if not (new, or its slot taken), its image is queued for upload
    /// again and it is drawn from the next frame.
    pub(super) fn again_skin(&mut self, location: &str) -> bool {
        if self.show_skin(location) {
            return true;
        }
        self.skin_downloads.borrow_mut().again(location);
        false
    }
    /// A player's skin and model as drawn: the downloaded skin once it is in
    /// the atlas, until then the profile's default (`DefaultPlayerSkin.get`),
    /// as `PlayerInfo.getSkin` gives before the texture has loaded.
    pub(super) fn drawn_skin(
        &mut self,
        uuid: u128,
        (skin, slim): (String, bool),
    ) -> (String, bool) {
        if (skin.starts_with("skins/") || skin == "sunrise/offline_skin") && !self.again_skin(&skin)
        {
            let skin = sunrise_client::tab_list::default_skin(uuid);
            return (skin.to_owned(), skin.contains("/slim/"));
        }
        (skin, slim)
    }
    /// `drawn_skin` for a face in the player list or spectator menu.
    pub(super) fn face_skin(&mut self, uuid: u128, skin: &str) -> String {
        self.drawn_skin(uuid, (skin.to_owned(), false)).0
    }
    /// ChatType.Bound.decorate (`Decoration::Chat`) and `decorateNarration`
    /// (`Decoration::Narration`): the chat type's decoration from the
    /// server's registry, applied to `content`. Legacy `§` codes in it (which plugins still
    /// send) style the line, as `remote_text`'s do.
    pub(super) fn decorate_chat(
        &self,
        bound: &play::ChatTypeBound,
        content: &serde_json::Value,
        decoration: Decoration,
    ) -> sunrise_client::text::Text {
        use sunrise_client::scoreboard::comp_text;
        let Some(r) = self.remote.as_ref() else {
            return comp_text(content).resolve_legacy();
        };
        let chat_type = r.registries.data("minecraft:chat_type", bound.chat_type);
        comp_text(&chat_type_decorate(chat_type, decoration, bound, content)).resolve_legacy()
    }
=======
    // remote_tab_players: remote/sync.rs
    // update_remote_skins: remote/status.rs
>>>>>>> sunrise/main
    fn remote_item_data(&mut self, id: i32, stack: &Option<Stack>) {
        let Some(stack) = self.to_stack(stack) else {
            return;
        };
        let pending = self
            .remote
            .as_mut()
            .and_then(|r| r.pending_items.remove(&id));
        if let Some(item) = self.dropped.iter_mut().find(|d| d.id == id as u32) {
            item.stack = stack;
            return;
        }
<<<<<<< sync-sunrise-1
        let Some((p, velocity)) = position else {
            return;
        };
        let at = Vec3::new(p[0], p[1], p[2]);
        // ItemEntity's constructor draws bobOffs from the item's own random.
        let mut random = sunrise_client::mobs::entity_random(id as u32, at);
        let bob_offset = random.next_float() * std::f32::consts::PI * 2.0;
        self.dropped.push(ItemEntity {
            random,
            ..ItemEntity::new(id as u32, stack, at, velocity, bob_offset)
        });
    }
    /// `Entity.lerpMotion` (and `recreateFromPacket`'s motion) for the
    /// entities whose own tick moves them on a client: dropped items, experience
    /// orbs, falling blocks and primed TNT.
    fn remote_motion(&mut self, id: i32, velocity: [f64; 3]) {
        self.remote_dragon_motion(id, velocity);
        let v = Vec3::new(velocity[0], velocity[1], velocity[2]);
        // `falling` and `tnts` are searched from the back: AddEntity calls this right after
        // appending the block or TNT, so a spawn doesn't walk the whole list.
        if let Some(m) = self.mob_index.get_mut(&mut self.mobs, id as u32) {
            // Squid.aiStep derives its body pitch and roll from network motion.
            m.velocity = v;
        } else if let Some(d) = self.dropped.iter_mut().find(|d| d.id == id as u32) {
            d.velocity = v;
        } else if let Some(i) = self.falling.iter().rposition(|f| f.id == id as u32) {
            self.falling.update(i, |f| f.velocity = v);
        } else if let Some(t) = self.tnts.iter_mut().rev().find(|t| t.id == id as u32) {
            t.velocity = v;
        } else if let Some(o) = self.orbs.iter_mut().find(|o| o.id == id as u32) {
            o.velocity = v;
        } else if let Some(i) = self.carts.iter().position(|c| c.id == id as u32) {
            self.carts.update(i, |c| c.velocity = v);
        } else if let Some(i) = self
            .creatures
            .projectiles
            .iter()
            .position(|p| p.id == id as u32)
        {
            self.creatures.projectiles.update(i, |p| {
                p.velocity = v;
                // AbstractArrow.lerpMotion: the stuck timer restarts, and an
                // arrow the server set moving leaves the block it was stuck in.
                if let Some(st) = p.arrow.as_mut() {
                    st.life = 0;
                    if st.in_ground && v.length_squared() > 0.0 {
                        st.in_ground = false;
                        p.stuck = false;
                    }
                }
            });
        } else if let Some(t) = self.tridents.iter_mut().find(|t| t.id == id as u32) {
            t.velocity = v;
            // AbstractArrow.lerpMotion (ThrownTrident keeps it): `life`
            // restarts, and a trident set moving leaves its block.
            t.in_ground_time = 0;
            if t.in_ground && v.length_squared() > 0.0 {
                t.in_ground = false;
            }
        } else if let Some(r) = self.rockets.iter_mut().find(|r| r.id == id as u32) {
            r.velocity = v;
        } else if self.remote_hook_motion(id as u32, v) {
        } else if let Some(p) = self
            .remote
            .as_mut()
            .and_then(|r| r.pending_items.get_mut(&id))
        {
            p.1 = v;
        } else if !self.potions.remote_thrown_motion(id as u32, v) {
            self.end.remote_eye_motion(id as u32, velocity);
        }
    }
    fn apply_remote_health(&mut self) {
        let Some((health, food, saturation)) = self.remote.as_ref().map(|r| r.health) else {
            return;
        };
        if let Some(p) = self.player.as_mut() {
            if (p.health.health() - health).abs() > f32::EPSILON {
                let _ = p.set_health(health.max(0.0), false);
            }
            p.health.food.food = food;
            p.health.food.saturation = saturation;
        }
    }
    /// ClientPacketListener.handleUpdateAttributes for a LocalPlayer made after the packet came
    /// (the first teleport, a respawn): the attributes the server last sent.
    fn apply_remote_attributes(&mut self) {
        let Some(snapshots) = self
            .remote
            .as_ref()
            .map(|r| r.player_attributes.clone())
            .filter(|s| !s.is_empty())
        else {
            return;
        };
        if let Some(player) = self.player.as_mut() {
            sunrise_client::lan::attributes::apply(&mut player.attributes, &snapshots);
        }
    }
    /// `ServerPlayer.dismountTo` lands a rider at the ghast's own (x, maxY, z)
    /// (`HappyGhast.getDismountLocationForPassenger`). A vanilla client
    /// simulates the ghast it rides, so its copy is where the server has it;
    /// this window's copy trails the server's by the stepped interpolation and
    /// would rise into the rider's feet, which lets the rider fall through.
    /// Put the copy under the landing spot.
    fn place_ghast_under(&mut self, id: u32, spot: Vec3) {
        let Some(m) = self
            .mobs
            .iter_mut()
            .find(|m| m.id == id && m.species() == sunrise_client::mobs::Species::HappyGhast)
=======
        let Some(PendingItem {
            join,
            position: [x, y, z],
            velocity,
        }) = pending
>>>>>>> sunrise/main
        else {
            return;
        };
        let at = Vec3::new(x, y, z);
        // ItemEntity's constructor draws bobOffs from the item's own random.
        let mut random = sunrise_client::mobs::entity_random(id as u32, at);
        let bob = super::item_entities::bob_offset(at, random.next_float(), self.clock.still());
        // Metadata can arrive after newer entities (including other items). Insert at the
        // reserved ClientLevel.addEntity slot, not the tail at metadata time.
        let index = self.dropped.partition_point(|item| item.join < join);
        self.dropped.insert(
            index,
            ItemEntity {
                join,
                random,
                ..ItemEntity::new(id as u32, stack, at, velocity, bob)
            },
        );
<<<<<<< sync-sunrise-1
        let yaw = if p.relatives & rel::Y_ROT != 0 {
            self.yaw + p.yaw
        } else {
            p.yaw
        };
        let relative_pitch = p.relatives & rel::X_ROT != 0;
        let pitch = position_packet_pitch(self.pitch, p.pitch, relative_pitch);
        let delta = sunrise_client::player_sync::teleport_velocity(
            velocity,
            (self.yaw, self.pitch),
            (yaw, pitch),
            p,
        );
        let mut setback = false;
        // BlockStatePredictionHandler.onTeleport: a prior prediction keeps its block, not its remembered player position.
        if let Some(r) = self.remote.as_mut() {
            r.last_teleport_sequence = r.sequence;
        }
        // The server's answer to a passage crossed (`cross_passage`), or any other.
        self.passages.crossing = None;
        // handleMovePlayer: a passenger keeps its own position and look (its vehicle
        // carries it), so a vehicle a datapack teleports every tick doesn't snap the
        // rider's view; the teleport is still accepted, with where the player is.
        if self.riding.is_some()
            && let Some(player) = self.player.as_ref()
        {
            let at = player.position;
            self.send(S::AcceptTeleportation(play::AcceptTeleportation {
                id: p.teleport,
                x: at.x,
                y: at.y,
                z: at.z,
                yaw: self.yaw,
                pitch: self.pitch,
            }));
            self.stop_remote_mining();
            self.mining = None;
            return Ok(());
        }
        match self.player.as_mut() {
            Some(player) => {
                // A short teleport back is a correction: the server would not take the move.
                let back = player.position.distance(position);
                let state = format!(
                    "sprinting {}, on ground {}, input {:#04x}",
                    player.sprinting,
                    player.ground.on_ground,
                    self.remote.as_ref().map_or(0, |r| r.last_input)
                );
                if back > 0.0
                    && back < 8.0
                    && let Some(r) = self.remote.as_mut()
                {
                    r.setbacks += 1;
                    setback = true;
                    if r.setbacks <= 5 || r.setbacks % 50 == 0 {
                        // What tells the causes apart: whether the server took any move (the
                        // last one sent, and when), and which teleport this is.
                        let [sx, sy, sz] = r.position_sender.last_sent();
                        log::info!(
                            "Sunrise: the server moved the player back {back:.2} blocks to \
                             {:.2} {:.2} {:.2} ({} times; teleport {}, relatives {:#x}; last \
                             sent {sx:.2} {sy:.2} {sz:.2}, {} ms ago; {state})",
                            position.x,
                            position.y,
                            position.z,
                            r.setbacks,
                            p.teleport,
                            p.relatives,
                            r.moved_at.map_or(-1, |at| at.elapsed().as_millis() as i64)
                        );
                        if r.setbacks == 1 || r.setbacks % 50 == 0 {
                            let now = Instant::now();
                            let trail: Vec<String> = r
                                .trail
                                .iter()
                                .map(|([x, y, z], ground, input, at)| {
                                    format!(
                                        "-{}ms {x:.3} {y:.3} {z:.3} {}{input:#04x}",
                                        (now - *at).as_millis(),
                                        if *ground { "ground " } else { "air " }
                                    )
                                })
                                .collect();
                            log::info!("Sunrise: the moves before it: {}", trail.join(" | "));
                        }
                    }
                }
                player.position = position;
                player.velocity = delta;
            }
            None => {
                let mut player = PlayerMotion::new(self.motion, position)?;
                // The id the server gave, for the hooks the motion fires.
                player.entity_id = self.own_entity_id();
                // Player.tick derives noPhysics from the current game mode.
                player.spectator = self.spectator();
                player.velocity = delta;
                self.take_carried_attributes(&mut player)?;
                self.hook.discard();
                self.player_state.attachments.retire();
                self.resident_entities.local_player.renew();
                self.player = Some(player);
                self.apply_remote_abilities();
                self.apply_remote_attributes();
                self.bind_local_player_residency();
                self.input_player_replaced();
                self.sway = Default::default();
                self.eye = 1.62;
                self.previous_eye = 1.62;
                self.clock.reset();
                self.apply_remote_health();
            }
        }
        if setback {
            self.stick_setback();
        }
        entity_section_steps::player_teleport(self, had_player, current, position);
        if !self.replay.playing() {
            self.previous_position = position;
        }
        self.yaw = yaw;
        self.pitch = pitch;
        // setValuesFromPositionPacket: setOldPosAndRot gives xRotO the same
        // change, so a relative pitch keeps the model's ease and an absolute
        // one snaps it.
        if let Some(m) = self.player_model.as_mut() {
            m.previous_head_pitch =
                position_packet_pitch(m.previous_head_pitch, p.pitch, relative_pitch);
            m.head_pitch = pitch;
        }
        if let Some(id) = self.remote.as_mut().and_then(|r| r.ghast_dismount.take()) {
            self.place_ghast_under(id, position);
        }
        // handleMovePlayer does not change LocalPlayer.sendPosition's baseline:
        // the next tick must still report the corrected state.
        self.send(S::AcceptTeleportation(play::AcceptTeleportation {
            id: p.teleport,
            x: position.x,
            y: position.y,
            z: position.z,
            yaw,
            pitch,
        }));
        if let Some(r) = self.remote.as_ref() {
            r.told_rotation.set(Some((yaw, pitch)));
        }
        // Then MultiPlayerGameMode.stopDestroyBlock: a dig the teleport (an
        // anticheat's setback, a /tp) interrupts is aborted on the wire, and the
        // attack strength starts again.
        self.stop_remote_mining();
        self.mining = None;
        Ok(())
    }

    /// `ClientPacketListener.tick`'s `LevelLoadTracker.tickClientLoad`: the
    /// player's section is shown once its column is lit; when the level is
    /// ready the loading screen closes and the server hears `PlayerLoaded`
    /// (`notifyPlayerLoaded`).
    fn tick_client_load(&mut self) {
        let Some(player) = self.player.as_ref() else {
            return;
        };
        if !self
            .remote
            .as_ref()
            .is_some_and(|r| r.level_load.is_active())
        {
            return;
        }
        // `LevelRenderer`'s playerCompiledSectionCallback: the section holding
        // the camera is there (the column is lit, as meshing needs).
        let (cx, cz) = (
            (player.position.x.floor() as i32) >> 4,
            (player.position.z.floor() as i32) >> 4,
        );
        let section_ready = self.streaming.is_lit(cx, cz);
        let camera_y = (player.position.y + f64::from(self.eye)).floor() as i32;
        let shape = self.world.shape;
        let check = sunrise_client::level_loading::PlayerCheck {
            outside_build_height: camera_y < shape.min_y || camera_y >= shape.min_y + shape.height,
            spectator: player.spectator,
            alive: player.health.health() > 0.0,
        };
        let now = client_millis();
        // LevelLoadingScreen.tick.
        self.tick_load_progress();
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        if section_ready && r.level_load.wants_player_section() {
            r.level_load.player_section_ready();
        }
        r.level_load.tick_client_load(now, check);
        if !r.level_load.is_level_ready(now) {
            return;
        }
        r.level_load.finish();
        if !r.loaded_sent {
            r.loaded_sent = true;
            log::info!(
                "Sunrise: the level around the player was ready {} ms after the play phase began",
                r.play_began.map_or(0, |at| at.elapsed().as_millis())
            );
            self.send(S::PlayerLoaded(Empty));
        }
        // LevelLoadingScreen.onClose.
        self.say_system_now(&sunrise_client::text::Text::translate(
            "narrator.ready_to_play",
            &[],
        ));
        self.capture_loaded_world();
    }

    /// One client tick while connected (Minecraft.tick with a ClientLevel).
    pub(super) fn tick_remote(&mut self) -> Result<()> {
        self.tick_remote_level()?;
        // The rockets boosting this player tick after it, also while it waits
        // for chunks.
        self.tick_own_rockets();
        // Starling: client mods made for the old hooks tick on any server, single player's own
        // included (native_compat.rs).
        self.native_tick();
        self.tick_old_mod_net();
        self.tick_book_snapshot_retry();
        // Far tiles the samplers asked for (`lod_net`).
        if let Some(r) = &self.remote {
            r.session.send_far_terrain();
        }
        self.tick_chat_session();
        // Minecraft.tick sends this even when the player cannot tick yet,
        // including while waiting for chunks. Keep it outside early returns.
        if self.remote.as_ref().is_some_and(Remote::has_level) {
            self.send(S::ClientTickEnd(Empty));
            if let Some(r) = self.remote.as_mut() {
                r.count_rate(0, 1);
                let now = Instant::now();
                if let Some(gap) = r.tick_end_at.replace(now).map(|at| now - at)
                    && gap >= GAP_LOGGED
                {
                    log::info!(
                        "Sunrise: {} ms between two ticks ended to the server",
                        gap.as_millis()
                    );
                }
            }
        }
        Ok(())
    }

    fn tick_remote_player(&mut self, running: bool, hold: bool) -> Result<()> {
        // Inventory.tick's ItemStack.inventoryTick: the pop time runs down.
        if !hold {
            for pop in &mut self.hotbar_pop {
                *pop = pop.saturating_sub(1);
            }
        }
        // ClientLevel.tick: the border's size change, while the level runs.
        if running {
            self.border.tick();
        }
        // ClientLevel.tick: the End's sky flash, frozen or not.
        self.tick_end_flash();
        self.player_border();
        // The entities added after the player tick after it: their pushes
        // (from where they ended last tick, onto where the player ended)
        // reach its next move, this one.
        let pushed_after = self.remote_pushes(false);
        self.tick_remote_entities(running);
        self.tick_remote_arm_poses();
        self.tick_random_textures();
        // Breeze.tick's block dust: drawn by each client for the breezes it sees.
        if running {
            self.tick_breeze_effects();
        }
        self.tick_remote_guardian_eyes(running);
        // Level.tickBlockEntities: a frozen level ticks no block entities.
        // A step advances the moving pistons created by the block event once,
        // rather than finishing them during the frozen ticks between steps.
        if running {
            self.tick_block_entity_anims();
            self.tick_remote_spawners();
            self.client_tick_beacons();
            self.tick_remote_potent_sulfur();
            self.with_level(|b, level, ticks| {
                b.client_tick_block_entities(level, ticks);
            });
            let pushes = std::mem::take(&mut self.block_ticks.piston_pushes);
            self.push_piston_entities(pushes);
            self.sync_moving_pistons();
        }
        self.tick_hand();
        if running {
            self.tick_particles();
        } else if self.clock.still() {
            self.settle_rain_fog();
        }
        self.tick_frame_visuals(running);
        // Under `SUNRISE_STILL` they wait for the world too: endermen's and blazes' particles
        // draw on the particles' random, and effects count down, on every tick a frozen
        // client runs, as many as the run took to get there.
        if !hold {
            self.tick_client_ambient();
            self.tick_client_effects();
        }
        // Gui.tick: the titles, the action bar and the held item's name.
        if !hold {
            self.tick_hud();
        } else {
            // Only its countdowns wait: lying down still opens the bed's chat, waking closes it.
            self.tick_in_bed_chat();
        }
        self.tick_subtitles();
        self.tick_client_load();
        let entity_collisions = self.player_entity_collisions();
        // Those added before it tick before it: their pushes reach this move.
        let pushed = pushed_after.add_vector(self.remote_pushes(true));
        let sprint_window = self.settings.sprint_window;
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        let (cx, cz) = (
            (player.position.x.floor() as i32) >> 4,
            (player.position.z.floor() as i32) >> 4,
        );
        let lit = self.streaming.is_lit(cx, cz);
        let chunks = self.world.chunks.len();
        if let Some(r) = self.remote.as_mut() {
            if lit {
                if r.waiting_ticks >= 100 {
                    log::info!(
                        "Sunrise: the chunk under the player came after {} s",
                        r.waiting_ticks / 20
                    );
                }
                r.waiting_ticks = 0;
            } else {
                r.waiting_ticks += 1;
                if r.waiting_ticks % 600 == 100 {
                    log::warn!(
                        "Sunrise: waiting {} s for the chunk under the player ({cx}, {cz}): {chunks} \
                         chunks here, {} packets skipped",
                        r.waiting_ticks / 20,
                        r.decode_errors
                    );
                }
            }
        }
        if !lit {
            // LocalPlayer.tick can wait for chunks, but Minecraft.tick still
            // calls sendChanges: rotations and the 20-tick position reminder
            // must continue, as well as moves into the unloaded area.
            if let Some(r) = self.remote.as_ref()
                && r.loaded_sent
            {
                let input = r.last_input;
                self.remote_send_state(input);
            }
            return Ok(());
        }
        // LocalPlayer.tick reaches Player.tick only once the client has loaded
        // and the player's chunk is there; its endpoints bracket that work
        // alone, also on the death screen, not the level pass before it.
        let player = self
            .remote
            .as_ref()
            .is_some_and(|r| r.loaded_sent)
            .then(|| {
                (mod_api::__private::listened(mod_api::HookId::PlayerTickPre)
                    || mod_api::__private::listened(mod_api::HookId::PlayerTickPost))
                .then(|| self.local_player())
                .flatten()
            })
            .flatten();
        let player_from = entity_section_steps::player_before(self);
        if let Some(player) = player {
            self.native_player_tick_pre(mod_api::__private::PlayerTickPreCall { player });
        }
        let result = self.tick_local_player(entity_collisions, pushed, sprint_window);
        entity_section_steps::player_after(self, player_from);
        // `EntityEvent.Size` for the pose the tick set.
        self.native_local_player_size();
        if let Some(player) = player {
            self.native_player_tick_post(mod_api::__private::PlayerTickPostCall { player });
        }
        result
    }

    /// `LocalPlayer.tick`'s `Player.tick` work: input, riding, motion, sleep
    /// and the state the server hears.
    fn tick_local_player(
        &mut self,
        entity_collisions: Vec<sunrise_core::geometry::Aabb>,
        pushed: Vec3,
        sprint_window: i32,
    ) -> Result<()> {
        // Player.tick's ItemCooldowns.tick, after Pre and after
        // handleKeybinds: this path replaces `tick_world`'s for a server's
        // client (multiplayer and the default single player), so the entries
        // advance here or the slot stays shaded until the server says the
        // entry ended. `onCooldownEnded` is the plain ItemCooldowns' no-op on
        // a client. Ticking it after the keybinds keeps a held use from
        // trying again a tick before the server's entry ends.
        self.cooldowns.tick();
        // Player.tick's sleep counter (the darkening and the wake-up fade).
        self.sleep_timer = super::tick_sleep_timer(self.sleep_timer, self.sleeping.is_some());
        let stick = self.pad_move();
        if self.player.is_none() {
            return Ok(());
        }
        // The LevelLoadingScreen takes the input until the level is ready.
        let loading = self
            .remote
            .as_ref()
            .is_some_and(|r| r.level_load.is_active());
        let playing = self.screen == Screen::Playing && !loading && !self.dialog_open();
        let held = |action| playing && self.keys.held(action);
        let mut keys = Keys {
            forward: held(Action::Forward),
            backward: held(Action::Back),
            left: held(Action::Left),
            right: held(Action::Right),
            jump: held(Action::Jump),
            shift: held(Action::Sneak),
            sprint: held(Action::Sprint),
            sprint_suppressed: false,
            analog: None,
        }
        .with_stick(stick.filter(|_| playing));
        // A server's anticheat simulates Java's keys: eight directions at full speed. A stick's
        // own angle and tilt (the touch stick's, a pad's) it refuses move after move
        // (PikaNetwork's game servers): there the stick presses the keys it points at, or
        // steers (`stick_steer`), as the player picked. Single player and a LAN host keep it.
        let mode = self.update_stick_mode();
        if mode != ServerStick::Analog {
            keys.analog = None;
        }
        if stick.is_some()
            && playing
            && let Some(r) = self.remote.as_mut()
        {
            r.stick_used_at = Some(Instant::now());
        }
        let steer = if mode == ServerStick::Free {
            self.stick_steer(stick.filter(|_| playing))
        } else {
            None
        };
        if steer.is_some() {
            (keys.forward, keys.backward, keys.left, keys.right) = (true, false, false, false);
            keys.analog = None;
        }
        let keys = self.mod_movement_keys(keys);
        if let Some(r) = self.remote.as_mut() {
            r.steer_yaw = steer;
        }
        let input = u8::from(keys.forward)
            | u8::from(keys.backward) << 1
            | u8::from(keys.left) << 2
            | u8::from(keys.right) << 3
            | u8::from(keys.jump) << 4
            | u8::from(keys.shift) << 5
            | u8::from(keys.sprint) << 6;
        // LocalPlayer.applyInput and aiStep: watching through another
        // entity, the player does not move itself; its keys still reach the
        // server, whose sneak check lets go of the camera.
        let keys = if self.controls_camera() {
            keys
        } else {
            Keys::default()
        };
        // The held lead's end eases from the look at the start of the tick
        // (`yRotO`/`xRotO`), as the local tick does; `tick` returns before it
        // reaches its own write when this window is a server client.
        self.rope_rot_old = (self.yaw, self.pitch);
        // A passenger moves with its vehicle (Entity.rideTick), not by itself.
        let riding = self.screen != Screen::Dead && self.remote_ride(keys, sprint_window)?;
        // The server sends no passenger's position: they sit where their
        // vehicles are now.
        self.position_riders();
        // Entity.getFluidInteractionBox asks the vehicle: a boat afloat shelters
        // its passenger's eyes (AbstractBoat.modifyPassengerFluidInteractionBox).
        let boat = self
            .riding
            .and_then(|id| self.boats.iter().find(|b| b.id == id))
            .map(|b| sunrise_client::motion::RiddenBoat {
                under_water: b.under_water(),
                max_y: b.bounds().max_position().y,
            });
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        player.fluids.boat = boat;
        if !riding && !self.replay.playing() {
            self.previous_position = player.position;
        }
        let before = (
            player.ground.on_ground,
            [player.velocity.x, player.velocity.y, player.velocity.z],
            [player.position.x, player.position.y, player.position.z],
        );
        let was_flying = player.flying;
        self.sync_player_body();
        // LivingEntity.tickDeath: the local player counts its deathTime once
        // per client tick while dead (the camera's death roll and zoom).
        if self.screen == Screen::Dead {
            self.ui.death_time = self.ui.death_time.saturating_add(1);
        }
        let sleeping = self.sleeping;
        let sleep_position = sleeping.and_then(|head| self.sleeping_position(head));
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        let was_gliding = player.fall_flying;
        let dead = self.screen == Screen::Dead;
        // LivingEntity.isImmobile: a sleeper lies still, even if its bed's
        // shape is unavailable here; only the server decides when to wake.
        if sleeping.is_some() {
            if let Some(at) = sleep_position {
                player.position = at;
            }
            player.velocity = Vec3::ZERO;
            player.pitch = 0.0;
        } else if !dead && !riding {
            player.client_side = true;
            player.velocity = player.velocity.add_vector(pushed);
            // LivingEntity.checkAutoSpinAttack: a Riptide spin that touches a
            // mob ends and bounces back; the server deals the hit.
            player.spin_targets.clear();
            if player.auto_spin_attack_ticks > 0 {
                player.spin_targets.extend(
                    self.mobs
                        .iter()
                        .filter(|m| !m.removed)
                        .map(|m| (m.id, m.bounds())),
                );
            }
            if let Some(yaw) = steer {
                player.yaw = yaw;
            }
            MotionWorld {
                entities: &entity_collisions,
                player,
                scene: &mut self.world,
                assets: &self.pack.assets,
                collision_binding: self.collision,
                resources: self.motion,
                health: &self.health_resources,
                loop_context: &self.loop_context,
                damage_hooks: &self.damage_hooks,
                tag_items: None,
                block_registries: None,
            }
            .controlled_simulation_tick_with_sprint_window(keys, sprint_window)?;
        } else {
            // LivingEntity.baseTick counts hurtTime down for a dead or riding
            // player too (controlled_simulation_tick does it otherwise): left
            // at 10, the hurt tilt restarted every frame and shook the camera.
            player.health.advance_clock();
            if riding && !dead {
                player.client_side = true;
                let mut passenger = MotionWorld {
                    entities: &entity_collisions,
                    player: &mut *player,
                    scene: &mut self.world,
                    assets: &self.pack.assets,
                    collision_binding: self.collision,
                    resources: self.motion,
                    health: &self.health_resources,
                    loop_context: &self.loop_context,
                    damage_hooks: &self.damage_hooks,
                    tag_items: None,
                    block_registries: None,
                };
                passenger.passenger_pose_tick()?;
                // LivingEntity.aiStep: a passenger freezes in powder snow too, and the
                // frostbite overlay shows it (#8156).
                passenger.passenger_block_effects_tick()?;
            }
        }
        // The server decides fire the window cannot predict (blaze fireballs,
        // fire aspect, lightning): Entity.isOnFire reads the synched flag, so
        // the predicted tick's countdown must not end it while it is set.
        player.fire_ticks = hold_server_fire(
            player.fire_ticks,
            self.remote.as_ref().is_some_and(|r| r.own_on_fire),
        );
        // LocalPlayer.onUpdateAbilities: flight turned on or off (a double
        // jump, a spectator, landing) is sent to the server.
        let flying = player.flying;
        // LocalPlayer.aiStep: a glide this client started is sent to the
        // server (ServerboundPlayerCommandPacket START_FALL_FLYING).
        let started_gliding = player.fall_flying && !was_gliding;
        let events = std::mem::take(&mut player.events);
        self.previous_eye = self.eye;
        self.eye += (player.dimensions.eye_height - self.eye) * 0.5;
        for event in events {
            match event {
                sunrise_client::motion::MotionEvent::Emission {
                    position,
                    state,
                    muffled,
                } => {
                    let at = [position.x, position.y, position.z];
                    let sounds = self.own_sound_type(at, state, mod_api::BlockSoundCause::Step);
                    // Player.playStepSound: muffled under the swim sound in water.
                    let (volume, pitch) = if muffled {
                        (sounds.volume * 0.05, sounds.pitch * 0.8)
                    } else {
                        (sounds.volume * 0.15, sounds.pitch)
                    };
                    self.play_client_player_sound(&sounds.step, volume, pitch);
                }
                sunrise_client::motion::MotionEvent::Swim { volume, pitch } => {
                    self.play_client_player_sound("minecraft:entity.player.swim", volume, pitch);
                }
                // Entity.doWaterSplashEffect on the client's own level: the
                // server's splash skips this player (Player.playSound), and
                // its particles are the client's own.
                sunrise_client::motion::MotionEvent::Splash {
                    high_speed,
                    volume,
                    pitch,
                } => {
                    let name = if high_speed {
                        "minecraft:entity.player.splash.high_speed"
                    } else {
                        "minecraft:entity.player.splash"
                    };
                    self.play_client_player_sound(name, volume, pitch);
                }
                sunrise_client::motion::MotionEvent::SplashParticle {
                    bubble,
                    position,
                    velocity,
                } => {
                    use sunrise_client::particles::{ParticleOptions, ParticleType};
                    let kind = if bubble {
                        ParticleType::Bubble
                    } else {
                        ParticleType::Splash
                    };
                    self.particles.add(
                        &ParticleOptions::simple(kind),
                        [position.x, position.y, position.z],
                        [velocity.x, velocity.y, velocity.z],
                    );
                }
                sunrise_client::motion::MotionEvent::Chime { volume, pitch } => {
                    self.play_client_player_sound(
                        "minecraft:block.amethyst_block.chime",
                        volume,
                        pitch,
                    );
                }
                // NetherPortalBlock.entityInside on the client's own level:
                // the portal overlay and its trigger sound (the server moves
                // the player).
                sunrise_client::motion::MotionEvent::PowderSnowMelt { pos } => {
                    self.powder_snow_melt(pos);
                }
                sunrise_client::motion::MotionEvent::InsidePortal { .. } => {
                    self.client_inside_nether_portal();
                }
                // HoneyBlock.maybeDoSlideEffects on the client's own level:
                // the slide sound, heard here (LocalPlayer.playSound); the
                // particles come from the server's event 53.
                sunrise_client::motion::MotionEvent::HoneySlide { sound: true, .. } => {
                    self.play_client_player_sound("minecraft:block.honey_block.slide", 1.0, 1.0);
                }
                // LivingEntity.causeFallDamage and HoneyBlock.fallOn on the
                // client's own level: the fall and honey sounds are heard here
                // (LocalPlayer.playSound); the server's copy skips this player
                // (Player.playSound).
                sunrise_client::motion::MotionEvent::Fall(event) => self.block_fall(event),
                _ => {}
            }
        }
        // A stand riding the player follows this move (tickPassenger).
        self.seat_own_stands();
        // Other players on this player (possibly through a plugin seat) follow this tick too;
        // the full pass only when something rides this player.
        if self.remote.as_ref().is_some_and(|r| {
            let me = r.entity_id as u32;
            r.passenger_vehicles.values().any(|&v| v == me)
        }) {
            self.position_riders();
        }
        // The server owns health and hunger.
        self.apply_remote_health();
        self.tick_avatar(before, false);
        // LivingEntity.tick: a sleeper's xRot is 0.
        if sleeping.is_some() {
            self.pitch = 0.0;
            self.snap_player_model_pitch();
        }
        // The player's own model (third person, the inventory's preview).
        if !self.replay.playing() {
            self.tick_player_model();
        }
        self.client_hooks_tick();
        if !dead {
            self.remote_tick_using();
        }
        if flying != was_flying {
            self.send(S::PlayerAbilities(play::PlayerAbilitiesRequest {
                flags: if flying { 2 } else { 0 },
            }));
        }
        if started_gliding {
            let id = self.remote.as_ref().map_or(0, |r| r.entity_id);
            self.send(S::PlayerCommand(play::PlayerCommand {
                id,
                action: 6,
                data: 0,
            }));
        }
        // LocalPlayer.sendChanges sends `input.keyPresses`, which an auto-jump's `makeJump` set to
        // jump for this tick: the server's prediction plays the jump from it. Sent as held, the jump
        // was the client's alone, and the server saw a jump its keys didn't allow.
        let auto_jumped = self.player.as_ref().is_some_and(|p| p.was_auto_jump);
        self.remote_send_state(input | u8::from(auto_jumped) << 4);
        Ok(())
    }
=======
    }

    // remote_teleport: remote/sync.rs

    // tick_remote and tick_remote_player: remote/tick.rs
>>>>>>> sunrise/main

    /// The last `ClientboundPlayerAbilities` the server sent, on a client.
    pub(super) fn remote_abilities(&self) -> Option<&play::PlayerAbilities> {
        self.remote.as_ref()?.abilities.as_ref()
    }

    // tick_remote_guardian_eyes: remote/tick.rs

    // tick_remote_entities: remote/tick.rs

    /// `AbstractMinecartRenderer.newExtractState`: sample the same weighted
    /// steps at the frame's partial tick, including corners within one tick.
    pub(super) fn remote_minecart_lerp(
        &self,
        id: u32,
        partial: f32,
    ) -> Option<([f64; 3], f32, f32)> {
        let lerp = self.remote.as_ref()?.cart_lerps.get(&(id as i32))?;
        if lerp.current.is_empty() {
            return None;
        }
        let (p, _, yaw, pitch) = lerp.at(partial);
        Some(([p.x, p.y, p.z], yaw, pitch))
    }

<<<<<<< sync-sunrise-1
    /// `ClientPacketListener.handleMinecartAlongTrack`: the steps join the ones
    /// the minecart has queued, if it runs `NewMinecartBehavior`.
    fn remote_move_minecart(&mut self, p: play::MoveMinecartAlongTrack) {
        if !self.minecart_improvements() || !self.carts.iter().any(|c| c.id == p.id as u32) {
            return;
        }
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let at = |v: [f64; 3]| Vec3::new(v[0], v[1], v[2]);
        r.cart_lerps
            .entry(p.id)
            .or_insert_with(CartLerp::new)
            .pending
            .extend(p.steps.iter().map(|s| CartStep {
                position: at(s.position),
                movement: at(s.movement),
                yaw: sunrise_protocol::buf::unpack_degrees(s.yaw),
                pitch: sunrise_protocol::buf::unpack_degrees(s.pitch),
                weight: s.weight,
            }));
    }
    /// `Consumable.onConsumeTick` for another player using a consumable: the
    /// crumbs at their mouth every 4 ticks once 21.875% of its use has passed
    /// (`shouldEmitParticlesAndSounds`; the sounds come from the server).
    fn remote_player_eating(
        &mut self,
        item: &str,
        ticks: u32,
        at: [f64; 3],
        eye: f64,
        pitch: f32,
        yaw: f32,
    ) {
        let Some(consumable) = self
            .res
            .items()
            .get(item)
            .and_then(|i| i.consumable.as_ref())
            .filter(|c| c.particles)
        else {
            return;
        };
        let duration = consumable.ticks().max(0) as u32;
        let remaining = duration - ticks.min(duration);
        if !consumable.should_emit(remaining as i32) {
            return;
        }
        let options = self.item_particle(&ItemStack::new(item, 1));
        let mut spawned = Vec::new();
        sunrise_client::particles::item_use_particles(
            &mut self.world.random,
            pitch,
            yaw,
            at,
            at[1] + eye,
            5,
            |p, v| spawned.push((p, v)),
        );
        for (p, v) in spawned {
            self.particles.add(&options, p, v);
        }
    }
    /// The end of the player's tick: `AbstractBoat.tick`'s paddles (the vehicle ticks before its
    /// passenger's `sendChanges`), then `LocalPlayer.sendChanges`: the input, and a passenger's look,
    /// vehicle and sprint, or `sendPosition`'s sprint and move. The held slot is not sent here:
    /// `MultiPlayerGameMode.tick` sends it at the start of the next tick (`remote_handle_input`).
    fn remote_send_state(&mut self, input: u8) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let (pos, on_ground, collision, sprinting) = (
            [p.position.x, p.position.y, p.position.z],
            p.ground.on_ground,
            p.collision.horizontal,
            p.sprinting,
        );
        let steer = self.remote.as_ref().and_then(|r| r.steer_yaw);
        // A finger aiming with tap to interact: the server sees the player look at what it acts on.
        let (look_yaw, look_pitch) = self.interaction_rotation();
        let (yaw, pitch) = (steer.unwrap_or(look_yaw), look_pitch);
        let riding = self.riding.is_some();
        let controls_camera = self.controls_camera();
        let (paddles, vehicle): (Vec<S>, Vec<S>) = self
            .remote_vehicle_packets()
            .into_iter()
            .partition(|p| matches!(p, S::PaddleBoat(_)));
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let mut out = paddles;
        if input != r.last_input {
            r.last_input = input;
            out.push(S::PlayerInput(play::PlayerInput { flags: input }));
        }
        // LocalPlayer.sendIsSprintingIfNeeded.
        let sprint = |r: &mut Remote, out: &mut Vec<S>| {
            if sprinting != r.was_sprinting {
                r.was_sprinting = sprinting;
                out.push(S::PlayerCommand(play::PlayerCommand {
                    id: r.entity_id,
                    action: if sprinting { 1 } else { 2 },
                    data: 0,
                }));
            }
        };
        let mut moved_or_rotated = false;
        if riding {
            // LocalPlayer.sendChanges: a passenger sends its look, then the
            // vehicle it controls and, only then, its sprint.
            out.push(S::MovePlayerRot(play::MovePlayerRot {
                yaw,
                pitch,
                flags: u8::from(on_ground) | u8::from(collision) << 1,
            }));
            if !vehicle.is_empty() {
                out.extend(vehicle);
                sprint(r, &mut out);
            }
            moved_or_rotated = true;
        } else {
            sprint(r, &mut out);
            // LocalPlayer.sendPosition: no move while the view is another
            // entity's (the server keeps the player on it).
            if controls_camera
                && let Some(packet) = {
                    if let Some(told) = r.told_rotation.take() {
                        r.position_sender.told_rotation(told);
                    }
                    r.position_sender
                        .update(pos, (yaw, pitch), on_ground, collision)
                }
            {
                r.count_rate(1, 0);
                if r.trail.len() == TRAIL {
                    r.trail.pop_front();
                }
                r.trail
                    .push_back((pos, on_ground, r.last_input, Instant::now()));
                log_movement_gap(&mut r.moved_at, r.play_began);
                out.push(packet);
                moved_or_rotated = true;
            }
        }
        if moved_or_rotated && r.session.is_recording() {
            let pkt = sunrise_protocol::packets::play::Clientbound::PlayerPosition(
                sunrise_protocol::packets::play::PlayerPosition {
                    teleport: 0,
                    position: pos,
                    delta: [0.0, 0.0, 0.0],
                    yaw,
                    pitch,
                    relatives: 0,
                },
            );
            r.session.record_clientbound(&pkt);
        }
        if r.session.is_recording() {
            let is_using = self.using.is_some();
            let living_flags: u8 = if is_using {
                1 | if self.hand == Hand::Off { 2 } else { 0 }
            } else {
                0
            };
            let crouching = p.local_crouching || matches!(p.pose, crate::pose::Pose::Crouching);
            let mut shared: u8 = 0;
            if crouching {
                shared |= 0x02;
            }
            if sprinting {
                shared |= 0x08;
            }
            if p.swimming {
                shared |= 0x10;
            }
            if p.fall_flying {
                shared |= 0x80;
            }
            if shared != r.was_shared_flags
                || living_flags != r.was_living_flags
                || r.last_recorded_pose != Some(p.pose)
            {
                r.was_shared_flags = shared;
                r.was_living_flags = living_flags;
                r.last_recorded_pose = Some(p.pose);
                use sunrise_protocol::entity_data::{Value, serializer};
                let pkt = sunrise_protocol::packets::play::Clientbound::SetEntityData(
                    sunrise_protocol::packets::play::SetEntityData {
                        id: r.entity_id,
                        values: vec![
                            (0, serializer::BYTE, Value::Byte(shared as i8)),
                            (6, serializer::POSE, Value::Id(p.pose.id())),
                            (8, serializer::BYTE, Value::Byte(living_flags as i8)),
                        ],
                    },
                );
                r.session.record_clientbound(&pkt);
            }
        }
        // Creative inventory edits are sent slot by slot (SetCreativeModeSlot).
        let mut creative_slots = Vec::new();
        if self.creative {
            // A server's open menu takes clicks, which it applies itself: what
            // this client predicts for them isn't an edit to send (it would
            // overwrite the server's stacks whenever the prediction is wrong).
            let in_menu = r.menu.as_ref().is_some();
            let stacks = creative_sync_stacks(&self.inventory, &self.offhand, &self.armor);
            for (i, stack) in stacks.into_iter().enumerate() {
                if stack != r.synced[i] {
                    r.synced[i] = stack.clone();
                    if !in_menu {
                        creative_slots.push((creative_sync_slot(i), stack));
                    }
                }
            }
        }
        for packet in out {
            self.send(packet);
        }
        for (slot, stack) in creative_slots {
            let item = self.to_network(&stack);
            self.send(S::SetCreativeModeSlot(play::SetCreativeModeSlot {
                slot,
                item,
            }));
        }
    }
    pub(in crate::game) fn creative_stacks(&self) -> Vec<Option<ItemStack>> {
        creative_sync_stacks(&self.inventory, &self.offhand, &self.armor)
    }
    /// `CreativeInventoryListener.slotChanged`: `handleCreativeModeItemAdd` for every slot the
    /// player's inventory menu changed. `CreativeModeInventoryScreen.slotClicked` ends in
    /// `broadcastChanges`, so a click's slots go out with the click, right behind a thrown
    /// stack, and not at the end of the tick: a slot update the server sends meanwhile (the
    /// pickup of what a held Q threw) can then never land on a change still unsent and undo it.
    pub(super) fn send_creative_slots(&mut self) {
        if !self.creative {
            return;
        }
        let stacks = self.creative_stacks();
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let mut changed = Vec::new();
        for (i, stack) in stacks.into_iter().enumerate() {
            if stack != r.synced[i] {
                r.synced[i] = stack.clone();
                changed.push((creative_sync_slot(i), stack));
            }
        }
        for (slot, stack) in changed {
            let item = self.to_network(&stack);
            self.send(S::SetCreativeModeSlot(play::SetCreativeModeSlot {
                slot,
                item,
            }));
        }
        self.remote_creative_menu_sync();
    }
=======
    // remote_player_eating: remote/use.rs
>>>>>>> sunrise/main
    /// Inventory as the server last described it.
    fn remote_synced(&mut self) {
        let stacks = self.creative_stacks();
        let menu = self.creative_menu_slots();
        if let Some(r) = self.remote.as_mut() {
            r.synced = stacks;
            if let Some(m) = r.creative_menu.as_mut() {
                *m = menu;
            }
        }
    }

    /// The slots a server packet changed since `before` are as the server has them; the
    /// others keep what was last sent, so a creative click made since still goes out.
    pub(in crate::game) fn remote_synced_since(&mut self, before: &[Option<ItemStack>]) {
        let now = self.creative_stacks();
        let menu = self.creative_menu_slots();
        if let Some(r) = self.remote.as_mut() {
            mark_synced(&mut r.synced, before, now);
            if let Some(m) = r.creative_menu.as_mut() {
                *m = menu;
            }
        }
    }
    /// The inventory menu's slots 1 to 8 and 45: the crafting grid, the armour (head first) and
    /// the off hand.
    fn creative_menu_slots(&self) -> [Option<ItemStack>; 9] {
        let g = &self.craft_grid;
        let [head, chest, legs, feet] = self.armor.clone();
        [
            g[0].clone(),
            g[1].clone(),
            g[2].clone(),
            g[3].clone(),
            head,
            chest,
            legs,
            feet,
            self.offhand.clone(),
        ]
    }
    /// CreativeModeInventoryScreen's slot listener for the slots the inventory's sync leaves
    /// out (the crafting grid, the armour, the off hand): each one a creative screen's clicks
    /// changed is sent as SetCreativeModeSlot, while one is open and once more as it closes.
    pub(super) fn remote_creative_menu_sync(&mut self) {
        const MENU: [i16; 9] = [1, 2, 3, 4, 5, 6, 7, 8, 45];
        let open = self.creative && self.screen == Screen::Creative;
        let now = self.creative_menu_slots();
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let Some(before) = r.creative_menu.as_mut() else {
            if open {
                r.creative_menu = Some(now);
            }
            return;
        };
        let changed: Vec<usize> = (0..MENU.len()).filter(|&i| before[i] != now[i]).collect();
        *before = now.clone();
        if !open {
            r.creative_menu = None;
        }
        for i in changed {
            let item = self.to_network(&now[i]);
            self.send(S::SetCreativeModeSlot(play::SetCreativeModeSlot {
                slot: MENU[i],
                item,
            }));
        }
    }

    // remote_mining: remote/use.rs

    // remote_use_block: remote/use.rs

    /// `MapItem.useOn` on this client: a map used on a banner is SUCCESS
    /// whatever its data says (the server toggles the marker, or fails
    /// quietly), so this hand swings and the click ends. The caller applies
    /// ItemStack.useOn's adventure predicate before selecting this phase.
    fn predict_map_on_banner(&self, stack: &ItemStack, hit: &super::Hit) -> bool {
        super::maps::map_id(stack).is_some()
            && self.named_block_in_tag("minecraft:banners", &self.block_name(hit.pos))
    }

    /// `BoneMealItem.useOn` with the acting hand's `stack` on this client's
    /// level (a copy: nothing grows here): whether it succeeds.
    fn remote_bone_meal_succeeds(&mut self, hit: &super::Hit, stack: &ItemStack) -> bool {
        let pos = sunrise_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
        let face = sunrise_core::geometry::Direction::ALL[face_direction(hit.face) as usize];
        let (ask, _) = self.held_bonemeal(pos, stack);
        self.with_level(|b, level, _| {
            b.bone_meal_succeeds_on_client(
                &mut DryLevel::new(level),
                pos,
                face,
                ask.crop,
                ask.check,
            )
        })
    }

    /// MultiPlayerGameMode.useItemOn inside startPrediction: the blocks the
    /// use changed on this client's level at once, each kept as predicted
    /// under the use's sequence until the server acknowledges it (a use the
    /// server refused goes back). Only the blocks a hand opens or flips: a
    /// wooden or copper door, trapdoor or fence gate, a lever or a button;
    /// whatever else the use changes waits for the server.
    fn predict_use(&mut self, (changes, sounds): &DryUse, seq: i32) {
        let behaviours = self.res.behaviours();
        let predicted = |state: BlockStateId| hand_operated(behaviours, state);
        if changes.is_empty()
            || !changes
                .iter()
                .all(|(p, s)| predicted(*s) && predicted(self.world.block(p.x, p.y, p.z)))
        {
            return;
        }
        self.predict_blocks(changes, seq);
        // `level.playSound(player, ...)`: the clicker hears it from its own use; the
        // server sends it to the others only.
        self.play_predicted_sounds(sounds);
    }

    fn play_predicted_sounds(&mut self, sounds: &[DrySound]) {
        for (pos, sound, source, volume, pitch) in sounds {
            let at = [
                f64::from(pos.x) + 0.5,
                f64::from(pos.y) + 0.5,
                f64::from(pos.z) + 0.5,
            ];
            let name = if sound.contains(':') {
                sound.to_string()
            } else {
                format!("minecraft:{sound}")
            };
            self.play_client_local_sound(
                LevelSound::new(&name, at, *volume, *pitch).in_source(*source),
            );
        }
    }

    // remote_block_reacts: remote/use.rs
    // remote_block_template: remote/use.rs

    // remote_place_preview: remote/use.rs

<<<<<<< sync-sunrise-1
    /// Whether the block hit takes a main-hand right click with `held`
    /// (`BlockState.useItemOn`, then `useWithoutItem`), once the caller let
    /// the block react (no sneaking with full hands): a door, a button, a
    /// container, and the blocks it changed. Vanilla's client runs it on its
    /// own level first; this runs it on a copy, and `predict_use` keeps the
    /// changes it predicts.
    fn remote_block_reacts(
        &mut self,
        hit: &super::Hit,
        held: Option<&ItemStack>,
    ) -> (InteractionResult, DryUse) {
        // AbstractCauldronBlock.useItemOn (CauldronInteractions): a bucket,
        // bottle or dye the cauldron takes fills or lowers it, and the click
        // ends there instead of falling through to Item.use (which would
        // empty a water bucket into the face of the cauldron).
        if let Some(reacts) = self.remote_cauldron_reacts(hit, held) {
            return reacts;
        }
        if self.jukebox_takes(hit.pos, held) {
            return (InteractionResult::Success, DryUse::default());
        }
        // RespawnAnchorBlock.useItemOn: PASS when the off hand's glowstone is to
        // charge the anchor, so useWithoutItem doesn't run and the off hand's turn
        // comes (glowstone in this hand charges it in `use_item_on`).
        if !self.anchor_takes(hit.pos, held) && self.anchor_takes(hit.pos, self.offhand.as_ref()) {
            return (InteractionResult::Pass, DryUse::default());
        }
        // A content mod's machine takes the click where its server would (processing.rs): a
        // depot's top, a millstone with an empty hand. Nothing is placed against it.
        if self.machine_takes(hit.pos, hit.face, held) {
            return (InteractionResult::Success, DryUse::default());
        }
        // BeehiveBlock.useItemOn: shears or a glass bottle on a full hive
        // (honey level 5) is SUCCESS, so the click ends after UseItemOn and
        // no UseItem follows (the server does the harvest, see `use_beehive`).
        if held.is_some_and(|s| matches!(&*s.item, "minecraft:shears" | "minecraft:glass_bottle"))
            && self
                .hive_state(hit.pos)
                .is_some_and(|(honey, _)| honey == Some(5))
        {
            return (InteractionResult::Success, DryUse::default());
        }
        // ComposterBlock.useItemOn: a compostable item on a composter below
        // level 8 is SUCCESS on the client too (only the server adds the
        // layer), so the other hand (a torch) is never tried.
        if self.composter_takes(hit.pos, held) {
            return (InteractionResult::Success, DryUse::default());
        }
        self.remote_block_template(hit, held, false)
    }
=======
    // remote_use_bucket: remote/use.rs
>>>>>>> sunrise/main

    // remote_start_held_use: remote/use.rs

    // remote_use_instrument: remote/use.rs

<<<<<<< sync-sunrise-1
    /// Run BlockItem.place on a disposable level: where it placed and the
    /// blocks it set. The real level supplies collision shapes and
    /// building-blocking entities; random draws, scheduled ticks and events
    /// stay in the preview.
    pub(super) fn remote_can_place(
        &mut self,
        hit: &super::Hit,
        stack: &ItemStack,
    ) -> Option<DryPlace> {
        // A crank takes the use (game/kinetics.rs): nothing is predicted against it.
        if !self.may_build() || self.kinetic_crank_at(hit.pos) {
            return None;
        }
        let ctx = sunrise_world::level::PlaceContext {
            clicked_pos: sunrise_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
            face: sunrise_core::geometry::Direction::ALL[face_direction(hit.face) as usize],
            hit: Vec3::new(hit.point[0], hit.point[1], hit.point[2]),
            inside: hit.face == [0; 3],
            yaw: self.interaction_rotation().0,
            pitch: self.interaction_rotation().1,
            sneaking: self.remote_shift_down(),
            item: stack.item.to_string(),
            can_use_game_master_blocks: self.can_use_game_master_blocks(),
        };
        self.with_level(|b, level, _| {
            let mut level = DryLevel::new(level);
            match b.place_block(
                &mut level,
                &mut sunrise_world::level::Ticks::default(),
                &ctx,
            ) {
                sunrise_world::level::PlaceOutcome::Placed { pos, .. } => {
                    Some((pos, level.changed.into_iter().collect()))
                }
                sunrise_world::level::PlaceOutcome::Failed => None,
            }
        })
    }

    /// `MultiPlayerGameMode.useItem`'s local `ItemStack.use` for a bucket:
    /// `BucketItem.use` picks its own target and runs `pickupBlock` or
    /// `emptyContents` on this client's level inside `startPrediction`, so
    /// its success (which decides `itemUsed`) and the blocks it changes are
    /// the client's own. A fill into a waterloggable block changes nothing
    /// here: `SimpleWaterloggedBlock.placeLiquid` tests `isClientSide` and
    /// leaves the block update to the server.
    fn remote_use_bucket(
        &mut self,
        stack: &ItemStack,
    ) -> (bool, Option<DryChanges>, Vec<DrySound>) {
        // `BucketItem.use`: `level.mayInteract` and `mayUseItemAt`.
        // `Player.mayUseItemAt` tests the clicked block against `can_place_on`.
        if !self.may_build() {
            let aim = if &*stack.item == "minecraft:bucket" {
                self.bucket_hit()
            } else {
                self.pick()
            };
            if !aim.is_some_and(|hit| self.can_place_on_in_adventure(hit.pos)) {
                return (false, None, Vec::new());
            }
        }
        if &*stack.item == "minecraft:bucket" {
            let Some(hit) = self.bucket_hit() else {
                return (false, None, Vec::new());
            };
            let pos = sunrise_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
            let creative = self.creative;
            return self.with_level(|b, level, ticks| {
                let mut level = DryLevel::new(level);
                let picked = b.bucket_pickup(&mut level, ticks, pos, creative).is_some();
                (
                    picked,
                    Some(level.changed.into_iter().collect()),
                    std::mem::take(&mut level.sounds),
                )
            });
        }
        let Some((content, sound, _)) = self.predicted_bucket_contents(stack) else {
            return (false, None, Vec::new());
        };
        let Some(hit) = self.pick() else {
            return (false, None, Vec::new());
        };
        let target = self.bucket_target(content, &hit);
        let pos = sunrise_core::position::BlockPos::new(target[0], target[1], target[2]);
        let [x, y, z] = hit.beside();
        let beside = Some(sunrise_core::position::BlockPos::new(x, y, z));
        let sneaking = self.remote_shift_down();
        let creative = self.creative;
        self.with_level(|b, level, ticks| {
            let mut level = DryLevel::new(level);
            let emptied = b.bucket_empty(
                &mut level, ticks, pos, beside, content, sound, sneaking, creative,
            );
            // Water poured into a liquid container (where it went, which may
            // be the block beside a refusing one) fills it server-side only.
            let client_change = !(content.is_water()
                && emptied.is_some_and(|at| {
                    use sunrise_world::level::LevelAccess as _;
                    b.is_liquid_container(level.block_state(at))
                }));
            (
                emptied.is_some(),
                client_change.then(|| level.changed.into_iter().collect()),
                std::mem::take(&mut level.sounds),
            )
        })
    }

    /// Whether `held`'s `Item.use` needs a target this client sees none of
    /// (not SUCCESS): BucketItem.use (a fluid source or powder snow to pick
    /// up through `ClipContext.Fluid.SOURCE_ONLY`, a block to empty a full
    /// one against, `mayUseItemAt`), BottleItem.use (water to fill from) and
    /// BoatItem.use (a block or fluid surface with room for the boat).
    fn use_finds_no_target(&self, held: &ItemStack) -> bool {
        let id = &*held.item;
        let facts = |pos: [i32; 3]| {
            self.res
                .facts()
                .get(self.world.block(pos[0], pos[1], pos[2]))
        };
        let source = |want: Option<Fluid>| {
            let fluid_hit = self.pick_fluid();
            // pick_fluid steps through waterlogged blocks, but their water is
            // a source the SOURCE_ONLY ray stops at (and a BucketPickup).
            if let Some(hit) = self.pick()
                && facts(hit.pos).waterlogged
            {
                let eye = self.aim().map_or(hit.point, |(eye, _)| eye);
                let d = |p: [f64; 3]| (0..3).map(|a| (p[a] - eye[a]).powi(2)).sum::<f64>();
                if fluid_hit
                    .as_ref()
                    .is_none_or(|fluid| d(fluid.point) >= d(hit.point))
                {
                    return want.is_none_or(|want| want == Fluid::Water);
                }
            }
            fluid_hit.is_some_and(|hit| {
                let f = facts(hit.pos);
                let fluid = f.fluid != Fluid::None && f.fluid_level == 8 && !f.fluid_falling;
                match want {
                    Some(want) => fluid && f.fluid == want,
                    None => fluid || self.is_powder_snow(hit.pos),
                }
            })
        };
        // SolidBucketItem only has useOn: its use is Item.use's PASS.
        if id == "minecraft:powder_snow_bucket" {
            return true;
        }
        if id == "minecraft:bucket" {
            return !self.may_build() || !source(None);
        }
        if sunrise_client::items::bucket_contents(id).is_some() {
            return !self.may_build() || self.pick().is_none();
        }
        if id == "minecraft:glass_bottle" {
            // A dragon's breath cloud never counts here: its owner is not
            // synced, so this client's BottleItem.use finds none and PASSes;
            // the packet still goes and the server fills the bottle.
            return !source(Some(Fluid::Water));
        }
        if super::boats::any_boat_wood(id).is_some() {
            return self.boat_to_place(held).is_none();
        }
        false
    }
    /// `Item.use` runs on the client too (MultiPlayerGameMode.useItem): an
    /// item held in use until the button lets go starts being used here, so
    /// that letting go sends RELEASE_USE_ITEM (Minecraft.handleKeybinds) and
    /// the server ends the use: a bow or a trident is let go, a crossbow's
    /// charge ends, a shield is lowered, a spyglass stops scoping. Their
    /// sounds, loaded arrows and thrown tridents come from the server.
    /// False when the use FAILs here: a bow or an uncharged crossbow with
    /// nothing to shoot.
    fn remote_start_held_use(&mut self, held: &ItemStack) -> bool {
        if self.using.is_some() {
            return true;
        }
        let bundle = self.items().get(&held.item).is_some_and(|i| i.bundle);
        let start = match &*held.item {
            // BundleItem.use: always starts; the server drops the contents.
            _ if bundle => true,
            // BowItem.use: with an arrow, or in creative.
            "minecraft:bow" => self.get_projectile(held).is_some() || self.creative,
            // CrossbowItem.use: charging when not charged and with ammunition;
            // a charged one shoots on the server.
            "minecraft:crossbow" => {
                if held.data().is_some_and(|d| !d.charged.is_empty()) {
                    return true;
                }
                self.get_projectile(held).is_some()
            }
            // TridentItem.use: not about to break, Riptide only in water or rain.
            "minecraft:trident" => {
                // FAIL: no dip, and the off hand gets its turn.
                return self.use_trident(held);
            }
            // SpyglassItem.use (its sound is the server's).
            "minecraft:spyglass" => true,
            // InstrumentItem.use: the horn plays here too (`playSound(player, ...)`
            // skips the player's own client), the server's copy tells the others.
            "minecraft:goat_horn" => {
                self.remote_use_instrument(held);
                return true;
            }
            // EnderEyeItem.use: startUsingItem unless an End portal frame is
            // aimed at, with or without a stronghold to throw the eye at.
            "minecraft:ender_eye" => !self.aims_at_end_portal_frame(),
            // Item.use: a kinetic weapon starts being used.
            id if sunrise_client::spear::Kinetic::of(id).is_some() => true,
            // ShieldItem.use: anything that blocks attacks, off cooldown.
            // Item.use: a consumable or `blocks_attacks` the server's patch
            // adds starts the use (Consumable.startConsuming, startUsingItem).
            _ => {
                if !self.raise_shield(self.held_slot()) && self.patched_held_use(held) {
                    self.start_using_item();
                }
                return true;
            }
        };
        if start {
            self.start_using_item();
        }
        start || !matches!(&*held.item, "minecraft:bow" | "minecraft:crossbow")
    }
    /// `InstrumentItem.use` on this client: unless the horn is on cooldown,
    /// it is held up, plays its sound at the player and starts its cooldown.
    fn remote_use_instrument(&mut self, held: &ItemStack) {
        let Some(instrument) = self.res.items().instrument(held) else {
            return;
        };
        let group = sunrise_client::cooldowns::group(held, None);
        if self.cooldowns.is_on_cooldown(&group) {
            return;
        }
        self.start_using_item();
        self.play_player_bound_sound(&instrument.sound, 2, instrument.range / 16.0, 1.0, true);
        if instrument.use_duration > 0.0 {
            self.add_cooldown(group, instrument.use_ticks());
        }
    }
    /// Whether `stack` stays in use until the button lets go (bows,
    /// crossbows, tridents, spyglasses, shields, eyes of ender, whose
    /// `getUseDuration` 0 never counts down to the end) or the server ends
    /// it (a mod's item): its use only counts ticks here.
    fn held_until_released(&self, stack: &ItemStack) -> bool {
        if self.mod_use(stack) {
            return true;
        }
        matches!(
            &*stack.item,
            "minecraft:bow"
                | "minecraft:crossbow"
                | "minecraft:trident"
                | "minecraft:spyglass"
                | "minecraft:ender_eye"
        ) || sunrise_client::spear::Kinetic::of(&stack.item).is_some()
            || self.blocks_attacks(stack).is_some()
            || self.patched_held_use(stack)
    }
=======
>>>>>>> sunrise/main
    /// Whether the server's patch gives `stack`, an item with no consumable
    /// of its own, a `consumable` with a use time or `blocks_attacks` (a
    /// sword made to block): `Item.use` starts a use of it
    /// (`Consumable.startConsuming`, `startUsingItem`) that lasts until the
    /// button lets go or the server stops it, as the client's
    /// `LivingEntity.updateUsingItem` never completes one.
    fn patched_held_use(&self, stack: &ItemStack) -> bool {
        let (ticks, blocks) = sunrise_client::item_sync::patched_use(stack);
        ticks.map_or(blocks, |ticks| ticks > 0)
            && self
                .items()
                .get(&stack.item)
                .is_none_or(|i| i.consumable.is_none())
    }

<<<<<<< sync-sunrise-1
    /// The rest of Minecraft.startUseItem once the main hand's target passed:
    /// the main hand's item, then the off hand's turn (its block, its item).
    /// The server does each packet's part with that packet's hand only.
    fn remote_use_hands(&mut self, attempt: mod_api::InteractionAttempt) {
        #[cfg(target_os = "android")]
        if self.mobile.touch_shield {
            let hand = if self
                .offhand
                .as_ref()
                .is_some_and(|s| self.blocks_attacks(s).is_some())
            {
                Hand::Off
            } else {
                Hand::Main
            };
            if self.remote_use_item(hand) {
                self.heights.item_used(hand);
            }
            return;
        }
        // Nothing in the crosshair: an empty hand's click is heard, by mods only.
        let miss = mod_api::__private::listened(mod_api::HookId::RightClickEmpty)
            && !self.entity_targeted()
            && self.pick().is_none();
        if miss && self.held().is_none() {
            self.native_right_click_empty(Hand::Main);
        }
        // A successful Item.use always drops the hand (LocalPlayer.itemUsed).
        if self.remote_use_item(Hand::Main) {
            self.heights.item_used(Hand::Main);
            return;
        }
        // The off hand's turn: the event again, then the entity or the block.
        if let Some(swing) = self
            .native_interaction(InteractionKey::Use, Hand::Off, attempt)
            .canceled_swing()
        {
            if swing {
                self.swing_hand(Hand::Off, SwingAnimation::DEFAULT);
            }
            return;
        }
        let off_target = if self.remote_use_targets_entity() {
            self.remote_use_entity(Hand::Off)
        } else {
            self.remote_use_block(Hand::Off)
        };
        if off_target {
            return;
        }
        if miss && self.offhand.is_none() {
            self.native_right_click_empty(Hand::Off);
        }
        if self.remote_use_item(Hand::Off) {
            self.heights.item_used(Hand::Off);
        }
    }
    /// `LivingEntity.updatingUsingItem` on a server client: an item held in
    /// use (`held_until_released`), and a consumable's eating animation and
    /// its crumbs until the server's USE_ITEM_COMPLETE. The server sends the
    /// stack, its hunger and the sounds.
    fn remote_tick_using(&mut self) {
        let Some((slot, ticks)) = self.using else {
            // A mod's use the game reset (the server stopping the use...).
            self.end_mod_use();
            return;
        };
        let stack = self.stack_in(slot).cloned();
        // The use stops when the hand no longer holds the item it began with
        // (ItemStack.isSameItem).
        let item = stack.as_ref().map(|s| Arc::clone(&s.item));
        if let Some(r) = self.remote.as_mut() {
            if ticks == 0 {
                r.use_item = item;
            } else if r.use_item != item {
                self.using = None;
                self.end_mod_use();
                return;
            }
        }
        // Another hotbar slot picked: LivingEntity.updatingUsingItem sees a
        // different stack in the hand. The off hand keeps its item.
        let in_hand = slot == self.selected || slot == super::shield::OFFHAND_SLOT;
        // A mod's item: held until the button lets go, counted for the mod.
        if self.mod_use_tick(slot, ticks, !self.right || !in_hand) {
            return;
        }
        // updatingUsingItem runs here too: a use a mod's tick ran out stays
        // frozen until let go (RELEASE_USE_ITEM) or the server finishes it.
        if let Some(s) = stack.as_ref().filter(|_| in_hand)
            && !self.use_item_ticked(s, ticks)
        {
            return;
        }
        // BrushItem.onUseTick: with no block aimed the use ends (the server
        // ends it too); a stroke shows its dust. Its sounds are the server's.
        if stack
            .as_ref()
            .is_some_and(|s| &*s.item == "minecraft:brush")
        {
            let elapsed = ticks + 1;
            let hit = self.pick().filter(|_| in_hand);
            if let Some(hit) = &hit
                && elapsed % 10 == 5
            {
                self.brush_dust(hit, slot == super::shield::OFFHAND_SLOT);
            }
            if hit.is_none() && in_hand {
                self.send_action(6);
            }
            self.using = hit.and(Some((slot, elapsed))).filter(|_| elapsed < 200);
            return;
        }
        // A bundle's 200-tick use ends with the server's entity event 9, not here.
        if stack
            .as_ref()
            .is_some_and(|s| self.items().get(&s.item).is_some_and(|i| i.bundle))
        {
            self.using = in_hand.then_some((slot, ticks + 1));
            return;
        }
        // A drawn bow, a charging crossbow, a raised shield...: held until
        // the button lets go.
        if let Some(s) = stack.as_ref().filter(|s| self.held_until_released(s)) {
            if in_hand && (self.mod_use(s) || self.blocks_attacks(s).is_some()) {
                self.item_on_use_tick(s, ticks);
            }
            self.using = in_hand.then_some((slot, ticks + 1));
            return;
        }
        // A goat horn is held up for its instrument's use duration.
        if let Some(duration) = stack
            .as_ref()
            .and_then(|s| self.res.items().instrument(s))
            .map(|i| i.use_ticks().max(0) as u32)
        {
            self.using = (in_hand && ticks + 1 < duration).then_some((slot, ticks + 1));
            return;
        }
        let consumable = stack.as_ref().and_then(|s| self.consumable(s));
        let (Some(stack), Some(consumable)) = (stack, consumable) else {
            self.using = None;
            return;
        };
        // Button release belongs to the next handleKeybinds phase, which
        // sends RELEASE_USE_ITEM. Do not silently clear a use begun by a
        // quick press/release between ticks.
        if !in_hand {
            self.using = None;
            return;
        }
        // Consumable.startConsuming eats one with no use time at once, without
        // a use (nor a USE_ITEM_COMPLETE).
        if consumable.ticks() <= 0 {
            self.using = None;
            return;
        }
        // Consumable.shouldEmitParticlesAndSounds, as in tick_used_item. The
        // countdown goes on past zero (LivingEntity.updateUsingItem completes
        // only on the server) until the server's USE_ITEM_COMPLETE.
        // getUseItemRemainingTicks: the duration this use started with.
        let remaining = self.use_duration - ticks as i32;
        if consumable.should_emit(remaining) {
            // `emit_consume` plays the eat sound and spawns the bites (when the
            // item has consume particles), as `tick_used_item` does on the
            // player's own server.
            self.emit_consume(&consumable, &stack, 5);
        }
        self.item_on_use_tick(&stack, ticks);
        self.using = Some((slot, ticks + 1));
    }
    /// `Player.handleEntityEvent(9)`: `completeUsingItem` on the client. A consumable's last crumbs, and FoodProperties.onConsume,
    /// which runs on the client too (Consumable.onConsume calls its
    /// ConsumableListeners on both sides): the client's FoodData eats at once,
    /// so a held right click finds `Player.canEat` false instead of starting
    /// another bite before the server's ClientboundSetHealth arrives.
    fn remote_complete_using(&mut self) {
        let Some((slot, _)) = self.using.take() else {
            return;
        };
        let Some(stack) = self.stack_in(slot).cloned() else {
            return;
        };
        let Some(info) = self.items().get(&stack.item).cloned() else {
            return;
        };
        // Consumable.onConsume's emitParticlesAndSounds(16): the last crumbs
        // and bite, which the server plays for the others only.
        if let Some(consumable) = self.consumable(&stack) {
            self.emit_consume(&consumable, &stack, 16);
        }
        if let Some((nutrition, saturation)) = info.food
            && let Some(r) = self.remote.as_mut()
        {
            // FoodData.eat(nutrition, saturation), as finish_consuming does;
            // the next ClientboundSetHealth replaces it.
            let (_, food, sat) = &mut r.health;
            *food = (*food + nutrition as i32).clamp(0, 20);
            *sat = (*sat + saturation).clamp(0.0, *food as f32);
            self.apply_remote_health();
        }
        // applyAfterUseComponentSideEffects, then LivingEntityUseItemEvent.Finish.
        self.remote_use_item_finished(slot, &stack, &info);
    }
    pub(super) fn request_remote_mining_stop(&mut self) {
        if let Some(r) = self.remote.as_mut() {
            r.stop_mining = true;
        }
    }

    fn remote_handle_input(&mut self) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let mut input = r.input.take_queued();
        let stop = std::mem::take(&mut r.stop_mining);
        // Minecraft.tick runs handleKeybinds only while `gui.screen() == null`: a
        // book, a sign editor, a dialog or a prompt over the world (each a
        // `Screen` in vanilla) stops the attack and use mappings from acting,
        // and `KeyMapping.releaseAll` let go of the buttons when it opened
        // (`track_toggle_screens`), so a dig held into a book ends (#9729).
        let ready = r.loaded_sent
            && self.screen == Screen::Playing
            && self.sleeping.is_none()
            && !self.screen_over_playing();
        // MultiPlayerGameMode.tick, first in Minecraft.tick, screen or not: the slot
        // chosen since the last tick (the wheel, the last tick's number keys, the
        // server's SetHeldSlot) goes out before anything else of this tick.
        self.remote_send_carried();
        if stop || !ready {
            self.stop_remote_mining();
        }
        if !ready {
            // A screen's opening released the keys (`KeyMapping.releaseAll`).
            self.hotbar_presses.clear();
            return;
        }
        let (attack, use_item) = (self.left, self.right);
        // handleKeybinds: keySwapOffhand.consumeClick, before attacking and using (an
        // anticheat's Post check wants it before the tick's move, not whenever the key went down).
        let swaps = self
            .remote
            .as_mut()
            .map_or(0, |r| std::mem::take(&mut r.swap_presses));
        if !self.spectator() {
            for _ in 0..swaps {
                self.remote_swap_hands();
            }
        }
        input.tick(&mut RemoteInputWorld(self), attack, use_item);
        self.remote_queued_keys();
        if let Some(r) = self.remote.as_mut() {
            input.merge_queued(std::mem::take(&mut r.input));
            r.input = input;
        }
    }
=======
    // remote_use_hands: remote/use.rs

    // remote_complete_using: remote/use.rs
>>>>>>> sunrise/main

    // remote_send_carried: remote/sync.rs

    /// `MultiPlayerGameMode.attack` runs `Player.attack` on this client too: its
    /// `isSweepAttack` for a damaging hit on `target` (`living`: a
    /// `LivingEntity`, which may be hit critically), heard by native mods on
    /// both sides. Only the server sweeps.
    fn remote_sweep_event(&mut self, target: &str, living: bool) {
        if mod_api::__private::mods().is_empty() {
            return;
        }
        let (damage, scale) = self.remote_attack_damage();
        let magic = scale * (self.enchanted_damage(target, damage, None) - damage);
        if damage <= 0.0 && magic <= 0.0 {
            return;
        }
        let crit = living && self.critical_attack(scale);
        self.sweep_decision(scale, crit, target);
    }

    /// The entity type of the boat, minecart, armor stand or End crystal with
    /// server id `id` that an attack reaches `Player.attack`'s sweep check on,
    /// and whether it is living. A painting or item frame is not: on this
    /// client `skipAttackInteraction` hurts it and stops the attack there.
    fn remote_attacked_object(&self, id: i32) -> Option<(String, bool)> {
        let id = id as u32;
        if let Some(b) = self.boats.iter().find(|b| b.id == id) {
            return Some((b.entity_type(), false));
        }
        if let Some(c) = self.carts.iter().find(|c| c.id == id) {
            return Some((c.kind.id().to_owned(), false));
        }
        if self.armor_stands.iter().any(|a| a.id == id) {
            return Some(("minecraft:armor_stand".to_owned(), true));
        }
        self.crystals
            .iter()
            .any(|c| c.id == id)
            .then(|| ("minecraft:end_crystal".to_owned(), false))
    }

    // remote_player_attack: remote/ui.rs

    /// The latency the server's player list gives this player
    /// (`PlayerInfo.getLatency`), none before the server lists it. A new entry reads 0 until the
    /// server's first latency update (every 600 ticks).
    pub(in crate::game) fn own_latency(&self) -> Option<i32> {
        let r = self.remote.as_ref()?;
        r.tab_players.get(&r.profile).map(|me| me.latency)
    }

    /// An attribute's value on this client's player, if its map has the attribute.
    fn own_attribute(&mut self, id: &str) -> Option<f64> {
        let p = self.player.as_mut()?;
        let key = p.attributes.key(id)?;
        p.attributes.lookup(Some(&key))?;
        p.attributes
            .value(Some(key), sunrise_world::attribute_map::ValueKind::Computed)
            .ok()
    }

<<<<<<< sync-sunrise-1
    /// Keys with a multiplayer meaning; returns true when consumed.
    pub(super) fn remote_key(&mut self, key: KeyCode) -> Result<bool> {
        if self.screen != Screen::Playing {
            if (key == KeyCode::KeyE || key == KeyCode::Escape) && self.screen == Screen::Inventory
            {
                self.remote_close_container();
                self.set_screen(Screen::Playing);
                self.capture(true);
                return Ok(true);
            }
            return Ok(false);
        }
        match key {
            KeyCode::KeyT | KeyCode::Slash | KeyCode::Enter => {
                // ChatScreen, over the server's command tree.
                self.open_chat_method(if key == KeyCode::Slash { "/" } else { "" });
                // `Minecraft.handleKeybinds` opens it in the call that then
                // releases an active use (Gui.handleKeybinds runs before the
                // isUsingItem branch).
                self.release_use_for_keybind_screen();
                Ok(true)
            }
            KeyCode::KeyQ => {
                if self.spectator() {
                    return Ok(true);
                }
                if let Some(r) = self.remote.as_mut() {
                    r.pending_keys.push(key);
                }
                Ok(true)
            }
            // Open the ridden vehicle's own screen (`HasCustomInventoryScreen`)
            // instead of the player's inventory: riding a horse, llama or
            // chest boat, the inventory key sends
            // ServerboundPlayerCommandPacket OPEN_INVENTORY.
            KeyCode::KeyE if self.riding_mount_with_menu() || self.riding_chest_boat() => {
                if self.in_portal_transition() {
                    return Ok(true);
                }
                if let Some(r) = self.remote.as_mut() {
                    r.pending_keys.push(key);
                }
                Ok(true)
            }
            KeyCode::KeyE => {
                if self.in_portal_transition() {
                    return Ok(true);
                }
                // The inventory opens without telling the server.
                if self.tutorial.open_inventory() {
                    self.persist_tutorial_step();
                }
                self.recipe_ui.reset_screen();
                if self.creative {
                    self.set_screen(Screen::Creative);
                    self.refresh_creative_tabs();
                    self.creative_screen.reopen();
                    self.refresh_saved_hotbars();
                } else {
                    self.set_screen(Screen::Inventory);
                }
                self.craft_size = 2;
                self.capture(false);
                // `Minecraft.handleKeybinds` opens the inventory in the same
                // call that then releases an active use.
                self.release_use_for_keybind_screen();
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    /// `Minecraft.handleKeybinds`: a key that opens a screen runs
    /// `Gui.setScreen`, whose `KeyMapping.releaseAll` clears the use key, and
    /// the same invocation then releases an active use
    /// (`player.isUsingItem() && !keyUse.isDown()` → `releaseUsingItem`).
    /// Only the screens opened from those keybinds do this (inventory, chat,
    /// advancements); the pause menu, server containers, mount inventories and
    /// dialogs release on the first tick after closing instead, as vanilla
    /// does.
    pub(super) fn release_use_for_keybind_screen(&mut self) {
        // Only a server's client holds a use to release.
        if self.remote.is_none() {
            return;
        }
        // `KeyMapping.releaseAll` clears key.use; without this the button still
        // reads held and a use would start again on the first tick back.
        self.right = false;
        sunrise_client::interaction_input::release_on_screen_open(&mut RemoteInputWorld(self));
    }
    /// `MultiPlayerGameMode.releaseUsingItem`: sends `RELEASE_USE_ITEM` and
    /// runs the client half of `Item.releaseUsing` (a Riptide launch moves the
    /// player itself). Vanilla reaches it both when the use key is let go and,
    /// in the same tick, when a keybind opens a screen.
    fn release_remote_use(&mut self) {
        self.send_action(6);
        // a mod's Stop may skip the item's release, and a mod's own release follows.
        let hand = self.used_hand();
        if let Some((slot, ticks)) = self.using.take()
            && let Some(hand) = hand
            && let Some(stack) = self.stack_in(slot).cloned()
        {
            self.with_hand(hand, |g| {
                if !g.use_item_stopped(&stack, ticks) {
                    return;
                }
                if &*stack.item == "minecraft:trident" {
                    g.release_trident(g.ticks_held(&stack, ticks));
                }
                g.mod_release_using(&stack, ticks);
            });
        }
    }
    /// ClientPacketListener.sendChat / sendCommand, signed with a chat
    /// session (game/network/remote_chat.rs). The Chat option's Hidden and System
    /// levels keep chat out and Hidden keeps the commands out (ChatAbilities).
    pub(super) fn send_signed_chat_input(&mut self, message: &str) {
        let message = message.trim();
        if message.is_empty() || !self.chat_abilities().allows(message) {
            return;
        }
        if let Some(command) = message.strip_prefix('/') {
            if self.studio_command(command) {
                return;
            }
            self.send_command(command);
        } else {
            self.send_signed_chat(message);
        }
    }
    /// key.swapOffhand pressed: the next tick sends it (`remote_swap_hands`).
    pub(super) fn remote_swap_pressed(&mut self) {
        if let Some(r) = self.remote.as_mut() {
            r.swap_presses += 1;
        }
    }
    /// ChatScreen.handleChatInput fires the client event for chat, not commands.
    pub(super) fn send_chat_from_screen(&mut self, message: &str) {
        if message.starts_with('/') || !mod_api::__private::listened(mod_api::HookId::ClientChat) {
            self.send_signed_chat_input(message);
        } else if let Some(uuid) = self.remote.as_ref().map(|r| r.profile) {
            let id = self.own_entity_id();
            let sent = chat_after_event(message, |outgoing, cancel| {
                self.native_client_chat(id, uuid, message, outgoing, cancel);
            });
            if let Some(outgoing) = sent {
                self.send_signed_chat(&outgoing);
            }
        }
    }
=======
    // remote_key: remote/ui.rs

>>>>>>> sunrise/main
    /// MultiPlayerGameMode.dropItem (key.drop): predict the removal, then send
    /// the action; swing only if a nonempty stack was removed.
    pub(super) fn remote_drop(&mut self, all: bool) {
        // ensureHasSentCarriedItem: the server drops from the slot just chosen.
        self.remote_send_carried();
        // MultiPlayerGameMode.dropItem predicts the removal and sends only the drop: the
        // server's stack agrees, so no creative slot edit follows at the end of the tick.
        let before = self.creative_stacks();
        let dropped =
            sunrise_client::items::take_held_drop(&mut self.inventory[self.selected], all, false);
        self.remote_synced_since(&before);
        self.send_action(if all { 4 } else { 5 });
        if dropped.is_some() {
            self.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
        }
    }

    /// Minecraft.handleKeybinds, key.swapOffhand: the server swaps the hands
    /// (ServerboundPlayerActionPacket SWAP_ITEM_WITH_OFFHAND at BlockPos.ZERO, facing down) and
    /// sends both slots back; nothing is predicted.
    pub(super) fn remote_swap_hands(&mut self) {
        self.send_action(7);
        // Equal stacks swap with no slot update to wait for.
        let (main, off) = (self.stack_in(self.selected), self.offhand.as_ref());
        let moves = !super::hand::matches_for_swap(main, off);
        if let Some(r) = self.remote.as_mut() {
            r.swap_answer = [moves; 2];
        }
    }
<<<<<<< sync-sunrise-1
    /// The server set inventory `slot` (`shield::OFFHAND_SLOT` for the off
    /// hand), `changed` from the stack this client held there. The stack
    /// that answers a swap (`setItemInHand`) is another object, which the
    /// hand shows after the swap animation (`hand_stack_replaced`); other
    /// updates of a hand's slot count as the held stack changing in place,
    /// as this client doesn't predict every use the way vanilla's does
    /// (a thrown snowball's stack shrinks only when the server says so).
    fn remote_hand_slot_set(&mut self, slot: usize, changed: bool) {
        let hand = if slot == super::shield::OFFHAND_SLOT {
            1
        } else if slot == self.selected {
            0
        } else {
            return;
        };
        let answer = self
            .remote
            .as_mut()
            .is_some_and(|r| std::mem::take(&mut r.swap_answer[hand]));
        if answer && changed {
            self.hand_stack_replaced(slot);
        }
    }
    /// A container click: the server answers with the authoritative slots.
    pub(super) fn remote_click(&mut self, right: bool) {
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return;
        };
        let batch = self.interface_batch(size);
        let Some(internal) = self.slot_at(&batch, self.cursor.0, self.cursor.1) else {
            return;
        };
        let input = if self.has_shift_down() {
            Input::QuickMove
        } else {
            Input::Pickup
        };
        if self.screen == Screen::Creative {
            // CreativeModeInventoryScreen.slotClicked: the Survival Inventory tab clicks the
            // player's own InventoryMenu, and its CreativeInventoryListener sends each slot that
            // changed (SetCreativeModeSlot). No ContainerClick: a server holding the carried
            // stack would give it back after the Destroy Item slot cleared it (#3307).
            self.click_open_inventory(Some(internal), i32::from(right), input);
            return;
        }
        self.remote_menu_ops(&[(Some(internal), i32::from(right), input)]);
    }
    /// `AbstractContainerMenu.setItem`/`initializeContents`: the state id a
    /// click answers with is the open menu's, not another window's.
    fn remote_menu_state(&mut self, container: i32, state: i32) {
        let open = self.remote_menu().0;
        if let Some(r) = self.remote.as_mut().filter(|_| container == open) {
            r.state_id = state;
        }
    }
    /// The server's open menu: its window id and menu type (0, "inventory"
    /// when none is open).
    pub(super) fn remote_menu(&self) -> (i32, String) {
        match self.remote.as_ref().and_then(|r| r.menu.as_ref().cloned()) {
            Some((id, name)) => (id, name),
            None => (0, "inventory".to_owned()),
        }
    }
    /// `MultiPlayerGameMode.handleContainerInput` for each click of `ops` on
    /// the server's open menu (a quick craft drag is several): the verified
    /// menu engine predicts them on this client's copy, then each goes out as
    /// a `ContainerClick` with the slots it changed and the carried stack
    /// after it; the server corrects anything it sees differently. A menu the
    /// engine doesn't model sends the clicks unpredicted. False when a click's
    /// slot isn't one of the menu's.
    pub(super) fn remote_menu_ops(&mut self, ops: &[(Option<usize>, i32, Input)]) -> bool {
        // CrafterScreen.slotClicked: a slot toggle goes out before the click.
        if let [(Some(slot), button, input)] = ops {
            self.crafter_slot_clicked(*slot, *button, *input);
        }
        let (_, menu) = self.remote_menu();
        let mut clicks = Vec::with_capacity(ops.len());
        for &(slot, button, input) in ops {
            let slot = match slot {
                None => -999,
                Some(internal) => match Self::menu_slot_index(&menu, internal) {
                    Some(s) => s as i16,
                    None => return false,
                },
            };
            clicks.push((slot, button, input));
        }
        self.remote_menu_clicks(&clicks);
        true
    }
    /// [`Game::remote_menu_ops`]'s clicks by the menu's own slot ids (a mod's `slots.click`).
    pub(super) fn remote_menu_clicks(&mut self, clicks: &[(i16, i32, Input)]) {
        let (container, _) = self.remote_menu();
        // Unpredicted: nothing changed, the carried stack as it is.
        let mut sent: Vec<Predicted> = clicks
            .iter()
            .map(|&(slot, button, input)| (slot, button, input, Vec::new(), self.carried.clone()))
            .collect();
        let mut threw = false;
        // The engine's slot indices are the menu's own, as on the network.
        self.menu_session(|m, env, _| {
            let Some((predicted, dropped)) = predict_clicks(m, env, clicks) else {
                // The server throws on it too: nothing is predicted.
                return Some(false);
            };
            sent = predicted;
            threw = dropped;
            Some(true)
        });
        self.send_clicks(container, sent);
        // LivingEntity.drop(.., Prediction.PREDICTED): the client swings its own
        // main hand; the server shows the swing only to the other players.
        if threw {
            self.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
        }
    }
    /// A QUICK_MOVE on the server's open menu with nothing predicted: the
    /// client's slots stay as they are and every slot it moved comes back from
    /// the server (`broadcastChanges`), the answer the next step waits for.
    /// A prediction the server agrees with gets no answer at all.
    pub(super) fn remote_quick_move_unpredicted(&mut self, internal: usize) -> bool {
        let (container, menu) = self.remote_menu();
        let Some(slot) = Self::menu_slot_index(&menu, internal) else {
            return false;
        };
        let carried = self.carried.clone();
        let click = (slot as i16, 0, Input::QuickMove, Vec::new(), carried);
        self.send_clicks(container, vec![click]);
        true
    }
    /// A Shift double click on a server (`AbstractContainerScreen.mouseReleased`):
    /// a predicted QUICK_MOVE for every slot the engine moves, each sent.
    pub(super) fn remote_quick_move_all(&mut self, slot: usize, moved: &ItemStack) {
        let (container, _) = self.remote_menu();
        let mut sent: Vec<Predicted> = Vec::new();
        self.menu_session(|m, env, to_menu| {
            let clicked = to_menu(slot)?;
            let mut before = menu_items(m);
            let mut each = |m: &sunrise_client::menu::Menu, t: usize| {
                let changed = changed_slots(&before, m);
                sent.push((t as i16, 0, Input::QuickMove, changed, m.carried.clone()));
                before = menu_items(m);
            };
            Some(
                m.quick_move_all_each(env, clicked, moved, &mut each)
                    .is_ok(),
            )
        });
        self.send_clicks(container, sent);
    }
    fn send_clicks(&mut self, container: i32, clicks: Vec<Predicted>) {
        // A result the engine claims to have changed is the one the server
        // agrees to (`click_agreed`), so it sends none: keep it. A result the
        // engine doesn't claim (a recipe it doesn't know) the server sends.
        if matches!(
            self.remote_menu().1.as_str(),
            "inventory" | "minecraft:crafting"
        ) && let Some(r) = self.remote.as_mut()
        {
            for (.., changed, _) in &clicks {
                if let Some((_, result)) = changed.iter().find(|(slot, _)| *slot == 0) {
                    r.craft_result = result.clone();
                }
            }
        }
        let state = self.remote.as_ref().map_or(0, |r| r.state_id);
        let hashed = |g: &Self, s: &Option<ItemStack>| g.hashed_stack(s);
        for (slot, button, input, changed, carried) in clicks {
            let changed = changed.iter().map(|(i, s)| (*i, hashed(self, s))).collect();
            let carried = hashed(self, &carried);
            self.send(S::ContainerClick(play::ContainerClick {
                container,
                state,
                slot,
                button: button as i8,
                input: input_ordinal(input),
                changed,
                carried,
            }));
        }
    }
    /// The server's open menu is the inventory or a chest-like container
    /// (chests, barrels, ender chests: `generic_9xN`; shulker boxes).
    pub(super) fn remote_storage_open(&self) -> bool {
        match self.remote.as_ref().and_then(|r| r.menu.as_ref()) {
            None => true,
            Some((_, name)) => {
                name == "inventory"
                    || name.starts_with("minecraft:generic_9x")
                    || name == "minecraft:shulker_box"
            }
        }
    }
    /// A click outside a server's window (slot -999): predicted like
    /// `MultiPlayerGameMode.handleContainerInput` (the carried stack after
    /// `doClick`; the server drops the stack and swings the hand).
    pub(super) fn remote_throw(&mut self, right: bool, input: sunrise_client::menu::Input) {
        use sunrise_client::menu::Input;
        let container = self
            .remote
            .as_ref()
            .and_then(|r| r.menu.as_ref())
            .map_or(0, |m| m.0);
        if input == Input::Pickup
            && let Some(c) = self.carried.as_mut()
        {
            if right && c.count > 1 {
                c.count -= 1;
            } else {
                self.carried = None;
            }
            self.swing_arm();
        }
        let carried = self.hashed_stack(&self.carried);
        let state = self.remote.as_ref().map_or(0, |r| r.state_id);
        self.send(S::ContainerClick(play::ContainerClick {
            container,
            state,
            slot: -999,
            button: i8::from(right),
            input: if input == Input::Throw { 4 } else { 0 },
            changed: Vec::new(),
            carried,
        }));
    }
    /// `CreativeModeInventoryScreen.slotClicked` outside the window: the server knows nothing of
    /// a stack picked from the creative screen, so the carried stack (an item tab's right click:
    /// one of it) is dropped through `handleCreativeModeItemDrop`, a SetCreativeModeSlot to slot
    /// -1, and the hand swings as `Player.drop` predicts.
    pub(super) fn remote_creative_drop(&mut self, right: bool) {
        let one = right && !self.creative_inventory_open();
        let Some(carried) = self.carried.as_mut() else {
            return;
        };
        let thrown = if one && carried.count > 1 {
            carried.count -= 1;
            ItemStack {
                count: 1,
                ..carried.clone()
            }
        } else {
            self.carried.take().expect("carried")
        };
        self.remote_creative_throw(thrown);
    }
    /// `MultiPlayerGameMode.handleCreativeModeItemDrop`: a stack thrown out of the creative
    /// screen goes to the server as a SetCreativeModeSlot to slot -1, which drops it from the
    /// player's hand; the client only swings (`Player.drop` on a client).
    pub(super) fn remote_creative_throw(&mut self, thrown: ItemStack) {
        if !self.creative {
            return;
        }
        self.swing_arm();
        let item = self.to_network(&Some(thrown));
        self.send(S::SetCreativeModeSlot(play::SetCreativeModeSlot {
            slot: -1,
            item,
        }));
    }
    pub(super) fn remote_close_container(&mut self) {
        let id = self
            .remote
            .as_ref()
            .and_then(|r| r.menu.as_ref().map(|m| m.0))
            .unwrap_or(0);
        self.send(S::ContainerClose(play::ContainerClose { container: id }));
        self.remote_close_local();
    }
    /// `MultiPlayerGameMode.handleInventoryButtonClick`: button `button` of
    /// the menu the server opened goes to it (`ContainerButtonClick`), whose
    /// menu decides what it does (`clickMenuButton`). No sound: that is the
    /// screen's. False while no such menu is open.
    pub(super) fn remote_button_click(&self, button: i32) -> bool {
        let Some(container) = self
            .remote
            .as_ref()
            .and_then(|r| r.menu.as_ref())
            .map(|(w, _)| *w)
        else {
            return false;
        };
        self.send(S::ContainerButtonClick(play::ContainerButtonClick {
            container,
            button,
        }));
        true
    }
    fn remote_reject_container(&mut self, container: i32) {
        self.remote_close_local();
        if self.screen == Screen::Inventory {
            self.set_screen(Screen::Playing);
            self.capture(true);
        }
        self.send(S::ContainerClose(play::ContainerClose { container }));
    }
    fn remote_close_local(&mut self) {
        // `Gui.setScreen` clears the focus of the screen it takes down: the
        // container's keyboard focus must not outlive it (a Tab bound to the
        // inventory moves the focus and closes the container in one press).
        self.pad_clear_focus();
        self.recipe_ui.reset_screen();
        if let Some(r) = self.remote.as_mut() {
            r.menu.set(None);
            r.set_menu_title(None);
            r.craft_result = None;
        }
        self.mount_menu_reset();
        // The client's menu has no level access: what it holds is the server's.
        self.station = None;
        // AbstractContainerMenu.removed: the server gives the grid and the
        // carried stack back to the inventory and syncs it; the copies here
        // are stale the moment the window closes.
        self.craft_grid.fill(None);
        self.carried = None;
        self.craft_size = 2;
        if let Some(key) = self.container.take() {
            // Server containers stay renderable but lose their copied slots.
            if let Some(entity) = self.world.block_entities.get_mut(&key) {
                for s in entity.slots_mut() {
                    *s = None;
                }
            }
        }
    }
    /// A server's trading window (MerchantScreen over a ClientSideMerchant).
    fn remote_open_merchant(
        &mut self,
        container: i32,
        title: String,
        text: sunrise_client::text::Text,
    ) {
        if let Some(r) = self.remote.as_mut() {
            r.menu
                .set(Some((container, "minecraft:merchant".to_owned())));
            r.set_menu_title(Some(text));
        }
        let session = sunrise_client::merchant::Session::new(Vec::new(), true);
        self.open_remote_merchant(session, (!title.is_empty()).then_some(title));
    }
    /// ClientboundMerchantOffersPacket.
    fn remote_merchant_offers(&mut self, p: play::MerchantOffers) {
        if self.open_menu_name(p.container) != "minecraft:merchant" {
            return;
        }
        let cost = |g: &Self, c: &play::NetCost| {
            g.to_stack(&Some(c.clone().into()))
                .map(|stack| sunrise_client::merchant::ItemCost { stack })
        };
        let offers: Vec<_> = p
            .offers
            .iter()
            .filter_map(|o| {
                let mut offer = sunrise_client::merchant::MerchantOffer {
                    base_cost_a: cost(self, &o.cost_a)?,
                    cost_b: match &o.cost_b {
                        Some(b) => Some(cost(self, b)?),
                        None => None,
                    },
                    result: self.to_stack(&Some(o.result.clone()))?,
                    uses: o.uses,
                    max_uses: o.max_uses,
                    reward_exp: true,
                    special_price_diff: o.special_price,
                    demand: o.demand,
                    price_multiplier: o.price_multiplier,
                    xp: o.xp,
                };
                // setToOutOfStock.
                if o.out_of_stock {
                    offer.uses = offer.max_uses;
                }
                Some(offer)
            })
            .collect();
        if let Some(m) = self.station.as_mut().and_then(|s| s.merchant.as_mut()) {
            m.session.offers = offers;
            m.session.level = p.level;
            m.session.xp = p.xp;
            m.session.villager = p.show_progress;
        }
    }
    /// ClientboundOpenScreenPacket for the menus Sunrise draws.
    fn remote_open_screen(&mut self, p: play::OpenScreen) {
        // A new RecipeBookComponent owns fresh ghost slots and an overlay,
        // even when the server replaces a menu without a close packet.
        self.recipe_ui.reset_screen();
        let menu = self
            .remote
            .as_ref()
            .and_then(|r| {
                r.builtin
                    .get("minecraft:menu")?
                    .name(p.menu)
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        let target = self
            .remote
            .as_mut()
            .and_then(|r| r.use_target.take())
            .map(|t| (t[0], t[1], t[2]));
        // The server names a mod menu's block itself: it, not the last
        // clicked block, is the one the menu is of. Only for this window:
        // any other's is stale.
        let extra = self
            .remote
            .as_mut()
            .and_then(|r| r.menu_extra.take())
            .filter(|e| e.window == p.container);
        let target = extra
            .as_ref()
            .map_or(target, |e| e.pos.map(|p| (p[0], p[1], p[2])));
        if menu == "minecraft:lectern" {
            // LecternScreen: the book and page come in the menu's content and data.
            if let Some(r) = self.remote.as_mut() {
                r.menu.set(Some((p.container, menu)));
                r.set_menu_title(None);
            }
            self.remote_open_lectern(target.unwrap_or_default());
            return;
        }
        if let Some(kind) = Self::remote_station_kind(&menu) {
            let title = menu_title(&p.title);
            let pos = target.map_or([0, 0, 0], |t| [t.0, t.1, t.2]);
            self.remote_open_station(p.container, menu, kind, title, pos);
            return;
        }
        if menu == "minecraft:merchant" {
            // A trading window: its offers follow in a MerchantOffers packet.
            let title = plain(
                &p.title,
                &self
                    .remote
                    .as_ref()
                    .map(|r| r.lang.clone())
                    .unwrap_or_default(),
            );
            self.remote_open_merchant(p.container, title, menu_title(&p.title));
            return;
        }
        let mod_type = sunrise_client::containers::mod_menu_type(&menu);
        let entity = match menu.as_str() {
            "minecraft:crafting" => None,
            "minecraft:furnace" | "minecraft:blast_furnace" | "minecraft:smoker" => {
                let kind = match menu.as_str() {
                    "minecraft:blast_furnace" => FurnaceKind::BlastFurnace,
                    "minecraft:smoker" => FurnaceKind::Smoker,
                    _ => FurnaceKind::Furnace,
                };
                Some(BlockEntity::Furnace(Furnace {
                    kind,
                    items: [None, None, None],
                    lit_remaining: 0,
                    lit_total: 0,
                    cooking_timer: 0,
                    cooking_total: 0,
                    speed: 1.0,
                    recipes_used: std::collections::HashMap::new(),
                }))
            }
            // HopperMenu, and DispenserMenu (MenuType.GENERIC_3x3: dispensers
            // and droppers).
            "minecraft:hopper" => Some(BlockEntity::Chest(vec![None; 5])),
            // BrewingStandMenu: its times come in ContainerSetData.
            "minecraft:brewing_stand" => Some(BlockEntity::Brewing(Box::new(
                sunrise_client::brewing::BrewingStand::new(),
            ))),
            "minecraft:generic_3x3" => Some(BlockEntity::Chest(vec![None; 9])),
            // CrafterMenu: the disabled slots and triggered come in ContainerSetData.
            "minecraft:crafter_3x3" => Some(BlockEntity::Crafter(Default::default())),
            m if m.starts_with("minecraft:generic_9x") || m == "minecraft:shulker_box" => {
                let rows = m
                    .strip_prefix("minecraft:generic_9x")
                    .and_then(|r| r.parse::<usize>().ok())
                    .unwrap_or(3);
                Some(BlockEntity::Chest(vec![None; rows * 9]))
            }
            // A content mod's menu type: its container's slots, which its
            // mod's screen places.
            _ if mod_type.is_some() => Some(BlockEntity::Mod(Box::new(
                sunrise_client::containers::ModBlockEntity::new(
                    String::new(),
                    sunrise_client::containers::ModBlockEntityFields {
                        items: vec![None; mod_type.map_or(0, |t| t.slots)],
                        ..Default::default()
                    },
                ),
            ))),
            _ => {
                // Unsupported menu: tell the server we closed it.
                self.remote_reject_container(p.container);
                self.add_chat_line(
                    sunrise_client::text::Text::literal(format!(
                        "(Sunrise cannot show {menu} screens yet)"
                    )),
                    sunrise_client::chat::ChatSource::Client,
                );
                return;
            }
        };
        let title = menu_title(&p.title);
        if let Some(r) = self.remote.as_mut() {
            r.menu.set(Some((p.container, menu.clone())));
            r.set_menu_title(Some(title));
        }
        self.craft_size = if menu == "minecraft:crafting" { 3 } else { 2 };
        self.container = None;
        if let Some(entity) = entity {
            // The clicked block shows the menu when it holds that container;
            // an ender chest's or a double chest's slots are no one block's,
            // so they go under a key no block has.
            let holds = |k: &(i32, i32, i32)| {
                let name = &self.res.facts().get(self.world.block(k.0, k.1, k.2)).name;
                BlockEntity::for_block(name).is_some_and(|e| {
                    e.id() == entity.id() && e.slots().len() == entity.slots().len()
                })
            };
            let key = target.filter(holds).unwrap_or((i32::MIN, 0, 0));
            self.world.block_entities.insert(key, entity);
            self.container = Some(key);
            self.open_native_screen(key, target, p.container, &menu, extra);
            // A mod menu type shows through its mod's screen only: without one
            // (`MenuScreens.create` finds no screen) the window closes again.
            if self.mod_screen.is_none() && mod_type.is_some() {
                self.world.block_entities.remove(&key);
                self.container = None;
                if let Some(r) = self.remote.as_mut() {
                    r.menu.set(None);
                    r.set_menu_title(None);
                }
                self.send(S::ContainerClose(play::ContainerClose {
                    container: p.container,
                }));
                return;
            }
        }
        self.set_screen(Screen::Inventory);
        self.capture(false);
    }
    /// `ClientPacketListener.handleMountScreenOpen`: a horse's or llama's
    /// inventory; for another entity the window is closed again.
    fn remote_open_mount(&mut self, p: play::MountScreenOpen) {
        let columns = usize::try_from(p.columns).unwrap_or(0).min(5);
        if !self.open_remote_mount_menu(p.entity as u32, columns) {
            self.remote_reject_container(p.container);
            return;
        }
        if let Some(r) = self.remote.as_mut() {
            r.menu
                .set(Some((p.container, format!("{MOUNT_MENU}{columns}"))));
            r.set_menu_title(None);
        }
    }
    /// The type of the menu a server opened (`MenuType`'s id).
    pub(super) fn remote_menu_type(&self) -> Option<&str> {
        Some(self.remote.as_ref()?.menu.as_ref()?.1.as_str())
    }
    /// The title the server gave the menu it opened.
    pub(super) fn remote_menu_title(&self) -> Option<sunrise_client::text::Text> {
        self.remote.as_ref()?.menu_title.clone()
    }
    /// A language change reaches the server-sent components too: their titles
    /// are resolved when the packet arrives, with the session's language.
    pub(super) fn refresh_remote_language(&mut self) {
        if let Some(lang) = sunrise_client::lang::current()
            && let Some(r) = self.remote.as_mut()
        {
            r.lang = Arc::new(lang.map().clone());
        }
    }
    /// A server's chest-like menu (`MenuType.GENERIC_9xN`): its rows and title.
    pub(super) fn remote_chest_menu(&self) -> Option<(usize, sunrise_client::text::Text)> {
        let r = self.remote.as_ref()?;
        let rows = r
            .menu
            .as_ref()?
            .1
            .strip_prefix("minecraft:generic_9x")?
            .parse()
            .ok()?;
        Some((
            rows,
            r.menu_title
                .clone()
                .unwrap_or_else(sunrise_client::text::Text::empty),
        ))
    }
=======

    // remote_click: remote/use.rs

>>>>>>> sunrise/main
    /// ServerboundPlayerCommandPacket STOP_SLEEPING.
    pub(super) fn remote_stop_sleeping(&mut self) {
        let id = self.remote.as_ref().map_or(0, |r| r.entity_id);
        self.send(S::PlayerCommand(play::PlayerCommand {
            id,
            action: 0,
            data: 0,
        }));
    }
    pub(super) fn remote_respawn(&mut self) {
        self.send(S::ClientCommand(play::ClientCommand { action: 0 }));
    }

    /// A connection's screen is over the world: connecting, reconfiguring, or
    /// the level loading screen, a portal's included (`remote_overlay`).
    pub(super) fn remote_screen_up(&self) -> bool {
        !self.replay.playing()
            && self
                .remote
                .as_ref()
                .is_some_and(|r| self.player.is_none() || r.level_load.is_active())
    }

    // remote_overlay: remote/ui.rs

    /// Connected or connecting with no player yet: `ConnectScreen`, then
    /// `LevelLoadingScreen`, over the panorama with its blur.
    pub(super) fn remote_loading(&self) -> bool {
        if self.replay.playing() {
            return false;
        }
        use sunrise_client::level_loading::Reason;
        self.remote.as_ref().is_some_and(|r| {
            if r.level_load.is_active() {
                // The portal screens have no panorama, so no blur.
                r.level_load.reason == Reason::Other
            } else {
                self.player.is_none()
            }
        })
    }

    /// `ServerReconfigScreen`: a vertical layout of cells padded by 10, the
    /// title (9 high) over Disconnect (150 x 20), centred. The title's and the
    /// button's top left corners.
    fn reconfigure_layout(&self, (w, h): (f32, f32)) -> ((f32, f32), (f32, f32)) {
        let title = sunrise_client::text::Text::translate("connect.reconfiguring", &[]);
        let title_w = self.gui.atlas.font.width_text(&title).ceil() as i32;
        let width = (title_w + 20).max(170);
        let left = ((w as i32 - width) as f32 * 0.5) as i32;
        let top = ((h as i32 - 69) as f32 * 0.5) as i32;
        let across = |child: i32| (10.0 + ((width - child - 10) as f32 - 10.0) * 0.5) as i32;
        (
            ((left + across(title_w)) as f32, (top + 10) as f32),
            ((left + across(150)) as f32, (top + 39) as f32),
        )
    }

    /// `ServerReconfigScreen` over the panorama; Disconnect wakes up after
    /// `DISCONNECT_TIME` (600 ticks).
    fn draw_reconfiguring(&self, batch: &mut HudBatch, since: u64) {
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        sunrise_client::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
        let title = sunrise_client::text::Text::translate("connect.reconfiguring", &[]);
        let ((tx, ty), (bx, by)) = self.reconfigure_layout((w, h));
        batch.text_styled(atlas, &title, tx, ty, u32::MAX, true);
        // CommonComponents.GUI_DISCONNECT
        let label = sunrise_client::text::Text::translate("menu.disconnect", &[]);
        let active = self.ticks.saturating_sub(since) >= 600;
        let hover = sunrise_client::ui::hovered(self.cursor, bx, by, 150.0, 20.0);
        sunrise_client::ui::button(batch, atlas, bx, by, 150.0, 20.0, &label, active, hover);
    }

    /// Shared flag 6 of an entity on a server's client (`None` without a server).
    pub(super) fn remote_glowing(&self, id: u32) -> Option<bool> {
        let r = self.remote.as_ref()?;
        Some(r.glowing.contains(&(id as i32)))
    }

    /// The profile name of another player the server sent, by entity id.
    pub(super) fn remote_player_name(&self, id: u32) -> Option<&str> {
        let r = self.remote.as_ref()?;
        r.level_player_name(id as i32, &r.player_uuid(id)?)
    }
    pub(super) fn remote_player_uuid(&self, id: u32) -> Option<u128> {
        let remote = self.remote.as_ref()?;
        if self.player_model.as_ref().is_some_and(|m| m.id == id) {
            Some(remote.profile)
        } else {
            remote.level_players.get(&(id as i32)).copied()
        }
    }

    /// Status for `sunrise-ctl status`.
    /// The other players' entities in the level and their names (their
    /// profiles', which outlive the player list's entries).
    pub(super) fn remote_player_names(&self) -> Vec<(u32, String)> {
        let Some(r) = self.remote.as_ref() else {
            return Vec::new();
        };
        r.level_players
            .iter()
            .filter(|(_, uuid)| **uuid != r.profile)
            .filter_map(|(id, uuid)| Some((*id as u32, r.level_player_name(*id, uuid)?.to_owned())))
            .collect()
    }

    // remote_status: remote/status.rs
}

/// `Entity.lookAt`'s rotation from `from` towards `to`, as (yaw, pitch).
fn look_at_rotation(from: [f64; 3], to: [f64; 3]) -> (f32, f32) {
    use sunrise_core::geometry::trig::atan2;
    let (dx, dy, dz) = (to[0] - from[0], to[1] - from[1], to[2] - from[2]);
    let horizontal = (dx * dx + dz * dz).sqrt();
    // `* 180.0F / (float)Math.PI` on the double.
    let degrees = |r: f64| r * 180.0 / f64::from(std::f32::consts::PI);
    let pitch = wrap_degrees((-degrees(atan2(dy, horizontal))) as f32);
    let yaw = wrap_degrees(degrees(atan2(dz, dx)) as f32 - 90.0);
    (yaw, pitch)
}

/// `PositionMoveRotation.calculateAbsolute`'s pitch: the packet's pitch,
/// added to `source` when it is relative, clamped to +-90.
fn position_packet_pitch(source: f32, change: f32, relative: bool) -> f32 {
    let offset = if relative { source } else { 0.0 };
    (offset + change).clamp(-90.0, 90.0)
}

/// `Mth.wrapDegrees`.
pub(super) fn wrap_degrees(a: f32) -> f32 {
    let a = a % 360.0;
    if a >= 180.0 {
        a - 360.0
    } else if a < -180.0 {
        a + 360.0
    } else {
        a
    }
}

/// `LivingEntity.tick`'s body target and `tickHeadTurn` for another player:
/// the body turns toward where it walks (backwards when walking back), or to
/// the look while the arm swings, and stays within 50 degrees of the look.
pub(super) fn remote_player_head_turn(m: &mut Mob) {
    let (dx, dz) = (m.position.x - m.previous.x, m.position.z - m.previous.z);
    let mut target = m.body_yaw;
    if (dx * dx + dz * dz) as f32 > 0.002_500_000_2 {
        let walk = (dz.atan2(dx) as f32).to_degrees() - 90.0;
        let facing = (wrap_degrees(m.yaw) - walk).abs();
        target = if 95.0 < facing && facing < 265.0 {
            walk - 180.0
        } else {
            walk
        };
    }
    if m.anim.attack_anim > 0.0 {
        target = m.yaw;
    }
    m.body_yaw += wrap_degrees(target - m.body_yaw) * 0.3;
    let head = wrap_degrees(m.yaw - m.body_yaw);
    if head.abs() > 50.0 {
        m.body_yaw += head - head.signum() * 50.0;
    }
}

<<<<<<< sync-sunrise-1
/// A click sent to a server: its slot, button and input, the slots it
/// changed and the carried stack after it.
type Predicted = (
    i16,
    i32,
    Input,
    Vec<(i16, Option<ItemStack>)>,
    Option<ItemStack>,
);

/// How many stacks the creative screen keeps in step with the server (`Remote::synced`).
const CREATIVE_SYNCED: usize = 41;

/// The player's stacks in `Remote::synced` order: the 36 inventory slots, the off hand, then
/// the armour from the head down. `CreativeInventoryListener.slotChanged` sends every
/// InventoryMenu slot the creative screen changes, the armour slots too.
fn creative_sync_stacks(
    inventory: &[Option<ItemStack>],
    offhand: &Option<ItemStack>,
    armor: &[Option<ItemStack>; 4],
) -> Vec<Option<ItemStack>> {
    let mut stacks: Vec<Option<ItemStack>> = inventory.iter().take(36).cloned().collect();
    stacks.resize(36, None);
    stacks.push(offhand.clone());
    stacks.extend(armor.iter().cloned());
    stacks
}

/// Entries of `creative_sync_stacks` that went from `before` to `now` are synced as `now`.
fn mark_synced(
    synced: &mut [Option<ItemStack>],
    before: &[Option<ItemStack>],
    now: Vec<Option<ItemStack>>,
) {
    for (i, stack) in now.into_iter().enumerate() {
        if before.get(i) != Some(&stack)
            && let Some(s) = synced.get_mut(i)
        {
            *s = stack;
        }
    }
}

/// InventoryMenu's slot for entry `i` of `creative_sync_stacks`: the hotbar is 36..=44, the
/// main inventory 9..=35, the off hand 45 and the armour 5..=8 (head first).
fn creative_sync_slot(i: usize) -> i16 {
    (match i {
        0..9 => 36 + i,
        36 => 45,
        37.. => 5 + (i - 37),
        _ => i,
    }) as i16
}

/// `MultiPlayerGameMode.handleContainerInput` for each click on the engine's
/// copy of the menu: the click with the slots it changed and the carried
/// stack after it, and whether a click dropped a stack. Every drop of
/// `AbstractContainerMenu.doClick` (a THROW, a click outside the window, a
/// swap or a result move with no room left) is `Prediction.PREDICTED`.
/// None when the engine refuses a click.
fn predict_clicks(
    m: &mut sunrise_client::menu::Menu,
    env: &sunrise_client::menu::Env,
    clicks: &[(i16, i32, Input)],
) -> Option<(Vec<Predicted>, bool)> {
    let mut predicted = Vec::with_capacity(clicks.len());
    let mut dropped = false;
    for &(slot, button, input) in clicks {
        let before = menu_items(m);
        let thrown = m.dropped.len();
        m.click(env, i32::from(slot), button, input).ok()?;
        dropped |= m.dropped.len() > thrown;
        let changed = changed_slots(&before, m);
        predicted.push((slot, button, input, changed, m.carried.clone()));
    }
    Some((predicted, dropped))
}

pub(super) fn menu_items(m: &sunrise_client::menu::Menu) -> Vec<Option<ItemStack>> {
    (0..m.slot_count()).map(|i| m.slot(i).cloned()).collect()
}

/// The slots whose stack is no longer `before`'s (`ItemStack.matches`).
/// `MultiPlayerGameMode.handleContainerInput`'s changed slots, in the order the packet writes them.
fn changed_slots(
    before: &[Option<ItemStack>],
    m: &sunrise_client::menu::Menu,
) -> Vec<(i16, Option<ItemStack>)> {
    let changed: Vec<i16> = (0..m.slot_count())
        .filter(|&i| m.slot(i) != before.get(i).and_then(Option::as_ref))
        .map(|i| i as i16)
        .collect();
    open_hash_order(&changed)
        .into_iter()
        .map(|i| (i, m.slot(i as usize).cloned()))
        .collect()
}

/// The order a new fastutil `Int2ObjectOpenHashMap` iterates `keys` put in this order: vanilla
/// collects a click's changed slots in one and writes them as it iterates. The key 0 comes first,
/// then the table from its last position down; a key sits at `HashCommon.mix(key) & mask` or the
/// next free position. The table starts at 32 (16 expected, load factor 0.75) and doubles when an
/// insert finds it holding 24 (`size++ >= maxFill`), re-inserting from its last position down.
fn open_hash_order(keys: &[i16]) -> Vec<i16> {
    fn mix(key: i16) -> usize {
        let h = i32::from(key).wrapping_mul(0x9E37_79B9_u32 as i32);
        (h ^ ((h as u32) >> 16) as i32) as u32 as usize
    }
    fn put(table: &mut [Option<i16>], key: i16) {
        let mask = table.len() - 1;
        let mut pos = mix(key) & mask;
        while table[pos].is_some() {
            pos = (pos + 1) & mask;
        }
        table[pos] = Some(key);
    }
    // HashCommon.maxFill(n, 0.75): min(ceil(n * 0.75), n - 1).
    let max_fill = |n: usize| (n * 3).div_ceil(4).min(n - 1);
    let mut table = vec![None; 32];
    let (mut zero, mut size) = (false, 0);
    for &key in keys {
        if key == 0 {
            zero = true;
        } else {
            put(&mut table, key);
        }
        size += 1;
        // `size++ >= maxFill`: the size before this key.
        if size > max_fill(table.len()) {
            // HashCommon.arraySize(size + 1, 0.75): the power of two holding size + 1 at that load.
            let n = ((size + 1) * 4).div_ceil(3).next_power_of_two();
            let mut grown = vec![None; n];
            for &key in table.iter().rev().flatten() {
                put(&mut grown, key);
            }
            table = grown;
        }
    }
    let rest = table.iter().rev().flatten().copied();
    zero.then_some(0).into_iter().chain(rest).collect()
}

=======
>>>>>>> sunrise/main
/// ServerboundContainerClickPacket's `ContainerInput` ordinal.
pub(super) fn input_ordinal(input: Input) -> i32 {
    match input {
        Input::Pickup => 0,
        Input::QuickMove => 1,
        Input::Swap => 2,
        Input::Clone => 3,
        Input::Throw => 4,
        Input::QuickCraft => 5,
        Input::PickupAll => 6,
    }
}

mod dry_level;
use dry_level::DryLevel;

type DrySound = (
    sunrise_core::position::BlockPos,
    std::sync::Arc<str>,
    mod_api::SoundSource,
    f32,
    f32,
);
/// The blocks a use on the dry level set.
type DryChanges = Vec<(sunrise_core::position::BlockPos, BlockStateId)>;
/// What a use on the dry level changed, and the sounds it played.
pub(super) type DryUse = (DryChanges, Vec<DrySound>);
/// Where `BlockItem.place` on the dry level placed, and the blocks it set.
pub(super) struct DryPlace {
    pub(super) pos: sunrise_core::position::BlockPos,
    matches_candidate: bool,
    pub(super) changes: DryChanges,
}

/// The blocks a guest's hand flips that its use predicts (and hears at once): a wooden or
/// copper door, trapdoor or fence gate, a lever, a button or a cauldron, whose fill level
/// changes as the bucket, bottle or dye lands. An iron door or trapdoor has no hand use.
pub(super) fn hand_operated(
    behaviours: &sunrise_world::level::Behaviours,
    state: BlockStateId,
) -> bool {
    behaviours
        .data(state)
        .is(sunrise_world::level::classes::Class::FenceGate)
        || named_hand_operated(behaviours.table.name(state))
}

fn named_hand_operated(name: &str) -> bool {
    let name = name.trim_start_matches("minecraft:");
    ((name.ends_with("_door") || name.ends_with("_trapdoor")) && !name.starts_with("iron_"))
        || name.ends_with("_button")
        || name == "lever"
        || name.ends_with("cauldron")
}

/// `InteractionHand` on the wire.
pub(super) fn hand_id(hand: Hand) -> i32 {
    match hand {
        Hand::Main => 0,
        Hand::Off => 1,
    }
}

/// A hit, a use, a block placed or broken, an item dropped: what Free Direction steering
/// rests after (`Game::send`).
fn is_interaction(packet: &S) -> bool {
    matches!(
        packet,
        S::PlayerAction(_) | S::Attack(_) | S::Interact(_) | S::UseItemOn(_) | S::UseItem(_)
    )
}

/// Whether Free Direction holds `packet` for the next tick (`Game::send`): an interaction
/// made while the last move steered (`steered`), and whatever comes after one held
/// (`queued`), in order. Digging (START_DESTROY_BLOCK to STOP_DESTROY_BLOCK, actions 0 to 3)
/// goes at once: this client has broken the block on its own level already, and this tick's
/// move may go into the space it left (flying up against the block just dug), which the
/// server and its anticheat take only once they have the break, as vanilla sends it before
/// the move. The server reads no look from it, and Grim's RotationBreak checks it against the
/// look of the move after it, the view's (the steering rests from it on).
fn waits_for_the_view(packet: &S, steered: bool, queued: bool) -> bool {
    let digging = matches!(packet, S::PlayerAction(a) if (0..=3).contains(&a.action));
    (is_interaction(packet) && !digging && steered) || queued
}

/// Whether `Item.use` of `info` ends `Minecraft.startUseItem`'s click, so the
/// off hand gets no turn. `FireworkRocketItem.use` answers SUCCESS while the
/// player glides (the rocket boosts them) and PASS otherwise.
pub(super) fn use_ends_click(info: &sunrise_client::items::ItemInfo, fall_flying: bool) -> bool {
    item_has_use(info) || (fall_flying && &*info.id == "minecraft:firework_rocket")
}

/// Whether `Item.use` of this item answers SUCCESS on this client, which
/// swings (`MultiPlayerGameMode.useItem`'s `result.shouldSwing()`), rather
/// than CONSUME, which only ends the click: a bow, crossbow, trident or
/// spear starting its charge, `ItemUtils.startUsingInstantly`'s food, drink,
/// spyglass, shield and goat horn, and the ender eye's SUCCESS_SERVER (the
/// server's swing source, not this client's).
pub(super) fn remote_use_swings(held: &ItemStack, consumable: bool) -> bool {
    let id = &*held.item;
    !matches!(
        id,
        "minecraft:bow"
            | "minecraft:crossbow"
            | "minecraft:trident"
            | "minecraft:spyglass"
            | "minecraft:goat_horn"
            | "minecraft:ender_eye"
            | "minecraft:brush"
    ) && !consumable
        && sunrise_client::spear::Kinetic::of(id).is_none()
}

/// Whether an object interaction on the client predicts a hand swing (`InteractionResult.SUCCESS`).
/// Boarding a minecart answers CONSUME and does not swing.
fn remote_object_swings(response: Option<bool>, is_boarding: bool) -> bool {
    response.unwrap_or(!is_boarding)
}

/// Whether right-clicking an item frame passes (`InteractionResult.PASS`).
/// A fixed frame or an empty frame when clicking with an empty hand passes.
fn item_frame_interact_passes(fixed: bool, frame_has_item: bool, hand_has_item: bool) -> bool {
    fixed || (!frame_has_item && !hand_has_item)
}

/// Whether right-clicking a boat passes (`InteractionResult.PASS`) on a client.
/// `AbstractBoat.interact` passes when sneaking or out of control (60 ticks), and
/// answers SUCCESS otherwise without checking for room (only the server's
/// `startRiding` does). `AbstractChestBoat.interact` opens the chest instead of
/// passing, unless it is out of control, not sneaked at and has room.
fn boat_interact_passes(chest: bool, sneaking: bool, out_of_control: bool, has_room: bool) -> bool {
    if chest {
        !sneaking && out_of_control && has_room
    } else {
        sneaking || out_of_control
    }
}

/// Whether `Item.use` usually acts on this item (eats, drinks, equips, raises,
/// throws, shoots, fills or places), which ends `Minecraft.startUseItem` at
/// its hand. Tools and materials pass, and the off hand gets its turn. The
/// server decides; this is the client's guess of whether to go on. A carrot or warped fungus
/// on a stick is not here: `FoodOnAStickItem.use` passes on a client, and only the server's
/// answer to a boost that can run swings the hand.
pub(super) fn item_has_use(info: &sunrise_client::items::ItemInfo) -> bool {
    let id = info.id.trim_start_matches("minecraft:");
    id.ends_with("_spear")
        || info.is("minecraft:fishing_rod") // a content mod's rod too
        || info.consumable.is_some()
        || (info.equip_slot.is_some() && info.swappable)
        || info.blocks_attacks.is_some()
        || sunrise_client::items::bucket_contents(&info.id).is_some()
        || id.ends_with("_boat")
        || id.ends_with("_raft")
        || id.ends_with("_bundle")
        || matches!(
            id,
            "bow"
                | "crossbow"
                | "trident"
                | "spyglass"
                | "snowball"
                | "egg"
                | "blue_egg"
                | "brown_egg"
                | "ender_pearl"
                | "ender_eye"
                | "wind_charge"
                | "splash_potion"
                | "lingering_potion"
                | "experience_bottle"
                | "bucket"
                | "glass_bottle"
                | "map"
                | "writable_book"
                | "written_book"
                | "goat_horn"
                | "bundle"
                | "brush"
                | "knowledge_book"
        )
}

/// The view distance a client asks for: its render distance, which Sunrise
/// lets go past vanilla's 32 (a vanilla server still caps it at its own).
pub(super) fn client_view_distance(render_distance: i32) -> i8 {
    render_distance.clamp(2, i32::from(i8::MAX)) as i8
}

/// `ClientPacketListener.handleBlockEntityData` loads the tag into the existing
/// block entity, so the state the client builds itself stays: copied from `old`
/// into `new`, the block entity decoded from the tag.
fn keep_client_state(old: Option<&BlockEntity>, new: &mut BlockEntity) {
    match (old, new) {
        // The turn of a vault's item and a trial spawner's mob (VaultClientData).
        (Some(BlockEntity::Vault(old)), BlockEntity::Vault(new)) => {
            new.spin = old.spin;
            new.o_spin = old.o_spin;
        }
        (Some(BlockEntity::TrialSpawner(old)), BlockEntity::TrialSpawner(new)) => {
            new.spin = old.spin;
            new.o_spin = old.o_spin;
        }
        // A beacon's beam (`BeaconBlockEntity.loadAdditional` reads only the
        // powers): a fresh one draws nothing until the next tick walks it again.
        (Some(BlockEntity::Beacon(old)), BlockEntity::Beacon(new)) => {
            new.sections.clone_from(&old.sections);
            new.checking.clone_from(&old.checking);
            (new.last_check_y, new.levels) = (old.last_check_y, old.levels);
        }
        _ => {}
    }
}

/// `LocalPlayer.onSyncedDataUpdated(DATA_LIVING_ENTITY_FLAGS)`: the server's
/// flags (1 using, 2 the off hand) start a use the client is not running
/// (`startUsingItem`, which needs an item in that hand), and stop one it is
/// (`stopUsingItem`). `using` is the slot in use and its ticks.
fn synced_local_use(
    using: Option<(usize, u32)>,
    flags: i8,
    selected: usize,
    has_item: impl Fn(usize) -> bool,
) -> Option<(usize, u32)> {
    let server_using = flags & 1 != 0;
    match using {
        None if server_using => {
            let slot = if flags & 2 != 0 {
                super::shield::OFFHAND_SLOT
            } else {
                selected
            };
            has_item(slot).then_some((slot, 0))
        }
        Some(_) if !server_using => None,
        using => using,
    }
}

/// The game as the tick's input sees it.
struct RemoteInputWorld<'g, 'a>(&'g mut Game<'a>);

struct RemoteUseWorld<'g, 'a>(&'g mut Game<'a>, mod_api::InteractionAttempt);

impl sunrise_client::interaction_input::TargetUseWorld for RemoteUseWorld<'_, '_> {
    fn use_entity(&mut self) -> bool {
        #[cfg(target_os = "android")]
        if self.0.mobile.touch_shield {
            return false;
        }
        self.0.remote_use_entity(Hand::Main)
    }
    fn use_block(&mut self) -> bool {
<<<<<<< sync-sunrise-1
        #[cfg(target_os = "android")]
        if self.0.mobile.touch_shield {
            let transformer = self
                .0
                .stack_in(self.0.selected)
                .and_then(|s| self.0.items().get(&s.item))
                .is_some_and(|i| i.block_transformer.is_some());
            if self.0.mobile.controls.settings.shield_button
                || self.0.mobile.controls.shield_held()
                || !transformer
                || self.0.pick().is_none()
            {
                return false;
            }
        }
        self.0.remote_use_block(Hand::Main)
=======
        self.0.remote_use_block(Hand::Main, self.1, None)
>>>>>>> sunrise/main
    }
    fn use_item(&mut self) {
        self.0.remote_use_hands(Hand::Main, self.1, None);
    }
}

impl sunrise_client::interaction_input::InteractionWorld for RemoteInputWorld<'_, '_> {
    fn pick_hit_result(&mut self) {
        self.0.pick_tick_targets();
    }
    fn handle_keys(&mut self) {
        self.0.handle_play_keybinds();
    }
    fn pending_clicks(&self, button: sunrise_client::interaction_input::Button) -> u32 {
        self.0.pending_mapping_clicks(button.mapping())
    }
    fn consume_click(&mut self, button: sunrise_client::interaction_input::Button) -> bool {
        self.0.consume_mapping_click(button.mapping())
    }
    fn is_down(&self, button: sunrise_client::interaction_input::Button) -> bool {
        self.0.mapped_key_down(button.mapping())
    }
    fn can_continue_attack(&self) -> bool {
        !self.0.mod_mappings_screen_open()
    }
    fn spectator_hotbar(&mut self) {
        while self.0.spectator() && self.0.consume_mapping_click("key.spectatorHotbar") {
            self.0.spectator_action_key();
        }
    }
    fn release_hit_result(&mut self) {
        self.0.release_tick_targets();
    }
    fn using_item(&self) -> bool {
        self.0.client_item_use.is_some() || self.0.using.is_some()
    }
    fn release_using(&mut self) {
        if !self.0.end_client_item_use(true) && self.0.using.is_some() {
            self.0.release_remote_use();
        }
    }
    fn start_attack(&mut self) -> bool {
        self.0.remote_start_attack()
    }
    fn start_use(&mut self, held: bool) {
        let attempt = if held {
            mod_api::InteractionAttempt::Held
        } else {
            mod_api::InteractionAttempt::Press
        };
        self.0.remote_use(attempt);
    }
    fn pick(&mut self) {
        // Minecraft.pickBlockOrEntity: an entity before the block is asked
        // for (`handlePickItemFromEntity`), else the block.
        if let Some(entity) = self.0.pick_entity_hit() {
            if let Some(id) = entity {
                if self
                    .0
                    .native_interaction(
                        InteractionKey::PickBlock,
                        Hand::Main,
                        mod_api::InteractionAttempt::Press,
                    )
                    .canceled
                {
                    return;
                }
                self.0.send(S::PickItemFromEntity(play::PickItemFromEntity {
                    id,
                    include_data: self.0.has_control_down(),
                }));
            }
        } else if let Some(hit) = self.0.pick() {
            if self
                .0
                .native_interaction(
                    InteractionKey::PickBlock,
                    Hand::Main,
                    mod_api::InteractionAttempt::Press,
                )
                .canceled
            {
                return;
            }
            self.0.send(S::PickItemFromBlock(play::PickItemFromBlock {
                pos: hit.pos,
                include_data: self.0.has_control_down(),
            }));
        }
    }
    fn use_delay(&self) -> u32 {
        self.0.use_delay
    }
    fn continue_attack(&mut self, down: bool) {
        self.0.remote_continue_attack(down);
    }
}

#[derive(Default)]
pub(super) struct ClientMenu {
    current: Option<(i32, String)>,
    pub(super) generation: u64,
}

impl ClientMenu {
    pub(super) fn set(&mut self, current: Option<(i32, String)>) {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("menu generation exhausted");
        self.current = current;
    }

    pub(super) fn as_ref(&self) -> Option<&(i32, String)> {
        self.current.as_ref()
    }
}

<<<<<<< sync-sunrise-1
impl Remote {
    pub(super) fn set_menu_title(&mut self, title: Option<sunrise_client::text::Text>) {
        self.menu_title_snapshot = title.try_into_api().ok().flatten().map(Arc::new);
        self.menu_title = title;
    }

    pub(super) fn menu_title_snapshot(&self) -> Option<&mod_api::Text> {
        self.menu_title_snapshot.as_deref()
    }

    pub(super) fn shared_menu_title_snapshot(&self) -> Option<Arc<mod_api::Text>> {
        self.menu_title_snapshot.clone()
    }

    /// Resolve registry keys only when registries or the dimension change.
    fn resolve_clocks(&mut self) {
        self.overworld_clock = self
            .registries
            .id("minecraft:world_clock", "minecraft:overworld");
        self.default_clock = self
            .dimension
            .default_clock
            .as_deref()
            .and_then(|name| self.registries.id("minecraft:world_clock", name));
        for id in [self.overworld_clock, self.default_clock]
            .into_iter()
            .flatten()
        {
            self.clocks.entry(id).or_insert(play::ClockState {
                clock: id,
                total_ticks: 0,
                partial_tick: 0.0,
                rate: 1.0,
            });
        }
    }

    fn clock_time(&self, id: Option<i32>) -> i64 {
        id.and_then(|id| self.clocks.get(&id))
            .map_or(0, |c| c.total_ticks)
    }

    pub(super) fn overworld_clock_time(&self) -> i64 {
        self.clock_time(self.overworld_clock)
    }

    /// `Minecraft.level != null`: from `handleLogin` until the next
    /// `handleConfigurationStart` (`clearClientLevel`). Minecraft.tick sends
    /// CLIENT_TICK_END only then, so a client waiting for the server's Login
    /// (a busy server takes seconds to place a new player) sends nothing.
    pub(in crate::game) fn has_level(&self) -> bool {
        client_has_level(self.logged_in, self.reconfiguring)
    }
    /// An entity the server added to this client level: its type
    /// (`minecraft:text_display`) and its position as last sent.
    pub(super) fn tracked_entity(&self, id: i32) -> Option<(&str, [f64; 3])> {
        self.entities
            .get(&id)
            .map(|t| (t.kind.as_str(), t.position))
    }
    /// The profile of the player entity `id`: this window's own player, or
    /// another one by the UUID it was added with and its profile name.
    pub(super) fn level_player(&self, id: i32) -> Option<(u128, Option<&str>)> {
        if id == self.entity_id {
            return Some((self.profile, Some(self.profile_name.as_str())));
        }
        let uuid = *self.level_players.get(&id)?;
        Some((uuid, self.level_player_names.get(&id).map(String::as_str)))
    }
    /// `PlayerInfo.getMessageValidator().updateAndValidate(message)` for `message.sender`.
    fn validate_chat(
        &mut self,
        message: &secure_chat::Incoming,
        now: i64,
    ) -> Option<secure_chat::Validated> {
        let sender = message.sender;
        if let Some(validator) = self.chat_validators.get_mut(&sender) {
            return validator.update_and_validate(message, now);
        }
        let (mut validator, settled) = self.new_chat_validator(sender);
        let validated = validator.update_and_validate(message, now);
        // While the services' keys are still on their way the validator is a stopgap, not kept.
        if settled {
            self.chat_validators.insert(sender, validator);
        }
        validated
    }
    /// `ClientPacketListener.initializeChatSession` for `sender`'s entry: a session the services'
    /// keys vouch for makes a keyed validator, anything else the fallback of
    /// `enforcesSecureChat()` (`canValidateProfileKeys() && serverEnforcesSecureChat`). `false`
    /// while the keys are not known yet.
    fn new_chat_validator(&self, sender: u128) -> (secure_chat::MessageValidator, bool) {
        let keys = self.services_keys.lock().unwrap_or_else(|e| e.into_inner());
        let (can_validate, settled) = match &*keys {
            ServicesKeys::Ready(_) => (true, true),
            ServicesKeys::Missing => (false, true),
            ServicesKeys::Pending => (false, false),
        };
        let fallback =
            secure_chat::MessageValidator::fallback(can_validate && self.enforces_secure_chat);
        let (ServicesKeys::Ready(keys), Some(Some((id, expires_at, key, signature)))) =
            (&*keys, self.chat_sessions.get(&sender))
        else {
            if matches!(&*keys, ServicesKeys::Missing)
                && self.chat_sessions.get(&sender).is_some_and(Option::is_some)
            {
                log::warn!(
                    "Ignoring chat session from {sender:032x} due to missing Services public key"
                );
            }
            return (fallback, settled);
        };
        let profile_key = secure_chat::ProfileKey {
            expires_at: *expires_at,
            key: key.clone(),
            signature: signature.clone(),
        };
        match secure_chat::ChatSession::validate(*id, profile_key, keys, sender) {
            Ok(session) => (secure_chat::MessageValidator::keyed(session), true),
            Err(why) => {
                log::error!("Failed to validate profile key for player {sender:032x}: {why}");
                (fallback, true)
            }
        }
    }
    /// How far the server sends chunks: its view distance and the ring
    /// around (`ChunkTrackingView`), at least two.
    pub(super) fn sent_radius(&self) -> i32 {
        self.view_distance.max(2).saturating_add(1)
    }
    /// A player entity's profile name: the one it was created with, else its
    /// player-list entry's (an entity the list named after it spawned).
    fn level_player_name(&self, id: i32, uuid: &u128) -> Option<&str> {
        self.level_player_names
            .get(&id)
            .or_else(|| self.players.get(uuid))
            .map(String::as_str)
    }
    /// Listed UUIDs always have a tab entry; removal clears both together.
    pub(super) fn listed_player_count(&self) -> usize {
        self.listed.len()
    }
    /// The listed player-list entries (`getListedOnlinePlayers`), in no order.
    pub(super) fn listed_tab_players(
        &self,
    ) -> impl Iterator<Item = &sunrise_client::tab_list::TabPlayer> {
        self.listed
            .iter()
            .filter_map(|uuid| self.tab_players.get(uuid))
    }
    /// The local player's profile UUID.
    pub(super) fn profile(&self) -> u128 {
        self.profile
    }
    /// Another player's profile UUID, by their entity id (None for the local player).
    pub(super) fn player_uuid(&self, id: u32) -> Option<u128> {
        let uuid = *self.level_players.get(&(id as i32))?;
        (uuid != self.profile).then_some(uuid)
    }
    /// The entity id of a player in the level, by profile UUID.
    pub(super) fn player_entity(&self, uuid: u128) -> Option<u32> {
        self.level_players
            .iter()
            .find(|(_, u)| **u == uuid)
            .map(|(id, _)| *id as u32)
    }
    /// The kind and last sent position of a tracked server entity.
    pub(super) fn tracked(&self, id: i32) -> Option<(&str, [f64; 3])> {
        self.entities.get(&id).map(|t| (&*t.kind, t.position))
    }
}

=======
>>>>>>> sunrise/main
fn object_skin_changed(published: &HashMap<String, String>, key: &str, skin: &str) -> bool {
    published.get(key).is_none_or(|previous| previous != skin)
}

/// The textures to resolve skins from: every list entry's, and for a player
/// entity whose entry is gone the profile it kept. `kept` first takes, for each
/// player entity without one, its entry's textures (`AbstractClientPlayer.getPlayerInfo`
/// looks the PlayerInfo up once and holds it, removed from the list or not).
fn skin_entries(
    level_players: &HashMap<i32, u128>,
    listed: &HashMap<u128, sunrise_client::skins::Textures>,
    kept: &mut HashMap<i32, sunrise_client::skins::Textures>,
) -> Vec<(u128, sunrise_client::skins::Textures)> {
    for (id, uuid) in level_players {
        if !kept.contains_key(id)
            && let Some(textures) = listed.get(uuid)
        {
            kept.insert(*id, textures.clone());
        }
    }
    let mut entries: Vec<_> = listed.iter().map(|(u, t)| (*u, t.clone())).collect();
    for (id, textures) in kept.iter() {
        if let Some(uuid) = level_players.get(id)
            && !listed.contains_key(uuid)
            && !entries.iter().any(|(u, _)| u == uuid)
        {
            entries.push((*uuid, textures.clone()));
        }
    }
    entries
}

/// `AttributeInstance.calculateValue` before the attribute's range: the base,
/// `ADD_VALUE`s, then `ADD_MULTIPLIED_BASE`s of that, then each
/// `ADD_MULTIPLIED_TOTAL`.
fn snapshot_value(s: &play::AttributeSnapshot) -> f64 {
    let amounts = |op: i32| {
        s.modifiers
            .iter()
            .filter(move |m| m.operation == op)
            .map(|m| m.amount)
    };
    let base = s.base + amounts(0).sum::<f64>();
    let mut value = base + amounts(1).map(|a| base * a).sum::<f64>();
    for amount in amounts(2) {
        value *= 1.0 + amount;
    }
    value
}

/// `Util.getMillis` for the level load tracker's timeouts.
pub(super) fn client_millis() -> u64 {
    static ORIGIN: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    ORIGIN
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

/// What `ChatScreen.handleChatInput` sends of typed chat once `ClientChatEvent` (`ask`, with
/// the outgoing message and the cancel flag) had its say: nothing when canceled or emptied.
fn chat_after_event(message: &str, ask: impl FnOnce(&mut String, &mut bool)) -> Option<String> {
    let mut outgoing = message.to_owned();
    let mut cancel = false;
    ask(&mut outgoing, &mut cancel);
    (!cancel && !outgoing.is_empty()).then_some(outgoing)
}
/// `Entity.isOnFire` for the window's player: while the server's flag is set
/// the predicted tick's countdown never takes `fire_ticks` below 1.
pub(super) fn hold_server_fire(fire_ticks: i32, server_on_fire: bool) -> i32 {
    if server_on_fire {
        fire_ticks.max(1)
    } else {
        fire_ticks
    }
}

/// `Entity.isColliding`: whether the collision `shape` of a block at `pos`
/// meets `entity`'s box. Shapes are block-local, so they move onto the block
/// first (`VoxelShape.move`), and any box that overlaps is a collision
/// (`Shapes.joinIsNotEmpty` with `BooleanOp.AND`).
pub(super) fn shape_meets(
    shape: &sunrise_core::geometry::VoxelShape,
    pos: [i32; 3],
    entity: sunrise_core::geometry::Aabb,
) -> bool {
    let at = Vec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]));
    shape
        .boxes()
        .iter()
        .any(|b| b.move_by(at).intersects(entity))
}

/// `ClientLevel.syncBlockState`'s snap: a restored block that meets the player
/// sends it back to the position the prediction remembered, unless the player
/// was teleported after that prediction was made
/// (`lastTeleportSequence < sequence`: the remembered position is stale).
pub(super) fn snap_target(
    recorded: [f64; 3],
    last_teleport_sequence: i32,
    sequence: i32,
    meets: bool,
) -> Option<[f64; 3]> {
    (meets && last_teleport_sequence < sequence).then_some(recorded)
}

/// [`Remote::has_level`]: logged in (a Login arrived) and no reconfiguration
/// since (`reconfiguring` is set by StartConfiguration, cleared by the next
/// Login).
fn client_has_level(logged_in: bool, reconfiguring: Option<u64>) -> bool {
    logged_in && reconfiguring.is_none()
}

/// `ClientChunkCache.Storage.inRange`: whether column `x`, `z` is within `calculateStorageRange`
/// (the server's radius, at least two, plus three) of the view centre. The radius and the centre
/// are the server's to send, any `i32`: the distances are taken without overflow, which panicked
/// a debug build and, wrapping in release, dropped every loaded chunk.
fn in_storage_range(view_distance: i32, center: (i32, i32), x: i32, z: i32) -> bool {
    let range = view_distance.max(2).saturating_add(3).unsigned_abs();
    x.abs_diff(center.0) <= range && z.abs_diff(center.1) <= range
}

// A packet's removals (`RemoveEntities`), apart: this file may not grow.
#[cfg(test)]
mod interaction_event_tests;

#[cfg(test)]
mod native_entity_tests;

#[cfg(test)]
mod effect_blend_tests;
#[cfg(test)]
mod level_sound_tests;

#[cfg(test)]
mod attachment_query_tests;
#[cfg(test)]
mod passage_cross_tests;
#[cfg(test)]
mod respawn_motion_tests;
impl Remote {
    /// Project the session address for display once, dropping the memory transport's marker.
    pub(super) fn display_address_for(address: &str) -> Option<String> {
        (address != "local").then(|| sunrise_client::net::shown_address(address))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::api_values::TryIntoApi;

    mod block_entity_state_tests {
        include!("block_entity_state_tests.rs");
    }
    mod chat_decoration_tests {
        include!("chat_decoration_tests.rs");
    }
    mod chat_event_tests {
        include!("chat_event_tests.rs");
    }
    mod chat_window_and_input_tests {
        include!("chat_window_and_input_tests.rs");
    }
    mod play_bundle_tests {
        include!("play_bundle_tests.rs");
    }
    mod chunk_storage_tests {
        include!("chunk_storage_tests.rs");
    }
    mod click_ending_tests {
        include!("click_ending_tests.rs");
    }
    mod client_attributes_tests {
        include!("client_attributes_tests.rs");
    }
    mod client_block_changed_tests {
        include!("client_block_changed_tests.rs");
    }
    mod client_camera_tests {
        include!("client_camera_tests.rs");
    }
    mod client_day_time_tests {
        include!("client_day_time_tests.rs");
    }
    mod client_epochs_tests {
        include!("client_epochs_tests.rs");
    }
    mod client_handles_tests {
        include!("client_handles_tests.rs");
    }
    mod client_level_environment_tests {
        include!("client_level_environment_tests.rs");
    }
    mod client_level_lifecycle_tests {
        include!("client_level_lifecycle_tests.rs");
    }
    mod client_menu_tests {
        include!("client_menu_tests.rs");
    }
    mod client_player_reads_tests {
        include!("client_player_reads_tests.rs");
    }
    mod client_read_tests {
        include!("client_read_tests.rs");
    }
    mod creative_sync_tests {
        include!("creative_sync_tests.rs");
    }
    mod damage_sound_tests {
        include!("damage_sound_tests.rs");
    }
    mod dig_abort_tests {
        include!("dig_abort_tests.rs");
    }
    mod dimension_transition_tests {
        include!("dimension_transition_tests.rs");
    }
    mod game_event_tests {
        include!("game_event_tests.rs");
    }
    mod hand_operated_tests {
        include!("hand_operated_tests.rs");
    }
    mod hand_swap_tests {
        include!("hand_swap_tests.rs");
    }
    mod hanging_entity_age_tests {
        include!("hanging_entity_age_tests.rs");
    }
    mod menu_click_order_tests {
        include!("menu_click_order_tests.rs");
    }
    mod menu_generation_tests {
        include!("menu_generation_tests.rs");
    }
    mod menu_packets_tests {
        include!("menu_packets_tests.rs");
    }
    mod menu_title_tests {
        include!("menu_title_tests.rs");
    }
    mod movement_snap_tests {
        include!("movement_snap_tests.rs");
    }
    mod object_interactions_tests {
        include!("object_interactions_tests.rs");
    }
    mod particle_group_tests {
        include!("particle_group_tests.rs");
    }
    mod placement_preview_tests {
        include!("placement_preview_tests.rs");
    }
    mod player_animation_tests {
        include!("player_animation_tests.rs");
    }
    mod player_sleep_tests {
        include!("player_sleep_tests.rs");
    }
    mod predicted_block_notice_tests {
        include!("predicted_block_notice_tests.rs");
    }
    mod predicted_throw_tests {
        include!("predicted_throw_tests.rs");
    }
    mod remote_columns_tests {
        include!("remote_columns_tests.rs");
    }
    mod server_fire_flag_tests {
        include!("server_fire_flag_tests.rs");
    }
    mod slot_mapping_tests {
        include!("slot_mapping_tests.rs");
    }
    mod sound_entity_tests {
        include!("sound_entity_tests.rs");
    }

    #[test]
    #[ignore = "daily"]
    fn named_remote_players_are_told_one_by_one() {
        use mod_api::{
            EntityTickListenedEvent, EntityTickPostEvent, EntityTickPreEvent, Hooks, HostSide,
            ModEntry, Mods, Registrar,
        };
        use std::sync::{Arc, Mutex};
        type Calls = Arc<Mutex<Vec<(bool, Vec<u32>)>>>;
        struct Named(Calls);
        #[mod_api::__subscriptions]
        impl Hooks for Named {
            fn entity_tick_listened(&self, _: HostSide<'_>, event: &mut EntityTickListenedEvent) {
                event.listened = true;
            }
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.entity_tick_each("minecraft:player");
            }
            fn entity_tick_pre(&self, _: HostSide<'_>, event: &mut EntityTickPreEvent<'_>) {
                self.0.lock().unwrap().push((true, event.entities.to_vec()));
            }
            fn entity_tick_post(&self, _: HostSide<'_>, event: &mut EntityTickPostEvent<'_>) {
                self.0
                    .lock()
                    .unwrap()
                    .push((false, event.entities.to_vec()));
            }
        }
        let heard: Calls = Arc::default();
        let mods: Mods = vec![ModEntry::new(
            "remote-player-ticks",
            Box::new(Named(heard.clone())),
        )]
        .into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_client_game(|game| {
                let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                game.connect_local(client, "avatar-tick-client").unwrap();
                let remote = game.remote.as_mut().unwrap();
                let kind = remote.player_type.as_ref().unwrap().clone();
                assert!(
                    !kind.flags.tick_alone,
                    "render model does not select tick routing"
                );
                for id in [66, 67] {
                    remote.level_players.insert(id, id as u128);
                    game.mobs.push(sunrise_client::mobs::Mob::new(
                        id as u32,
                        kind.clone(),
                        sunrise_core::geometry::Vec3::ZERO,
                        0.0,
                    ));
                }
                game.tick_remote_entities(false);
            });
        });
        assert_eq!(
            *heard.lock().unwrap(),
            [
                (true, vec![66]),
                (false, vec![66]),
                (true, vec![67]),
                (false, vec![67])
            ]
        );
    }

    mod spear_jab_tests {
        include!("spear_jab_tests.rs");
    }
    mod tab_publication_tests {
        include!("tab_publication_tests.rs");
    }
    mod thrown_item_swing_tests {
        include!("thrown_item_swing_tests.rs");
    }
    mod tracking_state_tests {
        include!("tracking_state_tests.rs");
    }
    mod use_flag_tests {
        include!("use_flag_tests.rs");
    }

    /// A client connecting through a local pipe and through configuration, as
    /// the client menu tests drive it: the server end's reader (what the
    /// client sends) and writer.
    pub(crate) fn connect_menu_test(
        game: &mut super::Game<'_>,
    ) -> (
        sunrise_protocol::connection::ConnReader,
        sunrise_protocol::connection::ConnWriter,
    ) {
        connect_play_test(game, |_, _| {})
    }

    pub(super) fn connect_play_test(
        game: &mut super::Game<'_>,
        configure: impl FnOnce(
            &mut sunrise_protocol::connection::ConnReader,
            &mut sunrise_protocol::connection::ConnWriter,
        ),
    ) -> (
        sunrise_protocol::connection::ConnReader,
        sunrise_protocol::connection::ConnWriter,
    ) {
        use sunrise_protocol::connection::{Connection, Pipe};
        game.options.execution_role = super::super::execution_role::ExecutionRole::Client;
        let (client, server) = Pipe::pair();
        game.connect_local(client, "MenuReadTest").unwrap();
        finish_play_test(game, Connection::local(server), configure)
    }

    fn finish_play_test(
        game: &mut super::Game<'_>,
        server: sunrise_protocol::connection::Connection,
        configure: impl FnOnce(
            &mut sunrise_protocol::connection::ConnReader,
            &mut sunrise_protocol::connection::ConnWriter,
        ),
    ) -> (
        sunrise_protocol::connection::ConnReader,
        sunrise_protocol::connection::ConnWriter,
    ) {
        use sunrise_protocol::packets::{Empty, configuration as cfg, login};
        let (mut reader, mut writer) = server.split();
        reader
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        reader.recv().unwrap();
        reader.recv().unwrap();
        writer
            .send(
                &login::Clientbound::LoginFinished(login::LoginFinished {
                    profile: login::GameProfile {
                        uuid: 42,
                        name: "MenuReadTest".into(),
                        properties: vec![],
                    },
                    session: 1,
                })
                .encode(),
            )
            .unwrap();
        let (id, body) = reader.recv().unwrap();
        assert!(matches!(
            login::Serverbound::decode(id, &body).unwrap(),
            login::Serverbound::LoginAcknowledged(_)
        ));
        reader.recv().unwrap();
        reader.recv().unwrap();
        configure(&mut reader, &mut writer);
        writer
            .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
            .unwrap();
        game.acknowledge_test_configuration();
        let (id, body) = reader.recv().unwrap();
        assert!(matches!(
            cfg::Serverbound::decode(id, &body).unwrap(),
            cfg::Serverbound::FinishConfiguration(_)
        ));
        (reader, writer)
    }

    /// The test server offers its payload manifest and reads the client's offer.
    pub(super) fn offer_test_payloads(
        reader: &mut sunrise_protocol::connection::ConnReader,
        writer: &mut sunrise_protocol::connection::ConnWriter,
        manifest: &sunrise_protocol::payload_negotiation::Manifest,
    ) {
        use sunrise_protocol::{
            packets::configuration as cfg,
            payload_negotiation::{CHANNEL, Control},
        };
        writer
            .send(
                &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                    channel: CHANNEL.into(),
                    data: Control::Offer(manifest.clone()).encode().unwrap(),
                })
                .encode(),
            )
            .unwrap();
        let (id, body) = reader.recv().unwrap();
        assert!(
            matches!(cfg::Serverbound::decode(id, &body).unwrap(), cfg::Serverbound::CustomPayload(p) if p.channel == CHANNEL)
        );
    }

    /// Spawns the connected client in the overworld as entity 7, profile 42.
    pub(crate) fn spawn_menu_test(
        game: &mut super::Game<'_>,
    ) -> sunrise_protocol::packets::play::SpawnInfo {
        use crate::game::remote::RemoteDimensionEntry;
        use sunrise_client::level_loading::Reason;
        use sunrise_protocol::packets::play::SpawnInfo;
        let spawn = SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 0,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
        let remote = game.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.entity_id = 7;
        remote.profile = 42;
        spawn
    }

    use super::{
        boat_interact_passes, chat_after_event, item_frame_interact_passes, remote_object_swings,
    };

    #[test]
    #[ignore = "daily"]
    fn declared_gate_uses_its_class_for_blasts_and_predicted_use() {
        use crate::game::native_guest_turn_tests::with_guests_at;
        use std::sync::Arc;
        use sunrise_client::{
            chunk::Chunk,
            lan::{Out, PlayerSender},
            mod_content::{ContentMod, resource_root},
        };
        use sunrise_core::position::BlockPos;
        use sunrise_mod_package::{
            content::{Block, Content, GateSound},
            enabled::EnabledMods,
        };
        use sunrise_protocol::{buf::Reader, packets::play::Clientbound};

        let base = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resources required"),
        );
        let scratch = tempfile::tempdir().unwrap();
        let mut content = Content::declared("gates");
        let use_sound = "gates:use.open";
        let blast_sound = "gates:blast.close";
        let wire_sound = "minecraft:entity.zombie.ambient";
        content
            .declare(
                &[Block {
                    id: "gates:sites".into(),
                    template: "minecraft:oak_fence_gate".into(),
                    item: false,
                    open_sound: Some(GateSound::Sites {
                        on_use: Some(use_sound.into()),
                        on_explosion: None,
                        on_redstone: Some(wire_sound.into()),
                    }),
                    close_sound: Some(GateSound::Sites {
                        on_use: None,
                        on_explosion: Some(blast_sound.into()),
                        on_redstone: None,
                    }),
                    ..Default::default()
                }],
                &[],
            )
            .unwrap();
        let root = resource_root(
            &base,
            scratch.path(),
            &[ContentMod {
                id: "gates".into(),
                content,
                jars: Vec::new(),
            }],
            scratch.path(),
            &EnabledMods::new([("gates".into(), mod_api::Side::Both)]),
        )
        .unwrap();
        with_guests_at(root, |game| {
            let pos = [3, 64, 4];
            let [x, y, z] = pos;
            let hit = crate::game::Hit {
                pos,
                face: [0, 1, 0],
                point: [3.5, 64.875, 4.5],
                inside: false,
            };
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.sync_tick_chunks();
            let gate = game
                .pack
                .assets
                .registry
                .default_state("gates:sites")
                .unwrap();
            let open = |game: &crate::game::Game<'_>| {
                let b = game.res.behaviours();
                b.table.bool(game.world.block(x, y, z), b.props.open)
            };

            let [actor, other] = [12, 13].map(|conn| {
                let (tx, rx) = std::sync::mpsc::channel();
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(conn, |guest| {
                        guest.sender = PlayerSender::new(tx, Arc::default());
                    })
                    .unwrap();
                rx
            });
            let sounds = |game: &crate::game::Game<'_>, rx: &std::sync::mpsc::Receiver<Out>| {
                let sync = game.lan.as_ref().unwrap().hub.sync_data();
                rx.try_iter()
                    .flat_map(|out| match out {
                        Out::Packet(body) => vec![body],
                        Out::Packets(bodies) => bodies,
                        Out::Disconnect(_) => Vec::new(),
                    })
                    .filter_map(|body| {
                        let mut reader = Reader::new(&body);
                        let id = reader.varint().unwrap();
                        match Clientbound::decode(id, reader.rest()).unwrap() {
                            Clientbound::Sound(packet) => {
                                let name = packet.sound.1.map(|(id, _)| id).unwrap_or_else(|| {
                                    sync.builtin
                                        .get("minecraft:sound_event")
                                        .unwrap()
                                        .name(packet.sound.0)
                                        .unwrap()
                                        .to_owned()
                                });
                                Some((name, packet.source, packet.volume))
                            }
                            _ => None,
                        }
                    })
                    .collect::<Vec<_>>()
            };
            let blocks = mod_api::SoundSource::Blocks.index();
            let vanilla_gate = game
                .pack
                .assets
                .registry
                .default_state("minecraft:oak_fence_gate")
                .unwrap();
            game.options.dedicated = Some(Vec::new());
            for (state, name) in [
                (vanilla_gate, "minecraft:block.fence_gate.open"),
                (gate, use_sound),
            ] {
                game.world.set(x, y, z, state).unwrap();
                game.audio.capture = Some(Vec::new());
                let (answer, dry) = game.remote_block_reacts(&hit, None, true);
                assert!(answer.consumes_action());
                game.predict_use(&dry, 1);
                assert!(open(game), "the gate opens without a server round trip");
                let capture = game.audio.capture.as_mut().unwrap();
                assert_eq!(capture.len(), 1);
                let predicted = capture.pop().unwrap();
                assert_eq!((predicted.event.as_str(), predicted.source), (name, blocks));

                game.world.set(x, y, z, state).unwrap();
                game.world.block_log = Some(Vec::new());
                assert!(
                    game.guest_use_item_on(
                        12,
                        game.guest_index(12).unwrap(),
                        hit,
                        super::Hand::Main,
                        None,
                    )
                    .unwrap()
                );
                assert!(open(game));
                game.lan_broadcast_effects();
                assert!(sounds(game, &actor).is_empty(), "no echo to the clicker");
                assert_eq!(sounds(game, &other), [(name.into(), blocks as i32, 1.0)]);
            }

            game.explosion_trigger_blocks(&mut [pos]).unwrap();
            assert!(!open(game));
            game.lan_broadcast_effects();
            for listener in [&actor, &other] {
                assert_eq!(
                    sounds(game, listener),
                    [(blast_sound.into(), blocks as i32, 1.0)]
                );
            }

            let power = game
                .pack
                .assets
                .registry
                .default_state("minecraft:redstone_block")
                .unwrap();
            assert!(game.with_level(|b, level, ticks| {
                b.set_block(level, ticks, BlockPos::new(x + 1, y, z), power, 3)
            }));
            game.process_level_events().unwrap();
            assert!(open(game));
            game.lan_broadcast_effects();
            for listener in [&actor, &other] {
                assert_eq!(
                    sounds(game, listener),
                    [(wire_sound.into(), blocks as i32, 1.0)]
                );
            }
            assert_eq!(game.audio.heard.get(blast_sound), Some(&1));
            let powered = game
                .with_props(gate, &[("open", "true"), ("powered", "true")])
                .unwrap();
            game.world.set(x, y, z, powered).unwrap();
            game.explosion_trigger_blocks(&mut [pos]).unwrap();
            assert_eq!(game.world.block(x, y, z), powered);
            assert_eq!(game.audio.heard.get(blast_sound), Some(&1));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_alias_survival_consumption_and_client_prediction() {
        use crate::game::{Audio, Game, Options, Progress, Settings, World, load_resources};
        use std::{path::PathBuf, sync::Arc};
        use sunrise_client::{
            chunk::Chunk,
            items::ItemStack,
            mod_content::{ContentMod, build},
        };
        use sunrise_mod_package::content::{Block, Content, Item};
        let base = PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resources required"),
        );
        let root = std::env::temp_dir().join(format!("sunrise-alias-game-{}", std::process::id()));
        let mut content = Content::declared("ruby");
        content
            .declare(
                &[Block {
                    id: "ruby:frame".into(),
                    template: "minecraft:stone".into(),
                    item: false,
                    ..Default::default()
                }],
                &["empty", "filled"].map(|name| Item {
                    id: format!("ruby:{name}"),
                    template: "minecraft:stick".into(),
                    block: Some("ruby:frame".into()),
                    ..Default::default()
                }),
            )
            .unwrap();
        let mods = [ContentMod {
            id: "ruby".into(),
            content,
            jars: Vec::new(),
        }];
        let inventory = sunrise_mod_package::enabled::EnabledMods::new([(
            "ruby".to_owned(),
            mod_api::Side::Both,
        )]);
        build(&base, std::path::Path::new(""), &mods, &root, &inventory).unwrap();
        let (registry, _) = sunrise_world::level::data::load_registry_with_flags(&root).unwrap();
        crate::game::api_values::register_game_keys((0..registry.len()).map(|id| {
            registry
                .state(sunrise_world::registry::BlockStateId(id as u32))
                .unwrap()
                .name
                .clone()
        }));
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.world = World::new(game.res.facts().air, 1);
        for x in -1..=1 {
            for z in -1..=1 {
                game.world
                    .chunks
                    .insert((x, z), Arc::new(Chunk::empty(x, z)));
            }
        }
        game.sync_tick_chunks();
        game.set_mode_id(0);
        game.player = None;
        game.selected = 0;
        let target = game
            .pack
            .assets
            .registry
            .default_state("ruby:frame")
            .unwrap();
        for (i, name) in ["ruby:empty", "ruby:filled"].iter().enumerate() {
            let pos = [i as i32 * 2, 70, 0];
            let hit = crate::game::Hit {
                pos,
                face: [0, 1, 0],
                inside: false,
                point: [f64::from(pos[0]) + 0.5, 70.0, 0.5],
            };
            let stack = ItemStack::new(name, 2);
            assert_eq!(game.items().get(name).unwrap().block, Some(target));
            let preview = game
                .remote_can_place(&hit, &stack)
                .expect("client predicts the alias");
            assert_eq!([preview.pos.x, preview.pos.y, preview.pos.z], pos);
            assert_eq!(
                game.world.block(pos[0], pos[1], pos[2]),
                game.res.facts().air,
                "prediction is dry"
            );
            *game.held_mut() = Some(stack);
            assert!(game.use_hand(Some(&hit)).unwrap());
            assert_eq!(game.world.block(pos[0], pos[1], pos[2]), target);
            let held = game.held().unwrap();
            assert_eq!((&*held.item, held.count), (*name, 1));
        }
        let blocked = crate::game::Hit {
            pos: [0, 70, 0],
            face: [0, 1, 0],
            inside: false,
            point: [0.5, 71.0, 0.5],
        };
        game.set_mode_id(2);
        let before = game.held().cloned();
        game.use_hand(Some(&blocked)).unwrap();
        assert_eq!(
            game.held(),
            before.as_ref(),
            "adventure refusal keeps the alias stack"
        );
        drop(game);
        std::fs::remove_dir_all(root).unwrap();
    }

    use sunrise_client::{
        containers::{Beacon, BlockEntity},
        items::ItemStack,
    };
    use sunrise_core::nbt::Tag;

    /// #12040: a server's client moves at the speed the server's attributes give it, never its
    /// effects' own. Vanilla's server removes a Speed that runs out in ServerPlayer.doTick, after
    /// the tick's sendDirtyEntityData: ClientboundRemoveMobEffectPacket goes out at once and the
    /// movement speed without the effect's modifier a tick later, so for that tick the client still
    /// runs fast (a potion drunk starts the same way). A client a tick off is what GrimAC's
    /// Simulation flagged.
    #[test]
    #[ignore = "daily"]
    fn a_servers_client_takes_its_speed_from_the_attributes_not_the_effect() {
        use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
        use sunrise_client::motion::PlayerMotion;
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::packets::play::{
            AttributeModifier, AttributeSnapshot, Clientbound as C, RemoveMobEffect,
            UpdateAttributes, UpdateMobEffect,
        };
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resources required"),
        );
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::Client,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.init_remote_for_replay().unwrap();
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
        let me = game.remote.as_ref().unwrap().entity_id;
        let speed = game.remote_effect_network_id("speed").unwrap();
        let attribute = {
            let p = game.player.as_ref().unwrap();
            let key = p.attributes.key("minecraft:movement_speed").unwrap();
            key.attribute().registry_id()
        };
        // Player.createAttributes: 0.1F.
        let walking = f64::from(0.1f32);
        // Speed II: 0.2 (a float) a level, multiplying the total.
        let amount = f64::from(0.2f32) * 2.0;
        let attributes = |sped: bool| {
            C::UpdateAttributes(UpdateAttributes {
                id: me,
                values: vec![AttributeSnapshot {
                    attribute,
                    base: walking,
                    modifiers: sped
                        .then(|| AttributeModifier {
                            id: "minecraft:effect.speed".into(),
                            amount,
                            operation: 2,
                        })
                        .into_iter()
                        .collect(),
                }],
            })
        };
        // Each step: the packets of one server tick, then a client tick.
        let mut step = |packets: Vec<C>| {
            for packet in packets {
                game.handle_packet(packet).unwrap();
            }
            game.tick_client_effects();
            game.player
                .as_mut()
                .unwrap()
                .attribute("minecraft:movement_speed")
                .unwrap()
        };
        let effect = C::UpdateMobEffect(UpdateMobEffect {
            id: me,
            effect: speed,
            amplifier: 1,
            duration: 60,
            flags: 2 | 4,
        });
        let fast = walking * (1.0 + amount);
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        let seen = [
            step(vec![effect]),
            step(vec![attributes(true)]),
            step(Vec::new()),
            step(vec![C::RemoveMobEffect(RemoveMobEffect {
                id: me,
                effect: speed,
            })]),
            step(vec![attributes(false)]),
        ];
        let expected = [walking, fast, fast, fast, walking];
        assert!(
            seen.iter().zip(expected).all(|(&s, e)| close(s, e)),
            "speed on each tick: {seen:?}, vanilla's client: {expected:?}"
        );
    }

    use super::Game;
}

#[cfg(test)]
mod boat_push_order_tests;
#[cfg(test)]
mod item_use_tick_tests;
#[cfg(test)]
mod movement_input_tests;
#[cfg(test)]
mod swim_start_tests;
/// `Entity.baseTick`'s fluid update for a mob this window only displays (`fluids`, sampled before
/// the tick), with the splash it draws, and the block it stands on.
pub(super) fn client_fluid_step(
    m: &mut sunrise_client::mobs::Mob,
    fluids: Option<sunrise_client::mobs::living::Fluids>,
    standing: Option<String>,
    own: Option<i32>,
    fx: &mut Vec<sunrise_client::mobs::MobEvent>,
) {
    if let Some(fluids) = fluids {
        let was = m.fluids.in_water;
        let first = m.first_tick;
        // Entity.baseTick saves the preceding eye sample before the new fluid update.
        m.was_eye_in_water = m.fluids.eye_in_water;
        m.fluids = fluids;
        // Entity.doWaterSplashEffect on the client copy. Particles
        // only: ClientLevel draws addParticle, and playSeededSound
        // drops Entity.playSound (the server broadcasts that).
        if m.fluids.in_water && !was && !first && Some(m.id as i32) != own {
            sunrise_client::mobs::living::client_water_splash(m, fx);
        }
        m.first_tick = false;
    }
<<<<<<< sync-sunrise-1
    #[test]
    fn synchronized_damage_defaults_override_a_different_local_definition() {
        let local = sunrise_world::damage_source::Catalog::from_json(&serde_json::json!({
            "data_version": sunrise_world::DATA_VERSION,
            "types": [{"id":"demo:heat", "message_id":"heat", "exhaustion":"00000000", "scaling":"never", "effects":"burning", "death_message_type":"default"}]
        }), &serde_json::json!({})).unwrap();
        let mut registries = sunrise_protocol::registry::Dynamic::default();
        registries.add(sunrise_protocol::packets::configuration::RegistryData {
            registry: "minecraft:damage_type".into(),
            entries: vec![sunrise_protocol::packets::configuration::RegistryEntry {
                id: "demo:heat".into(),
                data: Some(sunrise_core::nbt::Tag::Compound(Default::default())),
            }],
        });
        assert_eq!(
            super::hurt_sound_for(&registries, &local, 0),
            "minecraft:entity.player.hurt"
        );
    }
    /// `ClientPacketListener.handleSetEntityPassengersPacket`: the vehicle
    /// ejects its passengers, then each listed entity `startRiding`s it, so
    /// `isVehicle` holds while the list is not empty, whatever the passenger's
    /// type: a server's item display riding a player hides that player's name
    /// tag like a text display does (#5666).
    #[test]
    fn any_tracked_passenger_makes_its_vehicle_a_vehicle() {
        use std::collections::{HashMap, HashSet};
        let tracked = |kind: &str| super::Tracked {
            kind: kind.to_owned(),
            position: [0.0, 64.0, 0.0],
            codec_base: [0.0, 64.0, 0.0],
            silent: false,
            vehicle: None,
        };
        let mut entities = HashMap::from([
            (42, tracked("minecraft:player")),
            (43, tracked("minecraft:player")),
            (9, tracked("minecraft:item_display")),
            (10, tracked("minecraft:block_display")),
        ]);
        let vehicles = |e: &HashMap<i32, super::Tracked>| -> HashSet<u32> {
            e.values().filter_map(|t| t.vehicle).collect()
        };
        super::seat_passengers(&mut entities, 42, &[9, 10]);
        assert_eq!(vehicles(&entities), HashSet::from([42]));
        // The list replaces the old one: 10 got off, 9 still rides.
        super::seat_passengers(&mut entities, 42, &[9]);
        assert_eq!(entities[&10].vehicle, None);
        assert_eq!(entities[&9].vehicle, Some(42));
        // startRiding leaves the old vehicle first.
        super::seat_passengers(&mut entities, 43, &[9]);
        assert_eq!(vehicles(&entities), HashSet::from([43]));
        // An empty list: nobody rides 43 any more.
        super::seat_passengers(&mut entities, 43, &[]);
        assert!(vehicles(&entities).is_empty());
    }
    /// `ClientboundSoundEntityPacket`: the sound is heard where the entity it
    /// names is — the local player included, which is no tracked entity — and
    /// an id this level doesn't have plays nothing at all.
    #[test]
    fn a_sound_entity_is_heard_where_its_entity_is() {
        use std::collections::HashMap;
        let mut tracked = HashMap::new();
        tracked.insert(
            7,
            super::Tracked {
                kind: "minecraft:cow".to_owned(),
                position: [8.5, 64.0, -3.5],
                codec_base: [8.5, 64.0, -3.5],
                silent: false,
                vehicle: None,
            },
        );
        let player = Some((2, [0.5, 70.0, 0.5], false));
        assert_eq!(
            super::entity_sound_state(2, player, None, &tracked, None)
                .filter(|s| !s.silent)
                .map(|s| s.position),
            Some([0.5, 70.0, 0.5]),
            "the local player is named by Remote.entity_id"
        );
        assert_eq!(
            super::entity_sound_state(7, player, None, &tracked, None)
                .filter(|s| !s.silent)
                .map(|s| s.position),
            Some([8.5, 64.0, -3.5]),
            "another entity the server spawned"
        );
        assert_eq!(
            super::entity_sound_state(9, player, None, &tracked, None)
                .filter(|s| !s.silent)
                .map(|s| s.position),
            None,
            "getEntity is null: handleSoundEntityEvent returns"
        );
        assert_eq!(
            super::entity_sound_state(2, None, None, &tracked, None)
                .filter(|s| !s.silent)
                .map(|s| s.position),
            None,
            "a player with no position yet"
        );
    }
    #[test]
    fn a_sound_entity_retains_silence_for_players_and_non_mob_targets() {
        use std::collections::HashMap;
        let mut tracked = HashMap::new();
        tracked.insert(
            7,
            super::Tracked {
                kind: "minecraft:arrow".to_owned(),
                position: [8.5, 64.0, -3.5],
                codec_base: [8.5, 64.0, -3.5],
                silent: true,
                vehicle: None,
            },
        );
        let state = super::entity_sound_state(7, None, None, &tracked, None).unwrap();
        assert!(state.silent);
        assert!(!state.removed);
        assert_eq!(state.position, [8.5, 64.0, -3.5]);
        let player =
            super::entity_sound_state(2, Some((2, [1.0, 2.0, 3.0], true)), None, &tracked, None)
                .unwrap();
        assert!(player.silent);
        assert!(super::entity_sound_state(9, None, None, &tracked, None).is_none());
    }
    #[test]
    fn a_dragon_sound_uses_live_movement_without_losing_mob_metadata() {
        use sunrise_client::sound_engine::SoundEntityState;
        let stale = SoundEntityState {
            position: [0.0, 64.0, 0.0],
            silent: true,
            removed: false,
        };
        let tracked = std::collections::HashMap::new();
        for position in [[24.0, 80.0, -16.0], [25.5, 81.0, -18.0]] {
            let state =
                super::entity_sound_state(7, None, Some(stale), &tracked, Some((7, position)))
                    .unwrap();
            assert_eq!(state.position, position);
            assert!(state.silent);
            assert!(!state.removed);
        }
        let unrelated =
            super::entity_sound_state(7, None, Some(stale), &tracked, Some((8, [99.0; 3])))
                .unwrap();
        assert_eq!(unrelated.position, stale.position);
        assert!(super::entity_sound_state(7, None, None, &tracked, Some((7, [99.0; 3]))).is_none());
    }

    /// Q over a slot of the inventory on a server (#3357): the predicted THROW
    /// drops the stack, so the player's hand swings, as `LivingEntity.drop`
    /// does for `Prediction.PREDICTED`; a plain pick up drops nothing.
    #[test]
    fn a_predicted_throw_from_a_slot_swings_the_hand() {
        use sunrise_client::{
            banner::Banners,
            brewing::Brewing,
            crafting::Recipes,
            creative::Names,
            enchant::Enchantments,
            items::{ItemStack, Items},
            menu::{Env, Input, Kind, Menu},
            stations::StationRecipes,
        };
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let none = &sunrise_world::loot::PackOverlay::NONE;
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let items = Items::catalog(&root, none, &defaults).unwrap();
        let recipes = Recipes::load(&root, &items, none, &crate::game::test_server_load()).unwrap();
        let brewing = Brewing::load(&root, &items, none);
        let banners = Banners::load(&root, &defaults);
        let station_recipes =
            StationRecipes::load(&root, &items, none, &crate::game::test_server_load());
        let enchantments = Enchantments::default();
        let names = Names::default();
        let env = Env {
            crafting_remainder: None,
            player: None,
            player_entity_id: None,
            loop_context: &crate::game::test_server_load()
                .loop_context(sunrise_world::native_load_context::LoopSide::Client, []),
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            limited_crafting: false,
            can_equip: None,
            known_recipes: None,
            slot_rules: None,
        };
        let mut menu = Menu::new(Kind::Inventory);
        let cobblestone = ItemStack {
            item: "minecraft:cobblestone".into(),
            count: 16,
            damage: 0,
            extra: None,
        };
        // The first hotbar slot of InventoryMenu.
        menu.set_slot(&env, 36, Some(cobblestone));
        let (sent, threw) =
            super::predict_clicks(&mut menu, &env, &[(36, 0, Input::Throw)]).unwrap();
        assert!(threw, "Q over a slot drops one and swings");
        assert_eq!(menu.slot(36).map(|s| s.count), Some(15));
        assert_eq!(sent.len(), 1);
        let (_, threw) = super::predict_clicks(&mut menu, &env, &[(36, 0, Input::Pickup)]).unwrap();
        assert!(!threw, "picking the stack up drops nothing");
        let (_, threw) =
            super::predict_clicks(&mut menu, &env, &[(-999, 1, Input::Pickup)]).unwrap();
        assert!(threw, "one of the carried stack outside the window");
    }

    #[test]
    fn free_direction_sends_a_break_before_the_move_that_goes_into_its_space() {
        use super::{Empty, S, play, waits_for_the_view};
        let action = |action| {
            S::PlayerAction(play::PlayerAction {
                action,
                pos: [0, 62, 0],
                direction: 0,
                sequence: 1,
            })
        };
        // Start, change direction, abort and stop go at once right after a steered move.
        for dig in 0..=3 {
            assert!(
                !waits_for_the_view(&action(dig), true, false),
                "action {dig}"
            );
        }
        // A drop (5, DROP_ITEM) and a use wait for a move with the view's look.
        let used = S::UseItem(play::UseItem {
            hand: 0,
            sequence: 2,
            yaw: 0.0,
            pitch: 0.0,
        });
        assert!(waits_for_the_view(&action(5), true, false));
        assert!(waits_for_the_view(&used, true, false));
        assert!(!waits_for_the_view(&used, false, false));
        // After something held everything waits, in order; a swing alone never does.
        assert!(waits_for_the_view(&action(0), true, true));
        assert!(waits_for_the_view(&S::Punch(Empty), false, true));
        assert!(!waits_for_the_view(&S::Punch(Empty), true, false));
    }

    #[test]
    fn placement_preview_checks_obstruction_without_changing_the_level() {
        use super::DryLevel;
        use sunrise_core::{position::BlockPos, random::LegacyRandom};
        use sunrise_world::{
            level::{
                Behaviours, LevelAccess, PlaceContext, PlaceOutcome, Ticks, data::load_registry,
            },
            registry::BlockStateId,
        };

        struct Level {
            air: BlockStateId,
            stone: BlockStateId,
            power: BlockStateId,
            client_side: bool,
            obstructed: bool,
            loaded: bool,
            ceiling: i32,
        }
        impl sunrise_world::level::BlockEntityNeighbors for Level {}
        impl LevelAccess for Level {
            fn block_state(&self, pos: BlockPos) -> BlockStateId {
                if pos == BlockPos::new(0, 0, 0) {
                    self.stone
                } else if pos == BlockPos::new(-1, 1, 0) {
                    self.power
                } else {
                    self.air
                }
            }
            fn set_raw(&mut self, _: BlockPos, _: BlockStateId) -> Option<BlockStateId> {
                panic!("preview must not write to the live level");
            }
            fn is_loaded(&self, _: BlockPos) -> bool {
                self.loaded
            }
            fn is_client_side(&self) -> bool {
                self.client_side
            }
            fn max_y(&self) -> i32 {
                self.ceiling
            }
            fn is_unobstructed(&self, pos: BlockPos, _: BlockStateId) -> bool {
                assert_eq!(pos, BlockPos::new(0, 1, 0));
                !self.obstructed
            }
            fn random(&mut self) -> &mut LegacyRandom {
                panic!("preview must not advance the live level's random source");
            }
            fn game_time(&self) -> i64 {
                0
            }
        }

        // As with the LAN tests, prepared game resources are optional locally.
        let resources = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !resources.join("block-behaviours.json").exists() {
            return;
        }
        let registry = load_registry(&resources).unwrap();
        let behaviours = Behaviours::load(&registry, &resources).unwrap();
        let mut base = Level {
            air: registry.default_state("minecraft:air").unwrap(),
            stone: registry.default_state("minecraft:stone").unwrap(),
            power: registry.default_state("minecraft:redstone_block").unwrap(),
            client_side: true,
            obstructed: false,
            loaded: true,
            ceiling: 320,
        };
        for item in [
            "minecraft:stone",
            "minecraft:oak_slab",
            "minecraft:oak_door",
            "minecraft:tnt",
            "minecraft:piston",
            "minecraft:sticky_piston",
            // GrowingPlantHeadBlock.getStateForPlacement draws a random age.
            "minecraft:twisting_vines",
        ] {
            let ctx = PlaceContext {
                clicked_pos: BlockPos::new(0, 0, 0),
                face: sunrise_core::geometry::Direction::Up,
                hit: sunrise_core::geometry::Vec3::new(0.5, 1.0, 0.5),
                inside: false,
                yaw: 0.0,
                pitch: 45.0,
                sneaking: false,
                item: item.into(),
                can_use_game_master_blocks: false,
            };
            for (client_side, obstructed, loaded, ceiling) in [
                (true, false, true, 320),
                (true, true, true, 320),
                (true, false, false, 320),
                (true, false, true, 1),
                (false, false, true, 320),
            ] {
                base.client_side = client_side;
                base.obstructed = obstructed;
                base.loaded = loaded;
                base.ceiling = ceiling;
                let valid = !obstructed && loaded && ceiling > 1;
                let mut preview = DryLevel::new(&base);
                let mut ticks = Ticks::default();
                let outcome = behaviours.place_block(&mut preview, &mut ticks, &ctx);
                assert_eq!(
                    matches!(outcome, PlaceOutcome::Placed { .. }),
                    valid,
                    "{item}, obstructed={obstructed}, loaded={loaded}, ceiling={ceiling}"
                );
                assert_eq!(preview.changed.is_empty(), !valid);
                if valid
                    && matches!(
                        item,
                        "minecraft:tnt" | "minecraft:piston" | "minecraft:sticky_piston"
                    )
                {
                    assert_eq!(
                        registry
                            .state(preview.block_state(BlockPos::new(0, 1, 0)))
                            .unwrap()
                            .name,
                        if item == "minecraft:tnt" && !client_side {
                            "minecraft:air"
                        } else {
                            item
                        },
                        "powered {item}, client_side={client_side}"
                    );
                    assert_eq!(
                        ticks.block_events.is_empty(),
                        client_side || item == "minecraft:tnt",
                        "only the server starts a powered piston"
                    );
                }
                assert_eq!(base.block_state(BlockPos::new(0, 1, 0)), base.air);
            }
        }
    }

    #[test]
    fn the_creative_sync_covers_inventory_menu_slots_5_to_45_once_each() {
        // CreativeInventoryListener sends every InventoryMenu slot but the crafting ones: the
        // armour (5..=8), the inventory (9..=44) and the off hand (45).
        let stack = |item: &str| Some(sunrise_client::items::ItemStack::new(item, 1));
        let inventory: Vec<_> = (0..36)
            .map(|i| stack(&format!("minecraft:item_{i}")))
            .collect();
        let offhand = stack("minecraft:shield");
        let armor = [
            stack("minecraft:diamond_helmet"),
            stack("minecraft:diamond_chestplate"),
            stack("minecraft:diamond_leggings"),
            stack("minecraft:diamond_boots"),
        ];
        let stacks = super::creative_sync_stacks(&inventory, &offhand, &armor);
        assert_eq!(stacks.len(), super::CREATIVE_SYNCED);
        let mut slots: Vec<i16> = (0..stacks.len()).map(super::creative_sync_slot).collect();
        // Each entry goes to the menu slot that holds that stack.
        for (i, &slot) in slots.iter().enumerate() {
            let game_slot = Game::inventory_screen_slot("inventory", i32::from(slot)).unwrap();
            let expected = match game_slot {
                0..36 => &inventory[game_slot],
                504 => &offhand,
                500..504 => &armor[game_slot - 500],
                _ => panic!("entry {i} went to game slot {game_slot}"),
            };
            assert_eq!(&stacks[i], expected, "entry {i}, menu slot {slot}");
        }
        slots.sort();
        assert_eq!(slots, (5..=45).collect::<Vec<i16>>());
    }

    #[test]
    fn a_server_slot_update_leaves_an_unsent_creative_click_to_be_sent() {
        let stack = |item: &str| Some(sunrise_client::items::ItemStack::new(item, 1));
        let none = || vec![None; super::CREATIVE_SYNCED];
        // Synced: nothing. A click puts a helmet on (entry 37, not sent yet), then the server
        // sets hotbar slot 0 (a pickup): only that entry counts as synced.
        let mut synced = none();
        let mut before = none();
        before[37] = stack("minecraft:diamond_helmet");
        let mut now = before.clone();
        now[0] = stack("minecraft:stick");
        super::mark_synced(&mut synced, &before, now.clone());
        assert_eq!(synced[0], now[0]);
        assert_eq!(
            synced[37], None,
            "the helmet still differs, so the next tick sends it"
        );
    }

    #[test]
    fn every_inventory_menu_slot_is_a_game_slot_and_45_is_the_off_hand() {
        // InventoryMenu: result, 2x2 grid, armour, 27 + 9 inventory slots, then the off hand.
        assert_eq!(Game::inventory_screen_slot("inventory", 45), Some(504));
        let mut seen = std::collections::HashSet::new();
        for slot in 0..46 {
            let internal = Game::inventory_screen_slot("inventory", slot);
            assert!(internal.is_some_and(|i| seen.insert(i)), "slot {slot}");
        }
        assert_eq!(Game::inventory_screen_slot("inventory", 46), None);
    }

    #[test]
    fn remote_clicks_find_slots_past_the_old_large_chest_cutoff() {
        for (menu, internal, index) in [
            ("minecraft:generic_9x6", 18, 63),
            ("minecraft:generic_9x6", 19, 64),
            ("minecraft:generic_9x6", 0, 81),
            ("minecraft:generic_9x6", 8, 89),
            ("minecraft:generic_9x6", 353, 53),
            ("minecraft:generic_9x5", 8, 80),
            ("minecraft:generic_9x4", 8, 71),
            ("minecraft:generic_9x3", 8, 62),
            ("inventory", 504, 45),
        ] {
            assert_eq!(
                Game::menu_slot_index(menu, internal),
                Some(index),
                "{menu}, game slot {internal}"
            );
        }
        assert_eq!(Game::menu_slot_index("minecraft:generic_9x6", 354), None);
        assert_eq!(Game::menu_slot_index("minecraft:generic_9x6", 504), None);
        assert_eq!(Game::menu_slot_index("unknown", 0), None);
    }

    /// `FireworkRocketItem.use` is SUCCESS while gliding: the main hand's
    /// rocket ends the click and the off hand's shield is not raised (#3424).
    /// On foot it passes, so the off hand still gets its turn.
    #[test]
    fn a_rocket_used_while_gliding_ends_the_click() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            eprintln!("skipping: no prepared resources in {}", root.display());
            return;
        }
        let registry = sunrise_world::level::data::load_registry(&root).unwrap();
        let facts = sunrise_client::blocks::BlockFacts::load(&root, &registry).unwrap();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let items = sunrise_client::items::Items::load(
            &root,
            &registry,
            &facts,
            &sunrise_world::loot::PackOverlay::NONE,
            &defaults,
        )
        .unwrap();
        let rocket = items.get("minecraft:firework_rocket").unwrap();
        assert!(super::use_ends_click(rocket, true), "gliding");
        assert!(!super::use_ends_click(rocket, false), "on foot");
        // Other items answer the same whether or not the player glides.
        let shield = items.get("minecraft:shield").unwrap();
        let stick = items.get("minecraft:stick").unwrap();
        for gliding in [false, true] {
            assert!(super::use_ends_click(shield, gliding));
            assert!(!super::use_ends_click(stick, gliding));
            // FoodOnAStickItem.use passes on a client: no swing while the boost cools down.
            for rod in [
                "minecraft:carrot_on_a_stick",
                "minecraft:warped_fungus_on_a_stick",
            ] {
                let rod = items.get(rod).unwrap();
                assert!(!super::use_ends_click(rod, gliding), "{}", rod.id);
            }
            // A carved pumpkin has an equip_slot (head) but is not swappable, so it has no Item.use.
            let pumpkin = items.get("minecraft:carved_pumpkin").unwrap();
            assert!(!super::use_ends_click(pumpkin, gliding));
        }
    }

    /// `ClientPacketListener.handleGameEvent`: verify that the game event packet
    /// types for arrow hit, pufferfish sting and limited crafting match vanilla.
    #[test]
    fn game_event_constants_and_types_match_vanilla() {
        use sunrise_protocol::packets::play::GameEvent;
        // Event types from ClientboundGameEventPacket:
        // 6 = PLAY_ARROW_HIT_SOUND, 7 = RAIN_LEVEL_CHANGE, 8 = THUNDER_LEVEL_CHANGE,
        // 9 = PUFFER_FISH_STING, 11 = IMMEDIATE_RESPAWN, 12 = LIMITED_CRAFTING.
        let arrow = GameEvent {
            event: 6,
            param: 0.0,
        };
        let rain = GameEvent {
            event: 7,
            param: 0.5,
        };
        let thunder = GameEvent {
            event: 8,
            param: 0.2,
        };
        let puffer = GameEvent {
            event: 9,
            param: 0.0,
        };
        let respawn = GameEvent {
            event: 11,
            param: 1.0,
        };
        let crafting = GameEvent {
            event: 12,
            param: 1.0,
        };

        assert_eq!(arrow.event, 6);
        assert_eq!(rain.event, 7);
        assert_eq!(thunder.event, 8);
        assert_eq!(puffer.event, 9);
        assert_eq!(respawn.event, 11);
        assert_eq!(crafting.event, 12);
        assert!(crafting.param == 1.0);
    }

    /// #14773: Flint and steel and fire charge useOn on a client level ends the
    /// click (returning true), so when held with an unusable off-hand item, the
    /// click does not fall through to UseItem or try the off hand.
    #[test]
    #[ignore = "needs prepared resources in SUNRISE_TEST_RESOURCES"]
    fn flint_and_steel_use_on_ends_the_click() {
        use super::super::{
            Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
        };
        use crate::game::Hit;
        use sunrise_client::{first_person::Hand, motion::PlayerMotion};
        use sunrise_core::geometry::Vec3;

        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES")
                .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
        );
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = ExecutionRole::Client;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        let (client, _server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(client, "FlintTest").unwrap();
        game.remote.as_mut().unwrap().entity_id = 7;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
        game.inventory[0] = Some(ItemStack::new("minecraft:flint_and_steel", 1));
        game.offhand = Some(ItemStack::new("minecraft:stick", 1));
        game.selected = 0;
        game.forced_hit = Some(Hit {
            pos: [0, 63, 0],
            face: [0, 1, 0],
            point: [0.5, 64.0, 0.5],
            inside: false,
        });

        let done = game.remote_use_block(Hand::Main);
        assert!(done, "flint and steel useOn must end the click");
=======
    if let Some(name) = standing {
        m.standing_on = Some(name);
>>>>>>> sunrise/main
    }
}

#[cfg(test)]
mod frozen_screen_effects_tests;
#[cfg(test)]
mod placement_prediction_tests;
#[cfg(test)]
#[path = "player_tick_tests.rs"]
mod player_tick_tests;
#[cfg(test)]
mod rider_tick_tests;
#[cfg(test)]
mod smoke_source_tests;
