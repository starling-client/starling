//! `host.transaction` (#41 §7): NeoForge 26.3's `TransactionContext` and `SnapshotJournal`, kept by
//! the game at the storage level, so a mod's handlers hold no state and write no journal.
//!
//! A capability write applies in place and keeps its storage's first before-image per depth.
//! Game-held vanilla containers defer implicit changed/comparator effects to root commit;
//! nested commits hand their snapshots and waiting work to the parent. An abort restores
//! storage quietly and discards those commit effects. Raw mod-owned storage writes and
//! restores synthesize no notice. Explicit notifications a mod issues run immediately and
//! survive abort: the journal restores storage, not observer calls or scheduled comparator
//! ticks that already happened.
//!
//! Covered here: block entities (item and data slots, saved compound, queued client update fields,
//! custom name, lock, loot roll, attachments), a hopper's cooldown and cauldron and composter
//! block states. The attachment map of every other holder (an entity, a player, a chunk,
//! the level) is kept on the first access to it through `attachment_map`.
//! A block's tentative state is stored only (`World::set_stored`): the
//! one real write, with its remesh, relight and logs, is at the root commit. A jukebox is
//! told at the root commit from its record before and after, so a disc swapped in one
//! transaction changes the song. A player's inventory (slots, armour, off hand, cursor) is kept
//! whole on its first write, from wherever the player's state lives then: the acting fields
//! or a parked guest's record. On abort, the journal follows that storage through guest turns.
//! An item a write drops from a player waits for
//! the root commit too. Item entities keep their stack, pickup delay, target and velocity on
//! first write. An item place drained by extraction is removed at root commit if still empty;
//! `ItemEntity.setItem` alone leaves an empty item for the tick to discard. General entity views
//! keep velocity, rotation and fall distance on first write. A guest's menu storage (ender chest,
//! crafting grid, a mod's chest menu) is kept whole on its first write through a menu slot; a
//! crafting grid changed by the transaction has its result made again at the root commit.
//! `LivingRef::set_health` keeps the player's synced float storage and unpublished update
//! events, or the mob's health and sync dirtiness. This does not journal other health,
//! damage, death or attribute operations. Mob effect writes keep their effect lists, dirty bit,
//! health and absorption, and the attribute modifier rows the effects own.
//! An entity added inside one goes at an abort, one removed is only marked until the root
//! commit, and mods hear of joins and leaves at the root commit (`transactions/entities.rs`).
use super::{
    BlockEntity, BlockStateId, Dim, Game, lan_mount_menu::MountClickWindow, machines::HopperState,
    maps::MapData,
};
use mod_api::{
    __private::TransactionFrame, LevelRef, PlayerRef, RecipeBookTypeKey, TransactionContext,
    TransactionLifecycle, TransactionScopeError,
};
use std::collections::{BTreeMap, HashMap};
use sunrise_client::{
    block_entities::BlockEntityIdentity,
    effects::EffectInstance,
    fast_hash::FxHashSet,
    items::ItemStack,
    living_attributes::ModifierChanges,
    menu::QuickCraftDrag,
    mobs::{Mob, StoredEffect, finalize::EffectModifierUndo, gear::Gear},
    recipe_book::BookFlags,
    world::{BlockEntityExtras, ModifiedChunks, World},
};
use sunrise_core::{geometry::Vec3, position::BlockPos, random::LegacyRandom};
use sunrise_items::equipment::Equipment;
use sunrise_mod_runtime::attachments::{AttachmentQueryScopes, Attachments, Holder};
use sunrise_world::level::flags;

#[path = "transactions/decision.rs"]
pub(in crate::game) mod decision;
#[path = "transactions/entities.rs"]
mod entities;
pub(in crate::game) use entities::{Announcement, ArrivalRollback};
#[path = "transactions/scratch.rs"]
mod scratch;

/// A notice names the container that received the write, not a replacement at its position.
pub(super) struct QuickMoveNotice {
    dim: Dim,
    holder: BlockEntityIdentity,
}

struct QuickMoveNotices<'a, 'g> {
    game: &'a mut Game<'g>,
    outer: Option<Vec<QuickMoveNotice>>,
}

impl Drop for QuickMoveNotices<'_, '_> {
    fn drop(&mut self) {
        self.game.transactions.quick_move_notices = self.outer.take();
    }
}

type Pos = (i32, i32, i32);

/// A level-local save follows that level's block writes. A transaction save follows the
/// dimension-keyed owner journal (e.g. an entity's equipment), even through a different level.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum SaveChunkScope {
    Level,
    Transaction,
}

/// The last pending value at this depth for a connection, menu generation and data index.
type MenuDataJournal = BTreeMap<(u32, u64, usize), i32>;
type ComposterRandoms =
    HashMap<(mod_api::DimensionKey, (i32, i32)), HashMap<Pos, sunrise_core::random::LegacyRandom>>;

/// The open transactions, innermost last, and which close is running, if any: what
/// `transaction_lifecycle` reads.
#[derive(Default)]
pub(super) struct OpenTransactions {
    levels: Vec<Level>,
    pub(in crate::game) attachment_query_scopes: AttachmentQueryScopes,
    // A live, non-ZST address distinguishes journal instances.
    tag: Option<Box<u8>>,
    next_frame: u64,
    /// Live scratch rows; the frame opening a loan owns its ID.
    scratch: HashMap<u64, scratch::Scratch>,
    closing: Option<TransactionLifecycle>,
    /// Implicit container notices waiting for this quick move's source settlement.
    quick_move_notices: Option<Vec<QuickMoveNotice>>,
    pub(super) menu_publications: HashMap<u32, super::lan_menus::MenuPublications>,
    /// How many writes the journals have been asked to keep, ever: a whole-state epoch that no
    /// restore moves back. A decision that read a player's inventory or a container's slots
    /// compares it before and after its listeners ran; an equal rewrite counts as a write.
    writes: u64,
    /// The same count for a player's own state only (inventory, cursor, menu storage): what a
    /// decision that must stay open to other writes (a mod's move into another block) reads.
    player_writes: u64,
    /// The root commits telling their joins and leaves, outermost first: a listener's command in
    /// another level can commit a transaction there while this level's commit is told.
    committing: Vec<entities::Committing>,
    composter_epoch: Option<mod_api::WorldEpoch>,
    /// Each composter port owns its transactional random stream, independently of Level.random.
    composter_randoms: ComposterRandoms,
    /// Active retained original-player attachment maps during `player_clone` dispatch,
    /// kept in sync with open transaction frames so mutations roll back on abort.
    retained_original_attachments:
        Vec<std::sync::Arc<std::sync::Mutex<super::original_player::OriginalAttachmentSlot>>>,
    /// Negative staged-open identity counter (`i32::MIN..=-1`), separate from live protocol ids.
    staged_menu_open: i32,
    #[cfg(test)]
    pub(in crate::game) trace: Option<std::sync::Arc<std::sync::Mutex<Vec<String>>>>,
}

impl OpenTransactions {
    fn next_frame(&mut self) -> TransactionFrame {
        let tag = self.tag.get_or_insert_with(|| Box::new(0));
        let journal = std::ptr::from_ref(&**tag) as usize;
        self.next_frame = self
            .next_frame
            .checked_add(1)
            .expect("transaction frame sequence exhausted");
        TransactionFrame::new(journal, self.next_frame, self.levels.len())
    }

    pub(super) fn push_retained_original(
        &mut self,
        slot: std::sync::Arc<std::sync::Mutex<super::original_player::OriginalAttachmentSlot>>,
    ) {
        self.retained_original_attachments.push(slot);
    }

    pub(super) fn pop_retained_original(&mut self) {
        self.retained_original_attachments.pop();
    }

    fn close_retained_originals(&self, frame: Option<TransactionFrame>, commit: bool) {
        if let Some(frame) = frame {
            for slot in &self.retained_original_attachments {
                slot.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .close_frame(frame, commit);
            }
        }
    }

    pub(super) fn level_frame(&mut self, dim: super::Dim) -> Option<&mut Level> {
        entities::open_in(self, dim)
    }

    fn insert_scratch(&mut self, entry: scratch::Scratch) -> mod_api::ScratchSlot {
        let slot = mod_api::ScratchSlot::fresh();
        self.scratch.insert(slot.id(), entry);
        slot
    }

    fn player_write(&mut self) -> Option<&mut Level> {
        self.player_writes = self.player_writes.wrapping_add(1);
        self.writes = self.writes.wrapping_add(1);
        self.last_mut()
    }

    pub(super) fn journal_modifier_changes(&mut self, dim: Dim, id: u32, changes: ModifierChanges) {
        if let Some(level) = self.last_mut() {
            level
                .modifier_changes
                .entry((dim, id))
                .or_default()
                .merge(changes);
        }
    }

    pub(super) fn keep_explicit_modifier_write(
        &mut self,
        dim: Dim,
        mob: u32,
        attribute: &str,
        id: &str,
    ) {
        for level in self.iter_mut() {
            if let Some(changes) = level.modifier_changes.get_mut(&(dim, mob)) {
                changes.keep_explicit_write(attribute, id);
            }
            for (_, kept) in level
                .effect_modifiers
                .iter_mut()
                .filter(|(owner, _)| *owner == (dim, mob))
            {
                kept.keep_explicit_write(attribute, id);
            }
        }
    }

    /// Keep a resident row just before its first write, without relinquishing the loan.
    pub(in crate::game) fn journal_block_entity_loan(
        &mut self,
        dim: super::Dim,
        pos: Pos,
        loan: &sunrise_client::block_entities::BlockEntityLoan<'_>,
        chunks: &sunrise_client::chunk_map::ChunkMap,
        name: Option<&sunrise_core::nbt::Tag>,
        hopper: Option<HopperState>,
    ) {
        self.writes = self.writes.wrapping_add(1);
        if let Some(level) = entities::open_in(self, dim) {
            level.block_entities.entry(pos).or_insert_with(|| Kept {
                entity: Some(BlockEntity::Mod(Box::new(loan.entity.clone()))),
                client_update_fields: loan.entity.snapshot_client_update_fields(),
                taken: None,
                generation: Some(loan.identity.generation),
                attachments: loan.attachments.cloned(),
                hopper,
                name: name.cloned(),
                extras: BlockEntityExtras::capture(chunks, pos),
                contents_watches: loan.contents_watches.snapshot(loan.identity),
            });
        }
    }

    pub(super) fn mark_scoped_menu_write(
        &mut self,
        dim: Dim,
        chunk: (i32, i32),
        modified: &mut ModifiedChunks,
    ) {
        if let Some(level) = entities::open_in(self, dim) {
            level
                .save_chunks
                .insert((dim, chunk, SaveChunkScope::Level));
        } else {
            modified.insert(chunk);
        }
    }

    /// Keep the first write to each field of a live general-entity subject.
    pub(super) fn journal_general_entity(
        &mut self,
        join: crate::join_seq::JoinSeq,
    ) -> Option<&mut KeptGeneralEntity> {
        Some(self.last_mut()?.general_entities.entry(join).or_default())
    }

    /// The journaled-write epoch.
    pub(in crate::game) fn writes(&self) -> u64 {
        self.writes
    }
    /// The journaled-write epoch of players' own state.
    pub(in crate::game) fn player_writes(&self) -> u64 {
        self.player_writes
    }
    /// Allocates a negative staged-open identity that never overlaps live protocol window
    /// ids (`0` for inventory, `1..=100` for `ServerPlayer.nextContainerCounter`).
    pub(in crate::game) fn next_staged_menu_open(&mut self) -> i32 {
        self.staged_menu_open = match self.staged_menu_open.checked_sub(1) {
            Some(next) if next < 0 => next,
            _ => -1,
        };
        self.staged_menu_open
    }
}

impl std::ops::Deref for OpenTransactions {
    type Target = Vec<Level>;
    fn deref(&self) -> &Vec<Level> {
        &self.levels
    }
}

impl std::ops::DerefMut for OpenTransactions {
    fn deref_mut(&mut self) -> &mut Vec<Level> {
        &mut self.levels
    }
}

/// Puts the closing phase back when a close ends, however it ends.
struct ClosingPhase<'a, 'g> {
    game: &'a mut Game<'g>,
    outer: Option<TransactionLifecycle>,
}

impl Drop for ClosingPhase<'_, '_> {
    fn drop(&mut self) {
        self.game.transactions.closing = self.outer;
    }
}

struct KeptBlock {
    state: BlockStateId,
    replay: Option<BlockReplay>,
    strict: bool,
}

pub(super) type RemovedRow = (Box<BlockEntity>, u64);

pub(super) fn take_replay_row(world: &mut World, pos: Pos) -> Option<RemovedRow> {
    let generation = world.block_entities.generation(&pos);
    world
        .block_entities
        .remove(&pos)
        .zip(generation)
        .map(|(row, generation)| (Box::new(row), generation))
}

/// The block-entity data `BlockItem.place` can set before `setPlacedBy`, kept
/// beside rather than in the live model. Storage-only writes don't publish it.
struct PlacementData {
    name: Option<sunrise_core::nbt::Tag>,
    extras: sunrise_client::world::BlockEntityExtras,
}

#[derive(Default)]
pub(super) struct ReplayEntity {
    row: Option<RemovedRow>,
    data: Option<Box<PlacementData>>,
}

impl ReplayEntity {
    pub(super) fn take(world: &mut World, pos: Pos) -> Self {
        let name = world.block_names.get(&pos).cloned();
        let extras = world.block_entity_extras(pos);
        let data = (name.is_some() || extras.source.is_some() || extras.pending_loot.is_some())
            .then(|| Box::new(PlacementData { name, extras }));
        Self {
            row: take_replay_row(world, pos),
            data,
        }
    }

    pub(super) fn install(self, world: &mut World, pos: Pos, placement: bool) {
        if world.block_entities.get(&pos).is_some() {
            return;
        }
        if placement {
            if let Some(data) = self.data {
                world.set_block_entity_extras(pos, data.extras);
                if let Some(name) = data.name {
                    world.block_names.insert(pos, name);
                } else {
                    world.block_names.remove(&pos);
                }
            } else {
                world.forget_block_entity_source(pos);
                world.block_names.remove(&pos);
            }
        }
        if let Some((entity, generation)) = self.row {
            let entities = &mut world.block_entities;
            entities.insert(pos, *entity);
            entities.restore_generation(pos, generation);
            entities.reattach_retired(pos, generation);
        }
    }
}

pub(super) enum BlockEntityCreation {
    Live,
    /// A later write owns the factory result for this same resident lifetime.
    Deferred,
    /// A factory was evaluated while staging; no generation records an explicit absence.
    Staged {
        generation: Option<u64>,
        entity: ReplayEntity,
    },
}

pub(super) struct BlockStep {
    pos: Pos,
    to: BlockStateId,
    replay: BlockReplay,
    placement: bool,
    // Some means staging took the resident row out (even if it was already absent).
    removed: Option<ReplayEntity>,
    creation: BlockEntityCreation,
}

pub(super) struct StrictBlockStep {
    pub(super) to: BlockStateId,
    // Before the first replacement on this cell; swapped for the new row at commit.
    pub(super) row: Option<ReplayEntity>,
    /// A strict replacement ends the old block's connection-group lifetime.
    pub(super) clear_connection_group: bool,
}

pub(super) type StrictBlockSteps = indexmap::IndexMap<Pos, StrictBlockStep>;

#[derive(Clone, Copy)]
struct BlockReplay {
    state: BlockStateId,
    flags: i32,
    /// Replay LevelWriteHost writes through the direct host's block-entity creation path.
    level_host: bool,
    /// A level-host write only hid the entity from other reads. At commit
    /// the real Level.setBlock still has to run its removal side effects.
    replay_removal: bool,
    /// Any tentative write at this position replaced the block with one that does not keep
    /// connection groups (`!Behaviours::keeps_block_entity`), even if a later write returned
    /// it to a compatible block.
    clear_connection_group: bool,
}

/// Preserve requested updates, and suppress an effect only if every write
/// suppressed it. Other bits describe the final write (e.g. moved by piston).
fn merge_block_update_flags(earlier: i32, later: i32) -> i32 {
    const REQUESTED: i32 =
        flags::UPDATE_NEIGHBORS | flags::UPDATE_CLIENTS | flags::UPDATE_IMMEDIATE;
    const SUPPRESSED: i32 = flags::UPDATE_INVISIBLE
        | flags::UPDATE_KNOWN_SHAPE
        | flags::UPDATE_SUPPRESS_DROPS
        | flags::UPDATE_SKIP_SHAPE_UPDATE_ON_WIRE
        | flags::UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS
        | flags::UPDATE_SKIP_ON_PLACE;
    ((earlier | later) & REQUESTED)
        | ((earlier & later) & SUPPRESSED)
        | (later & !(REQUESTED | SUPPRESSED))
}

/// Before-images of the fields a general-entity scope wrote, at one depth.
#[derive(Default)]
pub(super) struct KeptGeneralEntity {
    pub(super) velocity: Option<Vec3>,
    pub(super) rotation: Option<[f32; 2]>,
    pub(super) fall_distance: Option<f64>,
}

/// One journal, either an open frame or a level-local batch split from one at close.
#[derive(Default)]
pub(super) struct Level {
    frame: Option<TransactionFrame>,
    parent_attachment_query_scope: u64,
    dimension: Option<super::Dim>,
    /// Each block entity as it was before this transaction first wrote it.
    block_entities: HashMap<Pos, Kept>,
    /// Rows moved out of storage when their block was tentatively replaced. Commit replays
    /// removal with the row as it was then; abort uses the first write in `block_entities`.
    /// A removal without a resident row still marks the position for abort.
    removed_block_entities: HashMap<Pos, Option<RemovedRow>>,
    /// A decorated pot lives in its chunk's saved block-entity tags, not the slot table.
    saved_pots: HashMap<Pos, Option<sunrise_core::nbt::Tag>>,
    /// Each item slot (block entity, slot) a slot-only write changed, as it was before this
    /// transaction first wrote it, for a block entity not kept whole in `block_entities`: a
    /// move through a container keeps the slot it touches, not the entity's whole saved data.
    slots: HashMap<(Pos, usize), KeptSlot>,
    /// Each block's original state and the combined update flags of its writes.
    blocks: HashMap<Pos, KeptBlock>,
    /// The writes of a chosen placement before its veto are visible in this savepoint.
    capture_placement: bool,
    /// Each cell's original connection group before this transaction first changed it, used for
    /// silent rollback on abort and pre-replay restoration on root commit.
    connection_groups: HashMap<Pos, Option<u64>>,
    /// Level.random in this frame's own dimension before its first draw.
    level_random: Option<LegacyRandom>,
    /// Other levels drawn from at this depth. Only those draws allocate a map entry.
    parked_level_randoms: HashMap<LevelRef, LegacyRandom>,
    /// Named loot streams before tentative block loot.
    loot_sequences: Option<sunrise_items::random_sequences::RandomSequences>,
    /// Composter-local random streams before this depth first evaluated their layers.
    composter_randoms: ComposterRandoms,
    /// Each quart cell before this transaction first wrote it, keyed by level.
    biomes: HashMap<(Dim, Pos), u16>,
    /// Each player's inventory and saved data before this transaction first wrote them.
    players: HashMap<JournaledPlayer, KeptPlayer>,
    /// Ender storage has one journal shared by player and menu writes.
    ender_items: HashMap<JournaledPlayer, Vec<Option<ItemStack>>>,
    /// Saved player extra NBT (`level.player_extra` / `guest.saved`), shared by player and
    /// persistent-data writes, including dead players.
    player_extra: HashMap<JournaledPlayer, Option<sunrise_core::nbt::Tag>>,
    /// Each affected custom book's saved flags (or absence), in first-write order.
    player_books:
        indexmap::IndexMap<(JournaledPlayer, RecipeBookTypeKey), (PlayerRef, Option<BookFlags>)>,
    /// Synced player health and its unpublished update events before set_health, by player body.
    player_health: HashMap<PlayerBodyKey, KeptPlayerHealth>,
    /// Mob health and synchronization dirtiness before set_health, keyed by level.
    mob_health: HashMap<(Dim, u32), KeptMobHealth>,
    /// Active and hidden effects before the first write to the mob at this depth, keyed by level.
    mob_effects: HashMap<(Dim, u32), KeptMobEffects>,
    /// Effect-owned modifier changes in write order, so rows can return to their positions.
    effect_modifiers: Vec<((Dim, u32), EffectModifierUndo)>,
    /// Each entity's inventory (`entity_slots`) before this transaction first wrote it, by level.
    entities: HashMap<(Dim, u32), Vec<Option<ItemStack>>>,
    /// Each mob's or armor stand's equipment before this transaction first
    /// wrote it (#932), told apart for the way back and keyed by level.
    equipment: HashMap<(Dim, u32), KeptEquipment>,
    /// Live modifier diffs, also kept for rebuilds that write no slot, keyed by level.
    modifier_changes: HashMap<(Dim, u32), ModifierChanges>,
    /// Each chested mob's item list's length before `entity_slots` first grew it to its slots
    /// in this transaction, keyed by level.
    mob_lengths: HashMap<(Dim, u32), usize>,
    /// Each item entity's changed fields before this transaction first wrote them.
    item_entities: HashMap<crate::join_seq::JoinSeq, KeptItemEntity>,
    /// Each general entity subject's changed fields before the first write.
    general_entities: HashMap<crate::join_seq::JoinSeq, KeptGeneralEntity>,
    /// The merge fields of resident experience orbs before the first award touched them, by level.
    orbs: HashMap<(Dim, crate::join_seq::JoinSeq), (i32, u32, i32)>,
    /// Each detached station before the first click wrote it, by connection and owner.
    stations: HashMap<(u32, decision::MenuId), super::lan_stations::GuestStation>,
    /// Each guest's inventory crafting grid before the first write, by connection.
    menus: HashMap<u32, KeptMenus>,
    /// Drag, machine and preview state before a scoped default click.
    menu_clicks: HashMap<u64, KeptClick>,
    /// The first drag state written in a guest's mount window.
    mount_drags: HashMap<MountClickWindow, QuickCraftDrag>,
    /// Mount window identities to reconcile once the outermost transaction has settled.
    mount_replies: indexmap::IndexSet<MountClickWindow>,
    pub(super) menu_click_active: Option<u32>,
    /// Each native window's slots before the first write, by menu generation.
    native_menu_slots: HashMap<u64, Vec<Option<ItemStack>>>,
    menu_data: MenuDataJournal,
    /// The scoreboard before this transaction first wrote it (`journal_scoreboard`), whole.
    board: Option<sunrise_client::scoreboard::Scoreboard>,
    /// Each entity's, player's, chunk's and the level's attachment map before this transaction
    /// first reached it (a block entity's is kept with it). Per-level holders are keyed by level;
    /// player-owned attachment maps are keyed by player body across dimensions.
    attachments: HashMap<AttachmentKey, KeptAttachments>,
    /// Held once this transaction reaches an attachment map, until it closes: the save
    /// admission gate (`passes_settled`) refuses a save while a map may hold tentative
    /// values, which only the root commit makes saveable.
    save_gate: Option<sunrise_client::join_seq::visibility::PassLease>,
    /// Strict edits and saved pots hold the save gate with their owning level's journal.
    level_save_gate: Option<sunrise_client::join_seq::visibility::PassLease>,
    /// Each chunk's unsaved mark and save queue entry before this transaction first reached an
    /// attachment map in it: an abort puts them back, so a chunk only a tentative write marked
    /// is clean again.
    chunk_marks: HashMap<(i32, i32), sunrise_client::world::ChunkMark>,
    /// Chunks whose committed storage writes must be saved, in first-write order and in the
    /// levels written.
    save_chunks: indexmap::IndexSet<(Dim, (i32, i32), SaveChunkScope)>,
    /// New map data and its reserved IDs. Neither enters the saved map table until root commit.
    pub(super) new_maps: BTreeMap<i32, MapData>,
    /// What the root commit does, in order.
    after: Vec<After>,
    /// Internal decide/fire/apply state for menu actions.
    actions: decision::Journal,
    /// Merchant takes keep their session, mob, statistics and criterion progress together.
    pub(in crate::game) trades: super::lan_trade_decisions::Journal,
    pub(in crate::game) packets: Option<sunrise_client::lan::conn::transaction_packets::Scope>,
    captured_packets: Vec<sunrise_client::lan::conn::transaction_packets::Packets>,
    action: Option<decision::ActiveAction>,
    /// A scratch slot owned by this frame, and before-images of ancestor loans it wrote.
    owned_scratch: Option<u64>,
    scratch_kept: HashMap<u64, scratch::Scratch>,
    /// The entities it added and removed, and the announcements it holds.
    lifetimes: entities::EntityJournal,
    /// Each attachment map that went with an entity to another level after this transaction
    /// first reached it, by the id it arrived under, as it was then.
    travelled: HashMap<u32, KeptAttachments>,
    /// The index rows of the entities that left this level, kept until the root closes.
    departed: Vec<entities::Departure>,
    /// The entities taken for a transfer whose arrival is not adopted yet, by source id, each
    /// with the slot its arrival reads an abort's way back from.
    in_flight: HashMap<u32, entities::ArrivalRollback>,
}

