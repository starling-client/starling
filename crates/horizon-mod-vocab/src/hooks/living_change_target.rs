//! The server-side mob target change hook.

pub trait Hooks {
    /// A mob is about to change its target. `original_target` is the target
    /// requested by the game, or none when it is clearing the target.
    /// `new_target` starts as that value; a listener can redirect or clear it.
    /// Set `cancel` to retain the mob's existing target. `is_brain` identifies
    /// a brain-driven target change; false identifies a goal or other mob target
    /// setter. Handles refer to live entities and may be read through the host.
    /// A stale or invalid replacement, including a fake player, is refused.
    /// Authoritative server only.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        host = "read",
        target = "horizon_client::game::native_hooks::living_change_target::NativeHookHost<'_>::native_living_change_target",
        neoforge = "LivingChangeTargetEvent",
        cancellable = true
    )]
    fn living_change_target(
        mob: MobRef,
        original_target: Option<LivingRef>,
        new_target: &mut Option<LivingRef>,
        is_brain: bool,
        cancel: &mut bool,
    );
}
