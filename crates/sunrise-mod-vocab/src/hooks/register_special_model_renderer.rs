//! The `register_special_model_renderer` hook: declarations the mod API generator reads (see
//! `hooks/mod.rs`).

pub trait Hooks {
    /// Register factories for special item model types (NeoForge's `RegisterSpecialModelRendererEvent`).
    /// Called once at client startup, before resources load. The last registration of a type wins,
    /// including for vanilla's types. Each `minecraft:special` node is decoded and baked afresh on
    /// resource reload; the baked renderer submits to a scoped collector when the item is drawn.
    /// A listener that panics or incurs a phase refusal leaves the previous factories and
    /// replacement reports unchanged.
    #[notes = r#"Use `event.renderers.register(kind, SpecialModelFactory::new(decode, bake))` with a
validated `Identifier` (bare paths default to `minecraft:`, matching vanilla and NeoForge). Use
`SpecialModelConfig::decode` to read the inline `model` fields other than `type` into your own
config type. The bake callback consumes that config and can keep geometry leases for the current
resource load. Reloading rebuilds those leases and renderers, without firing this hook again.
Only registrations from a completed listener count when choosing the last factory and reporting
replacements; a panic or phase refusal discards that listener's whole batch.

On each draw, `SpecialModelRenderer::extract_argument` reads a borrowed `SpecialModelStack`
and returns the argument for `submit`. Submit geometry or nested stacks through
`SpecialModelCollector`, using `with_pose` to scope local transforms. The game applies the
`base` model's display transforms, particle and GUI light, and handles foil, packed light,
overlay and outline through the normal item draw path.

An unknown type, a disabled winning factory, or a decode or bake error or panic refuses that
item definition. The game does not retry a built-in decoder or a lower pack. Decode, bake,
argument extraction and submit run as the mod that registered the type. A failed draw rolls
back the whole special layer. Ordinary refusals are reported on every failed invocation without
disabling the callback. A panic logs and shows an error toast naming the mod, and disables only
that registration's affected callback for the session, including after resource reloads.
Only the outermost refused layer draws the missing model."#]
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        staged_surface = "renderers",
        context = "registration",
        neoforge = "RegisterSpecialModelRendererEvent",
        version = 3
    )]
    fn register_special_model_renderer(renderers: &mut dyn SpecialModelRenderers);
}
