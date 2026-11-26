//! The `level_fluids` area: fluid states, shapes, heights and their flow step. Handle reads' backend declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_fluids` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelFluidsBackend {
    /// Side: both.
    /// `FluidState.getFlow` at `pos`: the current that pushes an entity there, after the
    /// `fluid_flow` hook, as entities feel it. Zero where no fluid flows. None as for
    /// `block_state`.
    fn level_fluid_flow(&self, level: LevelRef, pos: [i32; 3]) -> Option<Vec3>;
    /// Side: server.
    /// The fluid state at `pos` of the level `level` names (`BlockGetter.getFluidState`). None
    /// for a stale handle, a chunk that isn't loaded or a `y` outside the world: a read never
    /// loads a chunk.
    fn level_fluid_state(&self, level: LevelRef, pos: [i32; 3]) -> Option<FluidState>;
    /// Side: both.
    /// The fluid's shape at `pos` (`FluidState.getShape`), as block-local boxes: none for no
    /// fluid, else one box from the floor to the fluid's height (`FluidState.getHeight`), a
    /// full block under the same fluid, else the fluid's own height (`amount / 9`). None as for
    /// `block_state`; an unloaded block above counts as no fluid there.
    fn level_fluid_shape(&self, level: LevelRef, pos: [i32; 3]) -> Option<Vec<[f64; 6]>>;
    /// Side: server.
    /// `Fluid.getTickDelay(level)`: the game ticks between a flow step of `fluid` at `pos` and the
    /// next. Water is 5, lava 30 (10 in the Nether, where it flows fast) and the empty fluid 0; a
    /// flowing form answers as its source. A mod's world fluid answers what it declared, as its
    /// `fluid_rules` parts change it at `pos`, floored at 1. None for a stale handle, a client,
    /// a level parked while the server ticks another, a position that isn't loaded or is outside
    /// the world, and a fluid with no world form.
    fn level_fluid_tick_delay(
        &self,
        level: LevelRef,
        fluid: FluidKey,
        pos: [i32; 3],
    ) -> Option<i32>;
    /// Side: both.
    /// `FluidState.getHeight(level, pos)` of the fluid at `pos`: 0 for none, a full block under
    /// the same fluid, else the fluid's own height (`amount / 9`). None as for `block_state`.
    /// An unloaded block above counts as no fluid there.
    fn level_fluid_height(&self, level: LevelRef, pos: [i32; 3]) -> Option<f32>;
    /// Side: both.
    /// The surface height of the fluid at `pos` that the camera's immersion test reads: a source
    /// with a block above whose underside is sturdy fills its cell (1), anything else is
    /// `fluid_height`. None as for `fluid_height`.
    fn level_fluid_height_for_camera(&self, level: LevelRef, pos: [i32; 3]) -> Option<f32>;
    /// Side: both.
    /// `FluidState.canBeReplacedWith(level, pos, fluid, direction)` for the fluid at `pos`: whether
    /// `fluid` flowing in `direction` takes its place. The empty fluid yields to anything, water
    /// only to a non-water fluid flowing down, lava to water once it stands 4/9 of a block high,
    /// and a mod's world fluid to nothing. False for a `fluid` with no world form. None as for
    /// `fluid_height`.
    fn level_fluid_can_be_replaced_with(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        fluid: FluidKey,
        direction: Direction,
    ) -> Option<bool>;
}
/// The handles' writes of the `level_fluids` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelFluidsBackendMut {
    /// Side: server.
    /// Run the flow step of the fluid at `pos` now (`FluidState.tick`, which `ServerLevel.tickFluid`
    /// calls for a due fluid tick): the fluid levels, spreads and schedules its next step with
    /// the same random draws, hooks and level events as a queued one. Ticking the empty fluid does
    /// nothing. The queue is not read or changed; `schedule_fluid_tick` queues one. False for a
    /// refused write, a stale or parked handle, a client and an unloaded or out-of-height
    /// position.
    fn level_fluid_tick(&mut self, level: LevelRef, pos: [i32; 3]) -> bool;
}
