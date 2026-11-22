//! The mid-setupAnim humanoid animation moment.

pub trait Hooks {
    /// Once per actual `HumanoidModel.setupAnim` invocation, after head/walk/riding
    /// setup and before arm priority/posing. `event.parts` lends the live
    /// [`crate::HumanoidPose`] for this callback, not a snapshot to replace or keep.
    /// `get` reads a present base part's local position and rotation; `set_position`
    /// and `set_rotation` write all three components immediately on any present base
    /// part, matching `HumanoidModel.setupAnim` with no finiteness gate. Later listeners
    /// see edits from earlier ones. Later vanilla ITEM, attack, crouch, bob and swim
    /// animation still apply. Armor stands then replace rotations with their saved pose,
    /// preserving position edits.
    /// `humanoid` contains this invocation's held stacks and use state. Time is
    /// captured for its view; `game_time` is absent outside a level, never wall time.
    /// The Host is read-only. No frame deduplication: layers that actually animate
    /// again get another invocation; copying parent poses does not dispatch.
    #[hook(
        receiver = "read",
        dispatch = "api_carrier",
        side = "client",
        contract = "listen",
        version = 4,
        event_per_listener = true,
        neoforge = "none (vanilla HumanoidModel.setupAnim; NeoForge has no mid-setupAnim event)"
    )]
    fn humanoid_setup_anim(
        entity_id: u32,
        entity_type: &str,
        humanoid: &HumanoidRenderState<'_>,
        partial_tick: f32,
        game_time: Option<i64>,
        parts: &mut dyn HumanoidPose,
    );
}
