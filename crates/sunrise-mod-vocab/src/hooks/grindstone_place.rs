//! The `grindstone_place` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A grindstone at `pos` made its result after its inputs changed, for
    /// the server player `player` (UUID) and `player_entity_id`
    /// (`GrindstoneEvent.OnPlaceItem`, matching NeoForge `CommonHooks.onGrindstoneChange`
    /// in `GrindstoneMenu.createResult`), including the three input notifications of an accepted take: `inputs` lends the input stacks and vanilla
    /// result. Runs inline without a decided-action transaction or rollback on cancel.
    /// Cancellation is checked before converting `edits.output` and clears slot 2
    /// without rolling back listener writes. Otherwise leave `edits.output` at `None`
    /// to keep vanilla's result and experience, or set a replacement stack and its
    /// `edits.xp` (any negative value falls through to vanilla's experience roll;
    /// nonnegative values set the award). An invalid replacement stack is logged and
    /// ignored, keeping vanilla's result. A panicking listener is caught and leaves
    /// vanilla's result. Server-side.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "GrindstoneEvent.OnPlaceItem",
        cancellable = true,
        version = 4
    )]
    fn grindstone_place(
        player: u128,
        player_entity_id: i32,
        pos: [i32; 3],
        inputs: &mod_api::item::menu::GrindstonePlaceInputs<'_>,
        edits: &mut mod_api::item::menu::GrindstonePlaceEdits,
        cancel: &mut bool,
    );
}