/// A slot's stack before a transaction first wrote it, and the generation of the block entity
/// it belongs to: an abort puts it back only into that same block entity, never into one that
/// replaced it.
struct KeptSlot {
    stack: Option<ItemStack>,
    generation: Option<u64>,
}

/// Identity of one player body for transaction rollback: either the local host player
/// (`lan_acting.is_none()`) or one guest record in `lan.hub.guests` (a real online guest shared
/// across dimensions, or a per-level connectionless fake player), keyed by its unique `conn` id.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum PlayerBodyKey {
    Host,
    Guest(u32),
}

/// Key for `Level::attachments`: per-level holders (`Entity`, `Chunk`, `Level`) are isolated by
/// dimension, whereas player-owned attachment maps are keyed by the specific player body
/// regardless of which level is swapped in. Retained originals have their own journal.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum AttachmentKey {
    Level(Dim, Holder),
    Player(PlayerBodyKey),
}

struct KeptAttachments {
    contents: Attachments,
    indexed: bool,
}

struct Kept {
    entity: Option<BlockEntity>,
    /// The pre-write batch, stored apart from the entity clone's fresh identity.
    client_update_fields: Option<sunrise_core::nbt::Compound>,
    taken: Option<sunrise_client::block_entities::TakenLeaseSnapshot>,
    generation: Option<u64>,
    /// None where it had no map: an abort removes one a write made.
    attachments: Option<Attachments>,
    hopper: Option<HopperState>,
    /// The custom name Sunrise keeps beside the model (`World::block_names`).
    name: Option<sunrise_core::nbt::Tag>,
    /// The container lock and loot table roll kept beside the model.
    extras: BlockEntityExtras,
    contents_watches: Vec<(Pos, u64)>,
}

impl Kept {
    fn capture(
        world: &sunrise_client::world::World,
        hoppers: &HashMap<Pos, HopperState>,
        pos: Pos,
    ) -> Self {
        let entity = world.block_entities.get(&pos);
        Self {
            entity: entity.cloned(),
            taken: world.block_entities.taken_lease_snapshot(&pos),
            client_update_fields: match entity {
                Some(BlockEntity::Mod(entity)) => entity.snapshot_client_update_fields(),
                _ => None,
            },
            generation: world.block_entities.generation(&pos),
            attachments: world.block_entities.attachments(pos).cloned(),
            hopper: hoppers.get(&pos).copied(),
            name: world.block_names.get(&pos).cloned(),
            extras: world.block_entity_extras(pos),
            contents_watches: world
                .block_entities
                .identity(pos)
                .map_or_else(Vec::new, |observer| {
                    world.block_entities.contents_watches.snapshot(observer)
                }),
        }
    }
}

struct KeptPlayerHealth {
    data: sunrise_world::synced_floats::SyncedFloats,
    events_len: usize,
    dead: bool,
    guest: Option<(u32, bool, u32)>,
}

struct KeptMobHealth {
    absorption: f32,
    health: f32,
    needs_sync: bool,
    death_time: i32,
    death_canceled: bool,
    processed: bool,
}

struct KeptMobEffects {
    active: Vec<StoredEffect>,
    chains: Vec<EffectInstance>,
    ambient: Vec<&'static str>,
    hidden: Vec<&'static str>,
    dirty: bool,
}

/// Storage identity, not a public player handle. Fake players in different levels
/// can share a UUID; their hub connections still distinguish their records.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum JournaledPlayer {
    Local(u128),
    Guest(u32),
}

enum PlayerStorageHome {
    Game,
    Guest(usize),
}

#[derive(Clone)]
struct KeptPlayer {
    inventory: Vec<Option<ItemStack>>,
    armor: [Option<ItemStack>; 4],
    offhand: Option<ItemStack>,
    equipment: Equipment,
    carried: Option<ItemStack>,
}

/// The raw slots before the first write, with a mob's separate gear overlay.
/// Projecting the slots here would bake the overlay into the resident stacks.
enum KeptEquipment {
    Mob {
        slots: Box<Equipment>,
        gear: Option<Box<Gear>>,
    },
    Stand(Box<Equipment>),
}

struct KeptClick {
    conn: u32,
    drag: Option<(u8, Vec<usize>)>,
    craft: Option<sunrise_client::lan::menu::ModCraft>,
    slot_changed_seen: Option<Vec<Option<ItemStack>>>,
    owned: Option<(u64, Box<sunrise_client::menu::Menu>)>,
    machine: Option<(i32, Box<sunrise_client::menu::Menu>)>,
    places: Option<super::native_menu_places::MenuSlotPlaces>,
}

#[derive(Clone)]
struct KeptMenus {
    craft_grid: [Option<ItemStack>; 4],
}

/// Deferred `LevelChunk.removeBlockEntity` / `BlockEntity.setRemoved` effects
/// for an explicit `BlockEntityRef::remove`.
#[derive(Clone, Copy)]
pub(super) struct RemovedBlockEntityEffects {
    pub(super) pos: Pos,
    pub(super) state: BlockStateId,
    pub(super) sculk: bool,
    pub(super) jukebox: bool,
    pub(super) beacon: bool,
    pub(super) gateway: bool,
    pub(super) had_shapes: bool,
}

/// What a write tells the world, at the root commit.
pub(super) enum After {
    /// Deferred `LevelChunk.removeBlockEntity` / `BlockEntity.setRemoved` effects
    /// (listener unregister, jukebox stop, beacon sound, gateway cleanup, shape updates).
    BlockEntityRemoved(RemovedBlockEntityEffects),
    /// Reserved raw bytes publish only when their connection incarnation is still live.
    Payload(super::payload_transport::Deferred),
    /// A mod's validated vanilla packet, bound to its physical protocol incarnation,
    /// and the submitting identity for publication errors.
    Packet(sunrise_client::net::PreparedPacket, Option<&'static str>),
    Composter(super::composter::Effects),
    DispenseEvent(i32, Pos, i32),
    /// A mod's world pickup or placement of fluid, its block already set: what is left of it
    /// (`Game::commit_fluid_write`).
    FluidWrite(Box<super::world_fluids::WrittenFluid>),
    /// A bucket's sound and fluid game event (`level_play_fluid_effects`).
    FluidEffects(Box<super::world_fluids::FluidEffects>),
    /// A dispenser drop in the source level, with optional overflow click and smoke.
    DispenserDrop {
        pos: Pos,
        facing: sunrise_core::geometry::Direction,
        spawn: Box<super::spill::Spawn>,
        play_events: bool,
    },
    /// Sound and smoke for a mod's accepted dispense reply, after its behaviour's effects.
    DispenserFeedback {
        pos: Pos,
        facing: sunrise_core::geometry::Direction,
        event: i32,
    },
    /// `BlockEntity.setChanged`.
    Changed(Pos),
    /// `BlockEntity.setChanged` dirty/comparator updates without reentering `contents_changed`.
    ChangedWithoutHook(Pos),
    /// A notice for a specific resident row, not a replacement at the same position.
    ChangedHolder {
        holder: BlockEntityIdentity,
        silent: bool,
    },
    GuestMenu(u32, Box<super::lan_crafting::GuestMenuEffects>),
    /// Publish both the connection-local state and its packets together, never on abort.
    PersonalSidebar(
        u32,
        Option<Box<sunrise_client::lan::scoreboard::SidebarContents>>,
    ),
    /// Equipment transitions from a mount menu, with distinct sound and game event positions.
    MountEquipment {
        dim: Dim,
        id: u32,
        at: [f64; 3],
        sound: Option<(String, [f64; 3])>,
        equips: bool,
    },
    MenuClick(Box<super::lan_menus::MenuClickReply>),
    MenuOpen(u32, i32, Box<super::native_menus::PendingMenuOpen>),
    MenuPlacesNotice(u32, u128, i32),
    FurnaceMenu(Box<super::lan_crafting::FurnaceMenuEffects>),
    StoreCartItems(Pos),
    BackedMenuClick(u32, u128, mod_api::MenuRef, i32),
    /// A provider's answer changed here (`level_invalidate_capabilities`): the capability
    /// generation moves on.
    Capabilities(Pos),

    /// `ServerLevel.destroyBlockProgress`: breaker id, position, stage. Sent once, at the root commit.
    BreakProgress(i32, [i32; 3], i32),
    /// Replay a block write (coalesced unless the cell has placement capture or a strict edit).
    Block(BlockStep),
    /// Publish the storage and watched writes of a strict bulk edit, not ordinary updates.
    BulkBlocks(
        sunrise_client::block_edit::BulkPublication,
        Vec<super::block_edits::Tracked>,
        Vec<[i32; 3]>,
        StrictBlockSteps,
    ),
    /// An explicit `LevelRef::set_connection_group` write, replayed in order with block writes at
    /// the root commit so cross-position cascades (e.g. popping a supported block) clear it.
    ConnectionGroup(Pos, Option<u64>),
    /// A validated level write host effect; never published on abort.
    LevelWrite(Box<super::journal_level_write::PendingWrite>),
    /// `Block.setPlacedBy` effects after its writes have been accepted.
    PlacementEffect(Box<sunrise_world::level::ticks::PlacementEffect>),
    WitherPattern(sunrise_client::block_pattern::PatternMatch, Vec<[i32; 3]>),
    /// Publish an update only if the staged block entity is still resident.
    UpdatePlacedBlockEntity(Pos, u64),
    BlockGameEvent(
        &'static str,
        [i32; 3],
        Option<sunrise_client::mobs::Who>,
        BlockStateId,
    ),
    DestroyBlockGameEvent {
        pos: [i32; 3],
        state: BlockStateId,
        breaker: Option<mod_api::EntityRef>,
    },
    BlockDropSpawns(Vec<super::spill::Spawn>, Vec<([f64; 3], u32, i32)>),
    /// Save, remesh and resend a changed biome column.
    Biome((i32, i32)),
    /// Deferred interaction effects, replayed in the same root-commit order.
    Effect(DeferredEffect),
    /// Effects of a tentative merchant take, published only at the root.
    Trade(Box<super::lan_trade_decisions::Effect>),
    /// `Block.popResource`.
    Drop([i32; 3], ItemStack),
    /// `Player.drop(stack, false)` by the player with this UUID: what no longer fits.
    PlayerDrop(u128, ItemStack),
    /// A respawn replaces the holder behind the player's UUID. Do not install a new
    /// attachment generation or copy tentative data until the old generation is committed.
    PlayerRespawn(u32),
    /// `/give`'s fake item and pickup sound in the receiving player's level.
    GivePickup {
        uuid: u128,
        dim: super::Dim,
        prototype: ItemStack,
    },
    InventoryTrigger(u128),
    GrindstoneEvent([i32; 3]),
    StationContent(u32, decision::MenuId, i32),
    /// What did not fit of a `/give` to the player `uuid`, thrown in the level `dim` that was in
    /// place when it was accepted; at `eye`, the player's eyes then, if they are dead or gone
    /// by the commit.
    GiveRest {
        uuid: u128,
        rest: ItemStack,
        owner: Option<[i32; 4]>,
        dim: super::Dim,
        eye: [f64; 3],
    },
    PlayerEquip(u128, String, Option<ItemStack>),
    /// The journal's original flags, for one publication of the current committed pair.
    RecipeBookSettings(
        JournaledPlayer,
        PlayerRef,
        RecipeBookTypeKey,
        Option<BookFlags>,
    ),
    /// An item split off an item entity, dropped where it is (`Block.popResource`'s delay).
    Spawn([f64; 3], ItemStack),
    /// The item entity with this id was emptied: removed, unless it holds items again.
    Emptied(u32),
    /// A component-only mob equipment write must reach the normal tracker even if the mob is idle.
    EquipmentSync(u32),
    /// `LivingEntity.onEquipItem`'s sound and equip/unequip game event, at the entity `who`
    /// is: what an equipment write through a transaction tells the world only at the root
    /// commit, so an abort stays quiet (#932).
    Equip {
        at: [f64; 3],
        sound: Option<String>,
        equips: bool,
        who: u32,
    },
}

impl After {
    fn level_owned(&self) -> bool {
        matches!(
            self,
            Self::Block(_)
                | Self::BulkBlocks(..)
                | Self::ConnectionGroup(..)
                | Self::DispenseEvent(..)
                | Self::Changed(_)
                | Self::ChangedWithoutHook(_)
                | Self::ChangedHolder { .. }
                | Self::Capabilities(_)
                | Self::LevelWrite(_)
                | Self::PlacementEffect(_)
                | Self::WitherPattern(..)
                | Self::UpdatePlacedBlockEntity(..)
                | Self::FluidWrite(_)
                | Self::FluidEffects(_)
                | Self::DispenserDrop { .. }
                | Self::DispenserFeedback { .. }
                | Self::BlockGameEvent(..)
                | Self::DestroyBlockGameEvent { .. }
                | Self::BlockDropSpawns(..)
                | Self::Biome(_)
                | Self::Drop(..)
                | Self::Spawn(..)
                | Self::Emptied(_)
        )
    }
}

/// Effects shared by transactional interaction contexts.
pub(super) enum DeferredEffect {
    CustomStat {
        player: u128,
        key: &'static str,
        amount: i32,
    },
    /// `Player.awardStat` of an item statistic (`used`, `crafted`...) for `item`.
    ItemStat {
        player: u128,
        stat: &'static str,
        item: std::sync::Arc<str>,
        amount: i32,
    },
    Sound {
        sound: &'static str,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
    },
    /// `PlayerRef::set_statistics` inside a transaction: the counters and their criterion
    /// scores are replaced together at the root commit.
    RestoreStatistics(mod_api::PlayerRef, Box<sunrise_gameplay::stats::StatValues>),
    /// `PlayerRef::award_advancement` / `revoke_advancement` inside a transaction:
    /// applied at the root commit so an abort rolls back progress, rewards and hooks.
    ChangeAdvancement {
        player: mod_api::PlayerRef,
        id: String,
        criterion: Option<String>,
        grant: bool,
    },
}

impl Level {
    /// Keep the first snapshot for this stream. `None` names the frame's own level.
    pub(super) fn keep_level_random(
        &mut self,
        owner: Option<LevelRef>,
        before: impl FnOnce() -> LegacyRandom,
    ) {
        match owner {
            Some(level) if self.dimension.map(super::Dim::key) != Some(level.dimension()) => {
                self.parked_level_randoms
                    .entry(level)
                    .or_insert_with(before);
            }
            _ => {
                self.level_random.get_or_insert_with(before);
            }
        }
    }

    pub(super) fn keep_bulk_block(&mut self, pos: Pos, state: u32) {
        self.level_save_gate
            .get_or_insert_with(sunrise_client::join_seq::visibility::PassLease::enter);
        self.blocks
            .entry(pos)
            .and_modify(|kept| kept.strict = true)
            .or_insert(KeptBlock {
                state: BlockStateId(state),
                replay: None,
                strict: true,
            });
    }

    pub(super) fn keep_connection_group(&mut self, pos: Pos, group: Option<u64>) {
        self.connection_groups.entry(pos).or_insert(group);
    }

    fn merge_level_writes(&mut self, child: &mut Self) {
        self.level_save_gate = self.level_save_gate.take().or(child.level_save_gate.take());
        for (pos, biome) in std::mem::take(&mut child.biomes) {
            self.biomes.entry(pos).or_insert(biome);
        }
        for (pos, group) in std::mem::take(&mut child.connection_groups) {
            self.keep_connection_group(pos, group);
        }
        // Slots before whole entities: a slot the outer transaction already kept the
        // whole entity of is older there, but one kept whole only by this child is not,
        // so the child's slot entries go up beside its capture.
        for ((pos, slot), kept) in std::mem::take(&mut child.slots) {
            if !self.block_entities.contains_key(&pos) {
                self.slots.entry((pos, slot)).or_insert(kept);
            }
        }
        for (pos, kept) in std::mem::take(&mut child.block_entities) {
            self.block_entities.entry(pos).or_insert(kept);
        }
        for (pos, tag) in std::mem::take(&mut child.saved_pots) {
            self.saved_pots.entry(pos).or_insert(tag);
        }
        for (pos, removed) in std::mem::take(&mut child.removed_block_entities) {
            let first = self.removed_block_entities.entry(pos).or_default();
            if first.is_none() {
                *first = removed;
            }
        }
        for (pos, block) in std::mem::take(&mut child.blocks) {
            self.blocks
                .entry(pos)
                .and_modify(|kept| {
                    kept.strict |= block.strict;
                    if let Some(replay) = block.replay {
                        if let Some(earlier) = &mut kept.replay {
                            earlier.flags = merge_block_update_flags(earlier.flags, replay.flags);
                            earlier.level_host = replay.level_host;
                            earlier.replay_removal |= replay.replay_removal;
                            earlier.clear_connection_group |= replay.clear_connection_group;
                        } else {
                            kept.replay = Some(replay);
                        }
                    }
                })
                .or_insert(block);
        }
        for (id, kept) in std::mem::take(&mut child.item_entities) {
            self.item_entities.entry(id).or_default().merge(kept);
        }
        for (join, kept) in std::mem::take(&mut child.general_entities) {
            let first = self.general_entities.entry(join).or_default();
            first.velocity = first.velocity.or(kept.velocity);
            first.rotation = first.rotation.or(kept.rotation);
            first.fall_distance = first.fall_distance.or(kept.fall_distance);
        }
        for (chunk, mark) in std::mem::take(&mut child.chunk_marks) {
            self.chunk_marks.entry(chunk).or_insert(mark);
        }
    }

    fn keep_player(
        &mut self,
        owner: JournaledPlayer,
        ender_items: &[Option<ItemStack>],
        player_extra: Option<&sunrise_core::nbt::Tag>,
        capture: impl FnOnce() -> KeptPlayer,
    ) {
        self.ender_items
            .entry(owner)
            .or_insert_with(|| ender_items.to_vec());
        self.player_extra
            .entry(owner)
            .or_insert_with(|| player_extra.cloned());
        self.players.entry(owner).or_insert_with(capture);
    }

    /// Keeps chested mob `id`'s item list's length, `len`, before `entity_slots` grows it, if
    /// this transaction hasn't yet.
    pub(super) fn keep_mob_length(&mut self, dim: Dim, id: u32, len: usize) {
        self.mob_lengths.entry((dim, id)).or_insert(len);
    }

    /// Keep a resolved equipment row at its first write in this transaction.
    /// `mob_gear` is absent for a stand; a mob's overlay may itself be absent.
    pub(super) fn keep_equipment(
        &mut self,
        dim: Dim,
        id: u32,
        equipment: &Equipment,
        mob_gear: Option<&Option<Box<Gear>>>,
    ) {
        self.equipment.entry((dim, id)).or_insert_with(|| {
            let slots = Box::new(equipment.clone());
            match mob_gear {
                Some(gear) => KeptEquipment::Mob {
                    slots,
                    gear: gear.clone(),
                },
                None => KeptEquipment::Stand(slots),
            }
        });
    }
}

pub(super) enum StorageSettlement {
    Committed(Option<scratch::Scratch>),
    Aborted,
    Refused,
}

/// RAII guard for one open transaction frame. Dropping it without `close` aborts this frame
/// and any nested frames still open inside it.
pub(super) struct StorageScope<'a, 'g> {
    pub(super) game: &'a mut Game<'g>,
    frame: Option<TransactionFrame>,
}

impl StorageScope<'_, '_> {
    pub(super) fn close(mut self, commit: bool) -> Option<StorageSettlement> {
        self.frame
            .take()
            .and_then(|frame| self.game.close_storage_frame(frame, commit))
    }
}

impl Drop for StorageScope<'_, '_> {
    fn drop(&mut self) {
        if let Some(frame) = self.frame.take() {
            let _ = self.game.close_storage_frame(frame, false);
        }
    }
}

impl<'g> Game<'g> {
    /// Actual transaction/restore events for storage-attempt regression traces.
    #[cfg(test)]
    pub(in crate::game) fn trace_storage_attempt(&self, entry: String) {
        if let Some(trace) = &self.transactions.trace {
            trace.lock().unwrap().push(entry);
        }
    }
    /// Pending payloads in the innermost transaction (before publication).
    #[cfg(test)]
    pub(in crate::game) fn pending_transaction_payloads(&self) -> usize {
        self.transactions
            .last()
            .unwrap()
            .after
            .iter()
            .filter(|after| matches!(after, After::Payload(_)))
            .count()
    }
    /// Pending item-stat effects in the innermost transaction (before publication).
    #[cfg(test)]
    pub(in crate::game) fn pending_transaction_item_stats(&self) -> usize {
        self.transactions
            .last()
            .unwrap()
            .after
            .iter()
            .filter(|after| matches!(after, After::Effect(DeferredEffect::ItemStat { .. })))
            .count()
    }

    /// Opens a transaction and gives back its frame identity.
    pub(in crate::game) fn open_storage_transaction(&mut self) -> TransactionFrame {
        mod_api::__private::game::CallbackOwner::call_host(|| {
            let frame = self.transactions.next_frame();
            if !self.world.block_entities.defers_retirement() {
                self.world.block_entities.defer_retirement();
            }
            let lifetimes = self.entity_journal_open();
            let parent_attachment_query_scope = self.transactions.attachment_query_scopes.enter();
            self.transactions.push(Level {
                frame: Some(frame),
                parent_attachment_query_scope,
                dimension: Some(self.dim),
                packets: sunrise_client::lan::conn::transaction_packets::Scope::nested(),
                lifetimes,
                ..Level::default()
            });
            for slot in &self.transactions.retained_original_attachments {
                slot.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .open_frame(frame);
            }
            #[cfg(test)]
            self.trace_storage_attempt("open".into());
            frame
        })
    }
    /// Closes `frame`, first aborting every transaction opened inside it and left open, so a
    /// scope closing its own is never stranded behind an abandoned child.
    pub(in crate::game) fn close_storage_frame(
        &mut self,
        frame: TransactionFrame,
        commit: bool,
    ) -> Option<StorageSettlement> {
        mod_api::__private::game::CallbackOwner::close_host_transaction(commit, |commit| {
            let at = self
                .transactions
                .iter()
                .rposition(|level| level.frame == Some(frame))?;
            if self.transactions.len() > at + 1 {
                log::warn!("Sunrise: aborted a transaction left open inside the one being closed");
            }
            while self.transactions.len() > at + 1 {
                let _ = self.close_storage_transaction(false);
            }
            self.close_storage_transaction(commit)
        })
    }
    /// An engine batch opens a frame only when it has a callback to journal.
    pub(super) fn storage_scope_guard(&mut self, open: bool) -> StorageScope<'_, 'g> {
        let frame = open.then(|| self.open_storage_transaction());
        StorageScope { game: self, frame }
    }

    /// The trident tick opens a frame at contact, then closes it after applying the
    /// decision. The same guard aborts an open frame if dispatch or apply unwinds.
    pub(super) fn with_late_storage_scope<R>(
        &mut self,
        f: impl FnOnce(&mut Self, &mut Option<TransactionFrame>) -> R,
    ) -> R {
        let mut scope = self.storage_scope_guard(false);
        f(&mut *scope.game, &mut scope.frame)
    }

    /// Runs `f` inside a new (or nested) storage transaction, committing iff `f` returns
    /// `(out, true)` and aborting on `false` or unwind.
    pub(in crate::game) fn storage_scope<R>(
        &mut self,
        f: impl FnOnce(&mut Self, TransactionFrame) -> (R, bool),
    ) -> R {
        self.run_storage_scope(f).0
    }

