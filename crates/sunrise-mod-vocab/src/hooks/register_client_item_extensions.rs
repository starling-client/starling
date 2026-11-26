//! Client item extensions, rebuilt with the equipment resource bindings.
pub trait Hooks {
    /// Register worn equipment providers on the client, like NeoForge's RegisterClientExtensionsEvent.
    /// Each client resource load bakes the requested models and binds the requested atlas sprites
    /// before reload listeners apply. Models may use any valid namespace. Draw callbacks borrow
    /// the live stack and asset layer and select from those bindings. An ordinary registration
    /// error rejects only that call and preserves other successful registrations. A panic discards only
    /// that listener's batch. Do not keep references from a discarded
    /// batch. Server-only packages cannot listen.
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        staged_surface = "extensions",
        version = 4,
        context = "registration",
        neoforge = "RegisterClientExtensionsEvent"
    )]
    fn register_client_item_extensions(extensions: &mut dyn mod_api::ClientItemExtensionsRegistrar);
}
