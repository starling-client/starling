pub use world::structure_type::TerrainAdjustment;
pub use world::structure_type::{
    ChunkPos, FoundStructure, GenerationContext, HeightmapType, NoiseBlock, NoiseColumn,
    PieceBeardifier, PieceContext, StructureError, StructurePiece, StructureType,
};
pub use world::structure_type::{
    DimensionPadding, JigsawLayout, JigsawReach, JigsawSettings, LiquidSettings, PoolAliasBinding,
};
mod __private {
    /// The registered structure types, for the executable to hand to the engine's
    /// `StructureRegistry` before any world opens.
    pub use crate::world::structure_type::{biome_id, jigsaw_layout, structure_type_declarations};
}