    /// Also returns the frame's own detached scratch row when it commits.
    fn run_storage_scope<R>(
        &mut self,
        f: impl FnOnce(&mut Self, TransactionFrame) -> (R, bool),
    ) -> (R, StorageSettlement) {
        let frame = self.open_storage_transaction();
        let guard = StorageScope {
            game: self,
            frame: Some(frame),
        };
        let (out, commit) = f(&mut *guard.game, frame);
        let settled = guard.close(commit).unwrap_or(if commit {
            StorageSettlement::Refused
        } else {
            StorageSettlement::Aborted
        });
        (out, settled)
    }
    /// Runs `f` inside a new (or nested) storage transaction, lending a scoped
    /// [`TransactionContext`]; commits on `Ok` and aborts on `Err` or unwind.
    pub(in crate::game) fn storage_transaction<T, E>(
        &mut self,
        f: impl for<'loan> FnOnce(&'loan mut Self, &'loan TransactionContext) -> Result<T, E>,
    ) -> Result<T, E> {
        self.storage_scope(|game, frame| {
            let outcome = frame.lend(|tx| f(game, tx));
            let commit = outcome.is_ok();
            (outcome, commit)
        })
    }
    /// Runs `f` inside a new (or nested) storage transaction and reports whether a requested
    /// commit was refused and rolled back.
    pub(in crate::game) fn storage_transaction_settled<T, E>(
        &mut self,
        f: impl for<'loan> FnOnce(&'loan mut Self, &'loan TransactionContext) -> Result<T, E>,
    ) -> Result<T, mod_api::TransactionError<E>> {
        let (outcome, settled) = self.run_storage_scope(|game, frame| {
            let outcome = frame.lend(|tx| f(game, tx));
            let commit = outcome.is_ok();
            (outcome, commit)
        });
        match (outcome, settled) {
            (Ok(value), StorageSettlement::Committed(_)) => Ok(value),
            (Ok(_), StorageSettlement::Refused | StorageSettlement::Aborted) => {
                Err(mod_api::TransactionError::CommitRefused)
            }
            (Err(error), _) => Err(mod_api::TransactionError::Aborted(error)),
        }
    }
    /// Runs an API scope and reports actual settlement, not just the callback's intent.
    pub(super) fn scoped_storage<T>(
        &mut self,
        f: impl for<'loan> FnOnce(&'loan mut Self, &'loan TransactionContext) -> Option<T>,
    ) -> Result<Option<T>, TransactionScopeError> {
        let (outcome, settled) = self.run_storage_scope(|game, frame| {
            let outcome = frame.lend(|tx| f(game, tx));
            let commit = outcome.is_some();
            (outcome, commit)
        });
        match settled {
            StorageSettlement::Committed(_) => Ok(outcome),
            StorageSettlement::Aborted => Ok(None),
            StorageSettlement::Refused => Err(TransactionScopeError::CommitRefused),
        }
    }
    /// Whether at least one storage transaction is currently open.
    pub(in crate::game) fn has_open_transaction(&self) -> bool {
        !self.transactions.is_empty()
    }
    /// The innermost open storage transaction's frame identity, if any.
    pub(in crate::game) fn current_storage_frame(&self) -> Option<TransactionFrame> {
        self.transactions.last().and_then(|level| level.frame)
    }
    /// Whether `tx` is the live loan for this game's innermost open transaction.
    pub(super) fn transaction_context_is_current(&self, tx: &TransactionContext) -> bool {
        self.current_storage_frame()
            .is_some_and(|frame| frame.matches(tx))
    }
    /// `transaction_lifecycle`: the close running, else whether one is open.
    pub(in crate::game) fn transaction_lifecycle(&self) -> TransactionLifecycle {
        match self.transactions.closing {
            Some(closing) => closing,
            None if self.transactions.is_empty() => TransactionLifecycle::None,
            None => TransactionLifecycle::Open,
        }
    }
    pub(super) fn current_transaction_is_here(&self) -> bool {
        self.transactions
            .last()
            .is_some_and(|frame| frame.dimension == Some(self.dim))
    }
    pub(super) fn has_level_transaction(&self) -> bool {
        self.transactions
            .iter()
            .any(|level| level.dimension == Some(self.dim))
    }
    /// Abort any storage transactions left open above `depth` (for example when a
    /// an engine decision unwinds before it can close its savepoint).
    pub(in crate::game) fn abort_abandoned_transactions(&mut self, depth: usize) {
        while self.transactions.len() > depth {
            let _ = self.close_storage_transaction(false);
        }
    }

    /// Closes the innermost transaction: `commit` hands its changes to the one around it, or
    /// at the root makes them final and tells the world; otherwise every storage it wrote goes
    /// back to how it was. Nothing happens with none open.
    ///
    /// Always switches into the top frame's recorded dimension for the entire close (retirement,
    /// commit or rollback) and restores the caller's active dimension afterwards, so an abandoned
    /// or cross-level frame never commits or restores its level-local snapshots into another level.
    fn close_storage_transaction(&mut self, commit: bool) -> Option<StorageSettlement> {
        let owner = self.transactions.last()?.dimension.unwrap_or(self.dim);
        if let Some(settlement) = self.with_effect_level(owner, |game| {
            game.close_storage_transaction_in_place(commit)
        }) {
            settlement
        } else {
            log::error!(
                "Sunrise: could not switch to {} to close transaction; dropping level-local writes",
                owner.id()
            );
            self.drop_unreachable_transaction_frame();
            Some(if commit {
                StorageSettlement::Refused
            } else {
                StorageSettlement::Aborted
            })
        }
    }
    /// Closes the innermost transaction with its owning level already in place.
    fn close_storage_transaction_in_place(&mut self, commit: bool) -> Option<StorageSettlement> {
        let mut level = self.transactions.pop()?;
        let refused = commit
            && level.packets.is_some()
            && sunrise_client::lan::conn::transaction_packets::Scope::refused();
        let commit = commit && !refused;
        // Expire the loan before replay can reenter mods. Its closed row must never join
        // another journal, even if the current frame's commit is nested.
        let owned = level.owned_scratch.take().and_then(|id| {
            level.scratch_kept.remove(&id);
            self.transactions.scratch.remove(&id)
        });
        self.transactions
            .attachment_query_scopes
            .leave(level.parent_attachment_query_scope);
        let closing = if self.transactions.is_empty() {
            TransactionLifecycle::RootClosing
        } else {
            TransactionLifecycle::Closing
        };
        // Kept and put back, so a transaction a root-commit listener opens and closes doesn't
        // end the root's phase early. The guard puts it back on a panic too, so a listener
        // that panics leaves the phase as it was.
        let outer = self.transactions.closing.replace(closing);
        // Each world's retired rows stay bound through its own root commit, including
        // transactions reentered by that commit's listeners.
        let retire = !self.has_level_transaction()
            && !entities::is_committing(&self.transactions.committing, self.dim);
        let guard = ClosingPhase { game: self, outer };
        #[cfg(test)]
        guard
            .game
            .trace_storage_attempt(if commit { "commit" } else { "abort" }.into());
        guard.game.close_level(level, commit);
        if retire {
            guard.game.world.block_entities.publish_retirement();
        }
        Some(if commit {
            StorageSettlement::Committed(owned)
        } else if refused {
            StorageSettlement::Refused
        } else {
            StorageSettlement::Aborted
        })
    }
    /// Defensive fallback if a frame's recorded dimension is no longer parked: pop the frame so
    /// abandon loops always terminate and discard its level-local snapshots rather than restoring
    /// them into the wrong active level.
    fn drop_unreachable_transaction_frame(&mut self) {
        let Some(mut level) = self.transactions.pop() else {
            return;
        };
        let _ = level.owned_scratch.take().and_then(|id| {
            level.scratch_kept.remove(&id);
            self.transactions.scratch.remove(&id)
        });
        self.transactions
            .attachment_query_scopes
            .leave(level.parent_attachment_query_scope);
        let closing = if self.transactions.is_empty() {
            TransactionLifecycle::RootClosing
        } else {
            TransactionLifecycle::Closing
        };
        let outer = self.transactions.closing.replace(closing);
        let guard = ClosingPhase { game: self, outer };
        let mut merchant_corrections = std::mem::take(&mut level.trades.refused_clicks);
        merchant_corrections.append(&mut level.trades.correct_on_abort);
        let mount_replies = std::mem::take(&mut level.mount_replies);
        let departed = std::mem::take(&mut level.departed);
        guard
            .game
            .transactions
            .close_retained_originals(level.frame, false);
        // Only roll back journals that don't rely on the owning level being in place.
        level.blocks.clear();
        level.connection_groups.clear();
        level.block_entities.clear();
        level.removed_block_entities.clear();
        level.saved_pots.clear();
        level.slots.clear();
        level.item_entities.clear();
        level.general_entities.clear();
        level.level_random = None;
        level.chunk_marks.clear();
        level.biomes.clear();
        level.after.clear();
        let _ = std::mem::take(&mut level.lifetimes);
        guard.game.roll_back(level);
        guard.game.close_departures(departed);
        if let Some(outer) = guard.game.transactions.last_mut() {
            outer.trades.refused_clicks.extend(merchant_corrections);
        } else {
            for (conn, window) in merchant_corrections {
                guard.game.resend_merchant_take(conn, window);
            }
        }
        guard.game.settle_mount_replies(mount_replies);
    }
    /// What closing `level` does, with the closing phase already set.
    fn close_level(&mut self, mut level: Level, commit: bool) {
        if let Some(scope) = level.packets.take()
            && commit
            && let Some(packets) = scope.finish()
        {
            level.captured_packets.push(packets);
        }
        let mut merchant_corrections = std::mem::take(&mut level.trades.refused_clicks);
        if !commit {
            merchant_corrections.append(&mut level.trades.correct_on_abort);
        }
        let mount_replies = std::mem::take(&mut level.mount_replies);
        let departed = std::mem::take(&mut level.departed);
        debug_assert_eq!(
            level.dimension,
            Some(self.dim),
            "transaction closed in a different level"
        );
        // A nested operation can briefly put another dimension in place. Its inline snapshot
        // belongs to that stream, not to the parent's in-place slot.
        let child_random_owner = if commit
            && level.level_random.is_some()
            && self
                .transactions
                .last()
                .is_some_and(|outer| outer.dimension != level.dimension)
        {
            self.native_lifecycle
                .epoch()
                .map(|epoch| self.level_ref(epoch, self.dim.key()))
        } else {
            None
        };
        let lifetimes = std::mem::take(&mut level.lifetimes);
        self.transactions
            .close_retained_originals(level.frame, commit);
        let mut independent = None;
        if commit && let Some(outer) = self.transactions.last_mut() {
            if outer.dimension == level.dimension {
                outer.merge_level_writes(&mut level);
            } else {
                let mut local = Level {
                    block_entities: std::mem::take(&mut level.block_entities),
                    saved_pots: std::mem::take(&mut level.saved_pots),
                    removed_block_entities: std::mem::take(&mut level.removed_block_entities),
                    slots: std::mem::take(&mut level.slots),
                    blocks: std::mem::take(&mut level.blocks),
                    connection_groups: std::mem::take(&mut level.connection_groups),
                    item_entities: std::mem::take(&mut level.item_entities),
                    general_entities: std::mem::take(&mut level.general_entities),
                    chunk_marks: std::mem::take(&mut level.chunk_marks),
                    level_save_gate: level.level_save_gate.take(),
                    biomes: std::mem::take(&mut level.biomes),
                    after: level
                        .after
                        .extract_if(.., |after| after.level_owned())
                        .collect(),
                    ..Level::default()
                };
                level.save_chunks.retain(|&(dim, chunk, scope)| {
                    if dim != self.dim || scope == SaveChunkScope::Transaction {
                        return true;
                    }
                    local.save_chunks.insert((dim, chunk, scope));
                    false
                });
                if let Some(owner) = entities::open_in(&mut self.transactions, self.dim) {
                    owner.merge_level_writes(&mut local);
                    owner.save_chunks.extend(local.save_chunks);
                    owner.after.extend(local.after);
                } else {
                    independent = Some(local);
                }
            }
        }
        match (commit, self.transactions.levels.last_mut()) {
            (true, Some(outer)) => {
                for (id, kept) in level.scratch_kept {
                    if self.transactions.scratch.contains_key(&id) {
                        outer.scratch_kept.entry(id).or_insert(kept);
                    }
                }
                outer.actions.merge(level.actions);
                outer.trades.merge(level.trades);
                if let Some(before) = level.level_random {
                    if outer.dimension == level.dimension {
                        outer.keep_level_random(None, || before);
                    } else if let Some(owner) = child_random_owner {
                        outer.keep_level_random(Some(owner), || before);
                    }
                }
                for (owner, before) in level.parked_level_randoms {
                    outer.keep_level_random(Some(owner), || before);
                }
                if outer.loot_sequences.is_none() {
                    outer.loot_sequences = level.loot_sequences;
                }
                for (chunk, randoms) in level.composter_randoms {
                    let kept = outer.composter_randoms.entry(chunk).or_default();
                    for (pos, random) in randoms {
                        kept.entry(pos).or_insert(random);
                    }
                }
                for (owner, kept) in level.players {
                    outer.players.entry(owner).or_insert(kept);
                }
                for (uuid, kept) in level.player_health {
                    outer.player_health.entry(uuid).or_insert(kept);
                }
                for (id, kept) in level.mob_health {
                    outer.mob_health.entry(id).or_insert(kept);
                }
                for (id, kept) in level.mob_effects {
                    outer.mob_effects.entry(id).or_insert(kept);
                }
                outer.effect_modifiers.extend(level.effect_modifiers);
                for (id, kept) in level.entities {
                    outer.entities.entry(id).or_insert(kept);
                }
                for (id, kept) in level.equipment {
                    outer.equipment.entry(id).or_insert(kept);
                }
                for (id, changes) in level.modifier_changes {
                    outer.modifier_changes.entry(id).or_default().merge(changes);
                }
                if outer.board.is_none() {
                    outer.board = level.board;
                }
                for (join, kept) in level.orbs {
                    outer.orbs.entry(join).or_insert(kept);
                }
                for (key, kept) in level.stations {
                    outer.stations.entry(key).or_insert(kept);
                }
                for (id, len) in level.mob_lengths {
                    outer.mob_lengths.entry(id).or_insert(len);
                }
                for (owner, kept) in level.ender_items {
                    outer.ender_items.entry(owner).or_insert(kept);
                }
                for (owner, kept) in level.player_extra {
                    outer.player_extra.entry(owner).or_insert(kept);
                }
                for (owner, kept) in level.player_books {
                    outer.player_books.entry(owner).or_insert(kept);
                }
                for (conn, kept) in level.menus {
                    outer.menus.entry(conn).or_insert(kept);
                }
                for (generation, kept) in level.menu_clicks {
                    outer.menu_clicks.entry(generation).or_insert(kept);
                }
                for (window, kept) in level.mount_drags {
                    outer.mount_drags.entry(window).or_insert(kept);
                }
                for (generation, kept) in level.native_menu_slots {
                    outer.native_menu_slots.entry(generation).or_insert(kept);
                }
                outer.menu_data.extend(level.menu_data);
                outer.captured_packets.extend(level.captured_packets);
                for (holder, kept) in level.attachments {
                    outer.attachments.entry(holder).or_insert(kept);
                }
                for (id, kept) in level.travelled {
                    outer.travelled.entry(id).or_insert(kept);
                }
                for (id, slot) in level.in_flight {
                    outer.in_flight.entry(id).or_insert(slot);
                }
                outer.save_gate = outer.save_gate.take().or(level.save_gate);
                outer.save_chunks.extend(level.save_chunks);
                outer.new_maps.append(&mut level.new_maps);
                outer.after.extend(level.after);
                // The nearest enclosing transaction of this level takes its entities back; a
                // parent in another level can't, and with none in this level they are told now.
                match (level.dimension)
                    .and_then(|dim| entities::open_in(&mut self.transactions, dim))
                {
                    Some(enclosing) => enclosing.lifetimes.merge(lifetimes),
                    None => self.commit_entity_journal(lifetimes, |game| {
                        if let Some(local) = independent {
                            game.root_commit(local);
                        }
                    }),
                }
            }
            (true, None) => {
                // The commit is in place before the effects run: a join one causes (a placed
                // item's leftover spawning) never announces what the transaction added and removed.
                self.commit_entity_journal(lifetimes, |game| game.root_commit(level));
            }
            (false, _) => {
                // The entities it added go first, so nothing is put back into one.
                let arrivals = self.roll_back_entity_journal(lifetimes);
                let dimension = level.dimension;
                self.roll_back(level);
                if let Some(enclosing) =
                    dimension.and_then(|dim| entities::open_in(&mut self.transactions, dim))
                {
                    enclosing.lifetimes.keep_arrivals(arrivals);
                }
            }
        }
        self.close_departures(departed);
        // Rejected client predictions need the final restored menu, whether the enclosing
        // transaction commits or aborts. Do not stage a correction using tentative state.
        if let Some(outer) = self.transactions.last_mut() {
            outer.trades.refused_clicks.extend(merchant_corrections);
        } else {
            for (conn, window) in merchant_corrections {
                self.resend_merchant_take(conn, window);
            }
        }
        self.settle_mount_replies(mount_replies);
    }
    /// Aborts abandoned frames before the next operation can join them. Call only when
    /// there is no caller-owned frame; the tick start also uses this as a backstop.
    pub(super) fn abort_open_transactions(&mut self) {
        while !self.transactions.is_empty() {
            let _ = self.close_storage_transaction(false);
        }
        // If called during a root close, the enclosing close still owns retirement.
        if self.transactions.closing.is_none() {
            self.world.block_entities.publish_retirement();
        }
    }

    /// `ComposterWrapper.transactionalRandom`: snapshot only the position's private owner,
    /// including its initial state on a first access that later aborts.
    pub(super) fn take_composter_random(
        &mut self,
        pos: [i32; 3],
    ) -> sunrise_core::random::LegacyRandom {
        let epoch = self.native_lifecycle.epoch();
        if self.transactions.composter_epoch != epoch {
            self.clear_composter_randoms();
            self.transactions.composter_epoch = epoch;
        }
        let chunk = (self.dim.key(), (pos[0] >> 4, pos[2] >> 4));
        let key = (pos[0], pos[1], pos[2]);
        let random = self
            .transactions
            .composter_randoms
            .entry(chunk)
            .or_default()
            .remove(&key)
            .unwrap_or_else(|| (*self.world.random).clone());
        if let Some(level) = self.transactions.last_mut() {
            level
                .composter_randoms
                .entry(chunk)
                .or_default()
                .entry(key)
                .or_insert_with(|| random.clone());
        }
        random
    }

    pub(super) fn put_composter_random(
        &mut self,
        pos: [i32; 3],
        random: sunrise_core::random::LegacyRandom,
    ) {
        self.transactions
            .composter_randoms
            .entry((self.dim.key(), (pos[0] >> 4, pos[2] >> 4)))
            .or_default()
            .insert((pos[0], pos[1], pos[2]), random);
    }

    /// Retire only an unloading chunk's cached owners and their pending savepoints.
    pub(super) fn forget_composter_chunk(
        &mut self,
        dimension: mod_api::DimensionKey,
        chunk: [i32; 2],
    ) {
        let key = (dimension, (chunk[0], chunk[1]));
        self.transactions.composter_randoms.remove(&key);
        for level in &mut self.transactions.levels {
            level.composter_randoms.remove(&key);
        }
    }

    pub(super) fn clear_composter_randoms(&mut self) {
        self.transactions.composter_epoch = None;
        self.transactions.composter_randoms.clear();
        for level in &mut self.transactions.levels {
            level.composter_randoms.clear();
        }
    }
    /// Keeps a pot's saved item and loot metadata until its level commits.
    pub(super) fn journal_saved_pot(&mut self, pos: Pos) {
        self.transactions.writes = self.transactions.writes.wrapping_add(1);
        if let Some(level) = entities::open_in(&mut self.transactions, self.dim) {
            level
                .saved_pots
                .entry(pos)
                .or_insert_with(|| super::spill::saved_entity(&self.world, [pos.0, pos.1, pos.2]));
            level
                .level_save_gate
                .get_or_insert_with(sunrise_client::join_seq::visibility::PassLease::enter);
        }
    }

    /// Block or container loot may draw from either the level or a named stream.
    pub(super) fn journal_block_loot_random(&mut self) {
        self.journal_level_random();
        if let Some(level) = entities::open_in(&mut self.transactions, self.dim) {
            level
                .loot_sequences
                .get_or_insert_with(|| self.rand_sequences.clone());
        }
    }

    /// Keeps the block entity at `pos` as it is, with its attachments and a hopper's
    /// cooldown, if a transaction is open and hasn't yet.
    pub(super) fn journal_block_entity(&mut self, pos: Pos) {
        self.transactions.writes = self.transactions.writes.wrapping_add(1);
        let Some(level) = entities::open_in(&mut self.transactions, self.dim) else {
            return;
        };
        level
            .block_entities
            .entry(pos)
            .or_insert_with(|| Kept::capture(&self.world, &self.hoppers, pos));
    }
    /// Puts back a removed block entity and its generation where none stands there now.
    fn restore_removed_block_entity(&mut self, pos: Pos, entity: BlockEntity, generation: u64) {
        let entities = &mut self.world.block_entities;
        if entities.get(&pos).is_none() {
            entities.insert(pos, entity);
            entities.restore_generation(pos, generation);
            entities.reattach_retired(pos, generation);
        }
    }
    /// Records an explicit `BlockEntityRef::remove` inside an open transaction: captures the
    /// original row in `block_entities`, marks `removed_block_entities` with a `None` sentinel
    /// without overwriting any earlier `Some(row)` queued for `preRemoveSideEffects` replay,
    /// and queues the deferred `setRemoved` effects to run at root commit.
    /// Returns `true` if a transaction is open in the current dimension (effects deferred),
    /// or `false` if outside a transaction (caller runs effects immediately).
    pub(super) fn journal_explicit_remove_block_entity(
        &mut self,
        pos: Pos,
        effects: RemovedBlockEntityEffects,
    ) -> bool {
        self.journal_block_entity(pos);
        let Some(level) = entities::open_in(&mut self.transactions, self.dim) else {
            return false;
        };
        level.removed_block_entities.entry(pos).or_insert(None);
        level.after.push(After::BlockEntityRemoved(effects));
        true
    }
    /// Hides the block entity from tentative reads. The first-write journal can undo all edits;
    /// the removed row itself still contains the edits that matter to removal at commit.
    pub(super) fn journal_remove_block_entity(&mut self, pos: Pos) {
        self.journal_block_entity(pos);
        let Some(level) = entities::open_in(&mut self.transactions, self.dim) else {
            self.world.block_entities.remove(&pos);
            return;
        };
        if let Some(After::Block(step)) = level.after.last_mut()
            && step.pos == pos
        {
            step.removed = Some(ReplayEntity::take(&mut self.world, pos));
        } else {
            let removed = take_replay_row(&mut self.world, pos);
            let first = level.removed_block_entities.entry(pos).or_default();
            if first.is_none() {
                *first = removed;
            }
        }
    }
    /// How many block entities the open transactions keep whole, and how many single slots:
    /// what a test reads to tell a slot journal from a whole capture without timing it.
    #[cfg(test)]
    pub(in crate::game) fn journal_kept(&self) -> (usize, usize) {
        self.transactions
            .iter()
            .fold((0, 0), |(whole, slots), level| {
                (
                    whole + level.block_entities.len(),
                    slots + level.slots.len(),
                )
            })
    }
    /// Keeps item slot `slot` of the block entity at `pos` as it is, if a transaction is open
    /// and keeps neither it nor the whole entity yet. Only for a write that changes that slot
    /// and nothing else of the entity (no cooldown, cooking or saved data beside it), so the
    /// journal holds one stack however large the entity's saved data is. A slot past the end of
    /// the entity's slot list (a mod's list shorter than its container) is a write that grows
    /// the list: the entity is kept whole instead, length and all.
    pub(super) fn journal_block_entity_slot(&mut self, pos: Pos, slot: usize) {
        self.transactions.writes = self.transactions.writes.wrapping_add(1);
        let entity = self.world.block_entities.get(&pos);
        // Claiming an opaque row changes the saved compound as well as the slot.
        if entity.is_none_or(|entity| slot >= entity.slots().len())
            || matches!(entity, Some(BlockEntity::Mod(m)) if m.has_opaque_item_slot(slot))
        {
            return self.journal_block_entity(pos);
        }
        let Some(level) = entities::open_in(&mut self.transactions, self.dim) else {
            return;
        };
        if level.block_entities.contains_key(&pos) {
            return;
        }
        level.slots.entry((pos, slot)).or_insert_with(|| KeptSlot {
            stack: entity.and_then(|entity| entity.slots()[slot].clone()),
            generation: self.world.block_entities.generation(&pos),
        });
    }
    /// Keeps `holder`'s attachment map as it is, if a transaction is open and hasn't yet: a
    /// block entity's with the rest of it, any other's alone.
    pub(super) fn journal_attachments(&mut self, holder: Holder) {
        let dim = self.dim;
        let level = if matches!(holder, Holder::BlockEntity(_)) {
            entities::open_in(&mut self.transactions, dim)
        } else {
            self.transactions.last_mut()
        };
        if let Some(level) = level {
            level
                .save_gate
                .get_or_insert_with(sunrise_client::join_seq::visibility::PassLease::enter);
        }
        // The chunk that a block entity's or chunk's map marks unsaved (`world_attachments`).
        let chunk = match holder {
            Holder::BlockEntity([x, _, z]) => Some((x >> 4, z >> 4)),
            Holder::Chunk(chunk) => Some((chunk[0], chunk[1])),
            _ => None,
        };
        if let Some(chunk) = chunk
            && let Some(level) = entities::open_in(&mut self.transactions, dim)
        {
            let modified = &self.world.modified;
            level
                .chunk_marks
                .entry(chunk)
                .or_insert_with(|| modified.mark(chunk));
        }
        if let Holder::BlockEntity([x, y, z]) = holder {
            return self.journal_block_entity((x, y, z));
        }
        let key = match holder {
            Holder::Player(uuid) => {
                let Some(body) = self.attachment_player_body(uuid) else {
                    return;
                };
                AttachmentKey::Player(body)
            }
            other => AttachmentKey::Level(dim, other),
        };
        let Some(level) = self.transactions.last() else {
            return;
        };
        if level.attachments.contains_key(&key) {
            return;
        }
        let entity_bound = if let Holder::Entity(id) = holder {
            std::num::NonZeroU32::new(id).and_then(|id| {
                match self.with_resident_slot(&dim.key(), id, |slot| slot.is_bound()) {
                    super::entity_residency::SlotAccess::Done(bound) => Some(bound),
                    _ => None,
                }
            })
        } else {
            None
        };
        let Ok(kept) = self
            .attachment_slot(holder, mod_api::AttachmentAccess::Existing)
            .map(|map| KeptAttachments {
                contents: map.clone(),
                indexed: entity_bound.unwrap_or_else(|| map.has_index_binding()),
            })
        else {
            return;
        };
        if let Some(level) = self.transactions.last_mut() {
            level.attachments.insert(key, kept);
        }
    }
    /// Keeps the inventory of `uuid`, whose state is acting in `Game` now, on its first
    /// write in the open frame.
    pub(super) fn journal_player(&mut self, uuid: u128) {
        let Some(level) = self.transactions.player_write() else {
            return;
        };
        let owner = self
            .lan_acting
            .map_or(JournaledPlayer::Local(uuid), JournaledPlayer::Guest);
        level.keep_player(
            owner,
            &self.ender_items,
            self.level.player_extra.as_ref(),
            || KeptPlayer {
                inventory: self.inventory.clone(),
                armor: self.armor.clone(),
                offhand: self.offhand.clone(),
                equipment: self.equipment.clone(),
                carried: self.carried.clone(),
            },
        );
    }
    /// The same journal for a player resolved to their own parked record at `index`.
    pub(super) fn journal_parked_player(&mut self, index: usize) {
        let Some(level) = self.transactions.player_write() else {
            return;
        };
        let guest = &self.lan.as_ref().expect("resolved guest").hub.guests[index];
        level.keep_player(
            JournaledPlayer::Guest(guest.conn),
            &guest.ender_items,
            guest.saved.as_ref(),
            || KeptPlayer {
                inventory: guest.inventory.clone(),
                armor: guest.armor.clone(),
                offhand: guest.offhand.clone(),
                equipment: guest.equipment.clone(),
                carried: guest.carried.clone(),
            },
        );
    }

