//! Client item extensions, rebuilt with the equipment resource bindings.
pub trait Hooks {
    /// Register this load's worn equipment models, textures and item providers together.
    /// Registration is client-only and repeats on every resource load, before any reload listener
    /// applies. The validated bindings are installed with the candidate models. Models are baked
    /// once and may use any namespace owned by the registering mod (its manifest namespaces or a
    /// content package it serves via `served_by`);
    /// frame callbacks choose references, never geometry. A duplicate item provider or an unknown
    /// texture returned by a completed listener refuses preparation with a named diagnostic.
    /// A panic or phase refusal discards that listener's models, textures and item providers
    /// together. Do not retain references from a discarded registration. Server-only packages
    /// cannot listen.
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        staged_surface = "extensions",
        version = 3,
        context = "registration",
        neoforge = "RegisterClientExtensionsEvent"
    )]
    fn register_client_item_extensions(extensions: &mut dyn mod_api::ClientItemExtensionsRegistrar);
}
