use super::*;

/// Vanilla `ServerGamePacketListenerImpl.handleUseItem` + `Entity.absSnapRotationTo`:
/// wrap both incoming packet angles via `Mth.wrapDegrees`, then snap each axis whose
/// wrapped value is finite (`Entity.setYRot` / `Entity.setXRot` ignore non-finite values,
/// keeping the previous axis unchanged).
fn snap_use_item_rotation(yaw: &mut f32, pitch: &mut f32, target_yaw: f32, target_pitch: f32) {
    let target_yaw = crate::game::remote::wrap_degrees(target_yaw);
    let target_pitch = crate::game::remote::wrap_degrees(target_pitch);
    if target_pitch != *pitch || target_yaw != *yaw {
        let next_yaw = target_yaw % 360.0;
        if next_yaw.is_finite() {
            *yaw = next_yaw;
        }
        if target_pitch.is_finite() {
            let next_pitch = target_pitch.clamp(-90.0, 90.0) % 360.0;
            if next_pitch.is_finite() {
                *pitch = (next_pitch % 360.0).clamp(-90.0, 90.0);
            }
        }
    }
}

impl Game<'_> {
    pub(super) fn guest_tick_death(&mut self, gi: usize) {
        let removed = self.lan.as_mut().and_then(|host| {
            host.hub.guests.update(gi, |guest| {
                if !guest.tick_death() || guest.death_time != 20 {
                    return None;
                }
                let dimensions = guest.dimensions();
                Some((
                    guest.entity_id,
                    guest.position,
                    dimensions,
                    guest.death_canceled,
                ))
            })
        });
        if let Some((entity, position, dimensions, death_canceled)) = removed {
            self.lan_entity_event(entity, 60, position);
            self.death_poof(
                position,
                f64::from(dimensions.width),
                f64::from(dimensions.height),
            );
            if !death_canceled {
                self.guest_drop_carried(gi);
            }
        }
    }

    pub(super) fn guest_drop_carried(&mut self, gi: usize) {
        let carried = self.with_own_record(gi, |host, _, _| {
            let conn = host.hub.guests[gi].conn;
            let mut stacks = host.hub.take_closed_menu_cursors(conn).collect::<Vec<_>>();
            let (to_return, to_drop) = host.hub.take_closed_processing_inputs(conn);
            stacks.extend(to_return);
            stacks.extend(to_drop);
            host.hub
                .guests
                .update(gi, |g| stacks.extend(g.take_menu_cursors()));
            Some(stacks)
        });
        if let Some(carried) = carried.filter(|stacks| !stacks.is_empty()) {
            self.drop_guest_items(gi, carried);
        }
    }

    pub(in crate::game) fn drop_guest_items(&mut self, gi: usize, stacks: Vec<ItemStack>) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let g = &host.hub.guests[gi];
        let eye = [
            g.position[0],
            g.position[1] + g.eye_height() - f64::from(0.3f32),
            g.position[2],
        ];
        let (yaw, pitch) = (g.yaw, g.pitch);
        let mut r = sunrise_client::random_source::entity(); // the guest's Entity.random
        for stack in stacks {
            let v = sunrise_client::entities::drop_motion(&mut r, yaw, pitch, false);
            self.spawn_item(stack, eye, v, 40);
        }
    }

    /// Whether a block outline occludes the straight line from `eye` to `point`.
    pub(super) fn block_between(&self, eye: [f64; 3], point: [f64; 3]) -> bool {
        let d = [point[0] - eye[0], point[1] - eye[1], point[2] - eye[2]];
        let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if dist < 1e-9 {
            return false;
        }
        let dir = [d[0] / dist, d[1] / dist, d[2] / dist];
        self.clip_ray(eye, dir, dist).is_some_and(|h| {
            let hd = [
                h.point[0] - eye[0],
                h.point[1] - eye[1],
                h.point[2] - eye[2],
            ];
            let hit_dist = (hd[0] * hd[0] + hd[1] * hd[1] + hd[2] * hd[2]).sqrt();
            hit_dist < dist - 1e-6
        })
    }

    /// Aim the swapped-in player at an entity; returns where it aims and the
    /// eye. A boat, minecart, armor stand, item frame or painting is aimed at
    /// the point the client hit (the interact packet's location) when that
    /// is on it, else at its centre.
    pub(super) fn look_at_entity(
        &mut self,
        gi: usize,
        id: i32,
        location: Option<[f64; 3]>,
    ) -> Option<([f64; 3], [f64; 3])> {
        let eye = {
            let g = &self.lan.as_ref()?.hub.guests[gi];
            [g.position[0], g.position[1] + g.eye_height(), g.position[2]]
        };
        let object = || {
            let (lo, hi, origin) = self.object_target(id)?;
            let middle = [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0);
            let Some(l) = location else {
                // A boat's mobs sit over its middle: aim low at its near
                // side, so the ray meets the boat before them (the server
                // attacks the entity it is sent).
                let carries = u32::try_from(id).is_ok_and(|v| {
                    self.boats.iter().any(|b| b.id == v) && !self.boat_mobs(v).is_empty()
                });
                if carries {
                    let near = |k: usize| eye[k].clamp(lo[k] + 0.01, hi[k] - 0.01);
                    return Some([near(0), lo[1] + 0.05, near(2)]);
                }
                return Some(middle);
            };
            let point = [0, 1, 2].map(|k| origin[k] + l[k]);
            // A little inside the box, so the ray from the eye meets it.
            let inside = [0, 1, 2].map(|k| {
                let margin = ((hi[k] - lo[k]) / 2.0).min(0.01);
                point[k].clamp(lo[k] + margin, hi[k] - margin)
            });
            let on = (0..3).all(|k| (point[k] - inside[k]).abs() < 0.1);
            Some(if on { inside } else { middle })
        };
        let mob_box = self.mobs.iter().find(|m| m.id as i32 == id).map(|m| {
            let b = m.bounds();
            let (mn, mx) = (b.min_position(), b.max_position());
            ([mn.x, mn.y, mn.z], [mx.x, mx.y, mx.z])
        });
        let centre = match mob_box {
            Some((lo, hi)) => visible_aim_point(eye, lo, hi, |p| self.block_between(eye, p)),
            None if self.object_target(id).is_some() => object()?,
            // A dragon part or an end crystal (game/mobs/dragon.rs).
            None => {
                let b = self.tracked_special_box(id)?;
                let (lo, hi) = (b.min_position(), b.max_position());
                [
                    (lo.x + hi.x) / 2.0,
                    (lo.y + hi.y) / 2.0,
                    (lo.z + hi.z) / 2.0,
                ]
            }
        };
        let d = [centre[0] - eye[0], centre[1] - eye[1], centre[2] - eye[2]];
        let horizontal = (d[0] * d[0] + d[2] * d[2]).sqrt();
        self.lan.as_mut()?.hub.guests.update(gi, |g| {
            g.yaw = (-d[0].atan2(d[2])).to_degrees() as f32;
            g.pitch = (-d[1].atan2(horizontal)).to_degrees() as f32;
        });
        Some((centre, eye))
    }

    /// `ServerGamePacketListenerImpl.handlePlayerAction` for STAB: the spear in
    /// the guest's main hand jabs once its charge allows it (with five ticks
    /// of tolerance). The guest's client is sent the dash Lunge gave them.
    pub(super) fn guest_jab(&mut self, conn: u32) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let at = g.position;
        let players = self.stab_players_for(conn);
        let sound = self
            .guest_hit_by(conn, |g| {
                if g.cannot_attack_with_item(g.attack_ticks, 5) {
                    None
                } else {
                    let outer = std::mem::replace(&mut g.stab_players, players);
                    let sound = g.jab();
                    g.stab_players = outer;
                    sound
                }
            })
            .flatten();
        self.land_player_stabs();
        // PiercingWeapon.makeSound: `playSound(causer, ...)`, so the guest's
        // own client plays it.
        if let Some(name) = sound {
            self.send_level_sound(LevelSound::new(name, at, 1.0, 1.0), Some(conn));
        }
    }

    /// `Level.getEntityOrPart(id).getBoundingBox()` for a guest's attack or
    /// interaction: the host, another player, a mob, a vehicle or object, a
    /// dragon part or crystal, a projectile. None for an id the level doesn't
    /// have (the handlers then find nothing either).
    /// `WorldBorder.isWithinBounds(target.blockPosition())` for an entity of this box:
    /// the block its feet are in (the box's bottom centre).
    pub(super) fn inside_border(&self, target: &Aabb) -> bool {
        let feet = target.bottom_center();
        self.border.contains(feet.x.floor(), feet.z.floor())
    }

    pub(in crate::game) fn guest_target_box(&self, gi: usize, id: i32) -> Option<Aabb> {
        let host = self.lan.as_ref()?;
        let conn = host.hub.guests[gi].conn;
        if let Some(g) = host
            .hub
            .guests
            .simulation_iter()
            .find(|g| g.entity_id == id && g.conn != conn && host.hub.here(g))
        {
            return Some(g.bounds());
        }
        if let Some(m) = self.mobs.iter().find(|m| m.id as i32 == id) {
            return Some(m.bounds());
        }
        if let Some((lo, hi, _)) = self.object_target(id) {
            return Some(Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            ));
        }
        if let Some(b) = self.tracked_special_box(id) {
            return Some(b);
        }
        if let Some(knot) = self.knot_at_id(id) {
            let knot = sunrise_client::mobs::leash::knot_state(knot);
            let p = knot.position;
            let half = knot.width / 2.0;
            return Some(Aabb::new(
                Vec3::new(p.x - half, p.y, p.z - half),
                Vec3::new(p.x + half, p.y + knot.height, p.z + half),
            ));
        }
        self.creatures
            .projectiles
            .iter()
            .find(|p| p.id != 0 && p.id as i32 == id)
            .map(|p| p.bounds())
    }

    /// The fence of the knot whose entity id is `id` (`lan_assign_ids`).
    pub(super) fn knot_at_id(&self, id: i32) -> Option<[i32; 3]> {
        self.knot_ids
            .iter()
            .find(|&(_, &knot)| knot as i32 == id)
            .map(|(&fence, _)| fence)
    }

    /// `ServerGamePacketListenerImpl.handleUseItemOn` for guest `conn` (at `gi`) clicking
    /// `hit` with `hand`, after the packet's sequence is acknowledged: reach, the hit's place in
    /// its block, the build height, spawn protection and the border, then
    /// `ServerPlayerGameMode.useItemOn`, and the blocks the client predicted sent back. True
    /// when the use ends the click. An optional receipt reports block and item phases before
    /// their block updates. The packet handler answers refusals. A fake player's
    /// `PlayerRef::perform_action` runs it as the packet does.
    pub(in crate::game) fn guest_use_item_on(
        &mut self,
        conn: u32,
        gi: usize,
        hit: Hit,
        hand: Hand,
        receipt: Option<i64>,
    ) -> Result<bool, ActionRefused> {
        use super::super::cauldron_prediction::{UseOnOutcome, use_result};

        let reply = |game: &Self, outcome| {
            if let (Some(receipt), Some(host)) = (receipt, &game.lan) {
                host.hub.send_to(conn, &use_result(receipt, outcome));
            }
        };
        let (mut consumed, mut block_fell_through) = (false, false);
        let Some(host) = self.lan.as_mut() else {
            return Err(ActionRefused::Stale);
        };
        // handleUseItemOn: within reach, then the hit point within
        // a block of the block's centre (logged when it isn't).
        let context = &host.hub.loop_context;
        if !host
            .hub
            .guests
            .update(gi, |g| g.reaches_block(context, hit.pos))
        {
            return Err(ActionRefused::OutOfReach);
        }
        let inside = (0..3).all(|k| (hit.point[k] - f64::from(hit.pos[k]) - 0.5).abs() < 1.0000001);
        if !inside {
            let d = sunrise_core::java_number::double_to_string;
            let at = |k: usize| d(hit.point[k]);
            let line = format!(
                "Rejecting UseItemOnPacket from {}: Location ({}, {}, {}) too far away from hit block BlockPos{{x={}, y={}, z={}}}.",
                host.hub.guests[gi].name,
                at(0),
                at(1),
                at(2),
                hit.pos[0],
                hit.pos[1],
                hit.pos[2]
            );
            self.lan_log_event(sunrise_client::lan::hub::HostEvent::Warn(line));
            return Err(ActionRefused::OutOfReach);
        }
        let spectator = self
            .lan
            .as_ref()
            .is_some_and(|host| host.hub.guests[gi].spectator());
        let face = hit.face;
        // handleUseItemOn: above or below the world nothing is used and
        // the player is told the limit (the blocks are not resent);
        // under spawn protection nothing is used or placed; the player
        // is told and the blocks below go back. mayInteract: nothing is
        // used on a block outside the world border either.
        let min_y = self.world.shape.min_y;
        let max_y = self.world.shape.max_y() - 1;
        if hit.pos[1] > max_y {
            self.build_limit_message(conn, true, max_y);
            return Err(ActionRefused::Protected);
        }
        if hit.pos[1] < min_y {
            self.build_limit_message(conn, false, min_y);
            return Err(ActionRefused::Protected);
        }
        let mut refused = None;
        if self.under_spawn_protection(gi, hit.pos) {
            self.spawn_protection_message(conn, hit.pos);
            refused = Some(ActionRefused::Protected);
        } else if self
            .border
            .contains(f64::from(hit.pos[0]), f64::from(hit.pos[2]))
        {
            if spectator && self.spectator_use_portal(conn, gi, hit.pos) {
                reply(self, UseOnOutcome::default());
                return Ok(true);
            }
            let logged = self.world.block_log.as_ref().map_or(0, Vec::len);
            let heard = self.audio.capture.as_ref().map_or(0, Vec::len);
            let used = self.as_guest_at(gi, conn, |g| {
                let attempt = g.was_block_placement_attempt(hand);
                let saved = (g.forced_hit, g.guest_use);
                g.forced_hit = Some(hit);
                g.guest_use = Some(GuestUse::on_block(hand));
                let swings = g.swings_started;
                let predicted = g.predicted_swings;
                let result = g.with_block_use_original(hand, |g| {
                    let result = g.use_item();
                    if matches!(result, Ok(Some(_))) && !g.use_failed {
                        g.finish_block_use(hit.pos, hand);
                    }
                    result
                });
                let fell_through = matches!(
                    g.guest_use,
                    Some(GuestUse::OnBlock {
                        block_fell_through: true,
                        ..
                    })
                );
                (g.forced_hit, g.guest_use) = saved;
                reset_use_attack_strength(&mut g.attack_ticks, swings, g.swings_started);
                (
                    g.swings_started != swings && g.predicted_swings == predicted,
                    result.map(|used| used.is_some()),
                    attempt,
                    fell_through,
                )
            });
            let (swung, result, attempt, fell_through) = used.map_err(|_| ActionRefused::Stale)?;
            block_fell_through = fell_through;
            self.lan_swing_back(conn, Some(swung));
            self.guest_use_sounds(conn, logged, heard);
            // A placement that did nothing at the top or bottom of the
            // world says why. Vanilla asks twice for the top (once
            // before the swing, once in the check that also has the
            // bottom), so a player there is told twice.
            consumed = match result {
                Ok(consumed) => consumed,
                Err(error) => {
                    log::warn!("block interaction for guest {conn} refused: {error:#}");
                    refused = Some(ActionRefused::InvalidTarget);
                    false
                }
            };
            if !consumed && attempt && refused.is_none() {
                if face == [0, 1, 0] && hit.pos[1] >= max_y {
                    self.build_limit_message(conn, true, max_y);
                    self.build_limit_message(conn, true, max_y);
                } else if face == [0, -1, 0] && hit.pos[1] <= min_y {
                    self.build_limit_message(conn, false, min_y);
                }
            }
        } else {
            self.build_limit_message(conn, true, max_y);
            refused = Some(ActionRefused::Protected);
        }
        // Let a waiting client run the item prediction while it still sees the
        // pre-use blocks, under the same sequence that will settle the changes.
        if refused.is_none() {
            reply(
                self,
                UseOnOutcome {
                    block_fell_through,
                    passed: !spectator && !consumed,
                },
            );
        }
        // The client predicted a result: correct it with what really is.
        let Some(host) = self.lan.as_mut() else {
            return refused.map_or(Ok(consumed), Err);
        };
        for pos in [hit.pos, hit.beside()] {
            host.hub.send_authoritative_block(conn, &self.world, pos);
        }
        refused.map_or(Ok(consumed), Err)
    }

    /// `ServerGamePacketListenerImpl.handleUseItem` for guest `conn` (at `gi`) using the item in
    /// `hand`, after the packet's sequence is acknowledged: a non-empty, enabled stack permits
    /// the packet rotation (`absSnapRotationTo`), then `ServerPlayerGameMode.useItem`.
    /// True when the use consumed (`InteractionResult` a success or a consume). A fake player's
    /// `PlayerRef::perform_action` runs it as the packet does, with no rotation.
    pub(in crate::game) fn guest_use_item(
        &mut self,
        conn: u32,
        gi: usize,
        hand: Hand,
        rotation: Option<(f32, f32)>,
    ) -> Result<bool, ActionRefused> {
        let Some(host) = self.lan.as_ref() else {
            return Err(ActionRefused::Stale);
        };
        let g = &host.hub.guests[gi];
        let held = if hand == Hand::Off {
            g.offhand.as_ref()
        } else {
            g.inventory.get(g.selected).and_then(Option::as_ref)
        };
        let Some(held) = held.filter(|s| s.count != 0 && s.item.as_ref() != "minecraft:air") else {
            return Ok(false);
        };
        if !self
            .creative_screen
            .catalog()
            .item_enabled(held, self.enabled_feature_ids())
        {
            return Ok(false);
        }
        let (spectator, spawn_egg) = (g.spectator(), held.item.ends_with("_spawn_egg"));
        if let Some((yaw, pitch)) = rotation {
            self.lan.as_mut().unwrap().hub.guests.update(gi, |g| {
                snap_use_item_rotation(&mut g.yaw, &mut g.pitch, yaw, pitch)
            });
        }
        // ServerPlayerGameMode.useItem returns PASS for spectators, after the turn.
        if spectator {
            return Ok(false);
        }
        // SpawnEggItem.use calls ServerLevel.mayInteract at its own fluid ray's position.
        if spawn_egg {
            let hit = self
                .as_guest_at(gi, conn, |g| g.with_hand(hand, |g| g.spawn_egg_fluid_hit()))
                .ok()
                .flatten();
            if hit.is_some_and(|h| self.under_spawn_protection(gi, h.pos)) {
                return Err(ActionRefused::Protected);
            }
        }
        let heard = self.audio.capture.as_ref().map_or(0, Vec::len);
        let used = self
            .as_guest_at(gi, conn, |g| {
                // A horn's sound, inline instruments included.
                let horn = g.with_hand(hand, |g| {
                    let held = g.held()?;
                    g.items().instrument(held).map(|i| i.sound)
                });
                let saved = g.guest_use;
                g.guest_use = Some(GuestUse::Item(hand));
                let swings = g.swings_started;
                let consumed = matches!(g.use_item(), Ok(Some(_)));
                g.guest_use = saved;
                reset_use_attack_strength(&mut g.attack_ticks, swings, g.swings_started);
                (g.swings_started != swings, horn, consumed)
            })
            .ok();
        let (swung, horn, consumed) = match used {
            Some((swung, horn, consumed)) => (Some(swung), horn, consumed),
            None => (None, None, false),
        };
        self.lan_swing_back(conn, swung);
        self.guest_instrument_sound(conn, heard, horn);
        Ok(consumed)
    }

    /// `ServerGamePacketListenerImpl.handleInteract`'s entity use for a guest or the persistent fake: the border and
    /// interaction range, then that entity and hand only, never an air-use fallback.
    pub(in crate::game) fn guest_interact(
        &mut self,
        conn: u32,
        gi: usize,
        id: i32,
        hand: Hand,
        location: [f64; 3],
    ) -> Result<bool, ActionRefused> {
        if let Some(target) = self.guest_target_box(gi, id) {
            if !self.inside_border(&target) {
                return Err(ActionRefused::Protected);
            }
            if self.as_guest_at(gi, conn, |g| {
                g.within_entity_interaction_range(&target, 3.0)
            }) != Ok(true)
            {
                return Err(ActionRefused::OutOfReach);
            }
        } else {
            return Err(ActionRefused::InvalidTarget);
        }
        // LeashFenceKnotEntity.interact: a knot has no entity here, only its fence.
        if let Some(knot) = self.knot_at_id(id) {
            let used = self
                .as_guest_at(gi, conn, |g| {
                    let swings = g.swings_started;
                    let consumed = g.with_hand(hand, |g| g.use_knot(knot, true));
                    reset_use_attack_strength(&mut g.attack_ticks, swings, g.swings_started);
                    (g.swings_started != swings, consumed)
                })
                .ok();
            self.lan_swing_back(conn, used.map(|u| u.0));
            return Ok(used.is_some_and(|u| u.1));
        }
        let host = self.lan.as_ref().ok_or(ActionRefused::Stale)?;
        let before = host.hub.guests[gi].vehicle;
        // Packet interactions aim the server's ray at the clicked entity. A fake already
        // supplies its own viewpoint; its explicit target must not change that rotation.
        if !host.hub.guests[gi].is_fake() && self.look_at_entity(gi, id, Some(location)).is_none() {
            return Ok(false);
        }
        let target = InteractTarget { id, location };
        let used = self
            .as_guest_at(gi, conn, |g| {
                let saved = g.guest_use;
                let failed = std::mem::replace(&mut g.use_failed, false);
                g.guest_use = Some(GuestUse::Entity(hand, target));
                let swings = g.swings_started;
                let consumed = matches!(g.use_item(), Ok(Some(_))) && !g.use_failed;
                g.guest_use = saved;
                g.use_failed = failed;
                reset_use_attack_strength(&mut g.attack_ticks, swings, g.swings_started);
                (g.swings_started != swings, consumed)
            })
            .ok();
        self.lan_swing_back(conn, used.map(|u| u.0));
        self.guest_boarded(conn, before);
        Ok(used.is_some_and(|u| u.1))
    }

    /// `ServerGamePacketListenerImpl.handleAttack` for guest `conn` (at `gi`) hitting
    /// entity `id`: the border and the attack range, then [`Self::guest_player_attack`]. True
    /// when the blow landed. A fake player's `PlayerRef::perform_action` runs it as the packet
    /// does.
    pub(in crate::game) fn guest_attack(
        &mut self,
        conn: u32,
        gi: usize,
        id: i32,
    ) -> Result<bool, ActionRefused> {
        let Some(host) = self.lan.as_ref() else {
            return Err(ActionRefused::Stale);
        };
        if host.hub.guests[gi].spectator() {
            return Ok(false);
        }
        // handleAttack: a target inside the world border, then
        // `isWithinAttackRange(mainHandItem, box, 3.0)` before anything else, players,
        // mobs, objects and fireballs alike.
        if let Some(target) = self.guest_target_box(gi, id) {
            if !self.inside_border(&target) {
                return Err(ActionRefused::Protected);
            }
            if self.as_guest_at(gi, conn, |g| g.within_attack_range(&target)) != Ok(true) {
                return Err(ActionRefused::OutOfReach);
            }
        }
        if self.guest_attack_stops(gi, id) {
            return Ok(false);
        }
        Ok(self.guest_player_attack(conn, gi, id, true)? == Struck::Landed)
    }

    /// `Player.attack` for guest `conn` (at `gi`) on entity `id`, from its start: the native
    /// mods' `attack_entity`, `cannotAttack`, then the attack on a player, a mob or another
    /// entity, or the deflection of a fireball, a wind charge or a shulker bullet. `packet` is
    /// the packet's attack, which keeps its swing and its own range check on a projectile;
    /// a mod's `PlayerRef::attack` runs without them, in an unswung [`Acting`] it holds.
    ///
    /// [`Acting`]: crate::game::native_server::Acting
    pub(in crate::game) fn guest_player_attack(
        &mut self,
        conn: u32,
        gi: usize,
        id: i32,
        packet: bool,
    ) -> Result<Struck, ActionRefused> {
        // Native mods, where Player.attack starts: the global `attack_entity`,
        // then the held item's `on_left_click_entity` Part. A canceled blow
        // does nothing (game/native_hooks.rs).
        let Some(host) = self.lan.as_ref() else {
            return Err(ActionRefused::Stale);
        };
        let aimed = if let Some(g) = host
            .hub
            .guests
            .iter()
            .find(|g| g.entity_id == id && g.conn != conn && host.hub.here(g))
        {
            Some(("minecraft:player".to_owned(), g.position))
        } else {
            // The mod hooks hear no natural fence knot (#13036).
            self.level_entity(id)
                .filter(|t| !matches!(t, crate::game::commands::Target::Knot(_)))
                .map(|t| {
                    (
                        self.target_type(t),
                        self.target_position(t).unwrap_or_default(),
                    )
                })
        };
        if let Some((kind, at)) = aimed
            && self.as_guest_at(gi, conn, |g| {
                let mut canceled = false;
                g.native_attack_entity(id, &kind, at, &mut canceled);
                canceled || g.native_left_click_entity_event(id, &kind)
            }) == Ok(true)
        {
            return Ok(Struck::Stopped);
        }
        if self.cannot_attack(id) {
            return Ok(Struck::Stopped);
        }
        if let Some(victim) = self.pvp_victim(conn, id) {
            let Ok(Some(strike)) = self.as_guest_at(gi, conn, |g| g.pvp_strike(id as u32)) else {
                return Ok(Struck::Stopped);
            };
            let Some(stops_sprint) = self.pvp_land(victim, &strike) else {
                return Ok(Struck::Spent);
            };
            let on_ground = self.pvp_victim_on_ground(victim);
            let sent = self
                .as_guest_at(gi, conn, |g| {
                    g.remember_player_target(Some(id as u32));
                    g.pvp_hit_after(id as u32, on_ground, stops_sprint, &strike)
                })
                .ok()
                .flatten();
            self.send_guest_motion(conn, sent);
            return Ok(Struck::Landed);
        }
        // A ghast's fireball or a wind charge: turned along the guest's own
        // view (ServerGamePacketListenerImpl.handleInteract, then
        // Player.attack's AIM_DEFLECT).
        if let Some(i) = self
            .creatures
            .projectiles
            .iter()
            .position(|p| p.id != 0 && p.id as i32 == id && crate::game::deflect::attackable(p))
        {
            let deflected = self.as_guest_at(gi, conn, |g| {
                if packet && !g.within_interaction_range(i) {
                    return false;
                }
                if matches!(
                    g.creatures.projectiles[i].kind,
                    sunrise_client::projectiles::ProjectileKind::ShulkerBullet { .. }
                ) {
                    g.hit_shulker_bullet(i);
                } else {
                    g.deflect(i);
                }
                true
            }) == Ok(true);
            return Ok(if deflected {
                Struck::Landed
            } else {
                Struck::Stopped
            });
        }
        if let Some(index) = self
            .mobs
            .iter()
            .position(|m| m.id as i32 == id && m.alive())
            .filter(|_| self.dragon.as_ref().is_none_or(|d| d.id as i32 != id))
        {
            let Some(host) = self.lan.as_ref() else {
                return Err(ActionRefused::Stale);
            };
            if host.hub.guests[gi].vehicle == Some(self.mobs[index].id) {
                return Ok(Struck::Stopped);
            }
            // A mace's smash sets the attacker's velocity and sends it at once, before the
            // Wind Burst's push (MaceItem.hurtEnemy, then the post-attack effects).
            let sent = self
                .as_guest_at(gi, conn, |g| {
                    let marked = g
                        .player
                        .as_mut()
                        .is_some_and(|p| std::mem::take(&mut p.health.sync_velocity));
                    g.attack_mob(index);
                    g.swing_arm();
                    let p = g.player.as_mut()?;
                    let sent = std::mem::replace(&mut p.health.sync_velocity, marked);
                    sent.then_some(p.velocity)
                })
                .ok()
                .flatten();
            self.send_guest_motion(conn, sent);
            // The mob now answers to this guest (game/network/lan_mobs.rs).
            if let Ok(id) = u32::try_from(id)
                && let Some(host) = self.lan.as_mut()
                && host.hub.guests[gi].participates_in_player_simulation()
            {
                host.mob_players.insert(id, conn);
            }
            return Ok(Struck::Landed);
        }
        let Some(host) = self.lan.as_ref() else {
            return Err(ActionRefused::Stale);
        };
        let (old_yaw, old_pitch) = (host.hub.guests[gi].yaw, host.hub.guests[gi].pitch);
        let boat = self.boats.iter().position(|boat| boat.id as i32 == id);
        let tnt = self
            .tnts
            .iter()
            .find(|tnt| tnt.id as i32 == id)
            .map(|tnt| tnt.id);
        let general = match u32::try_from(id).ok().and_then(|id| self.general_row(id)) {
            Some(row) => {
                let catalog = crate::game::general::general_type_catalog();
                if !catalog.pickable(row.type_slot) {
                    return Ok(Struck::Stopped);
                }
                Some((row.id, catalog.key(row.type_slot)))
            }
            None => None,
        };
        // A packet names its target already. Boats, primed TNT and general rows use that target
        // with the attacker's actual aim instead of casting a new ray at its center.
        if boat.is_none()
            && tnt.is_none()
            && general.is_none()
            && self.look_at_entity(gi, id, None).is_none()
        {
            return Err(ActionRefused::InvalidTarget);
        }
        // A mace's smash sets the attacker's velocity and sends it at once, before the
        // Wind Burst's push (MaceItem.hurtEnemy, then the post-attack effects).
        let sent = self
            .as_guest_at(gi, conn, |g| {
                let marked = g
                    .player
                    .as_mut()
                    .is_some_and(|p| std::mem::take(&mut p.health.sync_velocity));
                if let Some(index) = boat {
                    g.strike_boat(index);
                    g.swing_arm();
                } else if let Some(tnt_id) = tnt {
                    g.strike_general_row(tnt_id, "minecraft:tnt");
                } else if let Some((id, kind)) = general {
                    g.strike_general_row(id, kind);
                } else {
                    g.attack();
                }
                let p = g.player.as_mut()?;
                let sent = std::mem::replace(&mut p.health.sync_velocity, marked);
                sent.then_some(p.velocity)
            })
            .ok()
            .flatten();
        self.send_guest_motion(conn, sent);
        if tnt.is_some() || general.is_some() {
            // Primed TNT and plain rows refuse damage, even though the attack spent its charge.
            return Ok(Struck::Spent);
        }
        // The mob now answers to this guest (game/network/lan_mobs.rs).
        let Some(host) = self.lan.as_mut() else {
            return Err(ActionRefused::Stale);
        };
        if host.hub.guests[gi].participates_in_player_simulation()
            && let Ok(id) = u32::try_from(id)
        {
            host.mob_players.insert(id, conn);
        }
        {
            let value = old_yaw;
            host.hub.guests.update(gi, |guest| guest.yaw = value);
        }
        {
            let value = old_pitch;
            host.hub.guests.update(gi, |guest| guest.pitch = value);
        }
        Ok(Struck::Landed)
    }
}