    /// Save the guest's inventory and current cursor wherever their turn lent
    /// them. A suspended guest cannot be re-entered to take this snapshot.
    pub(super) fn journal_guest_inventory(&mut self, conn: u32) {
        let owner = JournaledPlayer::Guest(conn);
        if self
            .transactions
            .last()
            .is_none_or(|level| level.players.contains_key(&owner))
        {
            return;
        }
        let Some(home) = self.player_storage_home(owner) else {
            return;
        };
        let kept = match home {
            PlayerStorageHome::Game => KeptPlayer {
                inventory: self.inventory.clone(),
                armor: self.armor.clone(),
                offhand: self.offhand.clone(),
                equipment: self.equipment.clone(),
                carried: self.carried.clone(),
            },
            PlayerStorageHome::Guest(index) => {
                let g = &self.lan.as_ref().expect("resolved guest").hub.guests[index];
                KeptPlayer {
                    inventory: g.inventory.clone(),
                    armor: g.armor.clone(),
                    offhand: g.offhand.clone(),
                    equipment: g.equipment.clone(),
                    carried: g.carried.clone(),
                }
            }
        };
        self.transactions
            .player_write()
            .expect("open transaction")
            .players
            .insert(owner, kept);
    }

    /// Follow the actual storage, even when public handles cannot resolve the player.
    /// Each outstanding swap holds the immediately preceding actor in the inner guest's row.
    fn player_storage_home(&self, owner: JournaledPlayer) -> Option<PlayerStorageHome> {
        let parked_in = match owner {
            JournaledPlayer::Local(uuid) => {
                let local = self
                    .level
                    .player_uuid
                    .map(sunrise_client::lan::playerdata::ints_uuid);
                if local != Some(uuid) {
                    return None;
                }
                self.lan_guest_swaps.first().copied()
            }
            JournaledPlayer::Guest(conn) => self
                .lan_guest_swaps
                .iter()
                .position(|&swapped| swapped == conn)
                .map_or(Some(conn), |i| self.lan_guest_swaps.get(i + 1).copied()),
        };
        match parked_in {
            Some(conn) => self.guest_index(conn).map(PlayerStorageHome::Guest),
            None => Some(PlayerStorageHome::Game),
        }
    }
    /// Keeps the saved player extra NBT (`level.player_extra`) of `uuid`, whose state is
    /// acting in `Game` now, on its first write in the open frame.
    pub(super) fn journal_player_extra(&mut self, uuid: u128) {
        let Some(level) = self.transactions.player_write() else {
            return;
        };
        let owner = self
            .lan_acting
            .map_or(JournaledPlayer::Local(uuid), JournaledPlayer::Guest);
        level
            .player_extra
            .entry(owner)
            .or_insert_with(|| self.level.player_extra.clone());
    }
    /// The same journal for a player resolved to their own parked record at `index`.
    pub(super) fn journal_parked_player_extra(&mut self, index: usize) {
        let Some(level) = self.transactions.player_write() else {
            return;
        };
        let guest = &self.lan.as_ref().expect("resolved guest").hub.guests[index];
        level
            .player_extra
            .entry(JournaledPlayer::Guest(guest.conn))
            .or_insert_with(|| guest.saved.clone());
    }
    /// Keep the original setting of the acting player; return whether publication is deferred.
    pub(super) fn journal_player_book(
        &mut self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
    ) -> bool {
        let Some(level) = self.transactions.last_mut() else {
            return false;
        };
        let owner = self.lan_acting.map_or(
            JournaledPlayer::Local(player.uuid()),
            JournaledPlayer::Guest,
        );
        level
            .player_books
            .entry((owner, book.clone()))
            .or_insert_with(|| (player, self.level.recipe_book.custom.stored(book.as_str())));
        true
    }
    /// Called with the target player's body swapped in, before set_health changes it.
    pub(super) fn journal_player_health(&mut self) {
        let body = self.acting_player_body();
        let Some(level) = self.transactions.last_mut() else {
            return;
        };
        let Some(player) = self.player.as_ref() else {
            return;
        };
        let std::collections::hash_map::Entry::Vacant(entry) = level.player_health.entry(body)
        else {
            return;
        };
        let guest = self.lan_acting.and_then(|conn| {
            let guest = self.lan.as_ref()?.hub.guest(conn)?;
            Some((conn, guest.death_canceled, guest.death_time))
        });
        entry.insert(KeptPlayerHealth {
            data: player.health.data.clone(),
            events_len: player.events.len(),
            dead: player.health.dead,
            guest,
        });
    }
    pub(super) fn journal_mob_health(&mut self, id: u32) {
        let dim = self.dim;
        let Some(level) = self.transactions.last_mut() else {
            return;
        };
        if let Some(mob) = self.mobs.iter().find(|m| m.id == id && !m.removed) {
            level.mob_health.entry((dim, id)).or_insert(KeptMobHealth {
                health: mob.health,
                absorption: mob.absorption,
                needs_sync: mob.needs_sync,
                death_time: mob.death_time,
                death_canceled: mob.death_canceled,
                processed: self.died.contains(&id),
            });
        }
    }
    /// Writes one mob effect, or any active effect when `effect` is `None`. In a transaction,
    /// keep the effect lists and use the same health journal as `set_health` and `set_absorption`.
    pub(super) fn mutate_mob_effects<R>(
        &mut self,
        id: u32,
        effect: Option<&'static str>,
        mutate: impl FnOnce(&mut Mob) -> R,
    ) -> Option<R> {
        let dim = self.dim;
        self.journal_mob_health(id);
        let mob = self
            .mobs
            .iter_mut()
            .find(|mob| mob.id == id && !mob.removed)?;
        let invalidation = if !self.transactions.is_empty() {
            self.transactions.writes = self.transactions.writes.wrapping_add(1);
            let level = self.transactions.last_mut().expect("open transaction");
            level
                .mob_effects
                .entry((dim, id))
                .or_insert_with(|| KeptMobEffects {
                    active: mob.extra.effects.clone(),
                    chains: mob.extra.effect_chains.clone(),
                    ambient: mob.extra.ambient_effects.clone(),
                    hidden: mob.extra.hidden_effects.clone(),
                    dirty: mob.effects_dirty,
                });
            let modifiers = match effect {
                Some(effect) => mob.effect_modifier_undo(std::iter::once(effect)),
                None => mob.effect_modifier_undo(mob.extra.effects.iter().map(|effect| effect.id)),
            };
            if let Some(modifiers) = modifiers {
                level.effect_modifiers.push(((dim, id), modifiers));
                super::living_attributes::modifier_invalidation_undo(&self.motion.attributes, mob)
            } else {
                None
            }
        } else {
            None
        };
        let result = mutate(mob);
        // The ordered effect journal owns the seed; only the discarded live
        // equipment rows and detection state go to the general modifier journal.
        if let Some(changes) = invalidation
            && mob.extra.live.is_invalidated()
        {
            self.transactions.journal_modifier_changes(dim, id, changes);
        }
        Some(result)
    }
    /// Keeps `player`'s menu storage as it is, if a transaction is open and hasn't yet.
    pub(super) fn journal_menus(&mut self, player: mod_api::PlayerRef) {
        self.transactions.player_writes = self.transactions.player_writes.wrapping_add(1);
        self.transactions.writes = self.transactions.writes.wrapping_add(1);
        let Some(level) = self.transactions.last() else {
            return;
        };
        let Some(g) = self.native_menu_player(player) else {
            return;
        };
        let conn = g.conn;
        let grid = (!level.menus.contains_key(&conn)).then(|| KeptMenus {
            craft_grid: g.craft_grid.clone(),
        });
        let native = (g
            .menu
            .as_ref()
            .is_some_and(|menu| menu.slots == sunrise_client::lan::menu::Slots::Native)
            && !level.native_menu_slots.contains_key(&g.menu_generation))
        .then(|| (g.menu_generation, g.native_menu.clone()));
        if grid.is_none() && native.is_none() {
            return;
        }
        let owner = JournaledPlayer::Guest(conn);
        let ender = (!level.ender_items.contains_key(&owner)).then(|| {
            if self.lan_acting == Some(conn) {
                self.ender_items.clone()
            } else {
                g.ender_items.clone()
            }
        });
        if let Some(level) = self.transactions.last_mut() {
            if let Some(grid) = grid {
                level.menus.insert(conn, grid);
            }
            if let Some((generation, slots)) = native {
                level.native_menu_slots.insert(generation, slots);
            }
            if let Some(ender) = ender {
                level.ender_items.insert(owner, ender);
            }
        }
    }
    pub(super) fn journal_menu_click(&mut self, conn: u32) {
        let Some(level) = self.transactions.last_mut() else {
            return;
        };
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some(guest) = host.hub.guest(conn) else {
            return;
        };
        let Some(menu) = guest.menu.as_ref() else {
            return;
        };
        level
            .menu_clicks
            .entry(guest.menu_generation)
            .or_insert_with(|| {
                let owner = host.menus.get(&conn);
                KeptClick {
                    conn,
                    drag: menu.drag.clone(),
                    craft: menu.craft.clone(),
                    slot_changed_seen: menu.slot_changed_seen.clone(),
                    owned: owner.and_then(|owner| owner.owned_menu.clone()),
                    machine: owner.and_then(|owner| owner.crafting.machine.as_ref().cloned()),
                    places: owner.and_then(|owner| owner.slot_places.clone()),
                }
            });
    }

    pub(super) fn journal_mount_drag(&mut self, window: MountClickWindow, before: QuickCraftDrag) {
        if let Some(level) = self.transactions.last_mut() {
            level.mount_drags.entry(window).or_insert(before);
        }
    }

    /// On both commit and abort, defer a window's reply if any storage remains tentative.
    pub(super) fn settle_mount_replies(
        &mut self,
        replies: impl IntoIterator<Item = MountClickWindow>,
    ) {
        if let Some(level) = self.transactions.last_mut() {
            level.mount_replies.extend(replies);
        } else {
            for window in replies {
                self.send_settled_mount_reply(window);
            }
        }
    }

    /// Keeps entity `id`'s inventory as it is, if a transaction is open and hasn't yet.
    pub(super) fn journal_entity(&mut self, id: u32) {
        self.transactions.writes = self.transactions.writes.wrapping_add(1);
        if self.transactions.is_empty() {
            return;
        }
        let dim = self.dim;
        let Some(slots) = self.edit_entity_slots(id, |slots, _| slots.to_vec()) else {
            return;
        };
        if let Some(level) = self.transactions.last_mut() {
            level.entities.entry((dim, id)).or_insert(slots);
        }
    }
    /// Keeps the scoreboard as it is, if a transaction is open and hasn't yet: a copy of the
    /// whole board, taken on the transaction's first scoreboard write and never otherwise, so a
    /// transaction that writes none pays nothing. An abort puts it back.
    pub(super) fn journal_scoreboard(&mut self) {
        let Some(level) = self.transactions.last_mut() else {
            return;
        };
        if level.board.is_none() {
            level.board = Some(self.score.board.clone());
        }
    }
    /// Keeps the slots at their first write. Equipment collection records any
    /// actual modifier edits separately; a quiet edit does not run it.
    pub(super) fn journal_equipment(&mut self, id: u32) {
        let dim = self.dim;
        let Some(level) = self.transactions.last_mut() else {
            return;
        };
        if level.equipment.contains_key(&(dim, id)) {
            return;
        }
        if let Some(mob) = self.mobs.iter().find(|m| m.id == id && !m.removed) {
            level.keep_equipment(dim, id, &mob.extra.equipment, Some(&mob.extra.gear));
        } else if let Some(stand) = self.armor_stands.iter().find(|a| a.id == id) {
            level.keep_equipment(dim, id, &stand.equipment, None);
        }
    }
    /// `drop_plain` by the player whose state is in `Game` now, `uuid`: at once, or at the root
    /// commit inside a transaction.
    pub(super) fn drop_after_commit(&mut self, uuid: u128, stack: ItemStack) {
        match self.transactions.last_mut() {
            Some(level) => level.after.push(After::PlayerDrop(uuid, stack)),
            None => {
                self.drop_plain(stack);
            }
        }
    }
    /// Set or clear a connection group at `pos`. Inside an open Host transaction, writes
    /// storage quietly, records the prior group so abort restores it silently, and defers
    /// remesh + client sync to root commit. Returns `false` if unloaded, out of height, or
    /// unchanged.
    pub(super) fn journal_set_connection_group(
        &mut self,
        pos: [i32; 3],
        group: Option<u64>,
    ) -> bool {
        let dim = self.dim;
        let Some(level) = entities::open_in(&mut self.transactions, dim) else {
            return self
                .world
                .set_connection_group(pos[0], pos[1], pos[2], group, true);
        };
        let key = (pos[0], pos[1], pos[2]);
        let old = self.world.connection_group(pos[0], pos[1], pos[2]);
        let changed = self
            .world
            .set_connection_group_stored(pos[0], pos[1], pos[2], group);
        if changed {
            level.keep_connection_group(key, old);
            level.after.push(After::ConnectionGroup(key, group));
        }
        changed
    }
    /// Sets the block at `pos` to `state`. In a transaction, stored only, keeping the old
    /// state, and set with its updates at the root commit; otherwise at once
    /// (`Level.setBlock`).
    pub(super) fn journal_set_block(&mut self, pos: [i32; 3], state: BlockStateId) -> bool {
        self.journal_set_block_flags(pos, state, sunrise_world::level::flags::UPDATE_ALL, false)
    }
    /// The root commit replays one write per position with the combined update flags.
    pub(super) fn journal_set_block_flags(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        flags: i32,
        level_host: bool,
    ) -> bool {
        let placement = self.captures_placement();
        let Some(level) = entities::open_in(&mut self.transactions, self.dim) else {
            return self.level_set_with_flags(pos, state, flags).is_ok();
        };
        let key = (pos[0], pos[1], pos[2]);
        let old = self.world.block(pos[0], pos[1], pos[2]);
        let clears_group = !self.res.behaviours().keeps_block_entity(old, state);
        if !self.world.set_stored(pos[0], pos[1], pos[2], state) {
            return false;
        }
        // Mirror live `chunk_set_block_state`: replacing the block with one that does not keep
        // connection groups clears any tentative group at `pos`, while compatible state changes
        // preserve it.
        let old_group = clears_group
            .then(|| self.world.connection_group(pos[0], pos[1], pos[2]))
            .flatten();
        if old_group.is_some() {
            let _ = self
                .world
                .set_connection_group_stored(pos[0], pos[1], pos[2], None);
        }
        let block = level.blocks.entry(key).or_insert(KeptBlock {
            state: old,
            replay: None,
            strict: false,
        });
        let step = BlockReplay {
            state: old,
            flags,
            level_host,
            replay_removal: false,
            clear_connection_group: clears_group,
        };
        let replay = block.replay.get_or_insert(step);
        replay.flags = merge_block_update_flags(replay.flags, flags);
        replay.level_host = level_host;
        replay.clear_connection_group |= clears_group;
        if old_group.is_some() {
            level.keep_connection_group(key, old_group);
        }
        level.after.push(After::Block(BlockStep {
            pos: key,
            to: state,
            replay: step,
            placement,
            removed: None,
            creation: BlockEntityCreation::Live,
        }));
        true
    }

    /// Hide a replaced block entity from staged reads, without dropping its
    /// contents until the real write. Fluid pickup/pour instead rolls its own
    /// drops against the old entity before journalling the removal.
    pub(super) fn journal_block_replacement(&mut self, pos: Pos) {
        let placement = self.captures_placement()
            || self
                .world
                .block_entities
                .generation(&pos)
                .is_some_and(|generation| self.is_placement_entity(pos, generation));
        self.journal_remove_block_entity(pos);
        if placement {
            self.world.block_names.remove(&pos);
            self.world.forget_block_entity_source(pos);
            self.hoppers.remove(&pos);
        }
        let Some(level) = entities::open_in(&mut self.transactions, self.dim) else {
            return;
        };
        if let Some(After::Block(step)) = level.after.last_mut() {
            step.replay.replay_removal = true;
        }
        if let Some(replay) = level
            .blocks
            .get_mut(&pos)
            .and_then(|block| block.replay.as_mut())
        {
            replay.replay_removal = true;
        }
    }
    pub(super) fn journal_stage_placement_entity(&mut self, pos: Pos, entity: Option<BlockEntity>) {
        if let Some(entity) = entity {
            self.journal_block_entity(pos);
            self.world.block_names.remove(&pos);
            self.world.forget_block_entity_source(pos);
            self.world.block_entities.insert(pos, entity);
        }
        let level = entities::open_in(&mut self.transactions, self.dim).unwrap();
        let Some(After::Block(step)) = level.after.last_mut() else {
            unreachable!("factory staging follows the captured block write");
        };
        debug_assert_eq!(step.pos, pos);
        step.creation = BlockEntityCreation::Staged {
            generation: self.world.block_entities.generation(&pos),
            entity: ReplayEntity::default(),
        };
    }

    pub(super) fn begin_placement_capture(&mut self) {
        entities::open_in(&mut self.transactions, self.dim)
            .unwrap()
            .capture_placement = true;
    }

    pub(super) fn captures_placement(&self) -> bool {
        self.transactions
            .iter()
            .any(|frame| frame.dimension == Some(self.dim) && frame.capture_placement)
    }

    /// Snapshot each cell in first-write order, with its state as the listener will see it.
    pub(super) fn placement_footprint(&mut self) -> Vec<(BlockPos, BlockStateId)> {
        let level = entities::open_in(&mut self.transactions, self.dim).unwrap();
        level.capture_placement = false;
        let mut seen = FxHashSet::default();
        let mut writes = Vec::new();
        let mut keep = |(x, y, z)| {
            if seen.insert((x, y, z)) {
                writes.push((BlockPos::new(x, y, z), self.world.block(x, y, z)));
            }
        };
        for after in &level.after {
            match after {
                After::Block(step) => keep(step.pos),
                After::BulkBlocks(_, _, _, steps) => steps.keys().copied().for_each(&mut keep),
                _ => {}
            }
        }
        writes
    }

    pub(super) fn is_placement_entity(&self, pos: Pos, generation: u64) -> bool {
        self.transactions.iter().any(|frame| {
            frame.dimension == Some(self.dim) && frame.blocks.contains_key(&pos)
                && frame.after.iter().rev().any(|after| {
                    matches!(after, After::Block(step) if step.pos == pos && matches!(step.creation,
                        BlockEntityCreation::Staged { generation: Some(created), .. } if created == generation))
                })
        })
    }

    /// Block-entity edits to an unpublished placement must not send a block update yet.
    pub(super) fn defer_placed_block_entity_update(&mut self, pos: Pos) -> bool {
        if let Some(generation) = self.world.block_entities.generation(&pos)
            && self.is_placement_entity(pos, generation)
        {
            self.after_commit(After::UpdatePlacedBlockEntity(pos, generation));
            return true;
        }
        false
    }

    pub(super) fn journal_set_level_random(&mut self, random: LegacyRandom) {
        self.journal_level_random();
        *self.world.random = random;
    }

    pub(super) fn journal_level_random(&mut self) {
        self.journal_active_level_random(LegacyRandom::clone);
    }

    /// Keep an engine or restricted-writer draw from the stream currently in place.
    pub(super) fn journal_active_level_random(
        &mut self,
        before: impl FnOnce(&LegacyRandom) -> LegacyRandom,
    ) {
        let owner = match self.transactions.last() {
            None => return,
            Some(frame) if frame.dimension == Some(self.dim) => None,
            Some(_) => {
                let Some(epoch) = self.native_lifecycle.epoch() else {
                    return;
                };
                Some(self.level_ref(epoch, self.dim.key()))
            }
        };
        if let Some(frame) = self.transactions.last_mut() {
            frame.keep_level_random(owner, || before(&self.world.random));
        }
    }
    pub(super) fn journal_orb_merge(
        &mut self,
        join: crate::join_seq::JoinSeq,
        fields: (i32, u32, i32),
    ) {
        let dim = self.dim;
        if let Some(level) = self.transactions.last_mut() {
            level.orbs.entry((dim, join)).or_insert(fields);
        }
    }
    pub(super) fn journal_station_click(
        &mut self,
        conn: u32,
        owner: decision::MenuId,
        station: &super::lan_stations::GuestStation,
    ) {
        if let Some(level) = self.transactions.last_mut() {
            level
                .stations
                .entry((conn, owner))
                .or_insert_with(|| station.clone());
        }
    }
    /// Keep the original cell in the transaction for the level in place.
    pub(in crate::game) fn journal_biome(&mut self, pos: [i32; 3], old: u16) {
        let dim = self.dim;
        if let Some(level) = entities::open_in(&mut self.transactions, dim) {
            level
                .biomes
                .entry((dim, (pos[0] >> 2, pos[1] >> 2, pos[2] >> 2)))
                .or_insert(old);
        }
    }
    /// Mark a written chunk unsaved at commit, without issuing any notice.
    pub(super) fn journal_chunk_save(&mut self, chunk: (i32, i32), scope: SaveChunkScope) {
        let dim = self.dim;
        let level = match scope {
            SaveChunkScope::Level => entities::open_in(&mut self.transactions, dim),
            SaveChunkScope::Transaction => self.transactions.last_mut(),
        };
        if let Some(level) = level {
            level.save_chunks.insert((dim, chunk, scope));
        }
    }
    /// Queue a reusable interaction effect for the outermost successful commit.
    pub(super) fn defer_effect(&mut self, effect: DeferredEffect) {
        self.after_commit(After::Effect(effect));
    }
    /// Collect committed implicit `setChanged` notices while the callback runs without opening
    /// a transaction. Direct calls to `BlockEntityRef::set_changed` remain immediate.
    pub(super) fn collect_quick_move_notices<R>(
        &mut self,
        dispatch: impl FnOnce(&mut Self) -> R,
    ) -> (R, Vec<QuickMoveNotice>) {
        let outer = self.transactions.quick_move_notices.replace(Vec::new());
        let scope = QuickMoveNotices { game: self, outer };
        let result = dispatch(&mut *scope.game);
        let notices = scope
            .game
            .transactions
            .quick_move_notices
            .take()
            .expect("active quick-move notice scope");
        (result, notices)
    }

