//! Typed state an entity Part keeps on each entity of its type (#796 stage 3).
//!
//! The value lives in the entity's own attachment map, the store every other entity
//! attachment uses, through the same core as block entity Part state
//! ([`read_state`], [`edit_part_state`]): it saves with the entity through the type's
//! codec, is kept by transactions, and goes with the entity when it is removed, unloaded
//! or moved to another level.

use crate::*;
use std::fmt;

/// An entity Part's typed state: one value per entity of the Part's type, owned by the
/// game, not by the Part or a mod's statics.
///
/// Implement it beside [`EntityPart`] and attach the Part with
/// [`Registrar::entity_part_with_state`](crate::Registrar). Only mob types (vanilla and
/// mod) and mod general entity types keep it; a player, any other vanilla family and an
/// unknown type are refused when the game loads.
///
/// ```rust,ignore
/// struct Robot {
///     charge: AttachmentKey<u32>,
/// }
/// impl EntityPartState for Robot {
///     type Data = u32;
///     fn state_key(&self) -> &AttachmentKey<u32> {
///         &self.charge
///     }
/// }
/// // In a hook with `host: &mut dyn ServerHost` and the robot's `entity`:
/// let full = self.edit_state(host, entity, |_, _tx, charge| {
///     *charge += 1;
///     *charge >= 100
/// })?;
/// ```
///
/// Reads take `&dyn Host` and lend a value from a snapshot; edits take `&mut dyn ServerHost`, run in
/// their own nested transaction and store the new value only when the closure returns, as
/// [`BlockEntityPartState`]'s do. Every call first checks that `self.state_key()` belongs to
/// the active installation and that `entity` is of the open world and still names a resident
/// entity, in the level this host is in:
/// - a removed, unloaded or moved entity refuses with [`PartStateError::Stale`];
/// - an entity in another level refuses with [`PartStateError::OtherLevel`];
/// - an entity out of its level's lists for its own tick refuses with
///   [`PartStateError::HolderBusy`].
///
/// An entity that changes level without a player riding it is rebuilt under a new id: the
/// old handle is `Stale` and the state arrives with the entity under its new id, so a mod
/// that needs continuity follows the entity's UUID. A vehicle a player rides keeps its id,
/// and its handle still resolves in the new level. Inside an action that is rolled back
/// after the entity changed level, the state stays with the entity under its new id, and
/// the old handle stays `Stale`. Travel fires no join or leave hook for it.
///
/// Server-side only for now: the state key must not sync or copy on death, and a client reads
/// the default.
pub trait EntityPartState: EntityPart + Sized {
    /// The value each entity of the Part's type keeps.
    type Data: Send + Sync + 'static;
    /// The issued attachment key this Part keeps its per-entity state under.
    /// Register its [`AttachmentDescriptor`](crate::AttachmentDescriptor) during
    /// [`Hooks::register`](crate::Hooks::register) (or look up another package's key with
    /// [`Registrar::attachment_key`](crate::Registrar)) and store it on the Part; without a
    /// persistence codec the value is kept for the session only.
    fn state_key(&self) -> &AttachmentKey<Self::Data>;

    /// Lend the entity's value or its default. Inserts nothing.
    fn state<R>(
        &self,
        host: &dyn Host,
        entity: EntityRef,
        read: impl FnOnce(&Self::Data) -> R,
    ) -> Result<R, PartStateError> {
        let key = self.state_key();
        read_state(host, entity, key, |value| match value {
            Some(value) => read(value),
            None => read(&key.descriptor().default_value()),
        })
    }

    /// Lend the entity's value or `None`. Inserts nothing.
    fn state_existing<R>(
        &self,
        host: &dyn Host,
        entity: EntityRef,
        read: impl FnOnce(Option<&Self::Data>) -> R,
    ) -> Result<R, PartStateError> {
        read_state(host, entity, self.state_key(), read)
    }

    /// Changes the entity's value with `edit`, starting from the default when it has none,
    /// and stores the result: see [`edit_part_state`]. A stale handle never creates a map.
    /// If the host cannot open a transaction, returns [`PartStateError::Unavailable`] without
    /// calling `edit`.
    fn edit_state<R>(
        &self,
        host: &mut dyn ServerHost,
        entity: EntityRef,
        edit: impl FnOnce(&mut dyn ServerHost, &TransactionContext, &mut Self::Data) -> R,
    ) -> Result<R, PartStateError>
    where
        Self::Data: Clone,
    {
        edit_part_state(host, entity, self.state_key(), edit)
    }
}

/// Where the entity an [`EntityRef`] names is now, as its Part state checks it
/// ([`EntityRef::residency`]).
///
/// This is a process-local residency snapshot, not a `Wire` type.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::EntityResidency>();
/// ```
// no wire: local handle residency state for EntityPartState.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum EntityResidency {
    /// Resident in the level this host is in, and reachable.
    InPlace,
    /// Resident in this host's level, but out of its lists for its own tick.
    Busy,
    /// Resident in another level of the open world.
    OtherLevel,
    /// No entity: removed (or its removal pending), unloaded, moved under a new id, or the
    /// handle is of another world open.
    #[default]
    Gone,
}

