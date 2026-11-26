//! The `level_entities` area: entities and players in a level, and what a level spawns as items and experience. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_entities` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelEntitiesBackend {
    /// Side: server.
    /// The engine side of [`LevelRef::entity_by_uuid`], whose doc holds the contract.
    #[manual_handle]
    fn level_entity_by_uuid(&self, level: LevelRef, uuid: EntityUuid) -> Option<EntityRef>;
    /// Side: server.
    /// Visit members of the server level's entity lookup (`ServerLevel.getAllEntities`),
    /// including spectators and entities outside any spatial query. Dragon parts are not
    /// lookup members. Entities in hidden sections and vehicles being removed are skipped.
    ///
    /// The visitor can make shared reads on the same host, including another visit.
    /// No residency guard is held while it runs. Return `ControlFlow::Break(())` to stop;
    /// the visit returns `Ok(ControlFlow::Break(()))`. Returning `Continue(())` for every
    /// member gives `Ok(ControlFlow::Continue(()))`, including when there are no members.
    ///
    /// This does not build a result list. It walks eligible lookup members lazily in
    /// admission order, skipping any member that cannot resolve (matching
    /// `ServerLevel.getAllEntities` and [`LevelRef::visit_entities_in`]), and stops
    /// immediately when `visit` returns `ControlFlow::Break(())` without touching later
    /// members.
    ///
    /// A stale level returns [`LevelReadError::Stale`]. A parked level or a call
    /// outside server world access returns [`LevelReadError::Unavailable`]. Neither
    /// calls the visitor. If you need to write through the host, collect the handles
    /// you need and use them after the visit.
    ///
    /// Order is lookup admission order, preserved across temporary extraction;
    /// removal and re-admission moves an entity to the end. An entity spawned
    /// during play is admitted as it spawns, even when its id comes later. A
    /// chunk's entities join in the order it stored them: section by section,
    /// each in the order its entities entered it, as vanilla stores them. A
    /// server section crossing is recorded as the entity moves
    /// (`Callback.onMove`), whether or not a mod listens: a move out and back is
    /// two crossings, and a passenger enters after its vehicle. Minecarts,
    /// boats, projectiles, items, primed TNT, falling blocks, fireworks, orbs and
    /// the other server entities that report after their own update count
    /// several moves within one tick as one, from where it was to where it
    /// ended; a block piston's push is recorded only while a mod listens to
    /// `entity_entering_section`. Entities kept outside the residency families
    /// (lingering potion clouds, lightning bolts, eyes of ender) are not
    /// members.
    fn level_visit_all_entities(
        &self,
        level: LevelRef,
        visit: &mut dyn FnMut(EntityRef) -> core::ops::ControlFlow<()>,
    ) -> Result<core::ops::ControlFlow<()>, LevelReadError>;
    /// Side: both.
    /// Visit readable entities whose bounding boxes intersect this box (`Level.getEntities`).
    /// Face-only contact does not intersect. Inverted bounds are swapped on each axis,
    /// as in vanilla's `AABB`.
    ///
    /// The visitor can make shared reads on the same host, including another visit.
    /// No residency guard is held while it runs. Return `ControlFlow::Continue(())`
    /// to keep visiting; `ControlFlow::Break(())` stops before checking any later match.
    /// The result is `Ok(ControlFlow::Continue(()))` on completion (including an empty
    /// query) or `Ok(ControlFlow::Break(()))` on an early stop. No result list is built.
    /// Collect the handles you need only if you need them after the visit, for example
    /// to write through the host.
    ///
    /// A stale level returns [`LevelReadError::Stale`]. Non-finite bounds, a parked
    /// server level, a client that has not logged in or is not displaying this level,
    /// or unavailable world access return [`LevelReadError::Unavailable`]. A refused
    /// visit does not call the visitor.
    ///
    /// Ordinary entities are visited in level section order, and in entry order within
    /// each section. This includes block, item and text displays on both sides, as well as
    /// the client's own player if not spectating and placed non-spectator guests on the
    /// server. The dragon's intersecting parts come last, in vanilla part-table order.
    ///
    /// Displays have zero-sized physical boxes: their position must be strictly between
    /// the query's faces on all axes. The client uses the display's current tick position
    /// after movement interpolation, not the last destination or its render culling box.
    /// Ordinary crosshair picking still ignores displays.
    ///
    /// Unresolved handles, including held-out server entities, and entities in hidden
    /// server sections are skipped. Eyes of ender and leash knots are not visited. The projectile
    /// list is visible during `projectile_impact`, including live owners of hits that would consume them.
    fn level_visit_entities_in(
        &self,
        level: LevelRef,
        min: [f64; 3],
        max: [f64; 3],
        visit: &mut dyn FnMut(EntityRef) -> core::ops::ControlFlow<()>,
    ) -> Result<core::ops::ControlFlow<()>, LevelReadError>;
    /// Side: server.
    /// Visit `ServerLevel.players` in level player order, without building a list.
    /// In the active level, the local or acting player comes first, then the placed
    /// guests in server order. A parked level has only its placed guests. Each
    /// resolvable player is visited once under their own UUID. Fake players and
    /// players displaced by a nested guest turn are skipped.
    ///
    /// The visitor can make shared reads on the same host. Return `ControlFlow::Continue(())`
    /// to keep visiting or `ControlFlow::Break(())` to stop. The visit returns
    /// `Ok(ControlFlow::Break(()))` if stopped or `Ok(ControlFlow::Continue(()))` on
    /// completion, including a level with no players. A stale level returns
    /// [`LevelReadError::Stale`]; a client or unavailable world access returns
    /// [`LevelReadError::Unavailable`]. Neither error calls the visitor. Collect
    /// the players you need up front if you need mutable host access afterwards.
    fn level_visit_online_players(
        &self,
        level: LevelRef,
        visit: &mut dyn FnMut(PlayerRef) -> core::ops::ControlFlow<()>,
    ) -> Result<core::ops::ControlFlow<()>, LevelReadError>;
    /// Side: server.
    /// Visit experience orbs in `level` whose bounding boxes intersect `min`..`max`
    /// (`Level.getEntities(EntityTypeTest.forClass(ExperienceOrb.class), box, ...)`),
    /// in the level's order, without building a list. Face-only contact does not
    /// intersect, and inverted bounds are swapped on each axis as in vanilla's
    /// `AABB`. Orbs with `count == 0`, held-out orbs, and orbs pending removal are
    /// skipped. Works on both active and parked server levels.
    ///
    /// The visitor can make shared reads on the same host (including
    /// [`ExperienceOrbRef::value`], [`ExperienceOrbRef::count`], and
    /// [`ExperienceOrbRef::age`]). Return `ControlFlow::Continue(())` to keep
    /// visiting or `ControlFlow::Break(())` to stop early. Returns
    /// `Err(LevelReadError::Stale)` for a stale level and
    /// `Err(LevelReadError::Unavailable)` on a client, outside world access, or for
    /// non-finite bounds, without calling the visitor.
    fn level_visit_experience_orbs(
        &self,
        level: LevelRef,
        min: [f64; 3],
        max: [f64; 3],
        visit: &mut dyn FnMut(ExperienceOrbRef) -> core::ops::ControlFlow<()>,
    ) -> Result<core::ops::ControlFlow<()>, LevelReadError>;
    /// Side: server.
    /// Every entity the segment from `from` to `to` passes through, nearest first: the test
    /// `ProjectileUtil.getEntityHitResult` makes on each candidate, each entity's box
    /// inflated by `margin` on every side (`AABB.inflate`: positive grows, negative shrinks)
    /// and clipped against the segment. A ray that starts inside the adjusted box does not
    /// hit that entity, as in vanilla. `exclude` (the shooter) is skipped. Hits at equal
    /// returned distances keep spatial traversal order. Candidates are those
    /// [`LevelRef::visit_entities_in`] finds, so the same kinds are left out. Returns an
    /// empty list on a valid miss and `None` for non-finite geometry or a refused spatial
    /// query.
    ///
    /// Use this when you need every hit or must process hits in nearest-first order.
    /// If you just need the nearest hit matching a filter (such as `canHitEntity` or
    /// `isPickable`), use [`LevelRef::ray_trace_nearest_entity`] instead.
    fn level_ray_trace_entities(
        &self,
        level: LevelRef,
        from: [f64; 3],
        to: [f64; 3],
        margin: f64,
        exclude: Option<EntityRef>,
    ) -> Option<Vec<EntityRayHit>>;
    /// Side: both.
    /// The nearest accepted hit on the segment from `from` to `to`, without collecting hits.
    /// Uses the candidates of [`LevelRef::visit_entities_in`] in the segment's bounds padded
    /// by `margin.abs()`. Matching `ProjectileUtil.getEntityHitResult`, each non-excluded
    /// candidate is passed to `accept(entity)` in spatial traversal order **before** reading
    /// its bounding box and clipping against the segment. Accepted candidates have their box
    /// inflated by `margin` (`AABB.inflate`: positive grows, negative shrinks; any finite
    /// value is accepted) and clipped against the segment. A ray starting inside the
    /// adjusted box does not hit it, and `exclude` (for example, the shooter) is skipped
    /// before calling `accept`.
    ///
    /// `accept` can make shared reads on the same host; no residency guard is held. Among
    /// accepted candidates that the segment clips, the smallest squared distance from `from`
    /// wins. An exact tie in squared distance keeps the first visited hit. Use
    /// [`LevelRef::ray_trace_entities`] if you need an ordered list or must process hits
    /// nearest first.
    ///
    /// Returns `Ok(Some(hit))` on a match or `Ok(None)` when no accepted candidate is hit.
    /// A stale level returns [`LevelReadError::Stale`]. A non-finite margin, non-finite
    /// endpoints or padded bounds, a parked or undisplayed level, or unavailable world
    /// access return [`LevelReadError::Unavailable`]. A refusal does not call `accept`.
    fn level_ray_trace_nearest_entity(
        &self,
        level: LevelRef,
        from: [f64; 3],
        to: [f64; 3],
        margin: f64,
        exclude: Option<EntityRef>,
        accept: &mut dyn FnMut(EntityRef) -> bool,
    ) -> Result<Option<EntityRayHit>, LevelReadError>;
}
/// The handles' writes of the `level_entities` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelEntitiesBackendMut {
    /// Side: server.
    /// Drops `stack` as a broken block drops it (`Block.popResource`): an
    /// item entity at a random point within a quarter block of the middle of
    /// `pos` (three draws from the level random, made for an empty stack and
    /// with `block_drops` off too), with its own random motion and the
    /// default pickup delay of 10 ticks. None, with nothing spawned, for an
    /// empty stack or an unknown item, with the `block_drops` rule off, on a
    /// client, for a stale handle, a level not in place, once the entity ids run out, or an
    /// item a mod kept out (`entity_join_level`).
    /// Inside a transaction its join waits for the root commit, so a mod keeping the item out is not
    /// known at the call: the handle is returned and reads stale from the commit on.
    fn level_pop_resource(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        stack: ItemStack,
    ) -> Result<Option<EntityRef>, StackWriteError>;
    /// Side: server.
    /// Spawns `stack` as an item entity at `x y z` (`new ItemEntity(level, x,
    /// y, z, stack)` and `addFreshEntity`): its own random motion (up 0.2, up
    /// to 0.1 either way across), with signed `pickup_delay` ticks before it
    /// can be picked up, as `ItemEntity.setPickUpDelay` at construction
    /// (vanilla passes 10 for a block's drop, 40 for a player's throw, and
    /// 32767 for never). None, with nothing
    /// spawned, for an empty stack or an unknown item, a value that isn't
    /// finite, on a client, for a stale handle, a level not in place, or an
    /// item a mod kept out (`entity_join_level`).
    /// Inside a transaction its join waits for the root commit, so a mod keeping the item out is not
    /// known at the call: the handle is returned and reads stale from the commit on.
    fn level_spawn_item(
        &mut self,
        level: LevelRef,
        x: f64,
        y: f64,
        z: f64,
        stack: ItemStack,
        pickup_delay: i32,
    ) -> Result<Option<EntityRef>, StackWriteError>;
    /// Side: server.
    /// `spawn_item` with the item entity's motion set to
    /// `motion` (blocks a tick), as `new ItemEntity(level, x, y, z, stack,
    /// deltaX, deltaY, deltaZ)`, and the same signed `pickup_delay` ticks
    /// before it can be picked up (`ItemEntity.setPickUpDelay`). None in the
    /// same cases, and for a motion that isn't finite.
    fn level_spawn_item_with_motion(
        &mut self,
        level: LevelRef,
        x: f64,
        y: f64,
        z: f64,
        stack: ItemStack,
        motion: [f64; 3],
        pickup_delay: i32,
    ) -> Result<Option<EntityRef>, StackWriteError>;
    /// Side: server.
    /// Awards `amount` experience at `x y z` as vanilla's `ExperienceOrb.award`
    /// does: orbs of the largest sizes first (2477, 1237, ... 3, 1), each
    /// drawing the id `tryMergeToExisting` compares from the level random and
    /// merging into an orb of the same value whose box meets the 1 block box
    /// around `x y z`, or else a new orb with its own random motion. The
    /// `block_drops` rule doesn't apply (vanilla's blocks check it before
    /// they award). True once the orbs are awarded, even if a mod keeps a new
    /// one out (`entity_join_level`). False, with no orbs, for 0, a
    /// value that isn't finite, on a client, for a stale handle or a level
    /// not in place.
    fn level_award_experience(
        &mut self,
        level: LevelRef,
        x: f64,
        y: f64,
        z: f64,
        amount: u32,
    ) -> bool;
}