    /// Release accepted writes after source settlement, in their original levels and only to
    /// the same storage rows. Don't enqueue the notices in an enclosing quick move again.
    pub(super) fn publish_quick_move_notices(&mut self, notices: Vec<QuickMoveNotice>) {
        for QuickMoveNotice { dim, holder } in notices {
            let back = self.enter_command_level(dim);
            if self.dim == dim {
                self.publish_holder_changed(holder, false);
            }
            if let Some(back) = back {
                self.enter_command_level(back);
            }
        }
    }

    fn publish_holder_changed(&mut self, holder: BlockEntityIdentity, silent: bool) {
        let [x, y, z] = holder.position;
        let pos = (x, y, z);
        if self.world.block_entities.identity(pos) == Some(holder) {
            if silent {
                self.container_changed_without_hook(pos);
            } else {
                self.container_changed(pos);
            }
        }
    }

    fn container_change_effect(&self, pos: Pos, holder: Option<BlockEntityIdentity>) -> After {
        let silent = !self.contents_changing.is_empty()
            && self.native_level_here().is_some_and(|level| {
                self.contents_notice_active(level, BlockPos::new(pos.0, pos.1, pos.2))
            });
        let holder = holder.or_else(|| {
            // Other writes to a dispenser's slots share its identity-bound notice, so they
            // coalesce with the dispense rather than leaving a positional notice behind.
            let dispenser = &*self
                .res
                .facts()
                .get(self.world.block(pos.0, pos.1, pos.2))
                .name
                == "minecraft:dispenser";
            if dispenser || (!silent && self.transactions.quick_move_notices.is_some()) {
                self.world.block_entities.identity(pos)
            } else {
                None
            }
        });
        if let Some(holder) = holder {
            After::ChangedHolder { holder, silent }
        } else if silent {
            After::ChangedWithoutHook(pos)
        } else {
            After::Changed(pos)
        }
    }

    /// Journal a container notice already bound to the row that received the write.
    pub(super) fn after_holder_changed(&mut self, holder: BlockEntityIdentity) {
        let [x, y, z] = holder.position;
        self.after_commit(self.container_change_effect((x, y, z), Some(holder)));
    }

    /// Runs `after` at once, or at the root commit inside a transaction.
    pub(super) fn after_commit(&mut self, after: After) {
        let after = match after {
            After::Changed(pos) => self.container_change_effect(pos, None),
            after => after,
        };
        let frame = if after.level_owned() {
            entities::open_in(&mut self.transactions, self.dim)
        } else {
            self.transactions.last_mut()
        };
        match frame {
            Some(level) => level.after.push(after),
            None => self.run_after(after),
        }
    }
    fn run_after(&mut self, after: After) {
        match after {
            After::BlockEntityRemoved(effects) => self.apply_removed_block_entity_effects(effects),
            After::PlayerRespawn(conn) => self.commit_player_respawn(conn),
            After::Trade(effect) => self.commit_merchant_effect(*effect),
            After::Payload(payload) => payload.publish(),
            After::Packet(packet, owner) => self.publish_mod_packet(packet, owner),
            After::RecipeBookSettings(owner, player, book, before) => {
                let id = book.as_str();
                let flags = match self.player_storage_home(owner) {
                    Some(PlayerStorageHome::Game) => self.level.recipe_book.custom.get(id),
                    Some(PlayerStorageHome::Guest(index)) => {
                        let host = self.lan.as_ref().expect("resolved guest");
                        host.hub.guests[index].recipe_book.custom.get(id)
                    }
                    None => return,
                };
                if flags != before.unwrap_or_default() {
                    self.publish_player_book_settings(player, book, flags);
                }
            }
            After::Composter(effects) => self.commit_composter_effects(effects),
            After::DispenseEvent(id, pos, data) => self.dispenser_event(id, pos, data),
            After::FluidWrite(written) => self.commit_fluid_write(*written),
            After::FluidEffects(effects) => self.commit_fluid_effects(*effects),
            After::DispenserDrop {
                pos,
                facing,
                spawn,
                play_events,
            } => {
                self.commit_dispensed(pos, facing, *spawn, play_events);
            }
            After::DispenserFeedback { pos, facing, event } => {
                self.commit_dispense_feedback(pos, facing, event);
            }
            After::LevelWrite(write) => self.commit_level_write(*write),
            After::PlacementEffect(effect) => self.publish_placement_effect(*effect),
            After::WitherPattern(found, cells) => {
                let _ = self.finish_wither_pattern(found, &cells);
            }
            After::UpdatePlacedBlockEntity(pos, generation) => {
                if self.world.block_entities.generation(&pos) == Some(generation) {
                    self.block_entity_updated(pos);
                }
            }
            After::BlockGameEvent(id, pos, who, state) => {
                self.post_block_game_event(id, pos, who, Some(state));
            }
            After::DestroyBlockGameEvent {
                pos,
                state,
                breaker,
            } => {
                self.post_destroy_block_game_event(pos, state, breaker);
            }
            After::BlockDropSpawns(items, orbs) => self.publish_block_spawns(items, orbs),
            After::Changed(pos) => self.container_changed(pos),
            After::ChangedWithoutHook(pos) => self.container_changed_without_hook(pos),
            After::ChangedHolder { holder, silent } => {
                if !silent && let Some(queue) = &mut self.transactions.quick_move_notices {
                    queue.push(QuickMoveNotice {
                        dim: self.dim,
                        holder,
                    });
                } else {
                    self.publish_holder_changed(holder, silent);
                }
            }
            After::GuestMenu(conn, effects) => self.publish_guest_menu_effects(conn, *effects),
            After::PersonalSidebar(conn, contents) => {
                if contents.is_some() {
                    self.load_scoreboard();
                }
                let board = &self.score.board;
                if let Some(host) = &mut self.lan {
                    host.hub.update_guest(conn, |guest| {
                        guest.set_personal_sidebar(board, contents.map(|contents| *contents));
                    });
                }
            }
            After::MountEquipment {
                dim,
                id,
                at,
                sound,
                equips,
            } => {
                let _ = self.with_effect_level(dim, |game| {
                    if let Some((sound, center)) = sound {
                        game.audio.play(&sound, Some(center), 1.0, 1.0);
                    }
                    game.mob_equip_event(id, at, equips);
                });
            }
            After::MenuClick(reply) => self.publish_menu_click(*reply),
            After::MenuOpen(conn, identity, open) => self.publish_menu_open(conn, identity, open),
            After::MenuPlacesNotice(conn, uuid, window) => {
                self.queue_menu_places_notice(conn, uuid, window);
            }
            After::FurnaceMenu(effects) => self.publish_furnace_menu_effects(*effects),
            After::StoreCartItems(pos) => self.store_cart_items(pos),
            After::BackedMenuClick(conn, uuid, menu, window) => {
                self.publish_backed_menu_click(conn, uuid, menu, window);
            }
            After::Capabilities(pos) => self.world.capability_generations.invalidate(pos),

            After::BreakProgress(breaker, pos, progress) => {
                self.server_destroy_block_progress(breaker, pos, progress);
            }
            After::Biome(key) => {
                self.commit_biome_columns(&std::collections::HashSet::from([key]), &[key]);
            }
            After::Effect(effect) => match effect {
                DeferredEffect::CustomStat {
                    player,
                    key,
                    amount,
                } => {
                    self.native_as_player(player, |g| g.award_custom_stat(key, amount));
                }
                DeferredEffect::ItemStat {
                    player,
                    stat,
                    item,
                    amount,
                } => {
                    self.native_as_player(player, |g| g.award_stat(stat, &item, amount));
                }
                DeferredEffect::Sound {
                    sound,
                    at,
                    volume,
                    pitch,
                } => self.play_audio(sound, Some(at), volume, pitch),
                DeferredEffect::RestoreStatistics(player, values) => {
                    self.restore_player_statistics(player, *values);
                }
                DeferredEffect::ChangeAdvancement {
                    player,
                    id,
                    criterion,
                    grant,
                } => {
                    self.apply_deferred_advancement_change(
                        player,
                        &id,
                        criterion.as_deref(),
                        grant,
                    );
                }
            },
            // Only reached through a root commit, which replays blocks and connection groups in order.
            After::Block(_) | After::BulkBlocks(..) | After::ConnectionGroup(..) => {}
            After::Drop(pos, stack) => {
                self.spawn_item(stack, pos.map(|p| f64::from(p) + 0.5), [0.0; 3], 10);
            }
            After::GivePickup {
                uuid,
                dim,
                prototype,
            } => {
                self.native_as_player(uuid, |g| {
                    let back = g.enter_command_level(dim);
                    if g.dim == dim {
                        g.give_pickup(prototype);
                    }
                    if let Some(back) = back {
                        g.enter_command_level(back);
                    }
                });
            }
            After::InventoryTrigger(uuid) => {
                self.native_as_player(uuid, |g| g.check_inventory_trigger());
            }
            After::GrindstoneEvent(pos) => self.level_event_at(1042, pos, 0),
            After::StationContent(conn, owner, window) => {
                if self
                    .lan
                    .as_ref()
                    .and_then(|host| host.menus.get(&conn))
                    .is_some_and(|menus| {
                        menus.station.id() == owner
                            && menus.station.as_ref().is_some_and(|s| s.window == window)
                    })
                {
                    self.lan_send_station(conn);
                }
            }
            After::GiveRest {
                uuid,
                rest,
                owner,
                dim,
                eye,
            } => {
                let mut rest = Some(rest);
                self.native_as_player(uuid, |g| {
                    let back = g.enter_command_level(dim);
                    if let Some(rest) = rest.take() {
                        g.give_remainder(rest, owner);
                    }
                    if let Some(back) = back {
                        g.enter_command_level(back);
                    }
                });
                // The stack was accepted (the command succeeded, `give_stack` returned no
                // rest): a player who died or left since, so it was not thrown, still has it
                // land where they were.
                if let Some(rest) = rest {
                    let back = self.enter_command_level(dim);
                    self.give_remainder_unheld(rest, owner, eye);
                    if let Some(back) = back {
                        self.enter_command_level(back);
                    }
                }
            }
            After::PlayerEquip(uuid, slot, stack) => {
                self.native_as_player(uuid, |g| {
                    g.player_equipped(&slot, stack.as_ref());
                    g.apply_equipment();
                });
            }
            After::PlayerDrop(uuid, stack) => {
                // A player who left meanwhile drops nothing; their inventory had changed for good.
                self.native_as_player(uuid, |g| {
                    g.drop_plain(stack);
                });
            }
            After::Spawn(at, stack) => self.spawn_item(stack, at, [0.0; 3], 10),
            After::Emptied(id) => {
                let emptied = self
                    .dropped
                    .iter()
                    .position(|e| e.id == id && e.stack.count == 0);
                if let Some(i) = emptied {
                    self.remove_dropped(i);
                }
            }
            After::EquipmentSync(id) => {
                if let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id && !m.removed) {
                    mob.needs_sync = true;
                }
            }
            After::Equip {
                at,
                sound,
                equips,
                who,
            } => {
                // `equip_item_effects` rebuilt from the entity it names: the
                // sculk context reads the mob as it is at the root commit.
                let ctx = self.who_context(Some(sunrise_client::mobs::Who::Mob(who)), None);
                self.equip_item_effects(at, sound, equips, ctx);
            }
        }
    }
    pub(in crate::game) fn pending_menu_data(&self, key: (u32, u64, usize)) -> Option<i32> {
        self.transactions
            .iter()
            .rev()
            .find_map(|level| level.menu_data.get(&key).copied())
    }

    pub(in crate::game) fn journal_menu_data(
        &mut self,
        key: (u32, u64, usize),
        value: i32,
    ) -> bool {
        let Some(level) = self.transactions.last_mut() else {
            return false;
        };
        level.menu_data.insert(key, value);
        true
    }

    fn commit_menu_data(&mut self, journal: MenuDataJournal) {
        let Some(host) = &mut self.lan else { return };
        for ((conn, generation, index), value) in journal {
            host.hub.update_guest(conn, |guest| {
                if guest.menu_generation == generation {
                    guest.set_menu_data(index, value);
                }
            });
        }
    }

