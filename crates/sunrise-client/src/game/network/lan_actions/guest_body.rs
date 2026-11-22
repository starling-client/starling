//! Restore the guest's first body without borrowing another player's RNG.
use super::*;
use sunrise_client::{
    lan::{guest::Guest, hub::CommittedMove},
    landing::PlayerLandingPause,
    motion::{MotionResources, PlayerMotion, ServerTravel},
};
use sunrise_core::geometry::Vec3;

/// The server copy of the acting guest's body, borrowing its player and the level from `game`.
/// A macro so that the borrow can end where the game itself is needed (a hook's owning host)
/// and begin again after it.
macro_rules! motion_world {
    ($game:ident, $side:ident, $player:ident) => {
        MotionWorld {
            side: &$side,
            entities: &[],
            player: $player,
            scene: &mut $game.world,
            assets: &$game.pack.assets,
            collision_binding: $game.collision,
            resources: $game.motion,
            health: &$game.health_resources,
            tag_items: Some(world_items!($game)),
            loop_context: &$game.loop_context,
            damage_hooks: &$game.damage_hooks,
            block_registries: Some(&$game.pack.block_registries),
        }
    };
}

// End the low-level body loan before its death or protection has to use Game. At a
// distinct hurt site the next operation must already see the same hit's settlement.
macro_rules! settle_guest_hit {
    ($game:ident, $side:ident, $world:ident, $id:ident) => {
        if $world.player.health.damage_completion.is_some() {
            if let Err(collected) = $game.finish_guest_damage_before_resume() {
                return (collected, None, ServerTravel::Continue);
            }
            let Some(player) = $game.player.as_mut().filter(|p| p.entity_id as u32 == $id) else {
                return (Vec::new(), None, ServerTravel::Continue);
            };
            $side = game_side!($game);
            $world = motion_world!($game, $side, player);
        }
    };
}

pub(super) fn initialize(resources: &MotionResources, guest: &mut Guest) -> bool {
    let p = Vec3::new(guest.position[0], guest.position[1], guest.position[2]);
    let Ok(motion) = PlayerMotion::new(resources, p) else {
        return false;
    };
    let mut motion = motion;
    if let Some(tick_count) = guest.carried_tick_count.take() {
        motion.health.tick_count = tick_count;
    }
    guest.motion = Some(Box::new(motion));
    let entity_id = guest.entity_id as u32;
    let accepted_hit = guest.carried_hit.take();
    if let Some(motion) = guest.motion.as_mut() {
        motion.health.accepted_hit = accepted_hit;
        motion.random = sunrise_core::random::EntityRandom::own(
            sunrise_client::mobs::entity_random(entity_id, p),
        );
    }
    // The first body after joining burns, breathes and falls on as saved.
    let g = &mut *guest;
    if let Some(saved) = g.loaded_body.take()
        && let Some(m) = g.motion.as_mut()
    {
        saved.apply(m);
        let _ = m.set_walking_speed_base(g.abilities.walking_speed);
        // LivingEntity.readAdditionalSaveData: the saved attributes.
        let _ = m.apply_saved_attributes(g.saved.as_ref().and_then(|t| t.get("attributes")));
    }
    // ServerPlayer.restoreFrom: a body that follows another takes its
    // attributes (a scale of 0.1 stays 0.1 after a respawn or a trip
    // to another level).
    let g = &mut *guest;
    if let Some(carried) = g.carried_attributes.take()
        && let Some(m) = g.motion.as_mut()
    {
        let _ = m.apply_saved_attributes(Some(&carried));
    }
    // The saved golden hearts go on the first body of a player who
    // has been here before.
    let saved = guest.saved_absorption.take();
    if let (Some(m), Some(amount)) = (guest.motion.as_mut(), saved) {
        let _ = m.internal_set_absorption(amount);
    }
    let kept = guest.logs_kept.take();
    if let (Some(m), Some((hurts, attacks))) = (guest.motion.as_mut(), kept) {
        m.hurt_log = hurts;
        m.attack_log = attacks;
    }
    true
}

