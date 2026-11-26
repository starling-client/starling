//! The `level_effects` area: sounds, particles, level and game events, and explosions. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' writes of the `level_effects` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelEffectsBackendMut {
    /// Side: server.
    /// Plays sound event `sound` at `at` for the players in this level
    /// within range (16 blocks, times the volume above 1), as
    /// `ServerLevel.playSound(except, ...)` does, under `source`'s volume
    /// slider. `except` leaves that player out (a client that already played the sound itself
    /// for its own action); None plays it to everyone, as `playSound(null, ...)`. A fake player
    /// has no client to leave out, so it excludes no one; a stale or client-side `except` queues
    /// nothing. The server sends registered events by ID and other valid keys by name, with
    /// volume and pitch as supplied. True means queued, not heard.
    /// The request joins the innermost open transaction in the level active at the call. The
    /// `play_level_sound_at_position` hook runs synchronously; only the resolved sound waits for
    /// the owning level's outermost commit, in journal order, and is discarded on abort. Delivery
    /// uses the level named by the request, even if parked. Outside a transaction, a queued sound
    /// draws its seed at the call even if a later hook cancels it. Inside one, it draws a seed only
    /// when the resolved sound is published.
    fn level_play_sound(
        &mut self,
        level: LevelRef,
        sound: &str,
        at: [f64; 3],
        source: SoundSource,
        volume: f32,
        pitch: f32,
        except: Option<PlayerRef>,
    ) -> bool;
    /// Side: server.
    /// Draws the next seed of the stream this level's sounds take theirs from
    /// (`Level.soundSeedGenerator.nextLong()`), without playing a sound: the draw vanilla's
    /// `Level.playSound` makes before it calls `playSeededSound`. A mod that models a
    /// `playSound` call draws here first and then plays the sound with
    /// [`LevelRef::play_seeded_sound`](crate::LevelRef::play_seeded_sound), or plays nothing,
    /// and the stream has moved exactly as vanilla's would. [`LevelRef::play_sound`](crate::LevelRef::play_sound),
    /// [`EntityRef::play_sound`](crate::EntityRef::play_sound), [`EntityRef::play_bound_sound`](crate::EntityRef::play_bound_sound)
    /// draw from the same stream, and so do the sounds the game itself sends; the seeded plays
    /// draw nothing. The server keeps one stream for all its levels, where vanilla gives each
    /// level its own: a draw here also moves the seeds of sounds played in other levels.
    ///
    /// The draw happens at the call, inside an open transaction too, and is not undone when
    /// the transaction aborts, as with [`LevelRef::with_random`](crate::LevelRef::with_random).
    /// A level sound such a transaction holds draws its own seed only when it is published at
    /// commit, so it takes a later seed than a draw made after it in the same transaction.
    /// `None` on a client, for a stale handle, a level that is not open and where the server
    /// sends no sounds.
    fn level_next_sound_seed(&mut self, level: LevelRef) -> Option<i64>;
    /// Side: server.
    /// [`LevelRef::play_sound`](crate::LevelRef::play_sound) with the seed the caller gives,
    /// as `ServerLevel.playSeededSound(except, x, y, z, sound, source, volume, pitch, seed)`
    /// plays it: the clients pick the sound's variant with `seed`, and the level's sound-seed
    /// stream does not move. Pair it with
    /// [`LevelRef::next_sound_seed`](crate::LevelRef::next_sound_seed) to model
    /// `Level.playSound`, or pass a seed vanilla code drew elsewhere (a `getRandom().nextLong()`).
    /// `except`, the checks, the `play_level_sound_at_position` hook and the transaction rules
    /// are `play_sound`'s; the seed is kept through them. True means queued, not heard.
    fn level_play_seeded_sound(
        &mut self,
        level: LevelRef,
        sound: &str,
        at: [f64; 3],
        source: SoundSource,
        volume: f32,
        pitch: f32,
        except: Option<PlayerRef>,
        seed: i64,
    ) -> bool;
    /// Side: server.
    /// Sends `count` particles around `at` to the players in this level
    /// within 32 blocks, as `ServerLevel.sendParticles` does: each offset is
    /// a Gaussian times `spread`, and `speed` scales their motion (a zero
    /// count sends one particle moving along `spread`). Block particle types
    /// need `state`; item particles need `options.item_stack` (validated against
    /// the item registry and stack limits). Trail and vibration remain unsupported.
    /// A custom particle uses its registered stream codec and is sent only to Sunrise
    /// clients. The receiving factory reads the same decoded value for every spawn.
    /// True means queued, not drawn.
    /// The request joins the innermost open transaction in the level active at the call, and waits
    /// for that level's outermost commit in journal order. An abort discards it. Delivery uses the
    /// level named by the request, even if parked.
    fn level_send_particles(
        &mut self,
        level: LevelRef,
        options: &ParticleOptions,
        state: Option<BlockState>,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool;
    /// Side: server.
    /// Sends the same particle effect around each entry of `positions`, in slice order,
    /// including repeated positions. Each sends a separate packet to players in the named
    /// level whose block centre is strictly less than 32 blocks away, as in
    /// `ServerLevel.sendParticles`. There is no limit on the number of positions.
    ///
    /// Uses the same validation as `LevelRef::send_particles`. If any position or shared
    /// argument is invalid, nothing is queued and the call returns false. An empty slice
    /// returns true without checking the particle arguments or queueing anything, but a
    /// stale or unknown level or a call on the logical client is still refused.
    /// True means accepted, not that a player received or drew any particles.
    ///
    /// The requests join the innermost open transaction in the level active at the call.
    /// They wait for that level's outermost commit in journal order and are discarded on
    /// abort. Delivery uses the level named by the request, even if parked.
    #[forward_api_inputs]
    fn level_send_particles_batch(
        &mut self,
        level: LevelRef,
        options: &ParticleOptions,
        state: Option<BlockState>,
        positions: &[[f64; 3]],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool;
    /// Side: server.
    /// Plays level event `id` at `pos` with `data` for the players near it
    /// (`Level.levelEvent(null, id, pos, data)`): `1000`s are sounds, `2000`s
    /// particles, as vanilla's `LevelEvent` numbers them (`2001` breaks a
    /// block with state id `data`). No player is left out, the acting one
    /// included. False for an event that carries a block state (2001, 2014,
    /// 3008) when `data` isn't one.
    /// Inside an open transaction, delivery waits for the outermost commit for the owning level
    /// (the level active at the call), in journal order, and is discarded on abort.
    fn level_level_event(&mut self, level: LevelRef, id: i32, pos: [i32; 3], data: i32) -> bool;
    /// Side: server.
    /// Plays the break effect of `state` at `pos`: the particles and the sound of a block of
    /// that state breaking, for the players near it (`Level.levelEvent(2001, pos,
    /// Block.getId(state))`). Nothing is written: the block at `pos` stays. False off the level
    /// in place, for a stale handle and on a client.
    /// Inside an open transaction, follows the same commit/rollback deferral as
    /// [`level_level_event`](Self::level_level_event).
    fn level_block_break_effect(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockState,
    ) -> bool;
    /// Side: server.
    /// Plays level event `id` at `pos` with `data` for every player on the
    /// server, in every level and at any distance (`ServerLevel.globalLevelEvent`).
    /// Vanilla's ids are the wither's spawn (`1023`), the dragon's death
    /// (`1028`) and an End portal opening (`1038`): the client has a sound for
    /// them only through this global form, played two blocks from the camera
    /// towards the position the player is sent. Each player gets a position
    /// of their own: `pos` in this level within 32 blocks of its centre, the
    /// block 32 blocks from them towards that centre further away, and the
    /// block they stand in for a player in another level.
    /// With the `global_sound_events` game rule off it is a plain
    /// [`level_event`](Self::level_level_event), as vanilla falls back: only
    /// the players of this level near `pos` get it. False for the same events
    /// and data `level_event` refuses, a parked level, a stale handle or a
    /// client.
    /// Inside an open transaction, delivery waits for the outermost commit for the owning level
    /// (the level active at the call). It checks `global_sound_events` at commit, publishes in
    /// journal order and is discarded on abort.
    fn level_global_level_event(
        &mut self,
        level: LevelRef,
        id: i32,
        pos: [i32; 3],
        data: i32,
    ) -> bool;
    /// Side: server.
    /// Sends game event `event` (`minecraft:block_change`...) from `at`, as
    /// `Level.gameEvent(event, at, context)` does: sculk sensors, shriekers,
    /// wardens and allays receive the optional source and affected state. An accepted game event
    /// is posted at the call, even in a transaction; it is not a deferred presentation effect.
    /// Returns false without emitting for an unknown event, invalid position,
    /// stale or foreign source, unregistered state, client or inactive level.
    fn level_game_event(
        &mut self,
        level: LevelRef,
        event: &str,
        at: [f64; 3],
        context: GameEventContext,
    ) -> bool;
    /// Side: server.
    /// Explodes at `center` as vanilla's full `Level.explode(source,
    /// damageSource, calculator, x, y, z, radius, fire, interaction,
    /// smallParticles, largeParticles, blockParticles, sound)` does
    /// (`native_explode`), `source` being the exploding entity, or none. The
    /// causing entity comes from it, never from the caller: a primed TNT's
    /// owner, a living entity itself, a projectile's owner when that is
    /// living, else none. With both the damage is
    /// `minecraft:player_explosion`, else `minecraft:explosion`, unless
    /// `settings` names a damage type, and a kill credits the causing entity.
    /// `interaction` says what happens to blocks (`Mob` asks the source's
    /// `entity_mob_griefing`, `Trigger` only works buttons, levers, doors and
    /// the like); `fire` lights some of the air around whatever it is.
    /// `settings` holds the damage calculator, particles and sound, each
    /// vanilla's default when `None`. There is no radius limit, as in
    /// vanilla: a big blast costs the blocks its rays find, and one of 0 or
    /// below finds none. Refused, with nothing exploded and no hook told, as
    /// [`ExplodeError`] says.
    fn level_explode(
        &mut self,
        level: LevelRef,
        source: Option<EntityRef>,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        interaction: ExplosionInteraction,
        settings: ExplosionSettings,
    ) -> Result<(), ExplodeError>;
    /// Side: server.
    /// The resistance the blast `level.explode(host, source, center, radius, fire, interaction)`
    /// would set off would meet in the cell at `pos`, as its rays read it
    /// (`ExplosionDamageCalculator.getBlockExplosionResistance`), without exploding anything. The
    /// larger of the block's answer (its parts' `explosion_resistance` first, then its own value)
    /// and the fluid's (water and lava 100); `NoResistance` for air with no fluid. The blast's
    /// calculator is the one `explode` gives that source, read from the same selection: as
    /// vanilla's `EntityBasedExplosionDamageCalculator` asks the source, a primed TNT minecart's
    /// rails and the blocks under them resist 0 (and the blast spares them), and a dangerous
    /// wither skull caps a block it may destroy at 0.8; any other source has the default one.
    /// A respawn anchor's water cell
    /// belongs to the anchor's own sourceless blast, which no description names, so no read sees
    /// it. The causing entity and damage type come from `source` as for `explode`, and parts are
    /// asked with that `ExplosionStart`. `Unavailable` for a stale level or `source`, a chunk
    /// that isn't loaded (it is never loaded) or a state the game doesn't know; None only from a
    /// host without a game. The level does not change.
    fn level_explosion_resistance(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        source: Option<EntityRef>,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        interaction: ExplosionInteraction,
    ) -> Option<ExplosionResistance>;
    /// Side: server.
    /// `level_explosion_resistance` for every cell of `cells`, one answer a cell in the order
    /// `cells` gives them. A state is asked once per call only when every part on its block that
    /// answers `explosion_resistance` declares the answer pure
    /// (`BlockPart::explosion_resistance_is_pure`); otherwise its parts are asked once a cell.
    /// Refused, with nothing read, for an inverted area or an area whose cell count
    /// overflows what one allocation can hold: it runs on the server tick, so read big areas in
    /// pieces.
    fn level_explosion_resistances(
        &mut self,
        level: LevelRef,
        cells: &ResistanceCells,
        source: Option<EntityRef>,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        interaction: ExplosionInteraction,
    ) -> Result<Vec<ExplosionResistance>, ResistanceCellsError>;
    /// Side: server.
    /// Destroys the blocks at `positions` as the blast `level.explode(host, source, center,
    /// radius, fire, interaction)` would destroy them, without casting its rays, hurting or
    /// pushing entities, or any packet or sound: a mod that casts its own rays breaks what they
    /// found with it, spread over ticks if it likes. A Sunrise addition. Each call is one
    /// explosion to the hooks: `explosion_start` first, which may cancel it (then nothing breaks
    /// and the answer is 0), then, for a radius of at least 0.00001, `explosion_detonate` with
    /// the positions as its block list, in the order given, a repeated one once; listeners may
    /// remove, add or reorder positions (`ExplosionEvent.Detonate`), and the resulting list is
    /// kept in order with multiplicity intact, filtered only to loaded chunks within the level's
    /// height. Then the block step in vanilla's order: the positions shuffled with the level's
    /// random, and each block hit as
    /// the blast hits one (air skipped; `can_drop_from_explosion`; loot with the radius under
    /// the decay interaction; experience when the causing entity is a player;
    /// `on_block_exploded`, whose default sets air and primes TNT owned by the causing entity),
    /// the drops of the whole call merged and popped after the last block; then, with `fire`, a
    /// third of the cells left air above a solid block catch fire, under `None` too. `None`
    /// breaks nothing else, and `Trigger` only works the blocks it may. Unlike vanilla's rays, a
    /// position in a chunk that isn't loaded is skipped, never loaded and not counted, and so is
    /// one outside the level's height. The number of blocks destroyed: those that were not air
    /// when their turn came. `Err`, with nothing changed, for a stale level or `source`, or a
    /// centre or radius that isn't finite.
    fn level_explosion_destroy_blocks(
        &mut self,
        level: LevelRef,
        positions: &[[i32; 3]],
        source: Option<EntityRef>,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        interaction: ExplosionInteraction,
    ) -> Result<u32, ExplosionDestroyError>;
}
/// The handles' client writes of the `level_effects` area, reached through [`ClientBackendMut`].
// hidden: mods call the handle, not the game-side implementation.
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelEffectsClientBackendMut {
    /// Side: client.
    /// Queues the ordinary hit crumb on `face` for the supplied state at `pos`
    /// (`ClientLevel.addBreakingParticles`). Uses the level random and the state's outline.
    /// Does not play a sound or redispatch the hit Part. Tint sources can still be asked.
    /// `Ok(false)` means no sprite or outline. Returns an error for an unavailable level or
    /// an unknown state. The engine still controls group admission.
    fn level_add_breaking_particles(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockState,
        face: Direction,
    ) -> Result<bool, LevelWriteError>;
    /// Side: client.
    /// Queues one `TerrainParticle` with the given world position and velocity. The sprite
    /// comes from the supplied state; `source_pos` is where its tint is sampled.
    /// Does not play a sound or redispatch the destroy Part. Tint sources can still be asked.
    /// `Ok(false)` means non-finite input or no sprite. Returns an error for an unavailable
    /// level or an unknown state. The engine still controls group admission.
    fn level_add_terrain_particle(
        &mut self,
        level: LevelRef,
        state: BlockState,
        source_pos: [i32; 3],
        at: [f64; 3],
        velocity: [f64; 3],
    ) -> Result<bool, LevelWriteError>;
}