    /// What the mod block entity at `pos` held before a committed transaction that kept
    /// `block_entities` and `slots`: its kept entity, or the live items with the kept slots put
    /// back. `None` when the transaction kept nothing there.
    fn committed_block_items_before(
        &self,
        block_entities: &HashMap<Pos, Kept>,
        slots: &HashMap<(Pos, usize), KeptSlot>,
        pos: Pos,
    ) -> Option<Vec<Option<ItemStack>>> {
        #[cfg(test)]
        super::slot_changes::probe::snapshot();
        if let Some(kept) = block_entities.get(&pos) {
            return match kept.entity.as_ref() {
                Some(BlockEntity::Mod(entity)) => Some(entity.items.clone()),
                _ => None,
            };
        }
        let Some(BlockEntity::Mod(entity)) = self.world.block_entities.get(&pos) else {
            return None;
        };
        let mut before = entity.items.clone();
        let mut patched = false;
        for (&(slot_pos, slot), kept) in slots {
            if slot_pos == pos {
                if before.len() <= slot {
                    before.resize(slot + 1, None);
                }
                before[slot] = kept.stack.clone();
                patched = true;
            }
        }
        patched.then_some(before)
    }
    fn root_commit(&mut self, mut level: Level) {
        // Reserve every deferred open's place before block replay or an older Close can
        // run a callback. The requests are made ready at their existing lifecycle points.
        for after in &level.after {
            if let After::MenuOpen(conn, identity, _) = after {
                self.reserve_menu_publication(*conn, *identity);
            }
        }
        // Storage is final. Publish save marks before any callback can unwind or start a save.
        for (dim, chunk, _) in level.save_chunks {
            let world = if dim == self.dim {
                Some(&mut self.world)
            } else {
                self.levels.parked_mut(dim).map(|level| &mut level.world)
            };
            if let Some(world) = world {
                world.modified.insert(chunk);
            }
        }
        // Captured packets and deferred listeners can refer to a new map.
        for (id, data) in level.new_maps {
            self.publish_map(id, data);
        }
        // Publish captured opens and resends before the final menu deltas, so a
        // snapshot taken before commit cannot overwrite a newer value.
        for packets in level.captured_packets {
            packets.publish();
        }
        self.commit_menu_data(level.menu_data);
        // A cell is rewound to its storage before its first edit when a captured placement
        // writes to it, when a strict edit shares it with an ordinary write, or when a strict
        // cell's connection group is rewound below: replay then shows each callback the block,
        // staged entity and group together. Work back from its final staged row to the row before
        // its first edit. A strict edit or captured creation takes the row it installed from the
        // next retirement, or from storage if it survived. Each removal then observes its own row.
        let mut ordered = indexmap::IndexMap::<Pos, bool>::new();
        for after in &level.after {
            if let After::Block(step) = after
                && (step.placement || level.blocks[&step.pos].strict)
            {
                *ordered.entry(step.pos).or_default() |= step.placement;
            }
        }
        for &pos in level.connection_groups.keys() {
            if level.blocks.get(&pos).is_some_and(|kept| kept.strict) {
                ordered.entry(pos).or_default();
            }
        }
        // Replay from the old group so committed callbacks see writes in order. A fresh
        // publication entry survives only if the final group differs from that baseline.
        let mut clean_groups = Vec::new();
        for (pos @ (x, y, z), was) in level.connection_groups {
            self.world.set_connection_group_stored(x, y, z, was);
            if self
                .world
                .connection_group_log
                .as_ref()
                .is_some_and(|log| !log.contains(&pos))
            {
                clean_groups.push((pos, was));
            }
        }
        // Each row also remembers whether a later write owns its factory. A plain write
        // before that factory must not fill the slot during replay if staging left it empty.
        let mut rows = HashMap::<Pos, (ReplayEntity, bool)>::new();
        for after in level.after.iter_mut().rev() {
            match after {
                After::Block(step) if ordered.contains_key(&step.pos) => {
                    use std::collections::hash_map::Entry;
                    let (row, later_factory) = match rows.entry(step.pos) {
                        Entry::Occupied(entry) => entry.into_mut(),
                        Entry::Vacant(entry)
                            if matches!(step.creation, BlockEntityCreation::Staged { .. })
                                || step.removed.is_some() =>
                        {
                            entry.insert((ReplayEntity::take(&mut self.world, step.pos), false))
                        }
                        Entry::Vacant(_) => continue,
                    };
                    if let BlockEntityCreation::Staged { generation, entity } = &mut step.creation {
                        if generation.is_some()
                            && row.row.as_ref().map(|(_, generation)| *generation) == *generation
                        {
                            *entity = std::mem::take(row);
                        }
                        *later_factory = true;
                    } else if *later_factory {
                        step.creation = BlockEntityCreation::Deferred;
                    }
                    if let Some(removed) = &mut step.removed {
                        *row = std::mem::take(removed);
                    }
                    if step.removed.is_some() || step.replay.clear_connection_group {
                        *later_factory = false;
                    }
                }
                After::BulkBlocks(_, _, _, writes) if !ordered.is_empty() => {
                    for (&pos, step) in writes {
                        if ordered.contains_key(&pos)
                            && let Some(removed) = &mut step.row
                        {
                            let (row, later_factory) = rows.entry(pos).or_insert_with(|| {
                                (ReplayEntity::take(&mut self.world, pos), false)
                            });
                            std::mem::swap(removed, row);
                            *later_factory = false;
                        }
                    }
                }
                _ => {}
            }
        }
        for (&pos, &placement) in &ordered {
            let (x, y, z) = pos;
            self.world.set_stored(x, y, z, level.blocks[&pos].state);
            if let Some((row, _)) = rows.remove(&pos) {
                row.install(&mut self.world, pos, placement);
            }
        }
        for after in &mut level.after {
            if let After::Block(step) = after
                && !ordered.contains_key(&step.pos)
                && let Some(ReplayEntity { row: Some(row), .. }) = step.removed.take()
            {
                let first = level.removed_block_entities.entry(step.pos).or_default();
                if first.is_none() {
                    *first = Some(row);
                }
            }
        }
        let mut told = std::collections::HashSet::new();
        let mut told_silent = FxHashSet::default();
        let mut told_holders = FxHashSet::default();
        let mut explicit_group_since_block = std::collections::HashSet::new();
        let mut biome_columns = std::collections::HashSet::new();
        let mut menu_lifecycle = Vec::new();
        for after in level.after {
            match after {
                // A writable open veto can replace a cell whose old block entity is still
                // waiting to be replayed and spilled. Publish it only after block replay,
                // keeping opens and closes in request order.
                After::MenuOpen(..) => menu_lifecycle.push(after),
                After::MenuClick(ref reply) if reply.closes_menu() => menu_lifecycle.push(after),
                // Each storage tells the world once, after every write.
                After::Changed(pos) if !told.insert(Ok(pos)) => {}
                After::ChangedWithoutHook(pos) if !told_silent.insert(pos) => {}
                After::ChangedHolder { holder, silent }
                    if !told_holders.insert((holder, silent)) => {}
                After::ConnectionGroup(pos @ (x, y, z), group) => {
                    self.world.set_connection_group(x, y, z, group, true);
                    explicit_group_since_block.insert(pos);
                }
                After::Block(step) if ordered.contains_key(&step.pos) => {
                    let (x, y, z) = step.pos;
                    explicit_group_since_block.remove(&step.pos);
                    if step.removed.is_some() && !step.replay.replay_removal {
                        self.world.block_entities.remove(&step.pos);
                    }
                    self.set_block_live_with_entity(
                        [x, y, z],
                        step.to,
                        step.replay.flags,
                        step.replay.level_host,
                        step.creation,
                    );
                    let _ = self.process_level_events();
                }
                After::BulkBlocks(publication, tracked, removed, steps) => {
                    for (pos, step) in steps {
                        if let Some(&placement) = ordered.get(&pos) {
                            let (x, y, z) = pos;
                            self.world.set_stored(x, y, z, step.to);
                            if let Some(row) = step.row {
                                self.world.block_entities.remove(&pos);
                                row.install(&mut self.world, pos, placement);
                            }
                        }
                        if step.clear_connection_group {
                            explicit_group_since_block.remove(&pos);
                            self.world.invalidate_replaced_group(pos);
                        }
                    }
                    self.publish_bulk_edit(publication, tracked, removed);
                }
                After::Block(step) => {
                    let pos = step.pos;
                    let clears_group = step.replay.clear_connection_group;
                    if !told.insert(Err(pos)) {
                        // A later replacement ends the lifetime of a group published
                        // between writes, even though the block's final state is already stored.
                        if clears_group && explicit_group_since_block.remove(&pos) {
                            self.world.invalidate_replaced_group(pos);
                        }
                        continue;
                    }
                    explicit_group_since_block.remove(&pos);
                    let now = self.world.block(pos.0, pos.1, pos.2);
                    let Some(BlockReplay {
                        state: was,
                        flags,
                        level_host,
                        replay_removal,
                        clear_connection_group,
                    }) = level.blocks[&pos].replay
                    else {
                        continue;
                    };
                    // LevelChunk.setBlockState runs preRemoveSideEffects on the resident row
                    // before removeBlockEntity and neighbor effects. Replay with the row as it
                    // stood at removal, or leave it in place if the old block kind survived.
                    let absent_after_writes = self.world.block_entities.get(&pos).is_none()
                        && level.removed_block_entities.contains_key(&pos);
                    let b = self.res.behaviours();
                    let net_keeps_group = b.keeps_block_entity(was, now);
                    let surviving = if now != was && !net_keeps_group {
                        take_replay_row(&mut self.world, pos)
                    } else {
                        None
                    };
                    if (replay_removal || b.block(now) == b.block(was))
                        && let Some((entity, generation)) =
                            level.removed_block_entities.remove(&pos).flatten()
                    {
                        self.restore_removed_block_entity(pos, *entity, generation);
                    }
                    if now != was {
                        // If an intermediate replacement cleared the group even though `was -> now`
                        // preserves groups (e.g. `oak_log[y] -> stone -> oak_log[x]`), clear it
                        // before replaying the block write so any hook on the replay can still set
                        // a new group.
                        if clear_connection_group && net_keeps_group {
                            self.world.invalidate_replaced_group(pos);
                        }
                        // The stored state back, so the one real write sees the change.
                        self.world.set_stored(pos.0, pos.1, pos.2, was);
                        let pos = [pos.0, pos.1, pos.2];
                        if level_host {
                            self.set_block_live(pos, now, flags, true);
                            let _ = self.process_level_events();
                        } else {
                            let _ = self.level_set_with_flags(pos, now, flags);
                        }
                        if self.world.block(pos[0], pos[1], pos[2]) == now {
                            let key = (pos[0], pos[1], pos[2]);
                            if let Some((entity, generation)) = surviving {
                                self.world.block_entities.remove(&key);
                                self.restore_removed_block_entity(key, *entity, generation);
                            } else if absent_after_writes {
                                // If the transaction ended with `BlockEntityRef::remove` at `pos`
                                // after replacing the block (e.g. `set_block(barrel); barrel.remove()`),
                                // the replay above replayed the first block's `preRemoveSideEffects`
                                // and created a fresh entity for `now`; strip that recreated entity
                                // while the final block state still matches `now`.
                                self.world.block_entities.remove(&key);
                            }
                        }
                    } else if clear_connection_group {
                        // A round trip that preserves the block state can still end the
                        // group's lifetime. Its group-only publication sends the current
                        // block to negotiated holders without notifying vanilla peers.
                        self.world.invalidate_replaced_group(pos);
                    }
                }
                After::Biome(key) if !biome_columns.insert(key) => {}
                after => self.run_after(after),
            }
        }
        for (pos @ (x, y, z), was) in clean_groups {
            if self.world.connection_group(x, y, z) == was
                && let Some(log) = self.world.connection_group_log.as_mut()
            {
                log.remove(&pos);
            }
        }
        for ((owner, book), (player, before)) in level.player_books {
            self.run_after(After::RecipeBookSettings(owner, player, book, before));
        }
        // A crafting grid the transaction changed has its result made again, once for each
        // player however many writes it took (`InventoryMenu.slotsChanged`).
        for (conn, kept) in &level.menus {
            let mut guests = self.lan.iter().flat_map(|host| host.hub.guests.iter());
            let changed =
                guests.any(|g| g.placed && g.conn == *conn && g.craft_grid != kept.craft_grid);
            if changed {
                self.guest_craft_result(*conn);
            }
        }
        // `menu_slot_changed` for the open mod menus whose storage this transaction changed: one
        // batch per menu, owned menus by ascending generation, then block-backed menus by
        // ascending position. A touched block costs one lookup in the open-menu index unless a
        // menu is open on it; only then is anything collected or copied.
        if mod_api::__private::has_part_hook(mod_api::HookId::MenuSlotChanged) {
            let owned = std::mem::take(&mut level.native_menu_slots);
            let mut owned_generations: Vec<_> = owned.into_iter().collect();
            owned_generations.sort_by_key(|(generation, _)| *generation);
            for (generation, before) in owned_generations {
                self.notify_owned_menu_slot_changed(generation, &before);
            }
            let mut viewed = Vec::new();
            if let Some(host) = self.lan.as_ref() {
                let touched = level.block_entities.keys().copied();
                for pos in touched.chain(level.slots.keys().map(|&(pos, _)| pos)) {
                    if super::slot_changes::block_menu_viewed(&host.hub.guests, pos)
                        && !viewed.contains(&pos)
                    {
                        viewed.push(pos);
                    }
                }
            }
            viewed.sort_unstable();
            for pos in viewed {
                let (kept, slots) = (&level.block_entities, &level.slots);
                self.notify_block_menu_slot_changed(pos, |game| {
                    game.committed_block_items_before(kept, slots, pos)
                });
            }
        }
        for (pos, kept) in level.block_entities {
            let record = |entity: Option<&BlockEntity>| match entity {
                Some(BlockEntity::Jukebox(j)) => j.record().cloned(),
                _ => None,
            };
            let was = record(kept.entity.as_ref());
            let now = record(self.world.block_entities.get(&pos));
            if was == now {
                continue;
            }
            // The song of the record it had stops; the one it has now starts.
            let pos = [pos.0, pos.1, pos.2];
            if was.is_some() {
                self.jukebox_stopped(pos);
            }
            if now.is_some() {
                self.jukebox_inserted(pos);
            }
        }
        for after in menu_lifecycle {
            self.run_after(after);
        }
    }
    fn roll_back(&mut self, mut level: Level) {
        self.roll_back_in_flight(&mut level);
        self.scratch_roll_back(std::mem::take(&mut level.scratch_kept));
        // Discard deferred effects before undoing tentative action storage.
        level.after.clear();
        // Restore slots and undo only their recorded modifier edits. Explicit
        // attribute writes survive an abort, and no item query runs here.
        let equipment = std::mem::take(&mut level.equipment);
        for ((dim, id), kept) in equipment {
            let _ = self.with_effect_level(dim, |game| match kept {
                KeptEquipment::Mob { slots, gear } => {
                    if let Some(mob) = game.mobs.iter_mut().find(|m| m.id == id && !m.removed) {
                        mob.extra.equipment = *slots;
                        mob.extra.gear = gear;
                    }
                }
                KeptEquipment::Stand(slots) => {
                    if let Some(stand) = game.armor_stands.iter_mut().find(|a| a.id == id) {
                        stand.equipment = *slots;
                    }
                }
            });
        }

        for ((dim, id), kept) in level.mob_effects {
            let _ = self.with_effect_level(dim, |game| {
                if let Some(mob) = game.mobs.iter_mut().find(|mob| mob.id == id) {
                    mob.extra.effects = kept.active;
                    mob.extra.effect_chains = kept.chains;
                    mob.extra.ambient_effects = kept.ambient;
                    mob.extra.hidden_effects = kept.hidden;
                    mob.effects_dirty = kept.dirty;
                }
            });
        }
        for ((dim, id), kept) in level.effect_modifiers.into_iter().rev() {
            let _ = self.with_effect_level(dim, |game| {
                if let Some(mob) = game.mobs.iter_mut().find(|mob| mob.id == id) {
                    kept.restore(mob);
                }
            });
        }
        for ((dim, id), changes) in std::mem::take(&mut level.modifier_changes) {
            let _ = self.with_effect_level(dim, |game| {
                if let Some(mob) = game.mobs.iter_mut().find(|m| m.id == id && !m.removed) {
                    super::living_attributes::undo_mob_modifiers(
                        &game.motion.attributes,
                        mob,
                        changes,
                    );
                }
            });
        }

        for ((x, y, z), tag) in level.saved_pots {
            super::spill::set_saved_entity(&mut self.world, [x, y, z], tag);
        }
        // Block entities whose published shapes the abort changed back.
        let mut reshaped = Vec::new();
        for (pos, kept) in level.block_entities {
            let current = self.world.block_entities.generation(&pos);
            let replaced = current != kept.generation;
            if replaced && current.is_some() {
                self.world.block_entities.remove(&pos);
            }
            if let (Some(BlockEntity::Mod(entity)), Some(fields)) =
                (&kept.entity, kept.client_update_fields)
            {
                entity.set_client_update_fields(fields);
            }
            match (kept.entity, self.world.block_entities.get_mut(&pos)) {
                (Some(entity), Some(now)) => {
                    fn shapes(
                        entity: &BlockEntity,
                    ) -> Option<&sunrise_client::containers::PublishedShapes> {
                        match entity {
                            BlockEntity::Mod(entity) => entity.shapes(),
                            _ => None,
                        }
                    }
                    if shapes(now) != shapes(&entity) {
                        reshaped.push(pos);
                    }
                    *now = entity;
                    self.world
                        .block_entities
                        .restore_generation(pos, kept.generation.unwrap());
                }
                // One this transaction removed comes back.
                (Some(entity), None)
                    if replaced || level.removed_block_entities.contains_key(&pos) =>
                {
                    if matches!(&entity, BlockEntity::Mod(entity) if entity.shapes().is_some()) {
                        reshaped.push(pos);
                    }
                    self.restore_removed_block_entity(pos, entity, kept.generation.unwrap());
                }
                (None, None) if replaced && let Some(taken) = kept.taken => {
                    let entities = &mut self.world.block_entities;
                    entities.restore_taken_lease(pos, taken);
                    entities.reattach_retired(pos, kept.generation.unwrap());
                }
                _ => {}
            }
            self.world
                .block_entities
                .put_back_attachments(pos, kept.generation, kept.attachments);
            match kept.name {
                Some(name) => self.world.block_names.insert(pos, name),
                None => self.world.block_names.remove(&pos),
            };
            // Restoring saved metadata is not a new write. The ordinary storage
            // setter marks its chunk unsaved, including when the extras are empty.
            let chunk = (pos.0 >> 4, pos.2 >> 4);
            let mark = self.world.modified.mark(chunk);
            self.world.set_block_entity_extras(pos, kept.extras);
            self.world.modified.restore(chunk, mark);
            if let Some(observer) = self.world.block_entities.identity(pos)
                && Some(observer.generation) == kept.generation
            {
                self.world
                    .block_entities
                    .contents_watches
                    .restore(observer, kept.contents_watches);
            }
            match kept.hopper {
                Some(hopper) => self.hoppers.insert(pos, hopper),
                None => self.hoppers.remove(&pos),
            };
        }
        // After the whole entities: a slot kept here was kept before any whole capture of its
        // entity at this depth, so its stack is the older one. Only into the block entity it
        // was taken from: one a write replaced (a `set_block` in the transaction) is left as is.
        for ((pos, slot), kept) in level.slots {
            if self.world.block_entities.generation(&pos) != kept.generation {
                continue;
            }
            if let Some(target) = self
                .world
                .block_entities
                .get_mut(&pos)
                .and_then(|entity| entity.slots_mut().get_mut(slot))
            {
                *target = kept.stack;
                #[cfg(test)]
                if let Some(trace) = &self.transactions.trace {
                    trace
                        .lock()
                        .unwrap()
                        .push(format!("restore:{},{},{}:{slot}", pos.0, pos.1, pos.2));
                }
            }
        }
        for ((dim, (qx, qy, qz)), biome) in level.biomes {
            let _ = self.with_effect_level(dim, |game| {
                let bottom_quart = game.world.bottom_y() >> 2;
                if let Some(chunk) = game.world.chunks.get_mut(&(qx >> 2, qz >> 2)) {
                    std::sync::Arc::make_mut(chunk).set_biome_cell(
                        qx & 3,
                        qy - bottom_quart,
                        qz & 3,
                        biome,
                    );
                }
            });
        }
        for (pos, block) in level.blocks {
            self.world.set_stored(pos.0, pos.1, pos.2, block.state);
        }
        for ((x, y, z), group) in level.connection_groups {
            self.world.set_connection_group_stored(x, y, z, group);
        }
        if let Some(random) = level.level_random {
            *self.world.random = random;
        }
        if let Some(sequences) = level.loot_sequences {
            self.rand_sequences = sequences;
        }
        for (owner, before) in level.parked_level_randoms {
            if let Some(random) = self.level_random_mut(owner) {
                *random = before;
            }
        }
        // The shapes are back: the neighbours that read them (a fluid beside it) are told, as
        // the publish told them.
        for pos in reshaped {
            let at = BlockPos::new(pos.0, pos.1, pos.2);
            let state = self.world.block(pos.0, pos.1, pos.2);
            self.with_level(|b, level, ticks| {
                let block = b.block(state);
                b.update_neighbors_at(level, ticks, at, block);
            });
        }
        for (chunk, randoms) in level.composter_randoms {
            self.transactions
                .composter_randoms
                .entry(chunk)
                .or_default()
                .extend(randoms);
        }
        for ((dim, id), kept) in level.entities {
            let _ = self.with_effect_level(dim, |game| {
                game.edit_entity_slots(id, |slots, _| {
                    for (slot, kept) in slots.iter_mut().zip(kept) {
                        *slot = kept;
                    }
                });
            });
        }
        for ((dim, id), len) in level.mob_lengths {
            let _ = self.with_effect_level(dim, |game| {
                if let Some(m) = game.mobs.iter_mut().find(|m| m.id == id) {
                    m.extra.items.truncate(len);
                }
            });
        }
        for (join, kept) in level.item_entities {
            if let Some(index) = self.dropped.locate(join) {
                kept.restore(&mut self.dropped[index]);
            }
        }
        let general = &mut self.resident_entities.general.rows;
        for (join, kept) in level.general_entities {
            if let Some(index) = general.locate(join) {
                let row = &mut general[index];
                if let Some(velocity) = kept.velocity {
                    row.velocity = velocity;
                }
                if let Some(rotation) = kept.rotation {
                    row.rotation = rotation;
                }
                if let Some(fall_distance) = kept.fall_distance {
                    row.fall_distance = fall_distance;
                }
            }
        }
        for ((dim, join), kept) in level.orbs {
            let _ = self.with_effect_level(dim, |game| {
                if let Some(i) = game.orbs.locate(join) {
                    let orb = &mut game.orbs[i];
                    (orb.value, orb.count, orb.age) = kept;
                }
            });
        }
        for (window, drag) in level.mount_drags {
            self.restore_mount_drag(window, drag);
        }
        if let Some(host) = self.lan.as_mut() {
            for ((conn, owner), kept) in level.stations {
                if let Some(menus) = host.menus.get_mut(&conn)
                    && menus.station.id() == owner
                    && menus
                        .station
                        .as_ref()
                        .is_some_and(|s| s.window == kept.window)
                {
                    menus.station.restore(Some(kept));
                }
            }
            for (generation, kept) in level.menu_clicks {
                if let Some(i) = host.hub.guests.connection_index(kept.conn)
                    && host.hub.guests[i].menu_generation == generation
                    && host.hub.guests[i].menu.is_some()
                {
                    host.hub.guests.update(i, |g| {
                        let menu = g.menu.as_mut().unwrap();
                        menu.drag = kept.drag;
                        menu.craft = kept.craft;
                        menu.slot_changed_seen = kept.slot_changed_seen;
                    });
                    let owner = host.menus.entry(kept.conn).or_default();
                    owner.owned_menu = kept.owned;
                    owner.crafting.machine.restore(kept.machine);
                    owner.slot_places = kept.places;
                }
            }
            // A removed window's snapshot is now held by its pending close notice, not
            // by the player. Never restore that window's stacks into a replacement.
            use sunrise_client::lan::menu::NativeMenuEvent;
            if !level.native_menu_slots.is_empty() {
                for event in &mut host.hub.native_menu_events {
                    if let NativeMenuEvent::Closed {
                        generation,
                        owned_slots,
                        ..
                    } = event
                        && !owned_slots.is_empty()
                        && let Some(before) = level.native_menu_slots.remove(generation)
                    {
                        *owned_slots = before;
                    }
                }
            }
            host.hub.guests.update_all(|g| {
                if let Some(kept) = level.menus.get(&g.conn).filter(|_| g.placed) {
                    g.craft_grid.clone_from(&kept.craft_grid);
                }
                if g.placed
                    && g.menu.is_some()
                    && let Some(before) = level.native_menu_slots.remove(&g.menu_generation)
                {
                    g.native_menu = before;
                }
            });
        }
        for (key, kept) in level.attachments {
            match key {
                AttachmentKey::Level(dim, holder) => {
                    let _ = self.with_effect_level(dim, |game| {
                        if let Ok(map) =
                            game.attachment_slot(holder, mod_api::AttachmentAccess::Existing)
                        {
                            map.replace_contents(kept.contents);
                            if !kept.indexed {
                                map.unbind_index();
                            }
                        }
                        if !kept.indexed
                            && let Holder::Entity(id) = holder
                            && let Some(id) = std::num::NonZeroU32::new(id)
                        {
                            game.with_resident_slot(&dim.key(), id, |slot| {
                                *slot = Default::default();
                            });
                        }
                    });
                }
                AttachmentKey::Player(body) => {
                    self.restore_player_attachments(body, kept);
                }
            }
        }
        self.restore_travelled(std::mem::take(&mut level.travelled));
        // After the maps: putting a chunk's map back marks it again.
        for (chunk, mark) in level.chunk_marks {
            self.world.modified.restore(chunk, mark);
        }
        for ((dim, id), kept) in level.mob_health {
            let _ = self.with_effect_level(dim, |game| {
                if let Some(mob) = game.mobs.iter_mut().find(|m| m.id == id) {
                    mob.health = kept.health;
                    mob.absorption = kept.absorption;
                    mob.needs_sync = kept.needs_sync;
                    mob.death_time = kept.death_time;
                    mob.death_canceled = kept.death_canceled;
                    if kept.processed {
                        game.died.insert(id);
                    } else {
                        game.died.remove(&id);
                    }
                }
            });
        }
        for (body, kept) in level.player_health {
            // Restoration must reach that exact player body even when tentatively set to zero
            // health or when another level holds a fake player sharing the same UUID.
            let target = if self.acting_player_body() == body {
                Some(super::commands::Target::Player)
            } else if let PlayerBodyKey::Guest(conn) = body {
                Some(super::commands::Target::Guest(conn))
            } else {
                None
            };
            if let Some(target) = target {
                self.for_player(target, |g| {
                    if let Some(player) = g.player.as_mut() {
                        player.health.data = kept.data;
                        player.events.truncate(kept.events_len);
                        player.health.dead = kept.dead;
                    }
                    if let Some((conn, canceled, time)) = kept.guest
                        && let Some(host) = g.lan.as_mut()
                        && let Some(gi) = host.hub.guests.connection_index(conn)
                    {
                        host.hub.guests.update(gi, |guest| {
                            guest.death_canceled = canceled;
                            guest.death_time = time;
                        });
                    }
                });
            }
        }
        level.actions.rollback_in(self);
        level.trades.rollback(self);
        if let Some(saved) = level.board {
            self.score.board.restore(saved);
            self.sync_score_registries();
        }
        for (owner, kept) in level.ender_items {
            match self.player_storage_home(owner) {
                Some(PlayerStorageHome::Game) => self.ender_items = kept,
                Some(PlayerStorageHome::Guest(index)) => {
                    let host = self.lan.as_mut().expect("resolved guest");
                    host.hub.guests.update(index, |g| g.ender_items = kept);
                }
                None => {}
            }
        }
        for (owner, kept) in level.player_extra {
            match self.player_storage_home(owner) {
                Some(PlayerStorageHome::Game) => self.level.player_extra = kept,
                Some(PlayerStorageHome::Guest(index)) => {
                    let host = self.lan.as_mut().expect("resolved guest");
                    host.hub.guests.update(index, |g| g.saved = kept);
                }
                None => {}
            }
        }
        for ((owner, book), (_, kept)) in level.player_books {
            let id = book.as_str().to_owned();
            match self.player_storage_home(owner) {
                Some(PlayerStorageHome::Game) => self.level.recipe_book.custom.restore(id, kept),
                Some(PlayerStorageHome::Guest(index)) => {
                    let host = self.lan.as_mut().expect("resolved guest");
                    host.hub
                        .guests
                        .update(index, |g| g.recipe_book.custom.restore(id, kept));
                }
                None => {}
            }
        }
        for (owner, kept) in level.players {
            match self.player_storage_home(owner) {
                Some(PlayerStorageHome::Game) => {
                    self.inventory = kept.inventory;
                    self.armor = kept.armor;
                    self.offhand = kept.offhand;
                    self.equipment = kept.equipment;
                    self.carried = kept.carried;
                }
                Some(PlayerStorageHome::Guest(index)) => {
                    let host = self.lan.as_mut().expect("resolved guest");
                    host.hub.guests.update(index, |g| {
                        g.inventory = kept.inventory;
                        g.armor = kept.armor;
                        g.offhand = kept.offhand;
                        g.equipment = kept.equipment;
                        g.carried = kept.carried;
                    });
                }
                None => {}
            }
        }
    }

    /// Identity of the player body currently swapped into `Game`.
    fn acting_player_body(&self) -> PlayerBodyKey {
        if let Some(conn) = self.lan_acting {
            return PlayerBodyKey::Guest(conn);
        }
        if let Some(uuid) = self.lan_acting_uuid
            && let Some(conn) = self
                .lan
                .as_ref()
                .and_then(|host| host.hub.holder_guest(uuid, self.dim.id()))
                .map(|guest| guest.conn)
        {
            return PlayerBodyKey::Guest(conn);
        }
        PlayerBodyKey::Host
    }

    /// Resolves which player body `attachment_slot(Holder::Player(uuid), _)` addresses now:
    /// the acting body when its UUID matches, else the exact parked player, including the
    /// replacement before placement.
    fn attachment_player_body(&self, uuid: u128) -> Option<PlayerBodyKey> {
        if self.lan_acting_uuid == Some(uuid) {
            return Some(self.acting_player_body());
        }
        self.attachment_player_connection(uuid)
            .map(PlayerBodyKey::Guest)
    }

    /// Restore a player body's attachment map directly wherever that exact body's state lives
    /// now (acting body in `player_state` or parked guest `conn` in `lan.hub.guests`), without
    /// looking up by UUID across unrelated per-level fake players.
    fn restore_player_attachments(&mut self, body: PlayerBodyKey, kept: KeptAttachments) {
        let map = if self.acting_player_body() == body {
            Some(&mut self.player_state.attachments)
        } else if let PlayerBodyKey::Guest(conn) = body
            && !self.lan_guest_swaps.contains(&conn)
        {
            self.lan
                .as_mut()
                .and_then(|host| host.hub.guests.attachments_mut(conn))
        } else {
            None
        };
        if let Some(map) = map {
            map.replace_contents(kept.contents);
            if !kept.indexed {
                map.unbind_index();
            }
        }
    }
}

include!("transactions/item_entity.rs");

#[cfg(test)]
#[path = "transactions/player_tests.rs"]
mod player_tests;

#[cfg(test)]
mod tests {
    use super::super::api_values::TryIntoApi;
    use super::super::{Audio, BlockEntity, Game, Options, Progress, Settings, load_resources};
    use mod_api::HostTransaction;
    use mod_api::ServerHost;
    use mod_api::{
        Authority, BlockSide, BlockUpdateFlags, FLUIDS, FluidStack, ITEMS, ItemResource,
        TransactionContext, TransactionError,
    };
    use std::{sync::Arc, time::Instant};
    use sunrise_client::{chunk::Chunk, items::ItemStack};

    #[test]
    #[ignore = "daily"]
    fn packet_refusal_rolls_back_the_child_and_reaches_the_transaction_caller() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
        use mod_api::prelude::BlockStatesHost;
        use std::sync::{atomic::AtomicUsize, mpsc};
        use sunrise_client::lan::{PlayerSender, conn::transaction_packets::Scope};

