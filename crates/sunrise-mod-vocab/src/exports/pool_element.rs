pub use world::pool_element::JigsawSpace;
pub use world::pool_element::{
    ElementContext, ElementDecodeContext, ElementPlacement, JigsawBlock, JigsawJoint,
    JigsawJunction, PoolElement, PoolElementRef, PoolElementType, PoolPiece, Projection,
    TemplatePool,
};
mod __private {
    /// The registered pool element types, and the handles the game's adapter
    /// builds and reads back.
    pub use crate::world::pool_element::{
        pool_element_payload, pool_element_ref, pool_element_type_declarations, template_pool,
    };
}
