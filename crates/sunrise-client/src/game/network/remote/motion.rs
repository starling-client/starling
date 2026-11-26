use super::movement::stepped_move;
use super::*;
use crate::game::entity::Entity;

impl Game<'_> {
    /// A move packet for entity `id`: to the end of `path` (None: a
    /// rotation alone), turned to `rotation` if it has one. `packet` says
    /// whether a living entity more than 64 blocks away is put there at once
    /// (its `snaps`).
    pub(super) fn move_entity(
        &mut self,
        id: i32,
        path: Option<Path>,
        rotation: Option<(f32, f32)>,
        on_ground: bool,
        packet: MovePacket,
    ) {
        self.move_entity_with_old(id, path, rotation, on_ground, packet, None);
    }

    /// `move_entity` with an explicit old pose (`setValuesFromPositionPacket`'s snapped old pose).
    pub(super) fn move_entity_with_old(
        &mut self,
        id: i32,
        path: Option<Path>,
        rotation: Option<(f32, f32)>,
        on_ground: bool,
        packet: MovePacket,
        snap_old: Option<PosRot>,
    ) {
        // The dragon is drawn from its own state (game/mobs/dragon.rs), which
        // tick_remote_dragon eases to the server's moves through `lerps`. Its
        // copy in the mob list draws nothing: given the move, it kept the
        // dragon where it spawned (#2083).
        let dragon = self.dragon.as_ref().is_some_and(|d| d.id as i32 == id);
        let driven = self.driven_mount().is_some_and(|m| m.id as i32 == id)
            || u32::try_from(id).is_ok_and(|id| self.controls_boat(id));
        let mob = self
            .mob_index
            .get_mut(&mut self.mobs, id as u32)
            .filter(|_| !dragon);
        let tracked = self
            .remote
            .as_ref()
            .and_then(|r| r.entities.get(&id))
            .map(|t| t.position);
        let position = path
            .as_ref()
            .map(Path::end)
            .or(tracked)
            .or_else(|| {
                mob.as_ref()
                    .map(|m| [m.position.x, m.position.y, m.position.z])
            })
            .unwrap_or([0.0; 3]);
        if let Some(r) = self.remote.as_mut() {
            if let Some(t) = r.entities.get_mut(&id) {
                let d = [0, 1, 2].map(|i| position[i] - t.position[i]);
                let snap = matches!(packet, MovePacket::PositionSyncSnap)
                    || (matches!(packet, MovePacket::Teleport | MovePacket::PositionSync)
                        && d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > 4096.0);
                if path.is_some() {
                    t.position = position;
                }
                if let Some((yaw, pitch)) = rotation {
                    if yaw.is_finite() {
                        t.yaw = if snap { yaw } else { yaw % 360.0 };
                    }
                    if pitch.is_finite() {
                        t.pitch = (pitch % 360.0).clamp(-90.0, 90.0);
                    }
                }
                if snap {
                    if let Some(old) = snap_old {
                        t.previous = old.position;
                        t.previous_yaw = if old.yaw.is_finite() { old.yaw } else { t.yaw };
                        t.previous_pitch = if old.pitch.is_finite() {
                            (old.pitch % 360.0).clamp(-90.0, 90.0)
                        } else {
                            t.pitch
                        };
                    } else {
                        t.previous = t.position;
                        t.previous_yaw = t.yaw;
                        t.previous_pitch = t.pitch;
                    }
                }
            }
            if driven {
                // handleMoveEntity, handleEntityPositionSync: the mount or boat
                // this client moves (isLocalInstanceAuthoritative) keeps its own
                // position. An interpolation it had going runs on
                // (`interpolate_driven_mount`, the boat's own), with no new steps.
                return;
            }
            if let Some(m) = mob {
                // Shulker.createInterpolationHandler is NO_OP, so a move packet
                // is setPos at once. The 6-tick slide starts from the block
                // the shulker occupied before this packet.
                if m.species() == sunrise_client::mobs::Species::Shulker {
                    r.stepped.remove(&id);
                    if path.is_some() {
                        sunrise_client::mobs::shulker::client_set_pos(
                            m,
                            position[0],
                            position[1],
                            position[2],
                        );
                    }
                    if let Some((yaw, pitch)) = rotation {
                        m.yaw += wrap_degrees(yaw - m.yaw);
                        m.head_pitch = (pitch % 360.0).clamp(-90.0, 90.0);
                    }
                    let _ = m.update_retained_ground(
                        &self.world,
                        self.collision,
                        &world_items!(self).entity_type_tags,
                        on_ground,
                        None,
                    );
                    return;
                }
                // A living entity: LivingEntity.createInterpolationHandler's
                // SteppedInterpolationHandler, over its type's update interval.
                let entity = PosRot {
                    position: [m.position.x, m.position.y, m.position.z],
                    yaw: m.yaw,
                    pitch: m.head_pitch,
                };
                let handler = r.stepped.entry(id).or_insert_with(|| {
                    let interval = sunrise_client::lan::tracking::settings(&m.kind.id).1;
                    SteppedInterpolation::new(interval as i32)
                });
                if let Some(at) = stepped_move(handler, packet, entity, position, path, rotation) {
                    place_interpolated(m, at);
                    if let Some(old) = snap_old {
                        m.previous = Vec3::new(old.position[0], old.position[1], old.position[2]);
                        m.previous_yaw = old.yaw;
                        m.previous_head_pitch = (old.pitch % 360.0).clamp(-90.0, 90.0);
                    } else {
                        m.previous = m.position;
                        m.previous_yaw = m.yaw;
                        m.previous_head_pitch = m.head_pitch;
                    }
                }
                // ClientPacketListener sets onGround after moving or queuing the
                // interpolation; the support probe uses the current bounds.
                let _ = m.update_retained_ground(
                    &self.world,
                    self.collision,
                    &world_items!(self).entity_type_tags,
                    on_ground,
                    None,
                );
                return;
            }
            // An armor stand is a living entity too: the same handler, over
            // its type's update interval, stepped in `tick_remote_entities`.
            if let Some(a) = self.armor_stands.iter_mut().find(|a| a.id == id as u32) {
                let entity = PosRot {
                    position: [a.position.x, a.position.y, a.position.z],
                    yaw: a.yaw,
                    pitch: a.pitch,
                };
                let handler = r.stepped.entry(id).or_insert_with(|| {
                    let interval =
                        sunrise_client::lan::tracking::settings("minecraft:armor_stand").1;
                    SteppedInterpolation::new(interval as i32)
                });
                if let Some(at) = stepped_move(handler, packet, entity, position, path, rotation) {
                    a.snap(
                        Vec3::new(at.position[0], at.position[1], at.position[2]),
                        at.yaw,
                        at.pitch,
                    );
                    if let Some(old) = snap_old {
                        a.previous = Vec3::new(old.position[0], old.position[1], old.position[2]);
                        a.previous_yaw = old.yaw;
                        a.previous_pitch = (old.pitch % 360.0).clamp(-90.0, 90.0);
                    }
                }
                let _ = a.update_retained_ground(
                    &self.world,
                    self.collision,
                    &world_items!(self).entity_type_tags,
                    on_ground,
                    None,
                );
                return;
            }
            // Interaction extends Entity and keeps its NO_OP interpolation
            // handler: the server's position replaces its own at once.
            if let Some(i) = self.interactions.get_mut(&id) {
                let target_pos = Vec3::new(position[0], position[1], position[2]);
                let d = target_pos.subtract(i.position);
                let snap = matches!(packet, MovePacket::PositionSyncSnap)
                    || (matches!(packet, MovePacket::Teleport | MovePacket::PositionSync)
                        && d.x * d.x + d.y * d.y + d.z * d.z > 4096.0);
                if snap {
                    let (yaw, pitch) = rotation.unwrap_or((i.yaw, i.pitch));
                    i.snap_to(target_pos, yaw, pitch);
                    if let Some(old) = snap_old {
                        i.previous = Vec3::new(old.position[0], old.position[1], old.position[2]);
                        i.previous_yaw = if old.yaw.is_finite() { old.yaw } else { i.yaw };
                        i.previous_pitch = if old.pitch.is_finite() {
                            (old.pitch % 360.0).clamp(-90.0, 90.0)
                        } else {
                            i.pitch
                        };
                    }
                } else {
                    if path.is_some() {
                        i.position = target_pos;
                    }
                    if let Some((yaw, pitch)) = rotation {
                        i.set_rot(yaw, pitch);
                    }
                }
                return;
            }
            // Projectiles keep Entity's NO_OP InterpolationHandler too: the
            // server's position replaces theirs at once and their own tick
            // flies them on (`tick_remote_projectiles`). Eased towards it
            // instead, they only moved when an update came, every 10 to 20
            // ticks.
            if let Some(i) = self
                .creatures
                .projectiles
                .iter()
                .position(|p| p.id == id as u32)
            {
                self.creatures.projectiles.update(i, |p| {
                    if path.is_some() {
                        p.position = Vec3::new(position[0], position[1], position[2]);
                    }
                    if let Some((yaw, pitch)) = rotation {
                        p.yaw = yaw;
                        p.pitch = pitch;
                    }
                });
                return;
            }
            if path.is_some()
                && (self.potions.move_remote_thrown(
                    id as u32,
                    Vec3::new(position[0], position[1], position[2]),
                ) || self.end.move_remote_eye(id as u32, position))
            {
                return;
            }
            // A fishing hook keeps a LinearInterpolationHandler: eased towards the
            // update over three ticks, on top of its own movement (`tick_remote_hooks`).
            if path.is_some() {
                if let Some(h) = self.hook.as_mut().filter(|h| h.id() == id as u32) {
                    h.move_to(position, on_ground);
                    return;
                }
                if let Some(h) = self.remote_hooks.iter_mut().find(|h| h.id() == id as u32) {
                    h.move_to(position, on_ground);
                    return;
                }
            }
            // A thrown trident is an arrow: placed at once, flown on by its tick.
            if let Some(t) = self.tridents.iter_mut().find(|t| t.id == id as u32) {
                if path.is_some() {
                    t.position = Vec3::new(position[0], position[1], position[2]);
                }
                if let Some((yaw, pitch)) = rotation {
                    t.yaw = yaw;
                    t.pitch = pitch;
                }
                return;
            }
            // So does a firework rocket: its own tick flies it on.
            if path.is_some()
                && let Some(f) = self.rockets.iter_mut().find(|f| f.id == id as u32)
            {
                f.position = Vec3::new(position[0], position[1], position[2]);
                return;
            }
            if let Some(p) = r.pending_items.get_mut(&id) {
                p.position = position;
            }
            // ItemEntity, FallingBlockEntity and PrimedTnt keep Entity's
            // NO_OP InterpolationHandler: moveOrInterpolateTo puts them there
            // at once, and their own tick moves them on between the server's
            // updates (`tick_remote_entities`, `tick_remote_projectiles`).
            let at = Vec3::new(position[0], position[1], position[2]);
            if let Some(d) = self.dropped.iter_mut().find(|d| d.id == id as u32) {
                d.position = at;
                d.ground.on_ground = on_ground;
                return;
            }
            if let Some(i) = self.falling.iter().rposition(|f| f.id == id as u32) {
                self.falling.update(i, |f| f.position = at);
                return;
            }
            if let Some(t) = self.tnts.iter_mut().rev().find(|t| t.id == id as u32) {
                t.position = at;
                t.ground.on_ground = on_ground;
                return;
            }
            // ExperienceOrb.createInterpolationHandler is a
            // LinearInterpolationHandler: the correction is spread over 3
            // ticks (`tick_remote_entities`), while `ExperienceOrb.tick` keeps
            // moving it.
            if let Some(o) = self.orbs.iter_mut().find(|o| o.id == id as u32) {
                o.ground.on_ground = on_ground;
                r.lerps.insert(
                    id,
                    Lerp {
                        target: position,
                        rotation: None,
                        steps: 3,
                    },
                );
                return;
            }
            if r.cart_lerps.contains_key(&id) {
                // A minecart on NewMinecartBehavior keeps Entity's NO_OP
                // InterpolationHandler: a teleport puts it there at once.
                if let Some(i) = self.carts.iter().position(|c| c.id == id as u32) {
                    self.carts.update(i, |c| {
                        if path.is_some() {
                            c.position = at;
                        }
                        if let Some((yaw, pitch)) = rotation {
                            c.yaw = yaw;
                            c.pitch = pitch;
                        }
                    });
                }
            } else if !self.text_displays.contains_key(&id) {
                // AbstractInterpolationHandler.interpolateTo: without a
                // rotation, the one it was heading for.
                let rotation = rotation.or_else(|| r.lerps.get(&id).and_then(|l| l.rotation));
                r.lerps.insert(
                    id,
                    Lerp {
                        target: position,
                        rotation,
                        steps: 3,
                    },
                );
            }
        }
        self.turn_remote_object(id, rotation.map(|(yaw, _)| yaw), on_ground);
        match packet {
            MovePacket::PositionSync | MovePacket::PositionSyncSnap => {
                self.position_sync_remote_text_display(id, position, rotation);
            }
            MovePacket::Move | MovePacket::Teleport => {
                self.move_remote_text_display_opt(id, path.is_some().then_some(position), rotation);
            }
        }
        self.turn_remote_projectile(id, rotation);
    }
    /// `handleMoveVehicle`'s `getInterpolation().cancel()` for the mount this
    /// client drives.
    pub(in crate::game) fn cancel_driven_mount_interpolation(&mut self, id: u32) {
        if let Some(h) = self
            .remote
            .as_mut()
            .and_then(|r| r.stepped.get_mut(&(id as i32)))
        {
            h.cancel();
        }
    }

    /// `Entity.lerpMotion` (and `recreateFromPacket`'s motion) for the
    /// entities whose own tick moves them on a client: dropped items, experience
    /// orbs, falling blocks and primed TNT.
    pub(super) fn remote_motion(&mut self, id: i32, velocity: [f64; 3]) {
        self.remote_dragon_motion(id, velocity);
        let v = Vec3::new(velocity[0], velocity[1], velocity[2]);
        // `falling` and `tnts` are searched from the back: AddEntity calls this right after
        // appending the block or TNT, so a spawn doesn't walk the whole list.
        if let Some(m) = self.mob_index.get_mut(&mut self.mobs, id as u32) {
            // Squid.aiStep derives its body pitch and roll from network motion.
            m.velocity = v;
        } else if let Some(d) = self.dropped.iter_mut().find(|d| d.id == id as u32) {
            d.velocity = v;
        } else if let Some(i) = self.falling.iter().rposition(|f| f.id == id as u32) {
            self.falling.update(i, |f| f.velocity = v);
        } else if let Some(t) = self.tnts.iter_mut().rev().find(|t| t.id == id as u32) {
            t.velocity = v;
        } else if let Some(o) = self.orbs.iter_mut().find(|o| o.id == id as u32) {
            o.velocity = v;
        } else if let Some(i) = self.carts.iter().position(|c| c.id == id as u32) {
            self.carts.update(i, |c| c.velocity = v);
        } else if let Some(i) = self.boats.iter().position(|b| b.id == id as u32) {
            // Entity.lerpMotion: the boat the player drives floats on from the server's motion
            // (`tick_remote_boat_bubbles` drops it again on a boat nobody here drives).
            self.boats.update(i, |b| b.velocity = v);
        } else if let Some(i) = self
            .creatures
            .projectiles
            .iter()
            .position(|p| p.id == id as u32)
        {
            self.creatures.projectiles.update(i, |p| {
                p.velocity = v;
                // AbstractArrow.lerpMotion: the stuck timer restarts, and an
                // arrow the server set moving leaves the block it was stuck in.
                if let Some(st) = p.arrow.as_mut() {
                    st.life = 0;
                    if st.in_ground && v.length_squared() > 0.0 {
                        st.in_ground = false;
                        p.stuck = false;
                    }
                }
            });
        } else if let Some(t) = self.tridents.iter_mut().find(|t| t.id == id as u32) {
            t.velocity = v;
            // AbstractArrow.lerpMotion (ThrownTrident keeps it): `life`
            // restarts, and a trident set moving leaves its block.
            t.in_ground_time = 0;
            if t.in_ground && v.length_squared() > 0.0 {
                t.in_ground = false;
            }
        } else if let Some(r) = self.rockets.iter_mut().find(|r| r.id == id as u32) {
            r.velocity = v;
        } else if self.remote_hook_motion(id as u32, v) {
        } else if let Some(p) = self
            .remote
            .as_mut()
            .and_then(|r| r.pending_items.get_mut(&id))
        {
            p.velocity = v;
        } else if !self.potions.remote_thrown_motion(id as u32, v) {
            self.end.remote_eye_motion(id as u32, velocity);
        }
    }
}