        if run_fresh_cases(
            "game::transactions::tests::packet_refusal_rolls_back_the_child_and_reaches_the_transaction_caller",
            "SUNRISE_TRANSACTION_PACKET_REFUSAL_CHILD",
            &["run"],
        ) {
            return;
        }
        with_game(|game| {
            const AT: [i32; 3] = [1, 64, 1];
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let before = level.block_state(game, AT);
            let stone = BlockStatesHost::block_state_id(game, "minecraft:stone").unwrap();
            let (send, receive) = mpsc::channel();
            // A real bounded LAN sender with no room left to reserve another packet.
            let sender = PlayerSender::new(send, Arc::new(AtomicUsize::new(usize::MAX)));
            let _packets = Scope::begin();
            let result = game.transaction(|host, _| {
                let child = host.transaction(|host, _| {
                    assert_eq!(
                        level.set_block(host, AT, stone, BlockUpdateFlags::NONE),
                        Ok(true)
                    );
                    assert_eq!(level.block_state(host, AT), Some(stone));
                    sender.send(vec![1]);
                    Ok::<_, ()>(())
                });
                assert_eq!(child, Err(TransactionError::CommitRefused));
                assert_eq!(level.block_state(host, AT), before);
                Ok::<_, ()>(())
            });
            assert_eq!(result, Err(TransactionError::CommitRefused));
            assert_eq!(level.block_state(game, AT), before);
            assert!(receive.try_recv().is_err());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn cauldron_stat_listener_observes_committed_state_and_can_cancel() {
        use super::super::native_entity_refs::tests::run_fresh_cases;
        use crate::game::api_values::TryIntoApi;
        use mod_api::{
            Flow, Hooks, ItemSpawnEvent, ItemTossEvent, ModEntry, Mods, PlayerStatAwardEvent,
        };
        use std::sync::Mutex;
        if run_fresh_cases(
            "game::transactions::tests::cauldron_stat_listener_observes_committed_state_and_can_cancel",
            "SUNRISE_CAULDRON_STAT_CHILD",
            &["run"],
        ) {
            return;
        }
        struct Listener {
            seen: Arc<Mutex<Vec<(Option<mod_api::BlockState>, Option<String>)>>>,
            drops: Arc<Mutex<Vec<(&'static str, Option<mod_api::BlockState>)>>>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn player_stat_award(
                &self,
                host: &mut dyn ServerHost,
                event: &mut PlayerStatAwardEvent<'_>,
            ) -> Flow {
                if event.stat.as_str() != "minecraft:fill_cauldron" {
                    return Flow::Continue;
                }
                let level = host.level("minecraft:overworld").unwrap();
                let hand = mod_api::ItemPlace::PlayerHand {
                    player: event.player,
                    hand: mod_api::client::first_person::Hand::Main,
                };
                self.seen.lock().unwrap().push((
                    level.block_state(host, [2, 64, 2]),
                    hand.with_stack(host, |s| {
                        mod_api::__private::item_key_id(s.item())
                            .unwrap()
                            .to_string()
                    }),
                ));
                Flow::Cancel
            }
            fn item_toss(&self, host: &mut dyn ServerHost, event: &mut ItemTossEvent) -> Flow {
                let potion = event.item.with_stack(host, |stack| {
                    mod_api::__private::item_key_id(stack.item()) == Some("minecraft:potion")
                });
                if potion == Some(true) {
                    let level = host.level("minecraft:overworld").unwrap();
                    self.drops
                        .lock()
                        .unwrap()
                        .push(("toss", level.block_state(host, [2, 64, 2])));
                }
                Flow::Continue
            }
            fn item_spawn(&self, host: &mut dyn ServerHost, event: &mut ItemSpawnEvent<'_>) {
                if event.item == "minecraft:potion" {
                    let level = host.level("minecraft:overworld").unwrap();
                    self.drops
                        .lock()
                        .unwrap()
                        .push(("spawn", level.block_state(host, [2, 64, 2])));
                }
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let drops = Arc::new(Mutex::new(Vec::new()));
        let mods: Mods = vec![ModEntry::new(
            "observer",
            Box::new(Listener {
                seen: seen.clone(),
                drops: drops.clone(),
            }),
        )]
        .into();
        mod_platform::__private::install(mods);
        super::super::native_guest_turn_tests::with_guests(|game| {
            let pos = [2, 64, 2];
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let empty = game
                .pack
                .assets
                .registry
                .default_state("minecraft:cauldron")
                .unwrap();
            let full = game
                .pack
                .assets
                .registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:water_cauldron".into(),
                    properties: [("level".into(), "3".into())].into_iter().collect(),
                })
                .unwrap();
            game.world.set(pos[0], pos[1], pos[2], empty).unwrap();
            game.as_guest(12, |game| {
                game.selected = 0;
                game.creative = false;
                game.inventory[0] = Some(ItemStack::new("minecraft:water_bucket", 1));
                game.open_storage_transaction();
                assert_eq!(
                    game.use_on_cauldron(pos).unwrap(),
                    sunrise_client::cauldron::Outcome::Success
                );
                assert!(seen.lock().unwrap().is_empty());
                game.close_storage_transaction(false);
                assert!(seen.lock().unwrap().is_empty());
                assert_eq!(
                    game.use_on_cauldron(pos).unwrap(),
                    sunrise_client::cauldron::Outcome::Success
                );
                assert_eq!(
                    seen.lock().unwrap().as_slice(),
                    &[(
                        Some(empty.try_into_api().unwrap()),
                        Some("minecraft:bucket".into())
                    )]
                );
                game.world.set(pos[0], pos[1], pos[2], empty).unwrap();
                game.inventory[0] = Some(ItemStack::new("minecraft:water_bucket", 1));
                game.open_storage_transaction();
                assert_eq!(
                    game.use_on_cauldron(pos).unwrap(),
                    sunrise_client::cauldron::Outcome::Success
                );
                assert_eq!(seen.lock().unwrap().len(), 1);
                game.close_storage_transaction(true);
            })
            .unwrap();
            assert_eq!(
                *seen.lock().unwrap(),
                vec![
                    (
                        Some(empty.try_into_api().unwrap()),
                        Some("minecraft:bucket".into())
                    ),
                    (
                        Some(full.try_into_api().unwrap()),
                        Some("minecraft:bucket".into())
                    )
                ]
            );
            let level2 = game
                .pack
                .assets
                .registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:water_cauldron".into(),
                    properties: [("level".into(), "2".into())].into_iter().collect(),
                })
                .unwrap();
            game.as_guest(12, |game| {
                let setup = |game: &mut Game<'_>| {
                    game.world.set(pos[0], pos[1], pos[2], full).unwrap();
                    game.inventory
                        .iter_mut()
                        .for_each(|s| *s = Some(ItemStack::new("minecraft:stone", 64)));
                    game.offhand = Some(ItemStack::new("minecraft:stone", 64));
                    game.inventory[0] = Some(ItemStack::new("minecraft:glass_bottle", 2));
                };
                setup(game);
                assert_eq!(
                    game.use_on_cauldron(pos).unwrap(),
                    sunrise_client::cauldron::Outcome::Success
                );
                assert_eq!(
                    drops.lock().unwrap().as_slice(),
                    &[
                        ("toss", Some(full.try_into_api().unwrap())),
                        ("spawn", Some(full.try_into_api().unwrap()))
                    ]
                );
                drops.lock().unwrap().clear();
                setup(game);
                game.open_storage_transaction();
                assert_eq!(
                    game.use_on_cauldron(pos).unwrap(),
                    sunrise_client::cauldron::Outcome::Success
                );
                assert!(drops.lock().unwrap().is_empty());
                game.close_storage_transaction(false);
                assert!(drops.lock().unwrap().is_empty());
                game.open_storage_transaction();
                assert_eq!(
                    game.use_on_cauldron(pos).unwrap(),
                    sunrise_client::cauldron::Outcome::Success
                );
                game.close_storage_transaction(true);
                assert_eq!(
                    drops.lock().unwrap().as_slice(),
                    &[
                        ("toss", Some(level2.try_into_api().unwrap())),
                        ("spawn", Some(level2.try_into_api().unwrap()))
                    ]
                );
            })
            .unwrap();
            assert_eq!(
                game.lan.as_ref().unwrap().play[&12]
                    .stats
                    .get("custom", "minecraft:fill_cauldron"),
                0
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn cauldron_effects_wait_for_root_and_abort_with_inventory() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let pos = [2, 64, 2];
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let empty = game
                .pack
                .assets
                .registry
                .default_state("minecraft:cauldron")
                .unwrap();
            game.world.set(pos[0], pos[1], pos[2], empty).unwrap();
            game.player = Some(
                super::super::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::new(0.5, 65.0, 3.5),
                )
                .unwrap(),
            );
            game.selected = 0;
            game.creative = false;
            game.inventory[0] = Some(ItemStack::new("minecraft:water_bucket", 1));
            let before = game.stats.get("custom", "minecraft:fill_cauldron");
            game.audio.capture = Some(Vec::new());
            game.open_storage_transaction();
            assert_eq!(
                game.use_on_cauldron(pos).unwrap(),
                sunrise_client::cauldron::Outcome::Success
            );
            assert_eq!(game.stats.get("custom", "minecraft:fill_cauldron"), before);
            assert!(game.audio.capture.as_ref().unwrap().is_empty());
            game.close_storage_transaction(false);
            assert_eq!(game.world.block(pos[0], pos[1], pos[2]), empty);
            assert_eq!(
                &*game.inventory[0].as_ref().unwrap().item,
                "minecraft:water_bucket"
            );
            assert_eq!(game.stats.get("custom", "minecraft:fill_cauldron"), before);
            assert!(game.audio.capture.as_ref().unwrap().is_empty());
            game.open_storage_transaction();
            game.open_storage_transaction();
            assert_eq!(
                game.use_on_cauldron(pos).unwrap(),
                sunrise_client::cauldron::Outcome::Success
            );
            game.close_storage_transaction(false);
            assert_eq!(game.world.block(pos[0], pos[1], pos[2]), empty);
            assert_eq!(
                &*game.inventory[0].as_ref().unwrap().item,
                "minecraft:water_bucket"
            );
            game.close_storage_transaction(true);
            assert_eq!(game.stats.get("custom", "minecraft:fill_cauldron"), before);
            assert!(game.audio.capture.as_ref().unwrap().is_empty());
            assert_eq!(
                game.use_on_cauldron(pos).unwrap(),
                sunrise_client::cauldron::Outcome::Success
            );
            assert_ne!(game.world.block(pos[0], pos[1], pos[2]), empty);
            assert_eq!(
                &*game.inventory[0].as_ref().unwrap().item,
                "minecraft:bucket"
            );
            assert_eq!(
                game.stats.get("custom", "minecraft:fill_cauldron"),
                before + 1
            );
            assert_eq!(
                game.audio
                    .capture
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(|s| s.event.as_str())
                    .collect::<Vec<_>>(),
                ["minecraft:item.bucket.empty"]
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_menu_write_outside_a_transaction_keeps_no_journal() {
        super::super::native_guest_turn_tests::with_guests(|game| {
            let player = game.player(12).unwrap();
            let menu = game.player_container_menu(player).unwrap();
            let stone = ItemStack::new("minecraft:stone", 4);
            game.journal_menus(player);
            *game.native_menu_storage(menu, 1).unwrap() = Some(stone.clone());
            assert!(game.transactions.is_empty());
            assert_eq!(
                menu.slot(game, 1),
                Some(Some(stone.try_into_api().unwrap()))
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn menu_journals_keep_only_the_first_write_inside_a_transaction() {
        super::super::native_guest_turn_tests::with_guests(|game| {
            let player = game.player(12).unwrap();
            let menu = game.player_container_menu(player).unwrap();
            let stone = ItemStack::new("minecraft:stone", 4);
            *game.native_menu_storage(menu, 1).unwrap() = Some(stone.clone());
            let place = mod_api::ItemPlace::MenuSlot { menu, slot: 1 };
            let resource = ItemResource::of(&stone.try_into_api().unwrap());
            let aborted: Result<(), ()> = game.storage_transaction(|game, tx| {
                assert_eq!(place.insert(game, tx, &resource, 2), Ok(2));
                assert_eq!(place.insert(game, tx, &resource, 3), Ok(3));
                let journal = &game.transactions.last().unwrap().menus;
                assert_eq!(journal.len(), 1);
                assert_eq!(journal[&12].craft_grid[0], Some(stone.clone()));
                assert_eq!(menu.slot(game, 1).unwrap().unwrap().count(), 9);
                Err(())
            });
            assert_eq!(aborted, Err(()));
            assert!(game.transactions.is_empty());
            assert_eq!(
                menu.slot(game, 1),
                Some(Some(stone.try_into_api().unwrap()))
            );
        });
    }

    fn insert(
        host: &mut dyn ServerHost,
        at: BlockSide,
        slot: usize,
        stack: ItemStack,
        tx: &TransactionContext,
    ) -> Option<ItemStack> {
        let handler = at
            .level
            .capability(host, &ITEMS, at.pos, at.side)
            .expect("item capability");
        let inserted = handler
            .insert_at(
                host,
                tx,
                slot,
                &ItemResource::of(&stack.try_into_api().unwrap()),
                stack.count,
            )
            .unwrap();
        (inserted < stack.count).then(|| ItemStack {
            count: stack.count - inserted,
            ..stack
        })
    }

    fn extract(
        host: &mut dyn ServerHost,
        at: BlockSide,
        slot: usize,
        amount: u32,
        tx: &TransactionContext,
    ) -> Option<mod_api::ItemStack> {
        let handler = at
            .level
            .capability(host, &ITEMS, at.pos, at.side)
            .expect("item capability");
        let resource = handler.resource(host, slot)?;
        let taken = handler
            .extract_at(host, tx, slot, &resource, amount)
            .unwrap();
        resource.to_stack(taken)
    }

    fn extract_fluid(
        host: &mut dyn ServerHost,
        at: BlockSide,
        tank: usize,
        amount: u32,
        tx: &TransactionContext,
    ) -> Option<FluidStack> {
        let handler = at
            .level
            .capability(host, &FLUIDS, at.pos, at.side)
            .expect("fluid capability");
        let resource = handler.resource(host, tank)?;
        let taken = handler
            .extract_at(host, tx, tank, &resource, amount)
            .unwrap();
        (taken > 0).then(|| resource.to_stack(taken))
    }

    fn chest_items(game: &Game<'_>) -> Vec<(usize, u32)> {
        let Some(BlockEntity::Chest(items)) = game.world.block_entities.get(&(0, 64, 0)) else {
            panic!("chest block entity missing")
        };
        let held = items.iter().enumerate();
        held.filter_map(|(i, s)| Some((i, s.as_ref()?.count)))
            .collect()
    }

    #[test]
    #[ignore = "daily"]
    fn container_limits_preserve_capacity_and_transaction_rollback() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            for (pos, id) in [
                ([0, 64, 0], "minecraft:chest"),
                ([4, 64, 0], "minecraft:jukebox"),
            ] {
                let state = game.pack.assets.registry.default_state(id).unwrap();
                game.level_set_block(level, pos, state, BlockUpdateFlags::NEIGHBORS)
                    .unwrap();
            }
            let mut large = ItemStack::new("minecraft:stick", 1);
            large.data_mut().set_other(
                "minecraft:max_stack_size",
                Some(sunrise_core::nbt::Tag::Int(99)),
            );
            let large = ItemResource::of(&large.try_into_api().unwrap());
            let handler = level.capability(game, &ITEMS, [0, 64, 0], None).unwrap();
            assert_eq!(handler.capacity(game, 0, Some(&large)), 99);
            assert_eq!(game.world.container_stack_limit((0, 64, 0)), None);
            let stick =
                ItemResource::of(&ItemStack::new("minecraft:stick", 1).try_into_api().unwrap());
            let host: &mut dyn ServerHost = game;
            let handler = level
                .capability(host, &mod_api::ITEMS, [0, 64, 0], None)
                .unwrap();
            let aborted = host.transaction(|host, tx| {
                assert_eq!(handler.set_max_stack_size(host, tx, 10), Ok(true));
                assert_eq!(handler.insert_at(host, tx, 0, &stick, 64), Ok(10));
                host.transaction(|host, tx| {
                    assert_eq!(handler.set_max_stack_size(host, tx, 4), Ok(true));
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(handler.capacity(host, 0, Some(&stick)), 4);
                Err::<(), ()>(())
            });
            assert_eq!(aborted, Err(TransactionError::Aborted(())));
            assert_eq!(handler.capacity(host, 0, Some(&stick)), 64);
            assert_eq!(handler.capacity(host, 0, Some(&large)), 99);
            assert_eq!(handler.amount(host, 0), 0);
            host.transaction(|host, tx| {
                assert_eq!(handler.set_max_stack_size(host, tx, 10), Ok(true));
                let _ = host.transaction(|host, tx| {
                    assert_eq!(handler.set_max_stack_size(host, tx, 4), Ok(true));
                    Err::<(), ()>(())
                });
                assert_eq!(handler.capacity(host, 0, Some(&stick)), 10);
                for invalid in [0, 100, u32::MAX] {
                    assert_eq!(handler.set_max_stack_size(host, tx, invalid), Ok(false));
                    assert_eq!(handler.capacity(host, 0, Some(&stick)), 10);
                }
                assert_eq!(handler.set_max_stack_size(host, tx, 64), Ok(true));
                assert_eq!(handler.insert_at(host, tx, 0, &stick, 20), Ok(20));
                assert_eq!(handler.set_max_stack_size(host, tx, 10), Ok(true));
                assert_eq!(handler.amount(host, 0), 20);
                assert_eq!(handler.insert_at(host, tx, 0, &stick, 1), Ok(0));
                assert_eq!(handler.extract_at(host, tx, 0, &stick, 15), Ok(15));
                assert_eq!(handler.amount(host, 0), 5);
                assert_eq!(handler.insert_at(host, tx, 0, &stick, 64), Ok(5));
                Err::<(), ()>(())
            })
            .unwrap_err();

            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _: Result<(), TransactionError<()>> = host.transaction(|host, tx| {
                    assert_eq!(handler.set_max_stack_size(host, tx, 10), Ok(true));
                    assert_eq!(handler.insert_at(host, tx, 0, &stick, 64), Ok(10));
                    panic!("abort container writes");
                });
            }));
            assert!(panic.is_err());
            assert_eq!(handler.capacity(host, 0, Some(&stick)), 64);
            assert_eq!(handler.capacity(host, 0, Some(&large)), 99);
            assert_eq!(handler.amount(host, 0), 0);
            host.transaction(|host, tx| {
                let pearl = ItemResource::of(
                    &ItemStack::new("minecraft:ender_pearl", 1)
                        .try_into_api()
                        .unwrap(),
                );
                assert_eq!(handler.set_max_stack_size(host, tx, 32), Ok(true));
                assert_eq!(handler.insert_at(host, tx, 0, &pearl, 64), Ok(16));
                let disc = level.capability(host, &ITEMS, [4, 64, 0], None).unwrap();
                assert_eq!(disc.set_max_stack_size(host, tx, 10), Ok(true));
                let record = ItemResource::of(
                    &ItemStack::new("minecraft:music_disc_13", 1)
                        .try_into_api()
                        .unwrap(),
                );
                assert_eq!(disc.insert_at(host, tx, 0, &record, 64), Ok(1));
                Err::<(), ()>(())
            })
            .unwrap_err();
            // Player clicks and shift-clicks use the menu's separate slot rules.
            let env = game.menu_env();
            let mut menu = sunrise_client::menu::Menu::new(sunrise_client::menu::Kind::Chest(1));
            menu.set_container_limits(vec![Some(10); 9]);
            menu.carried = Some(ItemStack::new("minecraft:stick", 20));
            menu.click(&env, 0, 0, sunrise_client::menu::Input::Pickup)
                .unwrap();
            assert_eq!(menu.slot(0).unwrap().count, 10);
            assert_eq!(menu.carried.as_ref().unwrap().count, 10);
            menu.carried = None;
            menu.set_slot(&env, 9, Some(ItemStack::new("minecraft:stick", 20)));
            menu.click(&env, 9, 0, sunrise_client::menu::Input::QuickMove)
                .unwrap();
            assert_eq!(menu.slot(1).unwrap().count, 10);
            assert_eq!(menu.slot(2).unwrap().count, 10);
            let hopper = sunrise_world::chunk::BlockState {
                name: "minecraft:hopper".into(),
                properties: [("facing".into(), "east".into())].into_iter().collect(),
            };
            let hopper = game.pack.assets.registry.resolve(&hopper).unwrap();
            let chest = game
                .pack
                .assets
                .registry
                .default_state("minecraft:chest")
                .unwrap();
            let flags = BlockUpdateFlags::NEIGHBORS;
            game.level_set_block(level, [0, 64, 4], hopper, flags)
                .unwrap();
            if let Some(BlockEntity::Chest(items)) = game.world.block_entities.get_mut(&(0, 64, 4))
            {
                items[0] = Some(ItemStack::new("minecraft:stick", 2));
            }
            let sticks = |count| ItemStack::new("minecraft:stick", count);
            let slots = |game: &Game<'_>, pos| match game.world.block_entities.get(&pos) {
                Some(BlockEntity::Chest(items)) => {
                    items.iter().flatten().map(|s| s.count).sum::<u32>()
                }
                _ => panic!("missing chest"),
            };
            assert_eq!(
                game.level_set_block(level, [1, 64, 4], chest, flags),
                Ok(true)
            );
            if let Some(BlockEntity::Chest(items)) = game.world.block_entities.get_mut(&(1, 64, 4))
            {
                items.fill(Some(sticks(10)));
                items[0] = Some(sticks(9));
            }
            let target = level.capability(game, &ITEMS, [1, 64, 4], None).unwrap();
            game.storage_transaction(|host, tx| {
                assert_eq!(target.set_max_stack_size(host, tx, 10), Ok(true));
                Ok::<_, ()>(())
            })
            .unwrap();
            game.hoppers.remove(&(0, 64, 4));
            game.hopper_try_move_items((0, 64, 4));
            assert_eq!(slots(game, (1, 64, 4)), 270);
            let _ = game.storage_transaction(|host, tx| {
                assert_eq!(target.set_max_stack_size(host, tx, 64), Ok(true));
                Err::<(), ()>(())
            });
            game.hoppers.remove(&(0, 64, 4));
            game.hopper_try_move_items((0, 64, 4));
            assert_eq!(slots(game, (1, 64, 4)), 270);
            assert_eq!(slots(game, (0, 64, 4)), 1);
            // Dropper uses add_item, dispenser ejects into the world instead.
            let dropper = sunrise_world::chunk::BlockState {
                name: "minecraft:dropper".into(),
                properties: [("facing".into(), "east".into())].into_iter().collect(),
            };
            let dropper = game.pack.assets.registry.resolve(&dropper).unwrap();
            game.level_set_block(level, [0, 64, 4], dropper, flags)
                .unwrap();
            if let Some(BlockEntity::Chest(items)) = game.world.block_entities.get_mut(&(0, 64, 4))
            {
                items[0] = Some(sticks(2));
            }
            game.dispense_from(sunrise_core::position::BlockPos::new(0, 64, 4), dropper)
                .unwrap();
            assert_eq!(slots(game, (1, 64, 4)), 270);
            assert_eq!(slots(game, (0, 64, 4)), 2);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_aborted_transaction_puts_back_what_it_and_its_committed_children_changed() {
        let Some(root) = super::super::native_entity_refs::tests::declared_part_root(
            "game::transactions::tests::an_aborted_transaction_puts_back_what_it_and_its_committed_children_changed",
        ) else {
            return;
        };
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            super::super::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = super::super::execution_role::ExecutionRole::IntegratedServer;
        options.dedicated = Some(Vec::new());
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
        )
        .unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.native_lifecycle
            .begin_open(Some(Authority::IntegratedServer));
        super::super::native_entity_refs::tests::resolve_parts(&game).unwrap();
        // Pins the opaque backend: a real game always hands one out, so a None
        // from Host::backend is a bug, never a fallback.
        assert!(mod_api::Host::backend(&game).is_some());
        assert!(ServerHost::backend_mut(&mut game).is_some());
        let level = game.level("minecraft:overworld").unwrap();
        let registry = &game.pack.assets.registry;
        let chest = registry.default_state("minecraft:chest").unwrap();
        let full = sunrise_world::chunk::BlockState {
            name: "minecraft:water_cauldron".to_owned(),
            properties: [("level".to_owned(), "3".to_owned())].into_iter().collect(),
        };
        let full = registry.resolve(&full).unwrap();
        let empty = registry.default_state("minecraft:cauldron").unwrap();
        let jukebox = registry.default_state("minecraft:jukebox").unwrap();
        let flags = BlockUpdateFlags::NEIGHBORS;
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], chest, flags),
            Ok(true)
        );
        assert_eq!(
            game.level_set_block(level, [2, 64, 0], full, flags),
            Ok(true)
        );
        assert_eq!(
            game.level_set_block(level, [4, 64, 0], jukebox, flags),
            Ok(true)
        );
        let sticks = |count| ItemStack::new("minecraft:stick", count);
        let stick = ItemResource::of(&sticks(1).try_into_api().unwrap());
        let rod = ItemResource::of(
            &ItemStack::new("minecraft:blaze_rod", 1)
                .try_into_api()
                .unwrap(),
        );
        let on = |pos, side| BlockSide { level, pos, side };
        let host: &mut dyn ServerHost = &mut game;

        // Five in, then a committed child takes two and an aborted one adds three: the
        // outer abort puts all of it back, the cauldron's water too.
        let refused: Result<(), TransactionError<()>> = host.transaction(|host, tx| {
            assert!(insert(host, on([0, 64, 0], None), 0, sticks(5), tx).is_none());
            let taken = host
                .transaction(|host, tx| Ok::<_, ()>(extract(host, on([0, 64, 0], None), 0, 2, tx)));
            assert_eq!(taken, Ok(Some(sticks(2).try_into_api().unwrap())));
            let _ = host.transaction(|host, tx| {
                insert(host, on([0, 64, 0], None), 1, sticks(3), tx);
                Err::<(), ()>(())
            });
            let water = extract_fluid(host, on([2, 64, 0], None), 0, 1000, tx);
            assert_eq!(water.map(|w| w.amount), Some(1000));
            Err(())
        });
        assert_eq!(refused, Err(TransactionError::Aborted(())));
        assert_eq!(chest_items(&game), []);
        assert_eq!(game.world.block(2, 64, 0), full);
        assert!(game.transactions.is_empty());

        // The same, committed: what the children left stands, the cauldron is set for good.
        let host: &mut dyn ServerHost = &mut game;
        let done: Result<(), TransactionError<()>> = host.transaction(|host, tx| {
            insert(host, on([0, 64, 0], None), 0, sticks(5), tx);
            let _ = host.transaction(|host, tx| {
                extract(host, on([0, 64, 0], None), 0, 2, tx);
                Ok::<_, ()>(())
            });
            let _ = host.transaction(|host, tx| {
                insert(host, on([0, 64, 0], None), 1, sticks(3), tx);
                Err::<(), ()>(())
            });
            extract_fluid(host, on([2, 64, 0], None), 0, 1000, tx);
            Ok(())
        });
        assert_eq!(done, Ok(()));
        assert_eq!(chest_items(&game), [(0, 3)]);
        assert_eq!(game.world.block(2, 64, 0), empty);

        // A transaction a mod opened through the helpers and never closed is aborted before
        // the next tick.
        game.open_storage_transaction();
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| {
            Ok::<_, ()>(insert(host, on([0, 64, 0], None), 0, sticks(1), tx))
        });
        game.abort_open_transactions();
        assert_eq!(chest_items(&game), [(0, 3)]);

        // A panic caught outside the root: what it changed is gone before the catch returns,
        // and the transaction is closed.
        let host: &mut dyn ServerHost = &mut game;
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            host.transaction(|host, tx| -> Result<(), ()> {
                insert(host, on([0, 64, 0], None), 0, sticks(1), tx);
                panic!("a broken handler")
            })
        }));
        assert!(caught.is_err());
        assert_eq!(chest_items(&game), [(0, 3)]);
        assert!(game.transactions.is_empty());

        // `transaction_matches` checks the exact innermost open frame only, and passing an
        // outer/ancestor `TransactionContext` to a write inside a nested transaction is refused
        // cleanly without modifying state.
        let host: &mut dyn ServerHost = &mut game;
        assert!(host.transaction_matches(None));
        let ancestor_check: Result<(), TransactionError<()>> = host.transaction(|host, outer| {
            assert!(!host.transaction_matches(None));
            assert!(host.transaction_matches(Some(outer)));
            let inner_ok: Result<(), TransactionError<()>> = host.transaction(|host, inner| {
                assert!(!host.transaction_matches(None));
                assert!(!host.transaction_matches(Some(outer)));
                assert!(host.transaction_matches(Some(inner)));
                let handler = level
                    .capability(host, &ITEMS, [0, 64, 0], None)
                    .expect("item capability");
                assert_eq!(
                    handler.insert_at(
                        host,
                        outer,
                        0,
                        &ItemResource::of(&sticks(1).try_into_api().unwrap()),
                        1,
                    ),
                    Err(TransactionError::InvalidContext)
                );
                Ok(())
            });
            assert_eq!(inner_ok, Ok(()));
            assert!(host.transaction_matches(Some(outer)));
            Ok(())
        });
        assert_eq!(ancestor_check, Ok(()));
        assert!(game.transaction_matches(None));
        assert_eq!(chest_items(&game), [(0, 3)]);
        assert!(game.transactions.is_empty());

        // A panic inside a nested `storage_transaction` rolls back only the inner frame;
        // the outer frame stays open and keeps its own writes until it closes.
        let outer_aborted: Result<(), ()> = game.storage_transaction(|game, outer| {
            assert!(insert(game, on([0, 64, 0], None), 0, sticks(1), outer).is_none());
            assert_eq!(chest_items(game), [(0, 4)]);
            let inner_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _: Result<(), ()> = game.storage_transaction(|game, inner| {
                    assert!(insert(game, on([0, 64, 0], None), 0, sticks(2), inner).is_none());
                    assert_eq!(chest_items(game), [(0, 6)]);
                    panic!("nested storage panic");
                });
            }));
            assert!(inner_panic.is_err());
            assert!(game.transaction_matches(Some(outer)));
            assert_eq!(chest_items(game), [(0, 4)]);
            Err(())
        });
        assert_eq!(outer_aborted, Err(()));
        assert_eq!(chest_items(&game), [(0, 3)]);
        assert!(game.transactions.is_empty());

        // An attachment map a write made inside an aborted transaction is gone with it.
        let chest_holder = mod_api::Holder::BlockEntity([0, 64, 0]);
        let chest_key = sunrise_mod_runtime::attachments::AttachmentType {
            namespace: "transaction_test",
            name: "chest",
            default: || 0u32,
            codec: None,
            copy_on_death: false,
            sync_codec: None,
        };
        // So is the unsaved mark it put on a clean chunk, through a committed child too.
        assert!(game.world.block_entities.attachments((0, 64, 0)).is_none());
        game.world.modified.clear();
        game.open_storage_transaction();
        game.open_storage_transaction();
        chest_key.set(&mut game, chest_holder, 31).unwrap();
        assert!(game.world.modified.contains(&(0, 0)));
        game.close_storage_transaction(true);
        assert!(game.world.block_entities.attachments((0, 64, 0)).is_some());
        game.close_storage_transaction(false);
        assert!(game.world.block_entities.attachments((0, 64, 0)).is_none());
        assert!(!game.world.modified.contains(&(0, 0)), "clean again");
        // A committed one keeps it.
        game.open_storage_transaction();
        chest_key.set(&mut game, chest_holder, 31).unwrap();
        game.close_storage_transaction(true);
        assert!(game.world.modified.contains(&(0, 0)));
        game.world.block_entities.set_attachments((0, 64, 0), None);

        // An aborted cauldron change leaves no write behind in the world's queues.
        assert_eq!(
            game.level_set_block(level, [2, 64, 0], full, flags),
            Ok(true)
        );
        game.world.block_log = Some(Vec::new());
        game.world.path_writes = Some(Vec::new());
        let queues = |game: &Game<'_>| {
            let world = &game.world;
            (
                world.block_log.as_ref().map(Vec::len),
                world.path_writes.as_ref().map(Vec::len),
                world.light_edits.len(),
                world.dirty_light.len(),
                world.watched_writes.len(),
            )
        };
        let before = queues(&game);
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| {
            extract_fluid(host, on([2, 64, 0], None), 0, 1000, tx);
            Err::<(), ()>(())
        });
        assert_eq!(game.world.block(2, 64, 0), full);
        assert_eq!(queues(&game), before);

        // A disc swapped for another in one transaction changes the song: the old one's
        // stops and the new one's starts.
        let disc = |item: &str| ItemStack::new(item, 1);
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| {
            let cat = disc("minecraft:music_disc_cat");
            Ok::<_, ()>(insert(host, on([4, 64, 0], None), 0, cat, tx))
        });
        let Some(BlockEntity::Jukebox(j)) = game.world.block_entities.get_mut(&(4, 64, 0)) else {
            panic!("jukebox block entity missing")
        };
        assert!(j.playing, "a disc put in through a transfer plays");
        j.ticks = 500;
        let host: &mut dyn ServerHost = &mut game;
        let swapped = host.transaction(|host, tx| {
            extract(host, on([4, 64, 0], None), 0, 1, tx);
            Ok::<_, ()>(insert(
                host,
                on([4, 64, 0], None),
                0,
                disc("minecraft:music_disc_13"),
                tx,
            ))
        });
        assert_eq!(swapped, Ok(None));
        let Some(BlockEntity::Jukebox(j)) = game.world.block_entities.get(&(4, 64, 0)) else {
            panic!("jukebox block entity missing")
        };
        assert_eq!(
            j.record().map(|r| &*r.item),
            Some("minecraft:music_disc_13")
        );
        assert!(j.playing);
        assert_eq!(j.ticks, 0, "the new disc's song starts from its beginning");

        // A player's place: one of three buckets in the hand is exchanged for a filled one; the
        // hand keeps two and the filled one goes to the inventory, and what doesn't fit would be
        // thrown. Aborted, the hand and the inventory are as they were, and nothing was thrown.
        game.player = Some(
            super::super::PlayerMotion::new(
                game.motion,
                sunrise_core::geometry::Vec3::new(0.5, 65.0, 3.5),
            )
            .unwrap(),
        );
        let bucket = |item: &str, count| ItemStack::new(item, count);
        game.inventory
            .iter_mut()
            .for_each(|slot| *slot = Some(bucket("minecraft:stone", 64)));
        game.selected = 0;
        game.inventory[0] = Some(bucket("minecraft:bucket", 3));
        game.carried = None;
        let before = (
            game.inventory.clone(),
            game.offhand.clone(),
            game.carried.clone(),
        );
        let dropped = game.dropped.len();
        let me = game.native_acting_id();
        let hand = mod_api::ItemPlace::PlayerHand {
            player: me,
            hand: mod_api::client::first_person::Hand::Main,
        };
        let water = ItemResource::of(&bucket("minecraft:water_bucket", 1).try_into_api().unwrap());
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| {
            assert_eq!(hand.exchange(host, tx, &water, 1), Ok(1));
            Err::<(), ()>(())
        });
        assert_eq!(
            (
                game.inventory.clone(),
                game.offhand.clone(),
                game.carried.clone()
            ),
            before
        );
        assert_eq!(
            game.dropped.len(),
            dropped,
            "a thrown bucket waits for the root commit"
        );

        // Committed, the split stands and the full inventory throws the filled bucket.
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| hand.exchange(host, tx, &water, 1));
        assert_eq!(game.inventory[0], Some(bucket("minecraft:bucket", 2)));
        assert_eq!(game.dropped.len(), dropped + 1);
        let thrown = &game.dropped[game.dropped.len() - 1].stack;
        assert_eq!(&*thrown.item, "minecraft:water_bucket");

        // A chest minecart's, a chest boat's and a chested donkey's slots: an abort puts back
        // what an insert changed; a commit keeps an exchange, whose new item goes into the next
        // free slot.
        let mut chested = sunrise_core::nbt::Compound::new();
        chested.insert("ChestedHorse".into(), sunrise_core::nbt::Tag::Byte(1));
        let vehicles = [
            (
                "minecraft:chest_minecart",
                sunrise_core::nbt::Compound::new(),
            ),
            (
                "minecraft:oak_chest_boat",
                sunrise_core::nbt::Compound::new(),
            ),
            ("minecraft:donkey", chested),
        ];
        // An aborted write leaves a chested donkey's item list as long as it was.
        let level = game.level("minecraft:overworld").unwrap();
        let id = game
            .level_summon(level, "minecraft:donkey", [8.5, 70.0, 8.5], {
                let mut chested = sunrise_core::nbt::Compound::new();
                chested.insert("ChestedHorse".into(), sunrise_core::nbt::Tag::Byte(1));
                chested
            })
            .unwrap()
            .unwrap()
            .id();
        let len = |game: &Game<'_>| {
            game.mobs
                .iter()
                .find(|m| m.id == id)
                .unwrap()
                .extra
                .items
                .len()
        };
        // Its list is short (none kept yet): reading it as slots grows it, and the abort puts
        // that back too.
        let mob = game.mobs.iter_mut().find(|m| m.id == id).unwrap();
        mob.extra.items.clear();
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| {
            let place = mod_api::ItemPlace::EntitySlot {
                entity: id,
                slot: 0,
            };
            assert!(place.with_stack(host, |_| ()).is_none());
            assert_eq!(place.insert(host, tx, &stick, 1), Ok(1));
            Err::<(), ()>(())
        });
        assert_eq!(len(&game), 0);
        // Grown three transactions deep, the middle one committing: the root's abort still
        // truncates it.
        let place = mod_api::ItemPlace::EntitySlot {
            entity: id,
            slot: 0,
        };
        let host: &mut dyn ServerHost = &mut game;
        let result = host.transaction(|host, _| {
            host.transaction(|host, _| {
                host.transaction(|host, tx| {
                    assert!(place.with_stack(host, |_| ()).is_none());
                    assert_eq!(place.insert(host, tx, &stick, 1), Ok(1));
                    Ok::<_, ()>(())
                })
                .unwrap();
                Ok::<_, ()>(())
            })
            .unwrap();
            Err::<(), ()>(())
        });
        assert!(result.is_err());
        assert_eq!(len(&game), 0);

        for (kind, nbt) in vehicles {
            let id = game
                .level_summon(level, kind, [8.5, 70.0, 8.5], nbt)
                .unwrap()
                .unwrap_or_else(|| panic!("{kind} not summoned"))
                .id();
            let place = |slot| mod_api::ItemPlace::EntitySlot { entity: id, slot };
            let host: &mut dyn ServerHost = &mut game;
            let went = host.transaction(|host, tx| place(0).insert(host, tx, &stick, 5));
            assert_eq!(went, Ok(5), "{kind}");
            let _ = host.transaction(|host, tx| {
                assert_eq!(place(0).insert(host, tx, &stick, 4), Ok(4), "{kind}");
                assert_eq!(place(1).insert(host, tx, &stick, 2), Ok(2), "{kind}");
                Err::<(), ()>(())
            });
            let host: &mut dyn ServerHost = &mut game;
            assert_eq!(
                place(0).with_stack(host, |stack| stack.project()).flatten(),
                Some(sticks(5).try_into_api().unwrap()),
                "{kind}"
            );
            assert_eq!(place(1).with_stack(host, |_| ()), None, "{kind}");
            let _ = host.transaction(|host, tx| place(0).exchange(host, tx, &rod, 1));
            let host: &mut dyn ServerHost = &mut game;
            assert_eq!(
                place(0).with_stack(host, |stack| stack.project()).flatten(),
                Some(sticks(4).try_into_api().unwrap()),
                "{kind}"
            );
            let rod =
                place(1).with_stack(host, |s| mod_api::__private::item_key_id(s.item()).unwrap());
            assert_eq!(rod, Some("minecraft:blaze_rod"), "{kind}");
        }

        // An item entity: emptied in an aborted transaction, it's back whole; one of four
        // exchanged in a committed one is dropped beside it at the commit; emptied in a
        // committed one, it's gone.
        game.spawn_item(sticks(4), [0.5, 70.0, 6.5], [0.0; 3], 10);
        let id = game.dropped.last().unwrap().id;
        let entity = mod_api::ItemPlace::ItemEntity { id };
        let count = |game: &Game<'_>| game.dropped.len();
        let items = count(&game);
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| {
            assert_eq!(entity.extract(host, tx, &stick, 4), Ok(4));
            assert!(entity.with_stack(host, |_| ()).is_none());
            Err::<(), ()>(())
        });
        let host: &mut dyn ServerHost = &mut game;
        assert_eq!(
            entity.with_stack(host, |stack| stack.project()).flatten(),
            Some(sticks(4).try_into_api().unwrap())
        );
        let _ = host.transaction(|host, tx| entity.exchange(host, tx, &rod, 1));
        let host: &mut dyn ServerHost = &mut game;
        assert_eq!(
            entity.with_stack(host, |stack| stack.project()).flatten(),
            Some(sticks(3).try_into_api().unwrap())
        );
        assert_eq!(count(&game), items + 1);
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| entity.extract(host, tx, &stick, 3));
        assert!(game.dropped.iter().all(|e| e.id != id));
        assert_eq!(count(&game), items);

        // A hopper moving into a mod's part: a truthful part (a pipe into the chest east of
        // it) takes the item; a part answering more than it was offered has its move undone,
        // so the item stays in the hopper and the chest it wrote is as it was.
        let hopper = sunrise_world::chunk::BlockState {
            name: "minecraft:hopper".to_owned(),
            properties: [("facing".to_owned(), "east".to_owned())]
                .into_iter()
                .collect(),
        };
        let hopper = game.pack.assets.registry.resolve(&hopper).unwrap();
        let chest = game
            .pack
            .assets
            .registry
            .default_state("minecraft:chest")
            .unwrap();
        assert_eq!(
            game.level_set_block(level, [0, 64, 4], hopper, flags),
            Ok(true)
        );
        assert_eq!(
            game.level_set_block(level, [2, 64, 4], chest, flags),
            Ok(true)
        );
        let slots = |game: &Game<'_>, pos| match game.world.block_entities.get(&pos) {
            Some(BlockEntity::Chest(items)) => items.iter().flatten().map(|s| s.count).sum::<u32>(),
            _ => panic!("no container at {pos:?}"),
        };
        for (id, moved) in [("test:pipe", 1), ("test:greedy", 0)] {
            let state = game.pack.assets.registry.default_state(id).unwrap();
            assert_eq!(
                game.level_set_block(level, [1, 64, 4], state, mod_api::BlockUpdateFlags::NONE),
                Ok(true)
            );
            if let Some(BlockEntity::Chest(items)) = game.world.block_entities.get_mut(&(0, 64, 4))
            {
                items[0] = Some(sticks(2));
            }
            if let Some(BlockEntity::Chest(items)) = game.world.block_entities.get_mut(&(2, 64, 4))
            {
                items.fill(None);
            }
            game.hoppers.remove(&(0, 64, 4));
            game.hopper_try_move_items((0, 64, 4));
            assert_eq!(slots(&game, (0, 64, 4)), 2 - moved, "{id}");
            assert_eq!(slots(&game, (2, 64, 4)), moved, "{id}");
            assert!(game.transactions.is_empty());
        }
    }
}

