//! The `game_rule_changed` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server game rule `rule` (`minecraft:keep_inventory`) was set to
    /// `value` (`GameRuleChangedEvent`, from `GameRules.set` with a server:
    /// `/gamerule`, the game rules screen, the management protocol, a test
    /// environment, `server.properties`). `actor` is the `PlayerRef` when a player
    /// caused the change. `level.visit_game_rules(host, &mut visit)` on any server level
    /// visits the available rules, including the new value. Server-side.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "listen",
        version = 2,
        neoforge = "GameRuleChangedEvent"
    )]
    fn game_rule_changed(rule: &str, value: GameRuleValue, actor: Option<PlayerRef>);
}
