//! The living travel gate (#2414 PR 2): the door the `living_travel` hook
//! fires through, at the two boundaries the maintainer ruling names - the
//! player's `Player.travel` branch selection (`MotionWorld::player_travel`)
//! and the mob's `LivingEntity.travel` entry (`Mob::travel`). A travel that
//! runs there asks this gate first; with no listener, or on a client's own
//! prediction, the resolved input and velocity stand and the tick cost is
//! one branch.
use mod_api::TravelMode;
use sunrise_core::geometry::Vec3;
use sunrise_world::native_load_context::{LoopContext, LoopSide};
use sunrise_world::travel::TravelMode as GameTravelMode;

/// The `living_travel` hook's target (#2414): the generated dispatch the
/// catalog's generator requires of a target, with the loop's own reader as
/// the context. The travel gate uses the same generated dispatch.
pub fn native_living_travel(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    mode: TravelMode,
    input: &mut [f64; 3],
    velocity: &mut [f64; 3],
    canceled: &mut bool,
) {
    let reader = sunrise_client_render::api_context::LoopReader(context);
    mod_api::dispatch_living_travel(
        &reader.context(),
        entity,
        kind,
        mode,
        input,
        velocity,
        canceled,
    );
}

/// The mode the mod API names for a game-side [`GameTravelMode`]
/// (`living_travel` of #2414): the game's own branch names, one for one.
fn mode_of(mode: GameTravelMode) -> TravelMode {
    match mode {
        GameTravelMode::Air => TravelMode::Air,
        GameTravelMode::Water => TravelMode::Water,
        GameTravelMode::Lava => TravelMode::Lava,
        GameTravelMode::Glide => TravelMode::Glide,
        GameTravelMode::Flying => TravelMode::Flying,
        GameTravelMode::VelocityOnly => TravelMode::VelocityOnly,
    }
}

/// Ask one travel invocation (#2414) about to run for `entity` of `kind` in
/// `mode`'s branch: the `living_travel` hook fires here - in this one
/// function - on the authoritative side, and only when a listener
/// subscribes; with none, or on a client's own prediction, the resolved
/// values stand and the cost is the subscriber branch. `mode` names the
/// branch only when a listener asks for it. A listener that cancels skips
/// the invocation - its nested move does not run and the flying vertical
/// damping is skipped; what the tick already did is not undone. A
/// replacement of `input` (each component within [-1, 1]) or `velocity`
/// (finite, each component within ±64 blocks) supplies the branch's
/// starting values. The dispatch checks each listener's pair as it
/// returns: one with a component that is not a finite number, or outside
/// its budget, is refused whole, naming the mod and the field, and the
/// pair the preceding listeners left stands (the values the game resolved
/// before any), never clamped. If a listener unwinds, only its proposal is
/// discarded and the callback is disabled for this hook for the session.
/// Earlier results stand and later eligible listeners still get the travel.
///
/// Returns whether the travel runs; `input` and `velocity` are the values
/// the branch starts from.
#[inline(never)]
pub fn decide(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    mode: impl FnOnce() -> GameTravelMode,
    input: &mut Vec3,
    velocity: &mut Vec3,
) -> bool {
    if context.side() != LoopSide::Server {
        return true;
    }
    if !mod_api::__private::listened(mod_api::HookId::LivingTravel) {
        return true;
    }
    let mode = mode_of(mode());
    let mut replaced_input = [input.x, input.y, input.z];
    let mut replaced_velocity = [velocity.x, velocity.y, velocity.z];
    let mut canceled = false;
    let reader = sunrise_client_render::api_context::LoopReader(context);
    mod_api::dispatch_living_travel(
        &reader.context(),
        entity,
        kind,
        mode,
        &mut replaced_input,
        &mut replaced_velocity,
        &mut canceled,
    );
    if canceled {
        return false;
    }
    // The dispatch checked each listener's pair as it returned (budgets in
    // `refused_travel_output`): what stands here is the last accepted pair.
    *input = Vec3::new(replaced_input[0], replaced_input[1], replaced_input[2]);
    *velocity = Vec3::new(
        replaced_velocity[0],
        replaced_velocity[1],
        replaced_velocity[2],
    );
    true
}
