//! The `grindstone_take` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The server player `player` (UUID) and `player_entity_id` took a
    /// grindstone's result at `pos` (`GrindstoneEvent.OnTakeItem`, matching NeoForge
    /// `CommonHooks.onGrindstoneTake` in `GrindstoneMenu.java.patch`): `inputs` lends
    /// the pre-take stacks. Runs inline after the click has taken the result, without a
    /// decided-action transaction or click rollback. `edits.xp` starts at the resolved
    /// experience (`>= 0`, from `grindstone_place` or vanilla's pre-rolled award);
    /// non-positive values (`<= 0`) spawn 0 orbs while still firing level event 1042 and
    /// replacing the inputs. Replacing each slot and finally notifying the
    /// input container each run `grindstone_place`. This completes before
    /// the click drops its taken stack or checks for another result. Use `edits.new_top` and `edits.new_bottom` to leave items
    /// in the input slots (invalid stacks are logged and ignored as empty slots).
    /// Set `cancelled` to skip experience, level event 1042 and input replacement;
    /// cancel is checked before converting remainders and does not undo the click or
    /// listener writes. A panicking listener is caught and completes the take with the
    /// initial experience and cleared inputs. Server-side.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "GrindstoneEvent.OnTakeItem",
        cancellable = true,
        version = 4
    )]
    fn grindstone_take(
        player: u128,
        player_entity_id: i32,
        pos: [i32; 3],
        inputs: &mod_api::item::menu::GrindstoneTakeInputs<'_>,
        edits: &mut mod_api::item::menu::GrindstoneTakeEdits,
        cancel: &mut bool,
    );
}
