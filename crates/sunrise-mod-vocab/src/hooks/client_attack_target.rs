pub trait Hooks {
    /// Before a client's ordinary melee click attacks entities (not a spear jab), after the
    /// interaction-key cancellation hook. `targets` starts with the crosshair entity, if any.
    /// Replace, clear or append targets for redirected or multi-target hits. There is no batch
    /// cap. Duplicates are sent once, in first-occurrence order. The committed crosshair entry keeps its tick-pick point and client
    /// occlusion policy; the tick's selection is not re-hit. Additional targets are rechecked for
    /// the current client epoch, attackability, reach, held-item range and occlusion. Clearing an entity hit does not mine the block behind
    /// it; an untouched empty list keeps vanilla's block/miss path. Server checks and cooldowns
    /// still apply to every attack. Returning `Flow::Cancel` cancels the whole batch.
    /// NeoForge's `AttackEntityEvent` is the corresponding attack notice; this hook is client-side only.
    #[notes = r#"### Client attack targeting

`client_attack_target` runs with a ClientHost from the remote client's start-attack input path after interaction cancellation and before ordinary melee dispatch. Spear jabs bypass it. Cancellation suppresses attacks; an explicit target batch is deduplicated in first-occurrence order and the unchanged crosshair entry preserves its committed tick-pick point and occlusion policy without re-hitting it. Additional targets are checked for lifetime, reach, held-item range and occlusion. The server remains authoritative."#]
    #[hook(
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "AttackEntityEvent",
        cancellable = true,
        version = 2
    )]
    fn client_attack_target(targets: &mut Vec<EntityRef>, canceled: &mut bool);
}
