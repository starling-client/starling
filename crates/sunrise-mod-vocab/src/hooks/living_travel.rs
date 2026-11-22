//! The `living_travel` hooks and helpers: declarations the mod API generator
//! reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before the server's `Player.travel` branch (after swim steering) or a
    /// mob's `LivingEntity.travel` branch. `entity`, `kind` and `mode` identify
    /// the body and the branch it is about to run, including species overrides.
    /// A body that doesn't travel, such as a shulker, fires no hook. Neither does
    /// a replayed player tick. A branch that calls `entity_move` can still meet
    /// a move veto there.
    ///
    /// Replace `input` (finite components in [-1, 1]) or `velocity` (finite
    /// components within ±64 blocks per tick) to change the branch's starting
    /// values, not the branch itself. An unchanged vector is not subject to the
    /// replacement budget. A malformed vector drops that listener's entire
    /// result, including cancellation; later listeners see the preceding accepted
    /// values, and the log names the mod and refused field each time.
    ///
    /// Set `cancelled` or return `Flow::Cancel` to skip the branch, its nested
    /// moves and flying vertical damping. An accepted cancellation cannot be
    /// cleared. Work before travel, such as swim steering, is not undone. If a
    /// listener panics or incurs a phase refusal, only its own proposal is
    /// discarded and later eligible listeners still run. A panic disables this
    /// mod's callback for the session. The hook takes a loop context, not a `Host`.
    ///
    /// For a player, the veto takes effect at the server's travel boundary:
    /// reports it already accepted are not rewound. While the veto stands,
    /// the server corrects new movement reports to its accepted position.
    /// An edit that changes the input or velocity sends one correction, even
    /// if collision absorbs it, if the player remains eligible and no
    /// correction awaits acknowledgement. A separate teleport in the same
    /// tick takes priority. The correction carries the live server position
    /// and velocity. The server compares the vectors bit for bit, except that
    /// inputs within vanilla's movement dead zone are equivalent.
    /// The hook never runs on the client.
    /// Beyond NeoForge: there is no counterpart.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        context = "loop",
        cancellable = true,
        version = 2,
        output_validator = "crate::entity::types::refused_travel_output",
        neoforge = "none (fork addition; see travel-and-move.md, #2308)",
        target = "sunrise_client_lib::travel_gate::native_living_travel"
    )]
    fn living_travel(
        entity: u32,
        kind: &str,
        mode: TravelMode,
        input: &mut [f64; 3],
        velocity: &mut [f64; 3],
        canceled: &mut bool,
    );
}