/// How far a guest's `Player.attack` went ([`Game::guest_player_attack`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum Struck {
    /// It ended before `onAttack`: a mod canceled it, or `cannotAttack`.
    Stopped,
    /// It spent the attack, but nothing took the blow.
    Spent,
    /// The blow landed.
    Landed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn use_item_rotation_matches_vanilla_wrap_and_abs_snap() {
        let (mut yaw, mut pitch) = (15.0_f32, 25.0_f32);
        snap_use_item_rotation(&mut yaw, &mut pitch, 405.0, 120.0);
        assert_eq!((yaw, pitch), (45.0, 90.0));

        // Non-finite yaw preserves previous yaw while still snapping finite pitch.
        snap_use_item_rotation(&mut yaw, &mut pitch, f32::NAN, -120.0);
        assert_eq!((yaw, pitch), (45.0, -90.0));

        // Non-finite pitch preserves previous pitch while still wrapping finite yaw.
        snap_use_item_rotation(&mut yaw, &mut pitch, -540.0, f32::INFINITY);
        assert_eq!((yaw, pitch), (-180.0, -90.0));

        // Both non-finite preserve both previous axes.
        snap_use_item_rotation(&mut yaw, &mut pitch, f32::NEG_INFINITY, f32::NAN);
        assert_eq!((yaw, pitch), (-180.0, -90.0));
    }

    #[test]
    #[ignore = "daily"]
    fn use_item_non_finite_angles_preserve_previous_axis_and_wrap_finite_ones() {
        use crate::game::lan::world_spawn_sync_tests::host_for;
        use crate::game::native_entity_refs::tests::run_fresh_cases;
        use mod_api::PlayPacketRequest;
        use sunrise_client::mod_packet_builders::encode_play_packet;
        use sunrise_protocol::ids::play::serverbound as sb;

        if run_fresh_cases(
            "game::lan_actions::interaction::tests::use_item_non_finite_angles_preserve_previous_axis_and_wrap_finite_ones",
            "SUNRISE_USE_ITEM_ROTATION_CHILD",
            &["rotation"],
        ) {
            return;
        }
        let mut resources = None;
        let mut game = crate::game::game_tests::tests::server_game(&mut resources);
        let _rx = host_for(&mut game);
        game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.dimension = "minecraft:overworld".into();
            g.yaw = 15.0;
            g.pitch = 25.0;
            g.inventory[0] = Some(ItemStack::new("minecraft:snowball", 4));
        });

        let cases = [
            (1, f32::NAN, 120.0_f32, 15.0_f32, 90.0_f32, 3_u32),
            (2, 405.0_f32, f32::INFINITY, 45.0_f32, 90.0_f32, 2_u32),
            (3, f32::NEG_INFINITY, f32::NAN, 45.0_f32, 90.0_f32, 1_u32),
        ];
        for (sequence, yaw, pitch, want_yaw, want_pitch, want_count) in cases {
            let built = encode_play_packet(&PlayPacketRequest::UseItem {
                hand: 0,
                sequence,
                yaw,
                pitch,
            })
            .expect("UseItem preserves non-finite float bits on the wire");
            let decoded = S::decode(sb::USE_ITEM, &built.body).unwrap();
            let before_proj = game.creatures.projectiles.len();
            game.lan_guest_packet(1, decoded);

            let guest = &game.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(guest.yaw, want_yaw, "sequence {sequence}");
            assert_eq!(guest.pitch, want_pitch, "sequence {sequence}");
            assert_eq!(
                guest.inventory[0].as_ref().map(|s| s.count),
                Some(want_count),
                "sequence {sequence} still uses the held snowball"
            );
            assert_eq!(
                game.creatures.projectiles.len(),
                before_proj + 1,
                "sequence {sequence} spawns a snowball"
            );
            let proj = game.creatures.projectiles.last().unwrap();
            assert!(
                proj.velocity.x.is_finite()
                    && proj.velocity.y.is_finite()
                    && proj.velocity.z.is_finite(),
                "sequence {sequence} projectile velocity stays finite: {:?}",
                proj.velocity
            );
        }
    }
}
