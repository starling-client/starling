//! Models a client mod attaches to a block position (`ClientRenderClientHost::set_block_extra_models`):
//! standalone models the chunk mesher draws as part of the block there, chosen at run time, and
//! why a call was refused.

/// Why `set_block_extra_models` changed nothing. The position keeps the models it had.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum BlockExtraModelsError {
    /// The client holds no column at the position, or the position is outside the level's height.
    NotLoaded,
    /// A handle is stale or is not a registered standalone model.
    Model(crate::ModelError),
}
