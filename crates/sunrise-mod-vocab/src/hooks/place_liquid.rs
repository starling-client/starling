//! The server liquid-container write.

pub trait Hooks {
    /// `LiquidBlockContainer.placeLiquid`, before the template fills `state` at
    /// `pos`. A bucket supplies a source `fluid`; spreading supplies the state
    /// that would flow there. Live structure templates supply the displaced
    /// fluid or a neighbouring source, without asking `can_place_liquid`.
    /// The first `Some(bool)` replaces the template. Return true after storing
    /// the fluid and scheduling any tick it needs, or false when it was not
    /// placed. `None` leaves it to the next part, then to the template. A plain
    /// bucket still empties on false, unlike a validated fluid-container operation.
    ///
    /// Server only. Player buckets and the fluid utility journal each part's
    /// writes separately; only `Some(true)` keeps its writes in the outer
    /// transaction. Spreading, dispensers and live template restores use the
    /// direct level write host: don't write before returning `None` or false.
    /// If a transaction is open, these paths log and skip the part, leaving
    /// the template's answer. On either host, `container_signal` reads stored
    /// slots without generating deferred loot.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "replace",
        scope = "block",
        key = "state",
        out = "answer",
        target = "sunrise_client::game::native_hooks::place_liquid::GameLevel<'_>::native_place_liquid",
        neoforge = "none (vanilla LiquidBlockContainer.placeLiquid)"
    )]
    fn place_liquid(
        pos: BlockPos,
        state: BlockState,
        fluid: mod_api::FluidState,
        answer: &mut Option<bool>,
        #[engine] ticks: &mut Ticks,
    );
}
