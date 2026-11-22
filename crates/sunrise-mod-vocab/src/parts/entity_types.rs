use super::*;

/// The bounds for general entity types, declared
/// once so the registrar, the codecs and the tests quote the same numbers.
/// The payload bound applies to mod-owned payload bytes only, never to a
/// record of a vanilla type. The quarantine bounds (with no vanilla
/// analogue) cap what the game preserves of a missing mod's records.
// exhaustive: the design's limits are one block; no further constants join it
pub mod limits {
    /// The most payload bytes one general entity record may carry. A payload
    /// one byte larger is refused, never truncated into a valid-looking
    /// entity.
    pub const PAYLOAD_LIMIT: usize = 65_536;
    /// The deepest nesting of a general entity's decoded data. The
    /// payload decode checks it.
    pub const MAX_NESTING: u32 = 32;
    /// The largest record a missing mod's quarantined general entity may
    /// occupy, as its encoded NBT size. A larger record is skipped with a
    /// warning rather than preserved. There is no vanilla analogue:
    /// vanilla deletes unknown-mod entities outright, and Sunrise's quarantine
    /// preserves records for missing mods.
    pub const QUARANTINE_RECORD_LIMIT: u64 = 69_632;
    /// The most quarantined general entity records one chunk may carry. The
    /// next one is skipped with a warning. There is no vanilla analogue:
    /// vanilla has no quarantine to bound.
    pub const QUARANTINE_CHUNK_LIMIT: usize = 256;
}

/// A general entity type's spawn category (supplied at
/// declaration; general types start as `MISC`, which never gains natural
/// spawning implicitly).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum GeneralEntityCategory {
    /// Neither monster nor creature: no natural-spawn policy of its own.
    Misc,
}

/// Whether instances of a declared general type persist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum GeneralSavePolicy {
    /// Instances save like ordinary entities do.
    Save,
    /// `noSave`: instances produce zero persisted records.
    NoSave,
}

/// A general entity type a mod declares, as the game reads it when it freezes
/// the type registry. The registry is data: no mod type adds an entry
/// to any hand-kept engine list.
#[derive(Clone, Debug)]
// exhaustive: only registration builds it; the game reads it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct GeneralEntityTypeDeclaration {
    /// The declaring mod's id: the first key of the registry's sort order.
    pub mod_id: String,
    /// The type's namespaced id (`mymod:robot`).
    pub id: String,
    /// The type's spawn category.
    pub category: GeneralEntityCategory,
    /// The type's finite width in blocks, as supplied to the vanilla builder.
    pub width: f64,
    /// The type's finite height in blocks, as supplied to the vanilla builder.
    pub height: f64,
    /// Optional finite vehicle-local passenger point in blocks (`[x, y, z]`).
    /// `None` uses the top center of the box.
    pub passenger_attachment: Option<[f64; 3]>,
    /// Whether instances of the type persist.
    pub save_policy: GeneralSavePolicy,
    /// Whether the type may be summoned by name.
    pub summonable: bool,
    /// Whether the client can target the row for use and attack (`Entity.isPickable`).
    pub pickable: bool,
    /// How instances respond to a piston. The vanilla default is `PushPull`.
    pub piston_push_reaction: crate::PushReaction,
    /// Tracking distance in chunks, stored directly like vanilla's signed integer.
    /// Zero keeps instances in server simulation without pairing them to clients.
    pub tracking_chunks: i32,
    /// Update interval in ticks, stored directly like vanilla's signed integer.
    /// `i32::MAX` is vanilla's no-update-interval value.
    pub update_interval: i32,
    /// The payload schema version instances of the type carry.
    pub payload_version: u16,
    /// The ids of loaded mods this type requires, every one of which must be
    /// present for the declaration to stand.
    pub required_features: Vec<String>,
}

