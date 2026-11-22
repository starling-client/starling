//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
use std::collections::BTreeMap;
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
    /// The controller that holds it: its slot in the installed controller table
    /// (`mod_api::TicketController::slot`).
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
/// The tickets one controller loaded, as `validate_tickets` hands them out
/// (NeoForge's `TicketHelper`, one for each controller). Every call acts as
/// the mod marked as running, and only the controller's owning mod sees or
/// removes anything, block owners and entity owners apart. The game releases
/// what a callback removed only if it completes successfully.
pub trait TicketValidation {
    /// The controller's block tickets: each block position with the chunks it
    /// holds without and with natural spawning (`TicketHelper.getBlockTickets`'
    /// `TicketSet`: `nonNaturalSpawning`, `naturalSpawning`), in position order.
    fn block_tickets(&self) -> Vec<([i32; 3], TicketSet)>;
    /// The controller's entity tickets: each entity UUID with the chunks it
    /// holds without and with natural spawning (`TicketHelper.getEntityTickets`),
    /// in UUID order.
    fn entity_tickets(&self) -> Vec<(u128, TicketSet)>;
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

// no wire: controller-owned mutable ticket bookkeeping retains static mod ownership; individual ModTicket values cross instead.
/// The game's [`TicketValidation`]: one controller's loaded tickets, and those removed.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ControllerTickets {
    /// The `[mod] id` that owns the controller.
    owner: &'static str,
    loaded: Vec<ModTicket>,
    removed: Vec<ModTicket>,
    // Loaded index -> removal order for the current callback.
    selected: BTreeMap<usize, usize>,
}
/// Shows only how many tickets the calling mod's controller holds, never another mod's.
impl std::fmt::Debug for ControllerTickets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControllerTickets")
            .field("own", &self.own().count())
            .finish()
    }
}
impl ControllerTickets {
    /// A validation of the `loaded` tickets of one controller, owned by mod
    /// `owner`. For the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(owner: &'static str, loaded: Vec<ModTicket>) -> Self {
        Self {
            owner,
            loaded,
            removed: Vec::new(),
            selected: BTreeMap::new(),
        }
    }
    /// Publish one callback's removals only if it completed successfully. For the game.
    // hidden: the engine validates the loaded tickets; mods receive only TicketValidation.
    #[doc(hidden)]
    pub fn register_listener(&mut self, listen: &mut dyn FnMut(&mut dyn TicketValidation) -> bool) {
        if listen(self) && !self.selected.is_empty() {
            let mut ordered = vec![None; self.selected.len()];
            let mut loaded_index = 0;
            let mut orders = self.selected.values().copied();
            for ticket in self.loaded.extract_if(.., |_| {
                let selected = self.selected.contains_key(&loaded_index);
                loaded_index += 1;
                selected
            }) {
                ordered[orders.next().expect("extracted a selected ticket")] = Some(ticket);
            }
            self.removed.extend(ordered.into_iter().flatten());
        }
        self.selected.clear();
    }

    /// The tickets removed. For the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn into_removed(self) -> Vec<ModTicket> {
        self.removed
    }

    /// The tickets that survived validation. For the game.
    // hidden: the engine consumes the validated tickets after all callbacks have run.
    #[doc(hidden)]
    pub fn into_loaded(self) -> Vec<ModTicket> {
        self.loaded
    }
}
impl TicketValidation for ControllerTickets {
    fn block_tickets(&self) -> Vec<([i32; 3], TicketSet)> {
        let mut grouped = BTreeMap::<[i32; 3], TicketSet>::new();
        for (owner, chunk, spawning) in self.own() {
            if let TicketOwner::Block(pos) = owner {
                grouped.entry(pos).or_default().push(chunk, spawning);
            }
        }
        grouped.into_iter().collect()
    }
    fn entity_tickets(&self) -> Vec<(u128, TicketSet)> {
        let mut grouped = BTreeMap::<u128, TicketSet>::new();
        for (owner, chunk, spawning) in self.own() {
            if let TicketOwner::Entity(uuid) = owner {
                grouped.entry(uuid).or_default().push(chunk, spawning);
            }
        }
        grouped.into_iter().collect()
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
impl ControllerTickets {
    fn own(&self) -> impl Iterator<Item = (TicketOwner, [i32; 2], bool)> + '_ {
        let owned = crate::content::crash::current_native_mod() == Some(self.owner);
        self.loaded
            .iter()
            .enumerate()
            .filter(move |(index, _)| owned && !self.selected.contains_key(index))
            .map(|(_, t)| (t.owner, [t.chunk.0, t.chunk.1], t.natural_spawning))
    }
    fn remove(&mut self, owner: TicketOwner, kind: Option<([i32; 2], bool)>) -> usize {
        if crate::content::crash::current_native_mod() != Some(self.owner) {
            return 0;
        }
        let start = self.selected.len();
        let mut next = start;
        for (index, ticket) in self.loaded.iter().enumerate() {
            if ticket.owner == owner
                && kind.is_none_or(|([x, z], spawning)| {
                    ticket.chunk == (x, z) && ticket.natural_spawning == spawning
                })
                && let std::collections::btree_map::Entry::Vacant(entry) =
                    self.selected.entry(index)
            {
                entry.insert(next);
                next += 1;
            }
        }
        next - start
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeHost, with_dispatch};
    use std::sync::{Arc, Mutex};

    #[test]
    fn a_panicking_ticket_validator_leaves_loaded_tickets_and_other_controllers_untouched() {
        struct Validator {
            slot: u32,
            seen: Arc<Mutex<Vec<u32>>>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Validator {
            fn validate_tickets(
                &self,
                _: &mut dyn ServerHost,
                event: &mut ValidateTicketsEvent<'_>,
            ) {
                let _owner = crate::dispatch_scope::ModGuard::fixture_owner(
                    crate::content::crash::ModCall {
                        id: if self.slot == 0 { "failing" } else { "healthy" },
                        hook: "validate_tickets",
                        part: None,
                    },
                );
                let slot = event.controller.slot();
                let removed = event
                    .validation
                    .remove_block_ticket([1, 64, 1], [2, 3], false);
                assert_eq!(removed, slot == self.slot);
                assert!(event.validation.block_tickets().is_empty());
                assert!(
                    !event
                        .validation
                        .remove_block_ticket([1, 64, 1], [2, 3], false)
                );
                self.seen.lock().unwrap().push(slot);
                if removed && slot == 0 {
                    panic!("failed after a valid selection");
                }
            }
        }

        let seen_a = Arc::new(Mutex::new(Vec::new()));
        let seen_b = Arc::new(Mutex::new(Vec::new()));
        with_dispatch(
            vec![
                ModEntry::new(
                    "failing",
                    Box::new(Validator {
                        slot: 0,
                        seen: seen_a.clone(),
                    }),
                ),
                ModEntry::new(
                    "healthy",
                    Box::new(Validator {
                        slot: 1,
                        seen: seen_b.clone(),
                    }),
                ),
            ]
            .into(),
            || {
                let mut host = FakeHost::new();
                let ticket = |controller| ModTicket {
                    controller,
                    owner: TicketOwner::Block([1, 64, 1]),
                    chunk: (2, 3),
                    natural_spawning: false,
                };
                let mut a = ControllerTickets::new("failing", vec![ticket(0)]);
                for _ in 0..2 {
                    dispatch_validate_tickets(
                        &mut host,
                        "minecraft:overworld",
                        TicketController { slot: 0 },
                        |listen| {
                            a.register_listener(listen);
                        },
                    );
                    assert!(a.removed.is_empty());
                    assert_eq!(a.loaded, [ticket(0)]);
                }
                let mut b = ControllerTickets::new("healthy", vec![ticket(1)]);
                dispatch_validate_tickets(
                    &mut host,
                    "minecraft:overworld",
                    TicketController { slot: 1 },
                    |listen| {
                        b.register_listener(listen);
                    },
                );
                assert!(b.loaded.is_empty());
                assert_eq!(b.into_removed(), [ticket(1)]);
                assert_eq!(*seen_a.lock().unwrap(), [0]);
                assert_eq!(*seen_b.lock().unwrap(), [0, 0, 1]);
                assert_eq!(
                    crate::__private::take_hook_panics(),
                    [crate::__private::HookPanicReport {
                        mod_id: "failing",
                        hook: "validate_tickets",
                    }]
                );
            },
        );
    }
}