impl Game<'_> {
    /// `handlePlayerPositionChange`: complete the accepted movement before later
    /// packets and `ServerPlayer.tick` can observe the player.
    pub(in crate::game) fn guest_accepted_move(&mut self, conn: u32, commit: CommittedMove) {
        let CommittedMove {
            movement: accepted,
            horizontal_collision,
            from,
            jump_yaw,
        } = commit;
        let Some(g) = self.lan.as_ref().and_then(|host| host.hub.guest(conn)) else {
            return;
        };
        let id = g.entity_id as u32;
        let spectator = g.spectator();
        let dimensions = g.dimensions();
        let at = accepted.position;
        if let Some(yaw) = jump_yaw {
            self.guest_jump(conn, from, yaw);
        }
        if crate::game::entity_sections::crossed(from, at) {
            self.entity_section_step(id, from, at);
        }
        if spectator {
            return;
        }
        let Some(events) = self.as_guest(conn, |game| {
            let leather_boots = super::super::worn_boots::leather_boots(&game.armor);
            let boat = game.riding_boat_fluid_box();
            let Some(player) = game.player.as_mut() else {
                return Vec::new();
            };
            player.fluids.boat = boat;
            player.leather_boots = leather_boots;
            player.shift_key_down = accepted.sneaking;
            player.record_fall_reset = game.adv.marks.fall.is_some();
            let mut side = game_side!(game);
            let mut w = motion_world!(game, side, player);
            let [x, y, z] = accepted.delta;
            let movement = Vec3::new(x, y, z);
            if w.set_on_ground_with_movement(accepted.on_ground, horizontal_collision, movement)
                .is_ok()
                && !touching_unloaded_chunk(at, dimensions, |x, z| {
                    w.scene.chunks.contains_key(&(x, z))
                })
            {
                let landed = (|| -> sunrise_client::Result<_> {
                    // Entity.doCheckFallDamage queries support again before fallOn.
                    w.check_supporting_block(accepted.on_ground, movement)?;
                    let pos = w.legacy_on_position()?;
                    let state = w.block_state(pos)?;
                    w.check_guest_fall_damage(y, accepted.on_ground, state, pos)
                })();
                let tail = match landed {
                    Ok(Some(PlayerLandingPause::Trample(trample))) => {
                        let mut collected = std::mem::take(&mut w.player.events);
                        game.trample_farmland(id, true, trample);
                        let Some(player) =
                            game.player.as_mut().filter(|p| p.entity_id as u32 == id)
                        else {
                            return collected;
                        };
                        collected.append(&mut player.events);
                        player.events = collected;
                        side = game_side!(game);
                        w = motion_world!(game, side, player);
                        w.finish_trample_fall(trample).ok().flatten()
                    }
                    Ok(Some(PlayerLandingPause::Damage(tail))) => Some(tail),
                    Ok(None) | Err(_) => None,
                };
                if let Some(tail) = tail {
                    if let Err(collected) = game.finish_guest_damage_before_resume() {
                        return collected;
                    }
                    let Some(player) = game.player.as_mut().filter(|p| p.entity_id as u32 == id)
                    else {
                        return Vec::new();
                    };
                    side = game_side!(game);
                    w = motion_world!(game, side, player);
                    let _ = w.finish_guest_landing(tail);
                }
            }
            let player = w.player;
            if y > 0.0 {
                player.reset_fall_distance();
            }
            player.record_fall_reset = false;
            let moved = player
                .position
                .subtract(Vec3::new(from[0], from[1], from[2]));
            let side = game_side!(game);
            let _ = motion_world!(game, side, player).check_movement_statistics(moved);
            game.movement_stats(moved);
            let Some(player) = game.player.as_mut() else {
                return Vec::new();
            };
            let events = std::mem::take(&mut player.events);
            game.record_guest_motion_combat(&events);
            events
        }) else {
            return;
        };
        let fall_end = events.iter().find_map(|event| match event {
            MotionEvent::FallDistanceReset(at) => Some(at.components()),
            _ => None,
        });
        self.guest_motion_events(conn, at, events);
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some(gi) = host.hub.guests.connection_index(conn) else {
            return;
        };
        // LivingEntity.hurtServer: the turn already applied death protection.
        // If it did not save the guest, finish ServerPlayer.die before the
        // next packet can move them or change their drops.
        if host.hub.guests[gi].health <= 0.0 {
            self.guest_die(gi, conn);
        }
        // ServerPlayer.resetFallDistance: the first reset in this packet ends
        // the tracked fall. Use its position and the health after protection.
        if let Some(end) = fall_end
            && self
                .lan
                .as_ref()
                .and_then(|host| host.play.get(&conn))
                .is_some_and(|play| play.marks.fall.is_some())
        {
            self.as_guest(conn, |game| {
                if let Some(start) = game.adv.marks.fall.take()
                    && game
                        .player
                        .as_ref()
                        .is_some_and(|p| p.health.health() > 0.0)
                {
                    game.trigger_distance_at("minecraft:fall_from_height", start, Some(end));
                }
            });
        }
    }

    /// The guest's connection-path body: fluid and block effects, hazards,
    /// regeneration, food and travel (`ServerPlayer.doTick`). Accepted reports
    /// have already run their fall checks; the body still walks their history
    /// for `Entity.applyEffectsFromBlocks` and the movement a glide reads.
    pub(in crate::game) fn guest_server_tick(&mut self, conn: u32, body: bool) {
        // The last move's fall stands in for the server's delta movement.
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let fell = host
            .hub
            .guest(conn)
            .and_then(|g| g.moves.last().map(|m| m.delta[1]))
            .unwrap_or(0.0);
        self.guest_glide_tick(conn, fell);
        let Some((moves, from_ground)) = self.lan.as_mut().and_then(|host| {
            host.hub.update_guest(conn, |g| {
                let moves = std::mem::take(&mut g.moves);
                let from_ground = if moves.is_empty() {
                    g.on_ground
                } else {
                    g.moves_from_ground
                };
                (moves, from_ground)
            })
        }) else {
            return;
        };
        let g = self.lan.as_ref().unwrap().hub.guest(conn).unwrap();
        let id = g.entity_id as u32;
        if g.spectator() {
            // ServerPlayer.doTick runs the player's whole tick for a spectator
            // too, unless its box touches an unloaded chunk
            // (`touchingUnloadedChunk`, `guest_body_ticks`): its effects
            // (LivingEntity.baseTick), Player.aiStep's shoulder entities and
            // tickRegeneration, and foodData.tick go on while it watches
            // (#11312). The hazards, the travel and the movement statistics
            // below are what a spectator's body skips.
            if !body {
                return;
            }
            if !g.effects.list.is_empty() {
                self.as_guest(conn, |game| game.tick_mob_effects());
            }
            self.as_guest(conn, |game| {
                game.handle_shoulder_entities();
                let rules = game.world.health_rules;
                let Some(player) = game.player.as_mut() else {
                    return;
                };
                player.health.advance_body_clock();
                let _ = player.peaceful_regeneration(&game.loop_context, rules);
                let _ = player.food_tick(
                    &game.loop_context,
                    crate::game::entity::damage_level_of!(
                        game,
                        &sunrise_client::health::NO_ATTACK,
                        no_push,
                        None,
                        acting_equipment!(game)
                    ),
                    &game.health_resources,
                    rules,
                );
                game.finish_guest_damage();
                let Some(player) = game.player.as_mut().filter(|p| p.entity_id as u32 == id) else {
                    return;
                };
                let events = std::mem::take(&mut player.events);
                game.record_combat(&events);
            });
            return;
        }
        let at = g.position;
        let travel_teleport = g.teleport_counter;
        let dimensions = g.dimensions();
        let (sneaking, sleeping) = (g.sneaking, g.sleeping);
        let riding = g.vehicle.is_some();
        if g.motion
            .as_ref()
            .is_some_and(|m| m.auto_spin_attack_ticks > 0)
        {
            self.as_guest(conn, |game| game.server_spin_attack());
        }
        let in_wall = !sleeping && self.in_wall(conn, at, dimensions);
        let mut soul = None;
        let events = self.as_guest(conn, |game| {
            let (rules, motion) = (game.world.health_rules, game.motion);
            let health = &motion.health;
            let leather_boots = super::super::worn_boots::leather_boots(&game.armor);
            let freeze_immune = game.wears_freeze_immune();
            let boat = game.riding_boat_fluid_box();
            let swim_listener = game.has_swim_eligibility_listener();
            let Some(player) = game.player.as_mut() else {
                return (Vec::new(), None, ServerTravel::Continue);
            };
            player.fluids.boat = boat;
            player.leather_boots = leather_boots;
            let mut side = game_side!(game);
            let mut w = motion_world!(game, side, player);
            w.player.health.advance_body_clock();
            if w.begin_fluid_tick().is_ok() {
                let decision = if swim_listener { w.swim_entry() } else { None };
                let answer = game.ask_swim_entry(decision);
                let Some(player) = game.player.as_mut().filter(|p| p.entity_id as u32 == id) else {
                    return (Vec::new(), None, ServerTravel::Continue);
                };
                side = game_side!(game);
                w = motion_world!(game, side, player);
                if w.swim_and_burn_tick(answer).is_ok() {
                    settle_guest_hit!(game, side, w, id);
                    w.finish_fluid_base_tick();
                }
            }
            // Player.aiStep: a flying player that isn't riding starts every tick
            // with no fall distance, so flying down in creative and switching to
            // survival lands without the flight's distance.
            if w.player.flying && !riding {
                w.player.fall_distance = 0.0;
            }
            // The tick's whole movement, for the delta movement a glide's wall
            // hit reads (`handleMovePlayer` gives the server copy the client's
            // own movement).
            let vector = |[x, y, z]: [f64; 3]| Vec3::new(x, y, z);
            let tick_movement = moves.iter().fold(Vec3::ZERO, |sum, accepted| {
                sum.add_vector(vector(accepted.delta))
            });
            // Entity.move's movementThisTick, using the actual accepted endpoints.
            let end = w.player.position;
            let start = moves.first().map_or(end, |first| {
                vector(first.position).subtract(vector(first.delta))
            });
            w.player.history.clear();
            let mut from = start;
            for accepted in &moves {
                let to = vector(accepted.position);
                w.player
                    .history
                    .add(sunrise_world::entity_move::MovementRecord {
                        from,
                        to,
                        original: Some(vector(accepted.delta)),
                    });
                from = to;
            }
            let _ = w.server_stuck_in_blocks(start);
            // A ServerPlayer travels on the server too
            // (MoveSimulationType.AUTHORITATIVE_SIDE_AND_SERVER), so every tick
            // on a ladder or vine LivingEntity.handleOnClimbable resets its fall
            // distance: climbing down doesn't hurt where it lands (#1255).
            if w.climbable().unwrap_or(false) {
                w.player.fall_distance = 0.0;
            }
            // ServerGamePacketListenerImpl.handlePlayerPositionChange ends the
            // impulse context on LivingEntity.hasLandedInLiquid: a move into a
            // liquid that does not rise, or a wind charge that lands in water
            // would waive the fall after it.
            let liquid = w.player.fluids.was_touching_water || w.player.is_in_lava();
            let delta_y = w.player.velocity.y;
            w.player.impulse.landed_in_liquid(delta_y, liquid);
            // Entity.applyEffectsFromBlocks along those moves: magma, cactus,
            // lit campfires and grown sweet berry bushes hurt (#1305).
            w.player.shift_key_down = sneaking;
            let _ = w.player.set_sleeping(sleeping);
            w.player.freeze_immune = freeze_immune;
            let mut contact = None;
            loop {
                let progress = w.server_block_damage_until_hit(start, &mut contact);
                settle_guest_hit!(game, side, w, id);
                if !matches!(progress, Ok(false)) {
                    break;
                }
            }
            let _ = w.update_soul_speed_block();
            // LivingEntity.baseTick's air supply: decided here, then `living_breathe` is
            // asked with the game itself, which `w` borrows, and its answer applied.
            if let Ok(step) = w.decide_air() {
                // This tick's events so far, kept if the hook leaves no acting player.
                let mut collected = std::mem::take(&mut w.player.events);
                let answer = match step {
                    Some(step) if sunrise_client::breathing::heard() => {
                        game.ask_player_breath(step).map(|breath| (step, breath))
                    }
                    step => step.map(|step| (step, step.vanilla)),
                };
                let Some(player) = game.player.as_mut() else {
                    return (collected, None, ServerTravel::Continue);
                };
                collected.append(&mut player.events);
                player.events = collected;
                side = game_side!(game);
                w = motion_world!(game, side, player);
                if let Some((step, breath)) = answer {
                    let _ = w.apply_air(step, breath);
                    settle_guest_hit!(game, side, w, id);
                }
                let _ = w.tick_lava_contact();
                settle_guest_hit!(game, side, w, id);
            }
            let _ = w.apply_fire_contact();
            settle_guest_hit!(game, side, w, id);
            if w.check_below_world().is_ok() {
                settle_guest_hit!(game, side, w, id);
                if in_wall {
                    let _ = w.suffocation_hurt();
                    settle_guest_hit!(game, side, w, id);
                }
            }
            let mut events = std::mem::take(&mut w.player.events);
            // Powder snow and cauldrons put the fire out along those moves.
            game.block_extinguish_player(&events);
            // Entity.baseTick: rain puts out a burning player.
            game.rain_extinguish_player();
            // WitherRoseBlock.entityInside along those moves, and an
            // `entity_inside` part's typed effect writes.
            for event in &events {
                match event {
                    MotionEvent::InsideWitherRose => game.wither_rose_contact(),
                    MotionEvent::InsideEffect {
                        id,
                        duration,
                        amplifier,
                    } if game.remote.is_none() && game.ui.difficulty != 0 => {
                        game.give_player_effect(id, *duration, i32::from(*amplifier));
                    }
                    _ => {}
                }
            }
            // LivingEntity.baseTick: a new block position runs the worn
            // enchantments' location_changed effects (frost walker).
            game.location_changed_effects([start.x, start.y, start.z], from_ground);
            // LivingEntity.baseTick: outside the world border, then tickEffects.
            game.border_damage();
            game.tick_mob_effects();
            // `LivingEntity.baseTick` ran effects before the remaining dirty
            // attributes refresh in `LivingEntity.tick`. `Player.tick` then
            // chooses its pose, including for a passenger (#4509 / #3308).
            let Some(player) = game.player.as_mut() else {
                return (events, None, ServerTravel::Continue);
            };
            let side = game_side!(game);
            let mut w = motion_world!(game, side, player);
            if riding {
                let _ = w.player.refresh_attributes();
                let _ = w.passenger_pose_tick();
            } else {
                let _ = w.server_player_pose_tick();
            }
            game.apply_player_waypoint_updates();
            // Soul Speed's effects (LivingEntity.baseTick's location_changed
            // and tick effects); its particle goes to the players around.
            soul = game.tick_soul_speed();
            game.handle_shoulder_entities();
            let Some(player) = game.player.as_mut() else {
                return (Vec::new(), None, ServerTravel::Continue);
            };
            let _ = player.peaceful_regeneration(&game.loop_context, rules);
            let _ = player.food_tick(
                &game.loop_context,
                crate::game::entity::damage_level_of!(
                    game,
                    &sunrise_client::health::NO_ATTACK,
                    no_push,
                    None,
                    acting_equipment!(game)
                ),
                health,
                rules,
            );
            game.finish_guest_damage();
            let Some(player) = game.player.as_mut().filter(|p| p.entity_id as u32 == id) else {
                return (events, None, ServerTravel::Continue);
            };
            events.append(&mut player.events);
            player.carry_client_movement(tick_movement);
            let side = game_side!(game);
            let travel = motion_world!(game, side, player)
                .server_travel()
                .unwrap_or(ServerTravel::Continue);
            game.finish_guest_damage();
            // Player.aiStep's entity contacts run with the player's body,
            // even when the level and its mobs are frozen. Flush their damage
            // events and marked velocity through the existing guest path.
            game.touch_pufferfish();
            let Some(player) = game.player.as_mut() else {
                return (events, None, ServerTravel::Continue);
            };
            // Landing damage from this travel reaches the tracker too.
            events.append(&mut player.events);
            // A hurt since the last tick marked the velocity (Entity.markHurt):
            // ServerEntity.sendChanges sends it as it is after this travel.
            let marked = std::mem::take(&mut player.health.sync_velocity);
            let velocity = marked.then_some(player.velocity);
            game.record_guest_motion_combat(&events);
            (events, velocity, travel)
        });
        let Some((events, marked, travel)) = events else {
            return;
        };
        let travel_synced = self
            .lan
            .as_mut()
            .is_some_and(|host| host.hub.apply_server_travel(conn, travel_teleport, travel));
        if let Some((at, velocity)) = soul
            && let Some(host) = self.lan.as_ref()
        {
            crate::game::lan_particles::send_soul_particle(host, at, velocity);
        }
        self.as_guest(conn, |game| {
            game.sculk_step_on_player();
            game.tick_player_sculk_events();
            // Entity.restituteMovementAfterCollisions: GameEvent.BOUNCE.
            for event in &events {
                if matches!(event, MotionEvent::Bounce { .. }) {
                    game.post_entity_event("minecraft:bounce", sunrise_client::mobs::Who::Player);
                }
            }
        });
        self.guest_motion_events(conn, at, events);
        if let Some(v) = marked {
            if mod_api::__private::listened(mod_api::HookId::PlayerVelocity)
                && let Some((uuid, name)) = self
                    .lan
                    .as_ref()
                    .and_then(|host| host.hub.guest(conn))
                    .map(|g| (g.uuid, g.name.clone()))
            {
                self.native_player_velocity(uuid, &name, [v.x, v.y, v.z]);
            }
            if !travel_synced
                && let Some(g) = self.lan.as_ref().and_then(|host| host.hub.guest(conn))
            {
                g.send(&C::SetEntityMotion(play::SetEntityMotion {
                    id: g.entity_id,
                    velocity: [v.x, v.y, v.z],
                }));
            }
        }
    }

    fn record_guest_motion_combat(&mut self, events: &[MotionEvent]) {
        // Player.causeFallDamage: a fall of 2 blocks or more counts.
        for event in events {
            if let MotionEvent::Fall(sunrise_client::landing::FallEvent::Statistic(cm)) = event {
                self.award_custom_stat("fall_one_cm", *cm);
            }
        }
        self.record_combat(events);
        self.wear_armor(events);
    }

    fn guest_motion_events(&mut self, conn: u32, at: [f64; 3], events: Vec<MotionEvent>) {
        for event in events {
            match event {
                MotionEvent::Health(sunrise_client::health::HealthEvent::DamageBroadcast {
                    source,
                }) => self.guest_damage_broadcast(conn, source),
                // LivingEntity.playHurtSound: the server's `Entity.playSound`
                // reaches every player around but the hurt one, whose own
                // client plays it from handleDamageEvent (#4149).
                MotionEvent::HurtSound(sound) => self.guest_hurt_sound(conn, sound),
                // LivingEntity.checkFallDamage: the crumbs reach every player.
                MotionEvent::Fall(sunrise_client::landing::FallEvent::Particles {
                    state,
                    position,
                    count,
                    speed,
                }) => self.landing_dust(state, position, count, speed),
                // A block's own sound names no player: the guest hears it too.
                MotionEvent::Fall(sunrise_client::landing::FallEvent::Sound {
                    sound,
                    position: Some(pos),
                    volume,
                    pitch,
                }) if sound.block_sound().is_some() => {
                    if let Some(name) = sound.block_sound() {
                        let at = [
                            f64::from(pos.x) + 0.5,
                            f64::from(pos.y) + 0.5,
                            f64::from(pos.z) + 0.5,
                        ];
                        self.audio.play_unshared(name, Some(at), volume, pitch);
                        self.send_level_sound(LevelSound::new(name, at, volume, pitch), None);
                    }
                }
                // LivingEntity.causeFallDamage, and HoneyBlock.fallOn's
                // `entity.playSound`: Player.playSound skips the guest, whose
                // own client plays it; everyone else hears it.
                MotionEvent::Fall(sunrise_client::landing::FallEvent::Sound {
                    sound,
                    volume,
                    pitch,
                    ..
                }) => {
                    if let Some(name) = sound.player_fall_sound() {
                        self.audio.play_unshared(name, Some(at), volume, pitch);
                        self.send_level_sound(LevelSound::new(name, at, volume, pitch), Some(conn));
                    }
                }
                // Player.causeFallDamage, mayfly: the mods hear the landing.
                MotionEvent::Fall(sunrise_client::landing::FallEvent::FlyableFall {
                    distance,
                    multiplier,
                }) => {
                    if let Some((uuid, id, name)) = self
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                        .map(|g| (g.uuid, g.entity_id, g.name.clone()))
                    {
                        self.as_guest(conn, |game| {
                            game.native_player_flyable_fall(uuid, id, &name, distance, multiplier);
                        });
                    }
                }
                // HoneyBlock.fallOn: entity event 54, including the player who
                // landed (`broadcastEntityEvent` has no excluded player).
                MotionEvent::Fall(sunrise_client::landing::FallEvent::Broadcast(event))
                    if sunrise_client::landing::honey_particle_count(event as i8).is_some() =>
                {
                    if let Some(host) = self.lan.as_ref()
                        && let Some(id) = host.hub.guest(conn).map(|g| g.entity_id)
                    {
                        host.hub.entity_event(id, event as i8);
                    }
                }
                MotionEvent::Fall(sunrise_client::landing::FallEvent::BlockFallSound {
                    position,
                    state,
                }) => {
                    let entity = self
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                        .map(|g| (g.entity_id as u32, "minecraft:player"));
                    let sound = self.block_sound_type(
                        [position.x, position.y, position.z],
                        state,
                        entity,
                        mod_api::BlockSoundCause::Fall,
                    );
                    self.audio.play_unshared(
                        &sound.fall,
                        Some(at),
                        sound.volume * 0.5,
                        sound.pitch * 0.75,
                    );
                    self.send_level_sound(
                        // `Player.playSound` plays in the players' source.
                        LevelSound::new(&sound.fall, at, sound.volume * 0.5, sound.pitch * 0.75)
                            .in_source(mod_api::SoundSource::Players),
                        Some(conn),
                    );
                }
                // Farmland a mod listens to: asked and applied as the landing guest.
                MotionEvent::Fall(event) => {
                    let player = self
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                        .map(|g| g.uuid);
                    // The rule's own game event goes out as the player whose
                    // landing it was (`FarmlandBlock.turnToBaseBlock`'s change,
                    // `TurtleEggBlock.decreaseEggs`' destroy): a sculk sensor, a
                    // calibrated sensor and a warden hear it.
                    if let Some(vibration) = self.landing_block_updates(player, &event) {
                        self.as_guest(conn, |game| game.post_landing_vibration(vibration));
                    }
                }
                // PowderSnowBlock.entityInside: every player around sees the
                // snowflake (`ServerLevel.sendParticles`).
                MotionEvent::Snowflake { position, velocity } => {
                    if let Some(host) = self.lan.as_ref() {
                        crate::game::lan_particles::send_snowflake_particle(
                            host,
                            [position.x, position.y, position.z],
                            [velocity.x, velocity.y, velocity.z],
                        );
                    }
                }
                // LivingEntity.baseTick's entity event 67, to every client
                // tracking the guest and its own; this window draws it too.
                // A mod's other bubble count goes out as particles instead.
                MotionEvent::Drown(bubbles) => {
                    if !self.drown_bubbles(at, [0.0; 3], bubbles) {
                        continue;
                    }
                    if let Some(host) = self.lan.as_ref()
                        && let Some(g) = host.hub.guest(conn)
                    {
                        host.hub
                            .entity_event(g.entity_id, crate::game::combat::DROWN_EVENT);
                    }
                    self.drown_particles(at, [0.0; 3], 8);
                }
                // ServerPlayer.onInsideBlock: the enter_block trigger, for the
                // player who moved.
                MotionEvent::EnterBlock { state } => {
                    self.as_guest(conn, |game| game.trigger_enter_block(state));
                }
                // HoneyBlock.maybeDoSlideEffects: the sound reaches the
                // others (Player.playSound excludes the player, whose client
                // plays it), the entity event 53 everyone around.
                MotionEvent::HoneySlide {
                    pos,
                    sound,
                    particles,
                } => {
                    if self.world.game_time % 20 == 0 {
                        self.as_guest(conn, |game| game.trigger_honey_slide([pos.x, pos.y, pos.z]));
                    }
                    let Some((id, at)) = self
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                        .map(|g| (g.entity_id, g.position))
                    else {
                        continue;
                    };
                    if sound {
                        let name = "minecraft:block.honey_block.slide";
                        self.audio.play_unshared(name, Some(at), 1.0, 1.0);
                        self.send_level_sound(LevelSound::new(name, at, 1.0, 1.0), Some(conn));
                    }
                    if particles {
                        self.honey_slide_particles(at);
                        if let Some(host) = self.lan.as_ref() {
                            host.hub.entity_event(id, 53);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
#[path = "guest_body/tests.rs"]
mod tests;
