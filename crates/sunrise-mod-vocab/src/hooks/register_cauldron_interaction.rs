pub trait Hooks {
    /// Register cauldron item interactions, by exact item or by item tag, on both sides before
    /// snapshot publication. Vanilla dye tags take precedence, then item tags in registration
    /// order, then exact items; registering the same key again overwrites the entry in place.
    /// Registered callbacks run on the server through [`Interaction::interact`]. A panic or phase
    /// refusal discards that listener's entries, including queued registrations to all
    /// dispatchers, before the next listener runs.
    #[notes = "## Cauldron interactions

`register_cauldron_interaction` registers item and item-tag callbacks on both sides. See [Registering cauldron interactions](../how-to/world/cauldron-interactions.md) for the server-side `Interaction::interact` callback, overwrite-in-place precedence and server-corrected prediction contract."]
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "both",
        contract = "listen",
        staged_surface = "interactions",
        version = 3,
        context = "registration",
        neoforge = "RegisterCauldronInteractionEvent"
    )]
    fn register_cauldron_interaction(interactions: &mut dyn CauldronInteractions);
}