impl PartStateHolder for EntityRef {
    fn part_state_holder(self, host: &dyn Host) -> Result<Holder, PartStateError> {
        match self.residency(host) {
            EntityResidency::InPlace => Ok(Holder::Entity(self.id())),
            EntityResidency::Busy => Err(PartStateError::HolderBusy),
            EntityResidency::OtherLevel => Err(PartStateError::OtherLevel),
            EntityResidency::Gone => Err(PartStateError::Stale),
        }
    }

    fn attachment_holder(self) -> AttachmentHolder {
        AttachmentHolder::Entity(self)
    }
}

/// An entity Part with state could not be attached: its state declaration was refused.
///
/// This is a process-local registration error, not a `Wire` type.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::EntityPartStateError>();
/// ```
// no wire: local entity Part state registration error.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EntityPartStateError {
    /// Its state declaration was refused.
    State {
        /// The Part type.
        part: &'static str,
        /// The entity type it was attached to.
        kind: &'static str,
        /// The state's namespace.
        namespace: &'static str,
        /// The state's name.
        name: &'static str,
        /// The state's Rust type.
        data: &'static str,
        /// Why.
        reason: PartStateRefusal,
    },
}

impl fmt::Display for EntityPartStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self::State {
            part,
            kind,
            namespace,
            name,
            data,
            reason,
        } = self;
        write!(
            f,
            "entity part {part} on {kind}: state {namespace}:{name} of type {data} refused: "
        )?;
        reason.describe(
            f,
            "entities (only a player respawns, and players keep no entity part state)",
        )
    }
}

impl std::error::Error for EntityPartStateError {}

/// One entity Part state declaration as registration kept it, for the game to check its
/// entity type once every type is known.
// no wire: host-local registration record holding a PartStateDeclaration with registration identity; never crosses the wire.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EntityPartStateDeclaration {
    /// The mod that declared it.
    pub mod_id: String,
    /// The Part type.
    pub part: &'static str,
    /// The entity type id the Part was attached to.
    pub kind: &'static str,
    /// The state declaration.
    pub state: PartStateDeclaration,
}

impl EntityPartStateDeclaration {
    /// The declaration registration keeps for mod `mod_id`'s Part `part` on type `kind`.
    pub fn new(
        mod_id: String,
        part: &'static str,
        kind: &'static str,
        state: PartStateDeclaration,
    ) -> Self {
        Self {
            mod_id,
            part,
            kind,
            state,
        }
    }

    /// The error refusing it for `reason`.
    pub fn refused(&self, reason: PartStateRefusal) -> EntityPartStateError {
        self.state.refused_on_entity(self.part, self.kind, reason)
    }

    /// The refusal for its entity type, if it has one: `minecraft:player`, a type that is
    /// neither a mob type (`mob`) nor a mod general entity type (`general`), named as another
    /// vanilla family when `known` holds the id and as an unknown type otherwise.
    pub fn kind_refusal(
        &self,
        mob: impl Fn(&str) -> bool,
        general: impl Fn(&str) -> bool,
        known: impl Fn(&str) -> bool,
    ) -> Option<PartStateRefusal> {
        let kind = self.kind;
        let reason = if kind == "minecraft:player" {
            "players: respawn identity is not decided"
        } else if mob(kind) || general(kind) {
            return None;
        } else if known(kind) {
            "no entity part runs on this kind"
        } else {
            "no such entity type"
        };
        Some(PartStateRefusal::EntityKind { kind, reason })
    }
}

/// The entity Part state declarations of every mod, in registration order.
pub fn entity_part_state_declarations() -> &'static [EntityPartStateDeclaration] {
    crate::parts::installed().map_or(&[], |installed| &installed.entity_part_states)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_residency_routes_through_backend_and_defaults_gone_without_one() {
        struct Bare;
        impl Host for Bare {}
        crate::impl_host_areas!(Bare);

        let entity = EntityRef::new(WorldEpoch::new(1), 7);
        assert_eq!(entity.residency(&Bare), EntityResidency::Gone);
        assert_eq!(entity.part_state_holder(&Bare), Err(PartStateError::Stale));

        struct WithResidency(EntityResidency);
        impl Host for WithResidency {
            fn backend(&self) -> Option<crate::BackendRef<'_>> {
                Some(crate::BackendRef::of(self))
            }
        }
        crate::impl_host_areas!(WithResidency);
        impl Backend for WithResidency {}
        crate::impl_backend_areas!(WithResidency, except(entities));
        impl crate::prelude::EntitiesBackend for WithResidency {
            fn entity_residency(&self, _: EntityRef) -> EntityResidency {
                self.0
            }
        }

        for (residency, expected) in [
            (EntityResidency::InPlace, Ok(Holder::Entity(7))),
            (EntityResidency::Busy, Err(PartStateError::HolderBusy)),
            (EntityResidency::OtherLevel, Err(PartStateError::OtherLevel)),
            (EntityResidency::Gone, Err(PartStateError::Stale)),
        ] {
            let host = WithResidency(residency);
            assert_eq!(entity.residency(&host), residency);
            assert_eq!(entity.part_state_holder(&host), expected);
        }
    }
}
