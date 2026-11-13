use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A tool is about to transform a block. Cancel or replace the final state;
    /// `None` suppresses the transformation.
    #[inline(never)]
    pub(in crate::game) fn native_block_tool_modification(
        &self,
        stack: &ItemStack,
        tool: &ToolUse,
        final_state: &mut Option<BlockStateId>,
        cancel: &mut bool,
    ) {
        self.fire_block_tool_modification(stack, tool, final_state, cancel);
    }
}
