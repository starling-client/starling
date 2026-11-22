//! The `living_get_projectile` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `LivingEntity.getProjectile(weapon)` (NeoForge's
    /// `LivingGetProjectileEvent`) for a player (`player` and `name`) or a
    /// mob (`entity`, `kind`): a skeleton or illusioner's shot, or the crossbow
    /// load of a pillager or piglin. `projectile` is the vanilla
    /// ammunition; replace it to choose another. A replaced stack is not taken
    /// from the inventory.
    #[hook(
        dispatch = "flat",
        side = "both",
        contract = "collect",
        out = "projectile",
        neoforge = "LivingGetProjectileEvent",
        version = 2
    )]
    fn living_get_projectile(
        player: Option<u128>,
        name: &str,
        entity: Option<u32>,
        kind: &str,
        weapon: &ItemStack,
        projectile: &mut Option<ItemStack>,
    );
}
