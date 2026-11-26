//! Additional positions for a client's breaking-crack display.

pub trait Hooks {
    /// When a crack is set or changes stage, supply additional positions drawn
    /// at that same stage. Only parts on the cracked `state` are called, for
    /// both the local player's predicted crack and other players' cracks.
    /// `positions` starts empty; the original `pos` is always drawn separately.
    /// Answers are stored with the crack, not requested during rendering, and
    /// disappear when the crack clears, expires or its client level closes.
    ///
    /// Display bounds: at most 64 distinct extras, anywhere in the supplied
    /// `level`. The origin and duplicates are dropped, and so is every extra past
    /// the 64th; they produce a debug log, never a panic. The crack over a
    /// position's block model is drawn only within vanilla's 32-block camera
    /// distance (`LevelExtractor.extractBlockDestroyAnimation`), so an extra far
    /// from the camera costs nothing there; a block entity's own draws read the
    /// stage through `BlockEntityDrawContext::crumbling_stage`, as vanilla's
    /// block-entity overlay does, with no distance filter of its own. These limits constrain only the display, not block
    /// interaction. This hook changes neither break speed nor what breaks.
    #[hook(
        side = "client",
        contract = "collect",
        out = "positions",
        scope = "block",
        key = "state",
        neoforge = "none (vanilla LevelRenderer.destroyBlockProgress)"
    )]
    fn break_progress_positions(
        level: LevelRef,
        pos: BlockPos,
        state: BlockState,
        progress: u8,
        positions: &mut Vec<BlockPos>,
    );
}
