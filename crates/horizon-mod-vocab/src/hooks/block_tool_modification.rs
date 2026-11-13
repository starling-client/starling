//! The `block_tool_modification` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A tool is about to transform a block. Cancel or replace the final state;
    /// `None` suppresses the transformation.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "cancel",
        host = "read",
        neoforge = "BlockEvent.BlockToolModificationEvent",
        cancellable = true
    )]
    fn block_tool_modification();
}
