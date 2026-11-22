//! The `can_start_swimming` hook declaration (see `hooks/mod.rs`).

pub trait Hooks {
    /// Replace `IEntityExtension.canStartSwimming` for the player being simulated.
    /// `Entry` is asked of a sprinting player who isn't already swimming or
    /// flying, before excluding passengers. `ShallowWater` is asked in local
    /// sprint eligibility when the vanilla shallow-water clause is false but
    /// the body touches a swimmable fluid; a `false` answer then means shallow.
    /// The first `Some(true)` or `Some(false)` wins. If no listener answers,
    /// Sunrise uses vanilla's saved-eye and feet-cell entry checks, or the
    /// ordinary vanilla shallow-water result. This does not change the player's
    /// fluid samples, flight or existing swim. Declare `side = "both"` and
    /// install the mod on both sides. Base the answer on facts available to
    /// both sides that tick; the server remains authoritative for position.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "both",
        version = 1,
        contract = "replace",
        out = "can_start_swimming",
        event_per_listener = true,
        host = "read",
        neoforge = "IEntityExtension.canStartSwimming"
    )]
    fn can_start_swimming(
        player: PlayerRef,
        side: LogicalSide,
        check: mod_api::SwimmingCheck,
        can_start_swimming: &mut Option<bool>,
    );
}
