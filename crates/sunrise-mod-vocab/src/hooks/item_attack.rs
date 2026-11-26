//! The `on_left_click_entity` and `on_entity_swing` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `IItemExtension.onLeftClickEntity`: `player` (the acting player's
    /// UUID), holding `stack` in the main hand, is about to attack the entity
    /// `entity` (`Entity.getId`) of type `kind` (`minecraft:cow`,
    /// `minecraft:player`, `minecraft:item_frame`): where `Player.attack`
    /// starts, once the global `attack_entity` let the blow through and
    /// before any damage, knockback, enchantment effect, `hurt_enemy` or the
    /// weapon's wear. Set `canceled` and the attack ends there: the target
    /// is not hurt, `hurt_enemy` does not fire and the stack takes no wear.
    /// Only the `ItemPart`s attached to `stack`'s item are asked. Server-side,
    /// for every player of the world (the helpers read the acting one).
    /// Read the borrowed stack with `get`; call `project` only to keep an owned copy.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "canceled",
        scope = "item",
        key = "stack",
        version = 2,
        neoforge = "IItemExtension.onLeftClickEntity"
    )]
    fn on_left_click_entity(
        player: u128,
        stack: mod_api::ItemStackView<'_>,
        entity: i32,
        kind: &str,
        canceled: &mut bool,
    );
    /// `IItemExtension.onEntitySwing`: the player in place (`entity`, its
    /// `Entity.getId` when known) is about to swing `stack` in `hand`:
    /// where `LivingEntity.swing` starts, for an attack, a use or a swing the
    /// server asked for. Set `canceled` and no swing starts: no animation and
    /// no swing packet. Only the `ItemPart`s attached to `stack`'s item are
    /// asked. Both sides, as vanilla's: answer the same on both, and change
    /// the world only where this `Game` runs the world, not on a server's
    /// client.
    /// Read the borrowed stack with `get`; call `project` only to keep an owned copy.
    #[hook(
        dispatch = "flat",
        side = "both",
        contract = "cancel",
        out = "canceled",
        scope = "item",
        key = "stack",
        version = 2,
        neoforge = "IItemExtension.onEntitySwing"
    )]
    fn on_entity_swing(
        stack: mod_api::ItemStackView<'_>,
        entity: Option<i32>,
        hand: Hand,
        canceled: &mut bool,
    );
}
