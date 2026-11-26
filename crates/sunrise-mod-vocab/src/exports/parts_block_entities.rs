//! Exports of `crate::parts` items defined in `parts/block_entities.rs`.
pub use parts::{BlockCapabilityProvider, BlockProvider, BlockProviders, block_providers};
mod __private {
    pub use crate::parts::{
        BlockEntityTickerSelector, RegisteredBlockEntityTickerSelector,
        RegisteredBlockEntityTransform, block_entity_ticker_selectors, block_entity_transforms,
        block_entity_type_listens, block_entity_types,
    };
}
