//! The `living_conversion_post` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A mob converted into another type (NeoForge's
    /// `LivingConversionEvent.Post`): `converted` replaces `entity`. Fires
    /// after the finalized new mob joined the level and before the old mob
    /// is removed, for the conversions `living_conversion_pre` tells.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "LivingConversionEvent.Post",
        cancellable = false
    )]
    fn living_conversion_post(entity: u32, kind: &str, converted: u32, converted_kind: &str);
}
