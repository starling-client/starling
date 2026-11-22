//! The client's stored pick decision, at both vanilla pick call sites.

pub trait Hooks {
    /// Whenever vanilla picks: once per client tick at `partial_tick = 1.0`,
    /// after the game mode tick and before key handling (also while paused),
    /// and once per frame at the frame's game-time partial tick before extraction.
    /// Both sites overwrite the same stored result. `origin` is the world-space
    /// XYZ eye position of the camera entity used by this pick: interpolated at
    /// the frame's partial tick, current for the tick pick. `original` is the
    /// vanilla answer; block and entity candidates are supplied independently.
    /// Each listener receives the same inputs. Only `result` is shared between
    /// listeners: it starts at `original` and may be replaced or cleared in
    /// dependency order. Tick inputs, including both hands of use, read
    /// the tick's result without another pick. The highlight reads the frame's
    /// result, which can differ from the tick's. Reentrant crosshair_target
    /// reads see `original`. A replacement is validated once at writeback;
    /// invalid output becomes a miss, never a recomputed pick. Consumption does
    /// not recheck block state or entity liveness. Server checks and item-specific
    /// rays (including buckets and brushes) are unchanged. No NeoForge 26.3 counterpart.
    #[hook(
        dispatch = "api_carrier",
        side = "client",
        version = 2,
        contract = "collect",
        out = "result",
        event_per_listener = true,
        neoforge = "none (vanilla Minecraft.pick, before tick inputs and frame extraction)"
    )]
    fn client_pick(
        partial_tick: f32,
        origin: [f64; 3],
        original: Option<CrosshairTarget>,
        block_candidate: Option<CrosshairTarget>,
        entity_candidate: Option<CrosshairTarget>,
        result: &mut Option<CrosshairTarget>,
    );
}
