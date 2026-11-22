//! The prose of `ClientHost::set_block_extra_models` (#2763), kept out of `notes.rs`.
pub trait Notes {
    #[notes = r#"## Block extra models

`ClientHost::set_block_extra_models(pos, models)` replaces the standalone models (`RenderAssets::model`) drawn as part of the block at `pos`; an empty list clears it. The lists are client-only and need no block entity. A resend of a resident column preserves attachments at unchanged cells; unloading the column clears them. A block state or level change also clears the affected lists.

The live client Host can set the list; reload and setup contexts do not expose the setter. `NotLoaded` means the position is outside a resident column or the level height; `Model` means the handle is stale or is not a registered standalone model. A refused call changes nothing. Resolve new handles in `client_resources_reloaded`, then set the lists again from a live client callback (`client_tick_pre`).

After the block geometry, extra faces are drawn in block and list order, with the block's light, neighbour culling and tints but their own render layers. Mod tint providers are asked before the mesh is scheduled; a provider that panics supplies white. Drawing stops before the next face whose worst-case expansion cannot fit the section's upload budget, so the last model may be partially drawn."#]
    fn block_extra_models();
}
