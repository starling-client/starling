//! The `dispense_method` hook and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `DispenserBlock.getDispenseMethod`, before the item's `dispense_item`:
    /// the block the dispenser faces chooses what the dispense does, for any
    /// item, including vanilla items, on the authoritative server (not
    /// droppers). `state` is the block at `pos` moved by `facing` (air
    /// included), and the part is asked only for that block. An answer wins:
    /// no item part is asked and vanilla's behaviour
    /// doesn't run. The answer is `dispense_item`'s: a remainder of None
    /// empties the slot, `Some(stack)` replaces it (the hook owns
    /// consumption: no extra item is removed afterwards), and `event` is the
    /// level event played with the smoke: 1000 the click, 1001 the fail sound,
    /// or 0 for no sound or smoke. Leave answer None and `dispense_item` decides.
    /// In an open transaction in the dispenser's level, sound and smoke follow the behaviour's
    /// published effects at the outermost commit and are discarded on abort.
    /// The remainder is detached until the dispenser writes its slot. An invalid remainder is
    /// reported without writing or notifying the slot, and no other behaviour runs. A disabled
    /// owner's subscribed method skips the slot write and dispense events unless an earlier
    /// enabled Part supplies a reply the engine accepts.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "replace",
        out = "answer",
        scope = "block",
        inert = "signal",
        key = "state",
        neoforge = "none (vanilla DispenserBlock.getDispenseMethod; NeoForge adds no event or extension for it)",
        version = 4
    )]
    fn dispense_method(
        dimension: &str,
        pos: [i32; 3],
        facing: Direction,
        state: BlockState,
        stack: &ItemStack,
        answer: &mut Option<mod_api::Dispensed>,
        #[engine] inert: &mut bool,
    );
}
