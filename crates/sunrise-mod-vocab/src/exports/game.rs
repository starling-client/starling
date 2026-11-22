// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod game;
pub use crate::game::{
    BlockEdit, BlockRun, Clock, ColumnBlocks, Config, ConfigFile, ConfigKind, CubeFace,
    DataMapType, ExperienceOrb, Falling, Heightmap, Hit, MapDecoration, MapInfo, MobRender,
    ModMismatch, ModMismatchResolution, PhantomSpawnResult, PlayerRender, Shake, SleepProblem,
};
pub use crate::game::{MAX_TERRAIN_COLUMNS, NbtUpgradeError, TerrainColumn};
mod __private {
    mod built {
        pub use crate::game::built::{ConfigFile, TerrainColumn};
    }
}
