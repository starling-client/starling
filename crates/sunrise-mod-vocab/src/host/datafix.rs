//! Upgrade standalone saved data on either side, without a world.
use crate::*;

/// The shared host's saved-data upgrade, reached through [`Host`].
#[sunrise_mod_api_macros::host_area]
pub trait DatafixHost {
    /// Side: both.
    /// Run the game's data fixers on one saved compound, from `data_version` to the current
    /// game data version. This needs no world, adds no `DataVersion` field, and runs no fixes
    /// that operate on the enclosing chunk or file. Use [`NbtDataType::EntityTree`] if the
    /// upgrade should include riders and passengers as well as the root entity. Like
    /// vanilla's standalone fixers, entity and block-entity roots can retain legacy
    /// keys alongside newly written ones.
    ///
    /// Unknown mod IDs are preserved; generic format changes can still apply (for example,
    /// an item stack's count and components). At the game's current version a well-formed
    /// compound comes back unchanged. Data older than the first schema is read with
    /// that schema, including unversioned data (`data_version = -1`).
    ///
    /// Returns [`NbtUpgradeError`] for a future source version, malformed
    /// data, unavailable legacy datafix tables, or input or output past vanilla's encoded
    /// NBT read depth.
    #[static_call]
    fn upgrade_nbt(
        &self,
        _kind: NbtDataType,
        _data_version: i32,
        _nbt: Compound,
    ) -> Result<Compound, NbtUpgradeError>;
}
