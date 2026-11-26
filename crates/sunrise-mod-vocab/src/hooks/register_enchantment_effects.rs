pub trait Hooks {
    /// Register codecs in the enchantment value and entity effect type registries.
    /// Selected definitions bind their named codec once during resource loading.
    /// The decoded effect runs at its component's stage, not as a global hit listener.
    /// A refusing or panicking registration callback publishes none of its codecs.
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        staged_surface = "effects",
        context = "registration",
        neoforge = "RegisterEvent"
    )]
    fn register_enchantment_effects(effects: &mut dyn EnchantmentEffects);
}
