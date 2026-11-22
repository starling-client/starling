use super::*;
use crate::{
    EntityTypeKey, Heightmap, SpawnPlacementError, SpawnPlacementOperation,
    SpawnPlacementPredicate, SpawnPlacementType,
};

/// One spawn predicate registration, in mod registration order. The game
/// resolves the declarations at resource load before running spawn checks.
// hidden: engine staging; mods use the spawn placement registrar.
// exhaustive: engine-only staging describes the full predicate the loader must resolve.
#[doc(hidden)]
// no wire: contains a native callback.
pub struct SpawnPlacementDeclaration {
    /// The canonical id of the entity type.
    pub id: String,
    /// The registrar key to give the predicate.
    pub kind: EntityTypeKey,
    /// The mod that registered the predicate, for load refusals.
    pub mod_id: &'static str,
    /// Where the type stands, if the registration sets it.
    pub placement: Option<SpawnPlacementType>,
    /// The surface used to place mobs during chunk generation, if set.
    pub heightmap: Option<Heightmap>,
    /// The native predicate to run for the type.
    pub predicate: Arc<dyn SpawnPlacementPredicate>,
    /// How to combine this predicate with those already registered.
    pub operation: SpawnPlacementOperation,
}

/// The spawn predicates mods registered, in registration order.
// hidden: the engine freezes the staged predicates; mods register through SpawnPlacementsRegistrar.
#[doc(hidden)]
pub fn spawn_placement_declarations() -> &'static [SpawnPlacementDeclaration] {
    installed().map_or(&[], |installed| &installed.spawn_placements)
}

/// Registrar operations for spawn placements.
pub trait SpawnPlacementsRegistrar {
    /// Register a native predicate in `Hooks::register` (NeoForge's
    /// `RegisterSpawnPlacementsEvent.register`). Request `kind` from `r.entity_type_key`
    /// with the full registry id. Any registered entity type is eligible, including
    /// `minecraft:ender_dragon` (which has a vanilla entry) and non-mob types such as
    /// `minecraft:arrow` (which do not). Ordinary and trial spawners check the resolved
    /// entity type's predicate before attempting to create it, even for non-mobs.
    /// Ordinary spawners with custom rules skip the predicate; trial spawners check
    /// custom rules afterward. Registration does not add a spawn path for a type.
    ///
    /// If the type has no vanilla entry, its first registration needs both `placement`
    /// (where it may stand) and `heightmap` (how chunk generation finds the surface).
    /// Natural spawning starts from `WorldSurface` instead. The first predicate becomes
    /// the original regardless of `operation`.
    /// For an existing entry, only [`SpawnPlacementOperation::Replace`] may supply either
    /// field; `None` keeps that field's current value, including earlier changes.
    ///
    /// If an existing entry was replaced, only the last replacement runs, even if
    /// later registrations add `Or` or `And` predicates. Otherwise the original runs
    /// first, then `Or` predicates until one passes. If the original or any `Or` passed,
    /// `And` predicates run until one fails. Each list runs in registration order.
    /// The predicates run at `SpawnPlacements.checkSpawnRules`, not at direct vanilla
    /// predicate calls such as patrols and village sieges. They share the caller's live
    /// random stream. A panicking predicate is disabled and its draws are rolled back;
    /// a failed replacement does not rerun the original. Where `mob_spawn_placement_check`
    /// fires, it gets the effective answer and the same stream after the predicates' draws.
    ///
    /// An unavailable key returns [`SpawnPlacementError::InvalidEntityType`] and records
    /// a registration refusal. Unloaded types, invalid combinations of metadata, and
    /// missing vanilla spawn placement data refuse resource loading, naming the mod and type.
    /// The accepted gameplay resources supply both the registry keys and their spawn facts.
    /// Recovery from a failed client pack load clears optional packs, not mod entity types.
    /// Missing or malformed facts (category or dimensions) refuse resource loading and name
    /// the affected entity type. Both spawners use those same resolved types.
    fn register_spawn_placement(
        &mut self,
        kind: EntityTypeKey,
        placement: Option<SpawnPlacementType>,
        heightmap: Option<Heightmap>,
        predicate: Arc<dyn SpawnPlacementPredicate>,
        operation: SpawnPlacementOperation,
    ) -> Result<(), SpawnPlacementError>;
}