/// A placeholder to build a declaration from: empty ids, a miscellaneous type that saves, vanilla's
/// default player-sized box and no requirements. Registration validates what a mod sets.
impl Default for GeneralEntityTypeDeclaration {
    fn default() -> Self {
        Self {
            mod_id: String::new(),
            id: String::new(),
            category: GeneralEntityCategory::Misc,
            width: 0.6,
            height: 1.8,
            passenger_attachment: None,
            save_policy: GeneralSavePolicy::Save,
            summonable: true,
            pickable: false,
            piston_push_reaction: crate::PushReaction::PushPull,
            tracking_chunks: 5,
            update_interval: 3,
            payload_version: 1,
            required_features: Vec::new(),
        }
    }
}

/// Why a mod's general entity type declaration was refused
/// ([`Registrar::declare_general_entity`](GeneralTypesRegistrar::declare_general_entity)).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum GeneralEntityRegistrationError {
    /// The registry is frozen: declarations are only accepted while the game
    /// is still in its registration phase, before any world or connection
    /// opens.
    Frozen,
    /// Only both-side packages (and both-side served namespace suppliers) may declare entity
    /// types, which the network names.
    WrongSide,
    /// The type's id is malformed, empty or in a namespace the declaring mod
    /// does not own (a vanilla type's included).
    InvalidId,
    /// Another declaration already covers the type.
    Duplicate,
    /// A dimension or passenger attachment component is NaN or infinite.
    NotFinite,
    /// A required feature names no loaded mod.
    UnknownDependency,
}

/// The general entity types mods declared.
pub fn general_entity_type_declarations() -> &'static [GeneralEntityTypeDeclaration] {
    installed().map_or(&[], |installed| &installed.general_entity_types)
}

/// Registration of general entity types: one frozen registry of
/// namespaced types, filled before worlds or connections open.
pub trait GeneralTypesRegistrar {
    /// Declare a general entity type of the mod's own (vanilla's
    /// `EntityType` builder, NeoForge's `RegisterEvent`): give its `path`, category,
    /// finite `width` and `height` in blocks, its passenger attachment, its save and
    /// summon policy, whether it is pickable, its signed integer tracking distance and
    /// update interval, its payload schema version and the loaded mods it requires.
    /// A `tracking_chunks` value of zero keeps instances in server simulation
    /// without pairing them to clients.
    ///
    /// Pass `false` for vanilla's plain `Entity.isPickable` default. Pass `true`
    /// to let the player's crosshair target the declared box for use and attack.
    /// This does not make the box solid or give it a middle-click item.
    /// `piston_push_reaction` defaults to vanilla's `PushPull` when omitted (`None`).
    ///
    /// `passenger_attachment` is the finite `[x, y, z]` passenger point in vehicle-local
    /// blocks, rotated with the vehicle's yaw. `None` uses `[0.0, height, 0.0]`.
    ///
    /// A bare `path` uses the calling mod's ID as its namespace (`robot` of `mymod` is
    /// `mymod:robot`). An explicit ID may use a namespace owned by the caller's manifest or by
    /// a content package it serves via `served_by`. Both the declaring mod and the namespace
    /// supplier must be `side = "both"`.
    ///
    /// An ID outside these namespaces, a malformed ID, a duplicate of this mod's or another's,
    /// a non-finite dimension or attachment, a required feature naming no loaded mod, or a
    /// declaration after the registry froze (the registration phase has ended) refuses, naming
    /// the mod, without panicking. Invalid input is refused, never clamped.
    fn declare_general_entity(
        &mut self,
        path: &str,
        category: GeneralEntityCategory,
        width: f64,
        height: f64,
        passenger_attachment: Option<[f64; 3]>,
        save_policy: GeneralSavePolicy,
        summonable: bool,
        pickable: bool,
        piston_push_reaction: Option<crate::PushReaction>,
        tracking_chunks: i32,
        update_interval: i32,
        payload_version: u16,
        required_features: &[&str],
    ) -> Result<(), GeneralEntityRegistrationError>;
}
