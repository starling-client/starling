//! `RealmsCreateWorldFlow`'s target: what the Create New World screen decided, for the
//! temporary world `createTemporaryWorldFolder` writes before the upload.

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct CreatedWorld {
    /// The world's level data, its name, game type, difficulty, game rules and features.
    pub level: crate::save::Level,
    pub world_gen: crate::save::WorldGen,
    pub flat_settings: Option<sunrise_core::nbt::Tag>,
    /// Selected data-pack preset or editor output.
    pub dimensions: Option<sunrise_core::nbt::Tag>,
    pub single_biome: Option<String>,
    /// level.dat's `DataPacks`: enabled and disabled ids.
    pub data_packs: (Vec<String>, Vec<String>),
    /// The Create World screen's temporary pack folder, moved into the world
    /// (`Files.move(tempDataPackDir, worldFolder.resolve("datapacks"))`).
    pub pack_dir: Option<PathBuf>,
}
