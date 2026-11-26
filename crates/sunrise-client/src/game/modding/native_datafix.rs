//! Bridge the generated game Host implementation to the standalone data fixer.
use super::Game;
use mod_api::{Compound, NbtDataType, NbtUpgradeError};

impl Game<'_> {
    pub(in crate::game) fn upgrade_nbt(
        kind: NbtDataType,
        data_version: i32,
        nbt: Compound,
    ) -> Result<Compound, NbtUpgradeError> {
        sunrise_world::datafix::upgrade_compound(kind, data_version, nbt)
    }
}
