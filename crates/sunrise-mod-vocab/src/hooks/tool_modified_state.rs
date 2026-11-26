//! The `tool_modified_state` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The state the block `state` becomes under a tool action (`IBlockExtension.getToolModifiedState`:
    /// an axe's strip, scrape or wax-off, a shovel's flatten or douse, a hoe's till), asked of the
    /// block a tool is used on (`tool.pos`). `final_state` starts with vanilla's answer, `None` for a
    /// block the tool does nothing to: set it to `Some(new)` to allow the action or replace its
    /// result (a mod log an axe strips, a path a shovel makes), and `Cancel` it to deny the action,
    /// so the tool does nothing. `tool.simulated` says the game is only asking what the tool would
    /// do (`LevelRef::block_transform`, NeoForge's `simulate`): answer the same and change nothing in
    /// the world. A part answers before the global `block_tool_modification`, which still hears the
    /// result and may change or cancel it; a cancelled part skips it. Asked for the block's own parts only, so
    /// a block with none costs nothing.
    /// A disabled owner's subscribed transform suppresses the unchanged template result,
    /// not the ability's notification or a listener's replacement. The global hook still
    /// sees the candidate, and later abilities can be tried if this one has no result.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "both",
        contract = "cancel",
        out = "cancel",
        scope = "block",
        inert = "signal",
        key = "state",
        host = "read",
        neoforge = "IBlockExtension.getToolModifiedState",
        version = 2
    )]
    fn tool_modified_state(
        state: BlockState,
        stack: &ItemStack,
        tool: &ToolUse,
        final_state: &mut Option<BlockState>,
        cancel: &mut bool,
        #[engine] inert: &mut bool,
    );
}