#[cfg(test)]
mod mod_capability_tests {
    use super::super::native_entity_refs::tests::with_game;
    use mod_api::ServerHost;
    use mod_api::{BlockSide, HostTransaction, ResourceHandler, resource_util::move_resources};
    use mod_platform::__private::part_tests::{CHEMICALS, ChemTank, Chemical};
    use std::sync::Arc;
    use sunrise_client::chunk::Chunk;
    use sunrise_client::containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields};
    use sunrise_core::nbt::{Compound, Tag};

    /// Two mods sharing a capability one of them defines, as Mekanism's chemicals are shared
    /// through its API: the first's tank and the second's vat each answer `CHEMICALS`, and a
    /// move between them in a committed transaction moves the chemical, kept in their saved
    /// data; in an aborted one, both are put back.
    #[test]
    #[ignore = "daily"]
    fn a_chemical_moves_between_two_mods_parts_and_an_abort_puts_it_back() {
        use super::super::native_entity_refs::tests::{declared_part_root, with_game_at};
        let Some(root) = declared_part_root(
            "game::transactions::mod_capability_tests::a_chemical_moves_between_two_mods_parts_and_an_abort_puts_it_back",
        ) else {
            return;
        };
        with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let hydrogen = Chemical("test:hydrogen".into());
            let mut nbt = Compound::new();
            nbt.insert("chemical".into(), Tag::String("test:hydrogen".into()));
            nbt.insert("amount".into(), Tag::Int(600));
            for (x, id, nbt) in [
                (0, "test:chem_tank", nbt),
                (1, "test:chem_vat", Compound::new()),
            ] {
                let state = game.pack.assets.registry.default_state(id).unwrap();
                game.world.set(x, 64, 0, state).unwrap();
                let fields = ModBlockEntityFields {
                    nbt,
                    ..Default::default()
                };
                let entity = BlockEntity::Mod(Box::new(ModBlockEntity::new(id.into(), fields)));
                game.world.block_entities.insert((x, 64, 0), entity);
            }
            let host: &mut dyn ServerHost = game;
            let tank = level
                .capability(host, &CHEMICALS, [0, 64, 0], None)
                .unwrap();
            let vat = level
                .capability(host, &CHEMICALS, [1, 64, 0], None)
                .unwrap();
            // Each answered by its own mod.
            assert_ne!(tank.owner(), vat.owner());
            assert!(![tank.owner(), vat.owner()].contains(&"the game".to_owned()));
            let amounts = |host: &mut dyn ServerHost| {
                let side = |x| BlockSide {
                    level,
                    pos: [x, 64, 0],
                    side: None,
                };
                (
                    ChemTank(side(0)).amount(host, 0),
                    ChemTank(side(1)).amount(host, 0),
                )
            };

            let moved = host.transaction(|host, tx| {
                move_resources(host, &tank, &vat, |c| *c == hydrogen, 250, Some(tx)).map_err(drop)
            });
            assert_eq!(moved, Ok(250));
            assert_eq!(amounts(host), (350, 250));
            assert_eq!(vat.resource(host, 0), Some(hydrogen.clone()));

            let aborted = host.transaction(|host, tx| {
                let moved = move_resources(host, &tank, &vat, |_| true, 100, Some(tx));
                assert_eq!(moved.ok(), Some(100));
                assert_eq!(amounts(host), (250, 350));
                Err::<(), ()>(())
            });
            assert!(aborted.is_err());
            assert_eq!(amounts(host), (350, 250));
            assert!(game.transactions.is_empty());
        });
    }

    /// A mod's attachments on an entity and on a player are kept by a transaction as a block
    /// entity's are: a commit keeps what it wrote, an abort puts each map back as it was before
    /// the transaction first reached it (none for an entity that had none), and a guest's is
    /// put back wherever their state lives at the abort, swapped in for their turn or not.
    #[test]
    #[ignore = "daily"]
    fn entity_and_player_attachments_stay_on_commit_and_go_back_on_abort() {
        use mod_api::{AttachmentDescriptor, AttachmentKey, Holder, Hooks, ModEntry, Registrar};
        use std::sync::Mutex;
        use sunrise_client::entities::ArmorStand;
        use sunrise_core::geometry::Vec3;
        struct EnergyMod(Arc<Mutex<Option<AttachmentKey<i32>>>>);
        #[mod_api::__subscriptions]
        impl Hooks for EnergyMod {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let key = r
                    .register_attachment_type(AttachmentDescriptor::new("robot:energy", || 0i32))
                    .unwrap();
                *self.0.lock().unwrap() = Some(key);
            }
        }
        let energy_slot = Arc::new(Mutex::new(None));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "robot",
            Box::new(EnergyMod(energy_slot.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let energy_key = energy_slot
                    .lock()
                    .unwrap()
                    .clone()
                    .expect("EnergyMod registered");
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.armor_stands.push(ArmorStand::new(13, Vec3::ZERO, 0.0));
                game.armor_stands.push(ArmorStand::new(14, Vec3::ZERO, 0.0));
                let guest = 0x5eed_u128;
                let config = sunrise_client::lan::hub::HubConfig {
                    default_game_mode: 0,
                    guest_commands: false,
                    force_game_mode: false,
                    hardcore: false,
                    view_distance: 10,
                    simulation_distance: 10,
                    dimension: "minecraft:overworld".into(),
                    allow_flight: false,
                    seed: 0,
                    max_players: 20,
                    integrated: false,
                    levels: Default::default(),
                    level_seeds: Default::default(),
                };
                game.lan_host(
                    super::super::lan::LanSettings {
                        port: 0,
                        guest_commands: false,
                        force_game_mode: false,
                    },
                    sunrise_client::lan::NetConfig {
                        announce: false,
                        ..sunrise_client::lan::NetConfig::lan(0, "test".into(), 2)
                    },
                    config,
                )
                .unwrap();
                let level = game.level(game.dim.id()).unwrap();
                let player_ref = game.fake_player(level, guest, "test").unwrap();
                let conn = game
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guests
                    .iter()
                    .find(|g| g.entity_id as u32 == player_ref.entity().id())
                    .unwrap()
                    .conn;
                let (stand, fresh, player) = (
                    Holder::Entity(13),
                    Holder::Entity(14),
                    Holder::Player(guest),
                );
                let energy = |host: &mut dyn ServerHost, holder| {
                    energy_key
                        .get_existing(host, holder, |value| value.cloned())
                        .unwrap()
                };

                let host: &mut dyn ServerHost = game;
                let done = host.transaction(|host, _| {
                    energy_key.set(host, stand, 100)?;
                    energy_key.set(host, player, 40)
                });
                assert_eq!(done, Ok(()));
                assert_eq!(
                    (energy(host, stand), energy(host, player)),
                    (Some(100), Some(40))
                );

                // Aborted, with a committed child inside: both maps as they were, and the entity
                // that had no value has none.
                let aborted: Result<(), _> = host.transaction(|host, _| {
                    energy_key.set(host, stand, 250)?;
                    energy_key.set(host, player, 90)?;
                    energy_key.set(host, fresh, 7)?;
                    host.transaction(|host, _| energy_key.set(host, stand, 300))
                        .unwrap();
                    assert_eq!(energy(host, stand), Some(300));
                    Err(mod_api::AttachmentError::MissingHolder)
                });
                assert!(aborted.is_err());
                assert_eq!(
                    (energy(host, stand), energy(host, player)),
                    (Some(100), Some(40))
                );
                assert_eq!(energy(host, fresh), None);

                // Written while the guest's state is swapped in for their turn, aborted after it
                // is swapped back out: the guest's own map is put back.
                game.storage_scope(|game, _| {
                    game.lan_acting_uuid = Some(guest);
                    std::mem::swap(
                        &mut game.player_state.attachments,
                        game.lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .attachments_mut(conn)
                            .unwrap(),
                    );
                    let host: &mut dyn ServerHost = game;
                    energy_key.set(host, player, 70).unwrap();
                    std::mem::swap(
                        &mut game.player_state.attachments,
                        game.lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .attachments_mut(conn)
                            .unwrap(),
                    );
                    game.lan_acting_uuid = None;
                    ((), false)
                });
                let host: &mut dyn ServerHost = game;
                assert_eq!(energy(host, player), Some(40));
                assert!(game.transactions.is_empty());
            });
        });
    }
}
