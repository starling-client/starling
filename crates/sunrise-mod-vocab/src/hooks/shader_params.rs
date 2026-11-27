//! The `shader_params` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The values native mods hand their shader packs in this frame,
    /// `partial` ticks past the last tick (0 to 1): a pack's WGSL reads
    /// them as `frame.mods` (eight `vec4<f32>`). Write the slots your own
    /// pack reads; a mod that bundles its pack as `native-<mod id>` is the
    /// one that pack is for. Asked once a frame while a pack is on; a value
    /// that isn't finite reads as 0 (`native_shader_params_now`).
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_shader_params",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "none (Sunrise-only; NeoForge has no counterpart)"
    )]
    fn shader_params(partial: f32, params: &mut [[f32; 4]; 8]);
}
