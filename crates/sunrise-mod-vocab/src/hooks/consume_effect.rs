//! The `consume_effect` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0086

pub trait Hooks {
    /// A custom entry of the stack's effective `consumable`, in list order,
    /// interleaved with built-in effects. `stack` is the read-only snapshot
    /// before consumption shrinks it, including components. `user` is the
    /// player or mob consuming it. An unclaimed kind does nothing.
    /// Register a custom kind's save codec and Wire value with
    /// `Registrar::register_consume_effect_type` before placing it in a stack
    /// that will be sent to a client. A registered kind's `data` holds the
    /// canonical fields returned by its save codec, with their exact NBT types.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_consume_effect",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        version = 3,
        neoforge = "none (runtime ConsumeEffect.apply; RegisterEvent registers effect types, not invocations)"
    )]
    #[notes = "`consume_effect` receives the custom kind, the canonical save-codec data for a registered kind (otherwise the retained definition data), the pre-shrink stack snapshot and the consuming entity (player or mob). Entries come from the effective consumable, including stack patches, and run in list order between the built-in entries."]
    fn consume_effect(
        kind: &str,
        data: &sunrise_core::nbt::Tag,
        stack: &ItemStack,
        user: mod_api::EntityRef,
    );
}