impl Game<'_> {
    /// `Entity.commonTick` for the mount this client drives, `mobs[i]`: an
    /// interpolation it had going when the client took it over runs on, its
    /// target carried along by the mount's own moves (where the mount fits).
    /// The mount's pose before the step: its tick saves the old pose
    /// (setOldPosAndRot) before the step in vanilla, so `drive_mount` puts this one
    /// back as the old pose once the tick has run.
    pub(in crate::game) fn interpolate_driven_mount(
        &mut self,
        i: usize,
    ) -> Option<(Vec3, f32, f32)> {
        let id = self.mobs[i].id as i32;
        let handler = self.remote.as_mut().and_then(|r| r.stepped.get_mut(&id))?;
        let m = &mut self.mobs[i];
        let mut scene = sunrise_client::collisions::SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        handler.follow(mob_pos_rot(m), |to| {
            let bounds = m.bounds_at(Vec3::new(to[0], to[1], to[2]));
            scene.unobstructed(bounds).unwrap_or(false)
        });
        let before = (m.position, m.yaw, m.head_pitch);
        let at = handler.interpolate()?;
        place_interpolated(m, at);
        // A step that crosses a section is filed where it happens.
        let to = m.position;
        if to != before.0 {
            let from = before.0;
            self.entity_section_step(id as u32, [from.x, from.y, from.z], [to.x, to.y, to.z]);
        }
        Some(before)
    }

    /// `Entity.getClientPositionAndRotation` of the mount this client drives:
    /// while an interpolation runs, where it is going.
    pub(in crate::game) fn driven_mount_heading(&self, m: &Mob) -> PosRot {
        let at = mob_pos_rot(m);
        self.remote
            .as_ref()
            .and_then(|r| r.stepped.get(&(m.id as i32)))
            .map_or(at, |h| h.heading(at))
    }

    /// The mount this client drives teleported to `at` (`moveOrInterpolateTo`):
    /// it eases there over its type's update interval.
    pub(in crate::game) fn interpolate_driven_mount_to(&mut self, i: usize, at: PosRot) {
        let m = &mut self.mobs[i];
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let handler = r.stepped.entry(m.id as i32).or_insert_with(|| {
            let interval = sunrise_client::lan::tracking::settings(&m.kind.id).1;
            SteppedInterpolation::new(interval as i32)
        });
        let entity = mob_pos_rot(m);
        if let Start::Snap(to) = handler.interpolate_to(
            entity,
            Some(Path::Linear(at.position)),
            Some((at.yaw, at.pitch)),
        ) {
            place_interpolated(m, to);
        }
    }

    /// `Entity.moveOrInterpolateTo` for a client mod (`EntityRef::interpolate_to`):
    /// routes to the family's own interpolation handler, falling back to a direct
    /// set for families without one. Refuses bodies the client simulates
    /// (`Entity.canSimulateMovement`), non-finite inputs, stale handles and
    /// server-side calls.
    pub(in crate::game) fn entity_interpolate_to(
        &mut self,
        entity: mod_api::EntityRef,
        position: [f64; 3],
        rotation: Option<[f32; 2]>,
    ) -> bool {
        if !position.iter().all(|v| v.is_finite())
            || rotation.is_some_and(|[yaw, pitch]| !yaw.is_finite() || !pitch.is_finite())
        {
            return false;
        }
        if self.is_logical_server() {
            return false;
        }
        let Ok((remote, id)) = self.native_client_entity_context(entity) else {
            return false;
        };
        let entity_id = entity.id();
        if id == remote.entity_id || self.resident_dragon_part(entity_id).is_some() {
            return false;
        }
        if remote.tracked_entity(id).is_none() {
            return false;
        }
        // Vanilla `Entity.canSimulateMovement`: refuse vehicles driven by the
        // local player (`AUTHORITATIVE_SIDE`) and every `SERVER_AND_CLIENT` body.
        let simulated = self.driven_mount().is_some_and(|m| m.id == entity_id)
            || self.controls_boat(entity_id)
            || self.dropped.iter().any(|d| d.id == entity_id)
            || remote.pending_items.contains_key(&id)
            || self.tnts.iter().any(|t| t.id == entity_id)
            || self.falling.iter().any(|f| f.id == entity_id)
            || self.orbs.iter().any(|o| o.id == entity_id)
            || self.creatures.projectiles.iter().any(|p| p.id == entity_id)
            || self.tridents.iter().any(|t| t.id == entity_id)
            || self.rockets.iter().any(|f| f.id == entity_id)
            || self.hook.as_ref().is_some_and(|h| h.id() == entity_id)
            || self.remote_hooks.iter().any(|h| h.id() == entity_id)
            || self.potions.thrown.iter().any(|t| t.id == entity_id)
            || self
                .dragon
                .as_ref()
                .is_some_and(|d| d.id == entity_id && (d.death_time > 0 || !d.alive()));
        if simulated {
            return false;
        }
        let on_ground = self.entity_on_ground(entity).unwrap_or(false);
        self.remote_move_entity_and_file(
            id,
            Some(Path::Linear(position)),
            rotation.map(|[yaw, pitch]| (yaw, pitch)),
            on_ground,
        );
        true
    }
}
