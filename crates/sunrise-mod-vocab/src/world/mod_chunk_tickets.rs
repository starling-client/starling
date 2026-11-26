//! Moved here from the game so mods can name it.
use std::collections::{BTreeMap, BTreeSet};
// no wire: encoded by the existing manual Wire implementation on TicketOwner.
/// What a ticket belongs to: a block (a machine) or an entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
// exhaustive: a closed set the game matches without a wildcard
pub enum TicketOwner {
    /// The block at `[x, y, z]`.
    Block([i32; 3]),
    /// The entity with this UUID.
    Entity(u128),
}
/// One chunk a mod holds loaded for an owner.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ModTicket {
    /// The controller that holds it: its slot in the installed controller table.
    pub controller: u32,
    pub owner: TicketOwner,
    pub chunk: (i32, i32),
    /// Whether the chunk joins the spawning chunks (inhabited time, thunder,
    /// a spawn attempt) with no player within 128 blocks
    /// (`forceChunk`'s `forceNaturalSpawning`). One owner holds the two kinds
    /// apart.
    pub natural_spawning: bool,
}
/// The chunks one owner holds, the two kinds apart (NeoForge's `TicketSet`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct TicketSet {
    /// Chunks held without natural spawning.
    pub non_natural_spawning: Vec<[i32; 2]>,
    /// Chunks held with natural spawning forced.
    pub natural_spawning: Vec<[i32; 2]>,
}
impl TicketSet {
    fn push(&mut self, chunk: [i32; 2], natural_spawning: bool) {
        if natural_spawning {
            self.natural_spawning.push(chunk);
        } else {
            self.non_natural_spawning.push(chunk);
        }
    }
}
/// The saved tickets lent to one controller's [`LoadingValidationCallback`](crate::LoadingValidationCallback)
/// (NeoForge's `TicketHelper`). The view contains only that controller's
/// staged rows, with block and entity owners apart. [`block_tickets`](Self::block_tickets)
/// and [`entity_tickets`](Self::entity_tickets) lend read-only maps of the
/// original saved tickets for the whole invocation and stay unchanged after
/// removals during the callback. Each successful removal excludes that row
/// from subsequent removals. Selections publish only when the callback
/// completes successfully; a published removal never forces its chunk.
pub trait TicketValidation {
    /// The controller's original saved block tickets: each block position with
    /// the chunks it held without and with natural spawning
    /// (`TicketHelper.getBlockTickets`' `TicketSet`: `nonNaturalSpawning`,
    /// `naturalSpawning`), in position order. Read-only and unchanged by
    /// removals during this validation call.
    fn block_tickets(&self) -> &BTreeMap<[i32; 3], TicketSet>;
    /// The controller's original saved entity tickets: each entity UUID with
    /// the chunks it held without and with natural spawning
    /// (`TicketHelper.getEntityTickets`), in UUID order. Read-only and
    /// unchanged by removals during this validation call.
    fn entity_tickets(&self) -> &BTreeMap<u128, TicketSet>;
    /// Remove every ticket the block holds, of both kinds
    /// (`removeAllTickets(BlockPos)`): how many went.
    fn remove_block_tickets(&mut self, pos: [i32; 3]) -> usize;
    /// Remove every ticket the entity holds, of both kinds
    /// (`removeAllTickets(UUID)`).
    fn remove_entity_tickets(&mut self, uuid: u128) -> usize;
    /// Remove the block's ticket of one kind on one chunk
    /// (`removeTicket(BlockPos, ChunkPos, forceNaturalSpawning)`): whether it
    /// held one. The other kind on the chunk stays.
    fn remove_block_ticket(
        &mut self,
        pos: [i32; 3],
        chunk: [i32; 2],
        natural_spawning: bool,
    ) -> bool;
    /// Remove the entity's ticket of one kind on one chunk
    /// (`removeTicket(UUID, ChunkPos, forceNaturalSpawning)`).
    fn remove_entity_ticket(&mut self, uuid: u128, chunk: [i32; 2], natural_spawning: bool)
    -> bool;
}

// no wire: borrows one controller's staged rows during native validation.
/// The game's [`TicketValidation`]: one controller's staged tickets.
// hidden: a game-side helper; mods receive the TicketValidation trait.
#[doc(hidden)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ControllerTickets<'a> {
    loaded: &'a mut Vec<ModTicket>,
    block_tickets: BTreeMap<[i32; 3], TicketSet>,
    entity_tickets: BTreeMap<u128, TicketSet>,
    selected: BTreeSet<usize>,
}
impl std::fmt::Debug for ControllerTickets<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControllerTickets")
            .field("loaded", &self.loaded.len())
            .finish()
    }
}
impl<'a> ControllerTickets<'a> {
    /// Borrow the staged tickets of one controller. For the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(loaded: &'a mut Vec<ModTicket>) -> Self {
        let mut block_tickets = BTreeMap::<[i32; 3], TicketSet>::new();
        let mut entity_tickets = BTreeMap::<u128, TicketSet>::new();
        for row in loaded.iter() {
            let chunk = [row.chunk.0, row.chunk.1];
            match row.owner {
                TicketOwner::Block(pos) => block_tickets
                    .entry(pos)
                    .or_default()
                    .push(chunk, row.natural_spawning),
                TicketOwner::Entity(uuid) => entity_tickets
                    .entry(uuid)
                    .or_default()
                    .push(chunk, row.natural_spawning),
            }
        }
        Self {
            loaded,
            block_tickets,
            entity_tickets,
            selected: BTreeSet::new(),
        }
    }

    /// Publish the staged removals after a successful callback. For the game.
    // hidden: engine-side commit step after a ticket validation callback succeeds.
    #[doc(hidden)]
    pub fn publish_removals(self) {
        if self.selected.is_empty() {
            return;
        }
        let mut index = 0;
        self.loaded.retain(|_| {
            let keep = !self.selected.contains(&index);
            index += 1;
            keep
        });
    }
}
impl TicketValidation for ControllerTickets<'_> {
    fn block_tickets(&self) -> &BTreeMap<[i32; 3], TicketSet> {
        &self.block_tickets
    }
    fn entity_tickets(&self) -> &BTreeMap<u128, TicketSet> {
        &self.entity_tickets
    }
    fn remove_block_tickets(&mut self, pos: [i32; 3]) -> usize {
        self.remove(TicketOwner::Block(pos), None)
    }
    fn remove_entity_tickets(&mut self, uuid: u128) -> usize {
        self.remove(TicketOwner::Entity(uuid), None)
    }
    fn remove_block_ticket(
        &mut self,
        pos: [i32; 3],
        chunk: [i32; 2],
        natural_spawning: bool,
    ) -> bool {
        self.remove(TicketOwner::Block(pos), Some((chunk, natural_spawning))) > 0
    }
    fn remove_entity_ticket(
        &mut self,
        uuid: u128,
        chunk: [i32; 2],
        natural_spawning: bool,
    ) -> bool {
        self.remove(TicketOwner::Entity(uuid), Some((chunk, natural_spawning))) > 0
    }
}
impl ControllerTickets<'_> {
    fn remove(&mut self, owner: TicketOwner, kind: Option<([i32; 2], bool)>) -> usize {
        let before = self.selected.len();
        for (index, row) in self.loaded.iter().enumerate() {
            if row.owner == owner
                && kind.is_none_or(|([x, z], spawning)| {
                    row.chunk == (x, z) && row.natural_spawning == spawning
                })
            {
                self.selected.insert(index);
            }
        }
        self.selected.len() - before
    }
}
