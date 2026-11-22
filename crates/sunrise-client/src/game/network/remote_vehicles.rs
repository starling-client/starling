//! Boats, minecarts, item frames, armor stands and paintings for a server's
//! client: single player on its server thread, a LAN guest, a player on a
//! dedicated server.
//!
//! The client aims at them like at mobs (`Minecraft.hitResult`, then
//! `ServerboundInteractPacket`), and rides the vehicle the server seats it in
//! (`ClientPacketListener.handleSetEntityPassengersPacket`). A boat is
//! controlled by its rider's client: it ticks here and its moves go to the
//! server (`LocalPlayer.tick`: `ServerboundMoveVehiclePacket`,
//! `AbstractBoat.tick`: `ServerboundPaddleBoatPacket`). So is a saddled horse,
//! donkey, mule or camel: its rider's client moves it and reports the moves,
//! and tells the server of a riding jump (`START_RIDING_JUMP`). Pigs and striders
//! also move on their rider's client when saddled and steered with the right stick.
//! A minecart
//! moves on the server with the rider's input, and the rider sits in it.
use super::{Game, Keys, ray_box, remote::Remote};
use std::collections::HashMap;
use sunrise_client::mobs::{Mob, equine, happy_ghast};
use sunrise_client::{keybinds, text::Text};
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_protocol::packets::play::{self, Serverbound as S};
use sunrise_world::stepped_interpolation::PosRot;

/// One edge of the riding graph sent by `SetPassengers`.
pub(super) struct PassengerSeat {
    pub vehicle: u32,
    pub index: usize,
}

fn passenger_chain(
    vehicles: &HashMap<u32, PassengerSeat>,
    id: u32,
) -> impl Iterator<Item = u32> + '_ {
    std::iter::successors(Some(id), |id| vehicles.get(id).map(|seat| seat.vehicle))
        .take(vehicles.len() + 2)
}

/// Walks to the root without allocating. In a cycle, returns the last reached
/// ID for the collision code's bounded stack walk; there is no actual root.
pub(super) fn passenger_root(
    vehicles: &HashMap<u32, PassengerSeat>,
    id: u32,
) -> std::result::Result<u32, u32> {
    let root = passenger_chain(vehicles, id).last().unwrap_or(id);
    if vehicles.contains_key(&root) {
        Err(root)
    } else {
        Ok(root)
    }
}

impl Remote {
    pub(super) fn riding_entity_exists(&self, id: i32, local_exists: bool) -> bool {
        if id == self.entity_id {
            local_exists
        } else {
            self.entities.contains_key(&id)
        }
    }

    fn forget_passengers(&mut self, vehicle: u32) {
        self.passenger_vehicles
            .retain(|_, seat| seat.vehicle != vehicle);
    }

    pub(super) fn passenger_vehicle(&self, passenger: u32) -> Option<u32> {
        self.passenger_vehicles
            .get(&passenger)
            .map(|seat| seat.vehicle)
    }

    /// Passenger IDs and their packet indices, not sorted.
    pub(super) fn passenger_seats(&self, vehicle: u32) -> impl Iterator<Item = (u32, usize)> + '_ {
        self.passenger_vehicles
            .iter()
            .filter_map(move |(&id, seat)| (seat.vehicle == vehicle).then_some((id, seat.index)))
    }

    pub(super) fn has_passengers(&self, vehicle: u32) -> bool {
        self.passenger_seats(vehicle).next().is_some()
    }
}

/// An object under the crosshair: its entity id, how far along the view it
/// was hit, and where (`EntityHitResult.getLocation` minus the entity's
/// position, the interact packet's location).
pub(super) struct ObjectHit {
    pub id: i32,
    pub distance: f64,
    pub location: [f64; 3],
}

impl Game<'_> {
    /// BlockAttachedEntity.isPickable: the nearest fence knot's box the view meets, with its id,
    /// distance and position. Its box is wider than the post inside it, so the crosshair meets it
    /// first and LeashFenceKnotEntity.interact takes the click. The fence block behind only ties
    /// the player's own mobs (LeadItem.useOn), it never takes one back.
    fn remote_knot_hit(&self) -> Option<(i32, f64, [f64; 3])> {
        let mut pick = crate::game::pick_accumulator::PickAccumulator::new(self.entity_reach);
        self.remote_knot_hit_with_pick(&mut pick)
    }
    fn remote_knot_hit_with_pick(
        &self,
        pick: &mut crate::game::pick_accumulator::PickAccumulator,
    ) -> Option<(i32, f64, [f64; 3])> {
        if let Some(hit) = self.cached_entity_pick() {
            let (entity, distance, _) = hit?;
            let block = *self.remote_knots.get(&entity.id())?;
            let p = sunrise_client::mobs::leash::knot_state(block).position;
            return Some((entity.id() as i32, distance, [p.x, p.y, p.z]));
        }
        let (eye, dir) = self.aim()?;
        self.remote_knots
            .iter()
            .filter_map(|(&id, &block)| {
                let knot = sunrise_client::mobs::leash::knot_state(block);
                let p = knot.position;
                let half = knot.width / 2.0;
                let bounds = Aabb::new(
                    Vec3::new(p.x - half, p.y, p.z - half),
                    Vec3::new(p.x + half, p.y + knot.height, p.z + half),
                );
                ray_box(eye, dir, self.entity_reach, bounds)
                    .filter(|&d| pick.consider(self, id, d))
                    .map(|d| (id as i32, d, [p.x, p.y, p.z]))
            })
            .last()
    }

    /// Whether a use starts on an entity: the crosshair's `EntityHitResult`, or a fence knot.
    /// Only a use: Sunrise's attacks still go to the fence behind a knot.
    pub(super) fn remote_use_targets_entity(&self) -> bool {
        if self.uses_stored_pick() {
            return self.entity_targeted();
        }
        self.entity_targeted()
            || self
                .remote_knot_hit()
                .is_some_and(|(_, d, _)| self.before_block(d))
    }

    /// The nearest pickable non-mob object the view hits within reach; also fence knots when
    /// using.
    pub(super) fn remote_object_hit(&self, attacking: bool) -> Option<ObjectHit> {
        let mut pick = crate::game::pick_accumulator::PickAccumulator::new(self.entity_reach);
        self.remote_object_hit_with_pick(attacking, &mut pick)
    }
    pub(super) fn remote_object_hit_with_pick(
        &self,
        attacking: bool,
        pick: &mut crate::game::pick_accumulator::PickAccumulator,
    ) -> Option<ObjectHit> {
        if let Some(hit) = self.cached_entity_pick() {
            let (entity, distance, point) = hit?;
            if self.mobs.iter().any(|m| m.id == entity.id())
                || self
                    .creatures
                    .projectiles
                    .iter()
                    .any(|p| p.id == entity.id())
            {
                return None;
            }
            let origin = self.entity_position(entity)?;
            return Some(ObjectHit {
                id: entity.id() as i32,
                distance,
                location: std::array::from_fn(|i| point[i] - origin[i]),
            });
        }
        let (eye, dir) = self.aim()?;
        let at = |d: f64| {
            [
                eye[0] + dir[0] * d,
                eye[1] + dir[1] * d,
                eye[2] + dir[2] * d,
            ]
        };
        let block_centre = |p: [i32; 3]| {
            [
                f64::from(p[0]) + 0.5,
                f64::from(p[1]) + 0.5,
                f64::from(p[2]) + 0.5,
            ]
        };
        let mut hit: Option<(i32, f64, [f64; 3])> = None;
        if let Some((i, d)) = self.painting_hit_with_pick(pick) {
            let p = &self.paintings[i];
            hit = Some((p.id as i32, d, block_centre(p.pos)));
        }
        if let Some((i, d)) = self.frame_hit_with_pick(pick) {
            let f = &self.item_frames[i];
            hit = Some((f.id as i32, d, block_centre(f.pos)));
        }
        if let Some((i, d, _)) = self.armor_stand_hit_with_pick(pick) {
            let a = &self.armor_stands[i];
            let p = a.position;
            hit = Some((a.id as i32, d, [p.x, p.y, p.z]));
        }
        if let Some((i, d, _)) = self.cushion_hit_with_pick(pick) {
            let c = &self.cushions[i];
            let p = c.position;
            hit = Some((c.id as i32, d, [p.x, p.y, p.z]));
        }
        if let Some((i, d)) = self.boat_hit_with_pick(pick) {
            let p = self.boats[i].position;
            hit = Some((self.boats[i].id as i32, d, [p.x, p.y, p.z]));
        }
        if let Some((i, d)) = self.cart_hit_with_pick(pick) {
            let p = self.carts[i].position;
            hit = Some((self.carts[i].id as i32, d, [p.x, p.y, p.z]));
        }
        // Interaction.isPickable: an invisible box the server clicks through
        // to its scripts, for both uses and attacks.
        let reach = self.entity_reach;
        for (&id, i) in &self.interactions {
            if let Some(d) =
                ray_box(eye, dir, reach, i.bounds()).filter(|&d| pick.consider(self, id as u32, d))
            {
                let p = i.position;
                hit = Some((id, d, [p.x, p.y, p.z]));
            }
        }
        if let Some(remote) = &self.remote {
            let catalog = crate::game::general::general_type_catalog();
            for (&id, row) in &remote.general.rows {
                if catalog.pickable(row.type_slot)
                    && let Ok(entity) = u32::try_from(id)
                    && let Some(distance) = ray_box(eye, dir, reach, row.bounds())
                    && pick.consider(self, entity, distance)
                {
                    hit = Some((id, distance, row.position));
                }
            }
        }
        if (!attacking || self.client_pick.ray.is_some())
            && let Some((id, d, p)) = self.remote_knot_hit_with_pick(pick)
        {
            hit = Some((id, d, p));
        }
        // A right click on a crystal interacts with it too (MultiPlayerGameMode.interact
        // sends the packet), then passes on to the item (Entity.interact).
        if let Some((i, d)) = self.crystal_hit_with_pick(eye, dir, pick) {
            let p = self.crystals[i].position;
            hit = Some((self.crystals[i].id as i32, d, [p.x, p.y, p.z]));
        }
        // EnderDragonPart.isPickable: the server's dragon hurts by part, and
        // a right click on one passes as a crystal's does.
        if let Some(part) = self.remote_dragon_part_hit_with_pick(eye, dir, reach, pick) {
            hit = Some(part);
        }
        let (id, distance, origin) = hit?;
        let point = at(distance);
        Some(ObjectHit {
            id,
            distance,
            location: [
                point[0] - origin[0],
                point[1] - origin[1],
                point[2] - origin[2],
            ],
        })
    }

    /// `EndCrystal.isPickable`: the nearest crystal the view hits within reach, by its
    /// 2 x 2 box, with the server's entity id (not special_targets' local projectile
    /// target id).
    pub(super) fn crystal_hit(&self) -> Option<(usize, f64)> {
        let (eye, dir) = self.aim()?;
        let mut pick = super::pick_accumulator::PickAccumulator::new(self.entity_reach);
        self.crystal_hit_with_pick(eye, dir, &mut pick)
    }

    fn crystal_hit_with_pick(
        &self,
        eye: [f64; 3],
        dir: [f64; 3],
        pick: &mut super::pick_accumulator::PickAccumulator,
    ) -> Option<(usize, f64)> {
        let mut hit = None;
        for (i, crystal) in self.crystals.iter().enumerate().filter(|(_, c)| c.id != 0) {
            if let Some(d) = ray_box(eye, dir, self.entity_reach, crystal.bounds())
                .filter(|&d| pick.consider(self, crystal.id, d))
            {
                hit = Some((i, d));
            }
        }
        hit
    }

    /// `ClientPacketListener.handleSetEntityPassengersPacket`: who rides
    /// `vehicle` now. The local player gets in or out; mobs and other
    /// players sit where the server keeps them.
    pub(super) fn remote_set_passengers(&mut self, vehicle: i32, mut passengers: Vec<i32>) {
        let Some(remote) = self.remote.as_mut() else {
            return;
        };
        // The local entity is known from Login, even before its motion body
        // arrives on PlayerPosition. Other entities must already be tracked.
        let local_exists = remote.logged_in || self.player.is_some();
        if !remote.riding_entity_exists(vehicle, local_exists) {
            log::warn!("Sunrise: ignoring passenger list for unknown vehicle {vehicle}");
            return;
        }
        // Forced client startRiding still respects Entity.couldAcceptPassenger.
        if remote
            .tracked(vehicle)
            .is_some_and(|(kind, _)| !super::seats::could_accept_passenger(kind))
        {
            return;
        }
        let vehicle_id = vehicle as u32;
        remote.forget_passengers(vehicle_id);
        let mut index = 0;
        passengers.retain(|&passenger| {
            if !remote.riding_entity_exists(passenger, local_exists) {
                return false;
            }
            let rider = passenger as u32;
            if remote.passenger_vehicle(rider) == Some(vehicle_id)
                || passenger_chain(&remote.passenger_vehicles, vehicle_id)
                    .any(|ancestor| ancestor == rider)
            {
                return false;
            }
            remote.passenger_vehicles.insert(
                rider,
                PassengerSeat {
                    vehicle: vehicle_id,
                    index,
                },
            );
            index += 1;
            true
        });
        self.apply_remote_passengers(vehicle, &passengers);
    }

    /// Removing a vehicle ejects its passengers even without a final packet.
    pub(super) fn remote_clear_passengers(&mut self, vehicle: i32) {
        if let Some(remote) = self.remote.as_mut() {
            remote.forget_passengers(vehicle as u32);
            self.apply_remote_passengers(vehicle, &[]);
        }
    }

    /// A death respawn replaces the local entity even if its level and ID stay.
    pub(super) fn reset_remote_player_riding(&mut self) {
        // The old body is gone, not dismounting. Clear local control before
        // updating the vehicle's remaining list so it won't play a dismount.
        if let Some(vehicle) = self.riding.take() {
            if let Some(mob) = self.mob_index.get_mut(&mut self.mobs, vehicle) {
                mob.rider = None;
            }
            if let Some(i) = self.boats.iter().position(|b| b.id == vehicle) {
                self.boats.update(i, |b| b.lerp = None);
            }
        }
        if let Some(remote) = self.remote.as_mut() {
            let player = remote.entity_id;
            remote.first_passenger = false;
            remote.passenger_index = 0;
            remote.ghast_dismount = None;
            let parent = remote.passenger_vehicles.remove(&(player as u32));
            self.remote_clear_passengers(player);
            if let Some(parent) = parent
                && let Some(remote) = self.remote.as_mut()
            {
                let mut siblings: Vec<_> = remote.passenger_seats(parent.vehicle).collect();
                siblings.sort_unstable_by_key(|&(_, index)| index);
                let ids: Vec<_> = siblings
                    .into_iter()
                    .enumerate()
                    .map(|(index, (id, _))| {
                        remote.passenger_vehicles.insert(
                            id,
                            PassengerSeat {
                                vehicle: parent.vehicle,
                                index,
                            },
                        );
                        id as i32
                    })
                    .collect();
                self.apply_remote_passengers(parent.vehicle as i32, &ids);
            }
        }
    }

    /// Apply the admitted list to the presentation and simulation copies too.
    fn apply_remote_passengers(&mut self, vehicle: i32, passengers: &[i32]) {
        let Some(me) = self.remote.as_ref().map(|r| r.entity_id) else {
            return;
        };
        let vehicle_id = vehicle as u32;
        self.remote_seat_passengers(vehicle_id, passengers);
        let player_controls = passengers.first().is_some_and(|&id| {
            self.remote
                .as_ref()
                .is_some_and(|r| r.level_player(id).is_some())
        });
        if let Some(i) = self.boats.iter().position(|b| b.id == vehicle_id) {
            self.boats.update(i, |boat| {
                boat.paddle_controller = player_controls;
            });
        }

        for m in &mut self.mobs {
            // `Entity.isVehicle` (the saddle reins of a ridden horse): the
            // server sends no rider on the mob itself, only this list.
            if m.id == vehicle_id {
                m.extra.rider = passengers.first().map(|&p| p as u32);
            }
            if m.extra.mount == Some(vehicle_id) && !passengers.contains(&(m.id as i32)) {
                m.extra.mount = None;
                m.extra.passenger = false;
            } else if passengers.contains(&(m.id as i32)) {
                m.extra.mount = Some(vehicle_id);
                m.extra.passenger = true;
            }
        }
        for (id, display) in &mut self.text_displays {
            if passengers.contains(id) {
                display.vehicle = Some(vehicle_id);
            } else if display.vehicle == Some(vehicle_id) {
                display.vehicle = None;
            }
        }
        for a in &mut self.armor_stands {
            if passengers.contains(&(a.id as i32)) {
                a.vehicle = Some(vehicle_id);
            } else if a.vehicle == Some(vehicle_id) {
                a.vehicle = None;
            }
        }
        for (&id, i) in &mut self.interactions {
            if passengers.contains(&id) {
                i.vehicle = Some(vehicle_id);
            } else if i.vehicle == Some(vehicle_id) {
                i.vehicle = None;
            }
        }
        if passengers.contains(&me)
            && let Some(r) = self.remote.as_mut()
        {
            r.first_passenger = passengers.first() == Some(&me);
            r.passenger_index = passengers.iter().position(|&p| p == me).unwrap_or(0);
        }
        if passengers.contains(&me) {
            if self.riding != Some(vehicle_id) {
                self.riding = Some(vehicle_id);
                // ClientPacketListener.handleSetEntityPassengersPacket: boarding says how to get off,
                // on the overlay and to the narrator.
                let key = if self.cushions.iter().any(|c| c.id == vehicle_id) {
                    "mount.sit"
                } else {
                    "mount.onboard"
                };
                let message = Text::translate(
                    key,
                    &[keybinds::display_name(self.settings.keys.get("key.sneak"))],
                );
                self.say_system_now(&message);
                self.titles.set_overlay(message, false);
                if let Some(i) = self.carts.iter().position(|c| c.id == vehicle_id) {
                    self.carts.update(i, |c| c.passenger_yaw_delta = 0.0);
                }
                if let Some(p) = self.player.as_mut() {
                    p.velocity = Vec3::ZERO;
                    p.fall_distance = 0.0;
                }
            }
        } else if self.riding == Some(vehicle_id) {
            // The server takes the player out, then teleports it. A boat's easing toward
            // a teleport this client was given ends with its control.
            self.riding = None;
            if let Some(i) = self.boats.iter().position(|b| b.id == vehicle_id) {
                self.boats.update(i, |b| b.lerp = None);
            }
            if let Some(m) = self.mob_index.get_mut(&mut self.mobs, vehicle_id) {
                m.rider = None;
            }
            // HappyGhast.removePassenger: a still timeout of 10 (the server's
            // STAYS_STILL confirms it), and the rider lands on top of the ghast
            // (`place_ghast_under`).
            if let Some(m) = self.mobs.iter_mut().find(|m| {
                m.id == vehicle_id && m.species() == sunrise_client::mobs::Species::HappyGhast
            }) {
                m.extra.counter = 10;
                if let Some(r) = self.remote.as_mut() {
                    r.ghast_dismount = Some(vehicle_id);
                }
            }
        }
    }

    /// Camera.alignWithEntity's NewMinecartBehavior branch: sample the
    /// vehicle's curved path at this frame, then apply the player's seat
    /// attachment. Camera's own eye-height interpolation still runs afterwards.
    pub(super) fn remote_minecart_rider_position(&self, partial: f32) -> Option<[f64; 3]> {
        if !self.minecart_improvements() {
            return None;
        }
        let id = self.riding?;
        self.carts
            .iter()
            .find(|c| c.id == id && c.kind == super::minecarts::CartKind::Rideable)?;
        let (mut position, _, _) = self.remote_minecart_lerp(id, partial)?;
        position[1] += super::minecarts::SEAT;
        Some(position)
    }

    /// AbstractBoat.getControllingPassenger: only the first player steers a boat.
    /// A non-remote world retains its existing local movement ownership.
    pub(super) fn controls_boat(&self, id: u32) -> bool {
        self.riding == Some(id) && self.remote.as_ref().is_none_or(|r| r.first_passenger)
    }

    /// Tick the local player's vehicle before the player tick:
    /// false when it rides nothing.
    pub(super) fn remote_ride(&mut self, keys: Keys) -> bool {
        let Some(id) = self.riding else {
            return false;
        };
        // Settle an invisible seat riding a moving avatar before reading it.
        // The normal end-of-tick pass alone leaves the local rider a tick behind.
        self.position_riders();
        if let Some(i) = self.mobs.iter().position(|m| m.id == id) {
            // LocalPlayer.aiStep: the jump bar charges while jump is held.
            let jump = self.charge_mount_jump(i, keys);
            if self.drives(&self.mobs[i]) {
                self.drive_mount(i, keys, jump);
            }
            // Another animal moves on the server with the rider's input
            // (PlayerInput); the rider sits where `positionRider` puts it.
            self.position_rider();
            return true;
        }
        // Generic entities are valid vehicles too (plugins use displays as seats).
        // Keep riding set so sendChanges sends rotation, never a walking position.
        if !self.cushions.iter().any(|c| c.id == id)
            && !self.boats.iter().any(|b| b.id == id)
            && !self.carts.iter().any(|c| c.id == id)
        {
            let index = std::collections::HashMap::new();
            if let Some(at) = self.passenger_riding_position(id, None, None, &index) {
                let before = self
                    .text_displays
                    .get(&(id as i32))
                    .map(|d| d.seat().1)
                    .or_else(|| {
                        self.armor_stands.iter().find(|a| a.id == id).map(|a| {
                            let delta = a.previous.subtract(a.position);
                            at + glam::DVec3::new(delta.x, delta.y, delta.z)
                        })
                    });
                if let Some(p) = self.player.as_mut() {
                    let scale = p.dimensions.width
                        / sunrise_client::pose::Pose::Standing.dimensions(1.0).width;
                    let (seat, previous) = generic_player_seat(at, before.unwrap_or(at), scale);
                    let previous = if before.is_some() {
                        previous
                    } else {
                        p.position
                    };
                    self.previous_position = if self.posed_rider != self.riding {
                        seat
                    } else {
                        previous
                    };
                    self.posed_rider = self.riding;
                    p.position = seat;
                    p.velocity = Vec3::ZERO;
                    p.fall_distance = 0.0;
                }
                return true;
            }
        }
        let seat = if let Some(c) = self.cushions.iter().find(|c| c.id == id) {
            // The seat does not move; the rider sits on the cushion.
            Some(c.seat())
        } else if let Some(i) = self.boats.iter().position(|b| b.id == id) {
            // AbstractBoat.tick on the controlling client: steer and move.
            // It steers with the input `LocalPlayer.rideTick` latched on the
            // previous tick, so the paddle packet describes the motion the
            // server is sent; this tick's keys are latched after it ticked.
            let mut b = self.boats[i].clone();
            if self.controls_boat(id) {
                // A boat taken over while it eased toward the server's position keeps
                // easing (the handler stays the entity's), and reports where it goes.
                if let Some((to, rotation, steps)) =
                    self.remote.as_mut().and_then(|r| r.take_object_lerp(id))
                {
                    let rotation = rotation.unwrap_or((b.yaw, b.pitch));
                    b.go_on_easing(Vec3::new(to[0], to[1], to[2]), rotation, steps);
                }
                b = self.tick_boat(b, true);
                b.input = [keys.left, keys.right, keys.forward, keys.backward];
            }
            // positionRider turns and clamps the rider's yaw without changing yRotO.
            self.yaw += b.delta_rotation;
            self.yaw = sunrise_client::camera::clamp_boat_rotation(self.yaw, b.yaw).0;
            self.store_boat_back(i, b);
            // In front of a mob that shares the boat.
            Some(self.player_boat_seat(i))
        } else if self.carts.iter().any(|c| c.id == id) {
            // Minecart.positionRider runs on the client after the cart tick.
            // The offset belongs to the cart, so switching vehicles cannot
            // transfer the previous vehicle's pending turn.
            let rotate =
                self.minecart_improvements() && self.settings.file.bool("rotateWithMinecart");
            if let Some(i) = self.carts.iter().position(|c| c.id == id) {
                self.carts.update(i, |c| {
                    c.passenger_yaw_delta = 0.0;
                    if rotate && c.kind == super::minecarts::CartKind::Rideable {
                        c.rotate_passenger(&mut self.yaw);
                    }
                });
            }
            // A minecart: the server moves it, the rider sits in it.
            self.carts.iter().find(|c| c.id == id).map(|c| {
                c.position
                    .add_vector(Vec3::new(0.0, super::minecarts::SEAT, 0.0))
            })
        } else if let Some(at) =
            self.passenger_riding_position(id, None, None, &std::collections::HashMap::new())
        {
            let scale = self.player.as_ref().map_or(1.0, |p| {
                p.dimensions.width / sunrise_client::pose::Pose::Standing.dimensions(1.0).width
            });
            Some(Vec3::new(at.x, at.y - f64::from(0.6_f32 * scale), at.z))
        } else {
            // Entity.baseTick: a passenger whose vehicle is gone stops
            // riding it. Nothing with the id is in the level any more (it
            // died, or the world it rode in was left), so the seat goes.
            self.riding = None;
            return false;
        };
        if let Some(p) = self.player.as_mut() {
            // The tick a rider takes its seat is a teleport (`Entity.moveTo`,
            // which sets the old position too): the camera must not sweep from
            // where the player stood to the cushion, boat or cart. The ticks
            // after it move with the vehicle and interpolate as usual.
            let first = self.posed_rider != self.riding;
            self.posed_rider = self.riding;
            if let Some(v) = seat {
                self.previous_position = if first { v } else { p.position };
                p.position = v;
            } else {
                self.previous_position = p.position;
            }
            p.velocity = Vec3::ZERO;
            p.fall_distance = 0.0;
        }
        true
    }

    /// `LocalPlayer.tick` for a passenger: the boat it controls goes to the
    /// server every tick with its paddles (the rider's own position does not).
    pub(super) fn remote_vehicle_packets(&self) -> Vec<S> {
        if let Some(m) = self.driven_mount() {
            return vec![mount_move(m, self.driven_mount_heading(m))];
        }
        let Some(b) = self
            .riding
            .filter(|&id| self.controls_boat(id))
            .and_then(|id| self.boats.iter().find(|b| b.id == id))
        else {
            return Vec::new();
        };
        // ServerboundMoveVehiclePacket.fromEntity: while a teleport eases the
        // boat, the spot it is going to.
        let (at, yaw, pitch) = b.client_position();
        vec![
            S::MoveVehicle(play::MoveVehicleRequest {
                position: [at.x, at.y, at.z],
                yaw,
                pitch,
                on_ground: b.ground.on_ground,
            }),
            S::PaddleBoat(play::PaddleBoat {
                left: b.paddling[0],
                right: b.paddling[1],
            }),
        ]
    }

    /// The vehicle this client moves, when it is `id`, as `PositionMoveRotation.of` sees it:
    /// its client position and rotation (`getClientPositionAndRotation`: while an
    /// interpolation runs, where it is going) and its known movement, which is its controller's
    /// (`getKnownMovement`): the local player's, nothing while it rides (`Entity.rideTick`).
    pub(super) fn driven_vehicle(&self, id: u32) -> Option<(Vec3, f32, f32, Vec3)> {
        let known = self.player.as_ref().map_or(Vec3::ZERO, |p| p.velocity);
        if let Some(m) = self.driven_mount().filter(|m| m.id == id) {
            let at = self.driven_mount_heading(m);
            let [x, y, z] = at.position;
            return Some((Vec3::new(x, y, z), at.yaw, at.pitch, known));
        }
        let b = self.driven_boat().filter(|&i| self.boats[i].id == id)?;
        let (at, yaw, pitch) = self.boats[b].client_position();
        Some((at, yaw, pitch, known))
    }

    /// The boat this client steers, as an index into `boats`.
    fn driven_boat(&self) -> Option<usize> {
        self.riding
            .filter(|&id| self.controls_boat(id))
            .and_then(|id| self.boats.iter().position(|b| b.id == id))
    }

    /// `Entity.getClientPosition` of the vehicle this client moves: for a boat a
    /// teleport is easing, where it is going.
    pub(super) fn driven_client_position(&self) -> Option<Vec3> {
        if let Some(m) = self.driven_mount() {
            let [x, y, z] = self.driven_mount_heading(m).position;
            return Some(Vec3::new(x, y, z));
        }
        self.driven_boat()
            .map(|i| self.boats[i].client_position().0)
    }

    /// `handleTeleportEntity` (`setValuesFromPositionPacket`) for the vehicle this
    /// client moves, with the packet's relative parts already counted: it eases there
    /// (`moveOrInterpolateTo`) unless it is more than 64 blocks away, when it is put
    /// there and reported to the server at once. Its deltaMovement and onGround are the
    /// packet's.
    pub(super) fn teleport_driven_vehicle(
        &mut self,
        to: [f64; 3],
        rotation: (f32, f32),
        velocity: Vec3,
        on_ground: bool,
    ) {
        let at = Vec3::new(to[0], to[1], to[2]);
        if let Some(id) = self.driven_mount().map(|m| m.id) {
            // tooBigToInterpolate measures from where the mount is going while it eases.
            let far = self.driven_mount().is_some_and(|m| {
                let [x, y, z] = self.driven_mount_heading(m).position;
                Vec3::new(x, y, z).distance_squared(at) > 4096.0
            });
            if far {
                self.remote_move_vehicle_and_file(to, rotation);
            } else if let Some(i) = self.mobs.iter().position(|m| m.id == id) {
                let at = PosRot {
                    position: to,
                    yaw: rotation.0,
                    pitch: rotation.1,
                };
                self.interpolate_driven_mount_to(i, at);
            }
            if let Some(m) = self.mob_index.get_mut(&mut self.mobs, id) {
                m.velocity = velocity;
                m.ground.on_ground = on_ground;
            }
            if far && let Some(report) = self.remote_vehicle_packets().into_iter().next() {
                self.send(report);
            }
            return;
        }
        let Some(i) = self.driven_boat() else {
            return;
        };
        // tooBigToInterpolate measures from where the boat is going while it eases.
        let far = self.boats[i].client_position().0.distance_squared(at) > 4096.0;
        let id = self.boats[i].id;
        self.boats.update(i, |b| {
            if far {
                b.lerp = None;
                b.position = at;
                b.previous = at;
                b.yaw = rotation.0;
                b.previous_yaw = rotation.0;
                b.pitch = rotation.1;
            } else {
                b.interpolate_to(at, rotation.0, rotation.1);
            }
            b.velocity = velocity;
            b.ground.on_ground = on_ground;
        });
        if far {
            self.entity_section_moved_id(id);
            if let Some(report) = self.remote_vehicle_packets().into_iter().next() {
                self.send(report);
            }
        }
    }

    /// `ClientPacketListener.handleMoveVehicle`: the server put the boat or
    /// the mount the player drives back (`absSnapTo`: position and rotation, the
    /// old ones too; its deltaMovement stays).
    pub(super) fn remote_move_vehicle(&mut self, position: [f64; 3], (yaw, pitch): (f32, f32)) {
        let at = Vec3::new(position[0], position[1], position[2]);
        // absSnapRotationTo.
        let (yaw, pitch) = (yaw % 360.0, pitch.clamp(-90.0, 90.0) % 360.0);
        if let Some(id) = self.driven_mount().map(|m| m.id) {
            self.cancel_driven_mount_interpolation(id);
        }
        if let Some(id) = self.driven_mount().map(|m| m.id)
            && let Some(m) = self.mob_index.get_mut(&mut self.mobs, id)
        {
            m.previous = at;
            m.position = at;
            m.yaw = yaw;
            m.previous_yaw = yaw;
            m.head_pitch = pitch;
            m.previous_head_pitch = pitch;
            return;
        }
        let Some(i) = self.driven_boat() else {
            return;
        };
        // After the interpolation is cancelled.
        self.boats.update(i, |b| {
            b.lerp = None;
            b.previous = at;
            b.position = at;
            b.yaw = yaw;
            b.previous_yaw = yaw;
            b.pitch = pitch;
        });
    }

    /// `AbstractBoat.handleEntityEvent`: 71 and 72, a bubble column dragging
    /// the boat down or throwing it up (`handleBubbleColumnEffect`, which the
    /// client applies to the boat it drives).
    pub(super) fn remote_boat_event(&mut self, id: i32, event: i8) {
        let drag_down = event == super::boats::BUBBLE_DOWN_EVENT;
        if !drag_down && event != super::boats::BUBBLE_UP_EVENT {
            return;
        }
        if let Some(i) = self.boats.iter().position(|b| b.id == id as u32) {
            self.handle_bubble_column_effect(i, drag_down);
        }
    }

    /// `AbstractBoat.tick`: the part that reaches bubble columns, for every
    /// boat on a client: `status`, the two `applyEffectsFromBlocks` (a boat
    /// above a column may splash; the boat the player drives is lifted or
    /// dragged by a column it is inside) and `tickBubbleColumn`'s wobble from
    /// the synched bubble time, followed by the oars on boats this client does not drive.
    pub(super) fn tick_remote_boat_bubbles(&mut self) {
        for i in 0..self.boats.len() {
            let driven = self.controls_boat(self.boats[i].id);
            if !driven {
                // `AbstractBoat.tick`: the non-authoritative copy carries no
                // cached motion into the next time this player drives it.
                self.boats.update(i, |b| b.velocity = Vec3::ZERO);
                self.boats.update(i, |b| b.delta_rotation = 0.0);
                self.refresh_boat_status(i);
            }
            let touches = self.boat_bubble_columns(i);
            for pass in 0..2 {
                if pass == 1 && driven {
                    self.boat_second_step(i);
                }
                for &column in &touches {
                    if column.surface {
                        if self.boat_splash_roll(i) {
                            self.boat_bubble_splash(i);
                        }
                    } else if driven {
                        self.boat_inside_bubble_column(i, column);
                    }
                }
            }
            self.boats.update(i, super::boats::Boat::tick_bubble_wobble);
            if !driven {
                self.boats
                    .update(i, |b| Self::tick_boat_paddles(b, &mut self.audio));
            }
        }
    }

    /// Whether this client moves `m`, the mount the player rides: as its
    /// first passenger (`AbstractHorse.getControllingPassenger`), on its
    /// saddle (`isLocalInstanceAuthoritative`).
    pub(super) fn drives(&self, m: &Mob) -> bool {
        self.riding == Some(m.id)
            && client_moves(
                m,
                self.remote.as_ref().is_some_and(|r| r.first_passenger),
                self.steers_mount(m),
            )
    }

    fn steers_mount(&self, m: &Mob) -> bool {
        super::mounts::steering_item(
            m.species(),
            self.held().map(|s| s.item.as_ref()),
            self.offhand.as_ref().map(|s| s.item.as_ref()),
        )
    }

    /// The mount this client moves, if the player rides one.
    pub(super) fn driven_mount(&self) -> Option<&Mob> {
        let id = self.riding?;
        self.mobs.iter().find(|m| m.id == id && self.drives(m))
    }

    /// `LocalPlayer.aiStep` decides sprint after the rider's base tick sampled fluids.
    pub(super) fn remote_rider_sprint(&mut self, keys: Keys, window: i32) -> crate::Result<()> {
        let mount = self.mobs.iter().position(|m| self.drives(m));
        let can_sprint = mount.is_some_and(|i| equine::can_sprint(self.mobs[i].species()));
        self.player_sprint_tick(keys, window, Some((can_sprint, mount.is_some())))
    }

    /// `LivingEntity.travelRidden`, then `LocalPlayer.aiStep`, on the
    /// controlling client. The vehicle ticks before its passenger
    /// (`ClientLevel.tickNonPassenger`), so the mount moves (`tickRidden`,
    /// then `travel`) on what the rider's previous tick left: `xxa`/`zza` from
    /// `applyInput` (the keys through `modifyInput`: ×0.98, square movement),
    /// the sprint and the jump `onPlayerJump` set. Only then does this tick's
    /// aiStep read the keys, send a released jump (`sendRidingJump`) and run
    /// `onPlayerJump`, for the mount's next tick. Moving on this tick's keys
    /// put every ridden move a tick ahead of the server's view of the input.
    fn drive_mount(&mut self, i: usize, keys: Keys, jump: f32) {
        let old_pose = self.interpolate_driven_mount(i);
        let steer = self.steers_mount(&self.mobs[i]);
        let (yaw, pitch) = (self.yaw, self.pitch);
        // The first ridden tick reads what the player's last walking tick applied.
        let walked = self.player.as_ref().map_or([0.0; 2], |p| p.applied_input);
        // Camel.getRiddenSpeed reads the live controller flag before the rider ticks.
        let sprint = self.player.as_ref().is_some_and(|p| p.sprinting);
        let m = &mut self.mobs[i];
        let last = m.rider.unwrap_or(equine::Rider {
            strafe: walked[0],
            forward: walked[1],
            ..Default::default()
        });
        // tickRidden reads the controller's look as it is now.
        m.rider = Some(equine::Rider {
            yaw,
            pitch,
            steer,
            sprint,
            client: false,
            ..last
        });
        let mut mob = self.mobs.remove(i);
        mob.extra.client_side = true;
        let mut lent = sunrise_client::health::LentLevel::take(&mut self.world);
        let mut events = Vec::new();
        let hooks = self.damage_hooks.client_side();
        {
            let world = self.mob_world_base(&[]);
            let mut level = sunrise_client::health::DamageLevel::lent(
                &self.world,
                self.pack.assets.registry,
                &mut lent,
                None,
                None,
                &hooks,
            )
            .on_client(true);
            mob.tick(&world, &mut level, &mut events);
        }
        // Only the server's packets take the mount away.
        mob.removed = false;
        lent.give_back(&mut self.world);
        // Entity.commonTick's setOldPosAndRot came before the interpolation step: the
        // frame eases from the last tick's pose, not from the stepped one.
        if let Some((position, yaw, pitch)) = old_pose {
            mob.previous = position;
            mob.previous_yaw = yaw;
            mob.previous_head_pitch = pitch;
        }
        self.mobs.insert(i, mob);
        // LocalPlayer.aiStep, after the vehicle's tick.
        if jump > 0.0
            && let Some(id) = self.remote.as_ref().map(|r| r.entity_id)
        {
            self.send(riding_jump(id, self.mount.jump_scale));
        }
        let [strafe, forward] = sunrise_client::input::modify_input(keys.direction(), None, None);
        let m = &mut self.mobs[i];
        if jump > 0.0 && equine::is_equine(m.species()) {
            // `onPlayerJump`: the mount slides on the rider's keys and rears
            // while the server does the same.
            equine::on_player_jump(m);
        }
        // playerJumpPendingScale waits for the ground (the tick spends it).
        let pending = m.rider.map_or(0.0, |r| r.jump);
        m.rider = Some(equine::Rider {
            strafe,
            forward,
            yaw,
            pitch,
            jump: if jump > 0.0 { jump } else { pending },
            steer,
            sprint,
            jumping: keys.jump,
            sneak: keys.shift,
            client: false,
        });
        // #4821: footstep, gallop, breathe, splash and landing sounds from mob.tick
        let _ = self.handle_mob_events(events);
    }
}

/// Whether a player's client moves `m`, the mount it rides
/// (`isLocalInstanceAuthoritative`): a saddled horse, donkey, mule or camel
/// it is the first passenger of (`AbstractHorse.getControllingPassenger`),
/// or a harnessed adult Happy Ghast it is the first passenger of
/// (`HappyGhast.getControllingPassenger`: wearing body armor, player on
/// top, not on a still timeout — the rider steers it with `travelRidden`
/// on their own client, and the mount goes to the server as `MoveVehicle`).
fn client_moves(m: &Mob, first_passenger: bool, steer: bool) -> bool {
    if !first_passenger || !m.alive() {
        return false;
    }
    if equine::moved_by_rider(m.species()) && m.extra.saddled() {
        return !matches!(
            m.species(),
            sunrise_client::mobs::Species::Pig | sunrise_client::mobs::Species::Strider
        ) || steer;
    }
    m.species() == sunrise_client::mobs::Species::HappyGhast
        && happy_ghast::harnessed_adult(m)
        && !happy_ghast::on_still_timeout(m)
}

/// `LocalPlayer.sendRidingJump`: START_RIDING_JUMP with
/// `floor(getJumpRidingScale() * 100)`, as `onPlayerJump` gets it.
fn riding_jump(player: i32, scale: f32) -> S {
    S::PlayerCommand(play::PlayerCommand {
        id: player,
        action: 3,
        data: (scale * 100.0).floor() as i32,
    })
}

/// `ServerboundMoveVehiclePacket.fromEntity`: where the mount this client
/// moves is, and how it is turned (`at`: getClientPositionAndRotation).
fn mount_move(m: &Mob, at: PosRot) -> S {
    S::MoveVehicle(play::MoveVehicleRequest {
        position: at.position,
        yaw: at.yaw,
        pitch: at.pitch,
        on_ground: m.ground.on_ground,
    })
}

/// Avatar's VEHICLE attachment scales independently of the vehicle's seat.
fn generic_player_seat(at: glam::DVec3, before: glam::DVec3, scale: f32) -> (Vec3, Vec3) {
    let offset = 0.6 * f64::from(scale);
    (
        Vec3::new(at.x, at.y - offset, at.z),
        Vec3::new(before.x, before.y - offset, before.z),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn local_player_on_block_display_and_interaction() {
        for kind in ["minecraft:block_display", "minecraft:interaction"] {
            let at = glam::DVec3::new(10.5, -59.5, 0.5);
            let before = glam::DVec3::new(10.0, -59.0, 0.5);
            let (seat, previous) = super::generic_player_seat(at, before, 1.0);
            assert_eq!(
                seat,
                sunrise_core::geometry::Vec3::new(10.5, -60.1, 0.5),
                "{kind}"
            );
            assert_eq!(
                previous,
                sunrise_core::geometry::Vec3::new(10.0, -59.6, 0.5),
                "{kind}"
            );
            let (scaled, _) = super::generic_player_seat(at, before, 2.0);
            assert_eq!(scaled.y, -60.7, "{kind}");
        }
    }

    use super::*;
    use sunrise_client::mobs::MobTypes;

    /// handleTeleportEntity and handleMoveVehicle for the vehicle this client moves (a saddled pig steered with a
    /// carrot on a stick): a relative teleport counts from where the pig is on this client and eases it there,
    /// reported as the spot it is going to; a server move puts it there and is answered at once with a
    /// ServerboundMoveVehiclePacket from that spot.
    #[test]
    #[ignore = "daily"]
    fn the_driven_vehicle_takes_the_servers_teleports_and_moves() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            use sunrise_client::{items::ItemStack, motion::PlayerMotion};
            use sunrise_protocol::packets::play::Clientbound as C;
            let root =
                std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
            let (mut peer, _writer) = crate::game::remote::tests::connect_menu_test(game);
            peer.set_read_timeout(Some(std::time::Duration::from_millis(50)))
                .unwrap();
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
            let types = MobTypes::load(&root).unwrap();
            let pig = std::sync::Arc::new(types.types["minecraft:pig"].clone());
            let mut m = Mob::new(7, pig, Vec3::new(3.5, 64.0, 2.5), 0.0);
            m.set_saddle(Some(ItemStack::new("minecraft:saddle", 1)));
            game.mobs.clear();
            game.mobs.push(m);
            game.inventory[game.selected] = Some(ItemStack::new("minecraft:carrot_on_a_stick", 1));
            game.riding = Some(7);
            game.remote.as_mut().unwrap().first_passenger = true;
            assert!(game.driven_mount().is_some());
            use play::relative as rel;
            game.apply_packet(C::TeleportEntity(play::TeleportEntity {
                id: 7,
                position: [0.0, 0.0, 4.0],
                delta: [0.0; 3],
                yaw: 0.0,
                pitch: 0.0,
                relatives: rel::X | rel::Y | rel::Z | rel::Y_ROT | rel::X_ROT,
                on_ground: false,
            }))
            .unwrap();
            // The pig eases there (its SteppedInterpolationHandler) and reports the spot
            // it is going to.
            assert_eq!(game.mobs[0].position, Vec3::new(3.5, 64.0, 2.5));
            match game.remote_vehicle_packets().first() {
                Some(play::Serverbound::MoveVehicle(p)) => assert_eq!(p.position, [3.5, 64.0, 6.5]),
                other => panic!("{other:?}"),
            }
            while peer.recv().is_ok() {}
            game.apply_packet(C::MoveVehicle(play::MoveVehicle {
                position: [1.5, 64.0, 1.5],
                yaw: 90.0,
                pitch: 0.0,
            }))
            .unwrap();
            assert_eq!(game.mobs[0].position, Vec3::new(1.5, 64.0, 1.5));
            let mut answers = Vec::new();
            while let Ok((id, bytes)) = peer.recv() {
                if let Ok(play::Serverbound::MoveVehicle(p)) = play::Serverbound::decode(id, &bytes)
                {
                    answers.push(p.position);
                }
            }
            assert_eq!(answers, [[1.5, 64.0, 1.5]]);
        });
    }

    #[test]
    #[ignore = "requires prepared resources and the vanilla mount-control reference"]
    fn steering_mount_control_matches_vanilla() {
        use super::super::{
            Audio, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
        };
        use sunrise_client::{items::ItemStack, motion::PlayerMotion};
        use sunrise_protocol::connection::Pipe;
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let reference = std::env::var_os("SUNRISE_MOUNT_CONTROL_REFERENCE").unwrap();
        let rows: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(reference).unwrap()).unwrap();
        assert_eq!(rows.len(), 32, "missing vanilla control cases");
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!("no menu handoff"),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = ExecutionRole::Client;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "SteeringTest").unwrap();
        game.remote.as_mut().unwrap().joined = true;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 300.0, 0.5)).unwrap());
        let types = MobTypes::load(&root).unwrap();
        for row in rows {
            let kind = row["kind"].as_str().unwrap();
            let t = std::sync::Arc::new(types.types[&format!("minecraft:{kind}")].clone());
            let mut m = Mob::new(7, t, Vec3::new(0.5, 300.0, 0.5), 0.0);
            if row["saddled"].as_bool().unwrap() {
                m.set_saddle(Some(ItemStack::new("minecraft:saddle", 1)));
            }
            let stick = if kind == "pig" {
                "minecraft:carrot_on_a_stick"
            } else {
                "minecraft:warped_fungus_on_a_stick"
            };
            let wrong = if kind == "pig" {
                "minecraft:warped_fungus_on_a_stick"
            } else {
                "minecraft:carrot_on_a_stick"
            };
            let hand = row["hand"].as_str().unwrap();
            game.inventory[game.selected] = match hand {
                "main" => Some(ItemStack::new(stick, 1)),
                "wrong" => Some(ItemStack::new(wrong, 1)),
                _ => None,
            };
            game.offhand = (hand == "off").then(|| ItemStack::new(stick, 1));
            game.mobs.clear();
            game.mobs.push(m);
            game.riding = Some(7);
            game.remote.as_mut().unwrap().first_passenger = row["first"].as_bool().unwrap();
            let controlled = row["controlled"].as_bool().unwrap();
            assert_eq!(game.driven_mount().is_some(), controlled, "{row}");
            let packets = game.remote_vehicle_packets();
            assert_eq!(packets.len(), usize::from(controlled), "{row}");
            if controlled {
                assert!(matches!(packets[0], S::MoveVehicle(_)), "{row}");
                let before = game.mobs[0].position;
                assert!(game.remote_ride(Keys::default()));
                let m = &game.mobs[0];
                assert_eq!(
                    f64::from(m.ai.zza),
                    row["forward"].as_f64().unwrap(),
                    "{row}"
                );
                assert!(m.position.z > before.z, "{row}: mount did not move forward");
                match &game.remote_vehicle_packets()[0] {
                    S::MoveVehicle(p) => assert_eq!(
                        p.position,
                        [m.position.x, m.position.y, m.position.z],
                        "{row}"
                    ),
                    packet => panic!("wrong movement packet: {packet:?}"),
                }
            }
            // The server reads the same held-item requirement, then waits for
            // this controlling client's MoveVehicle rather than traveling twice.
            if row["first"].as_bool().unwrap() {
                assert!(game.tick_mount_riding(Keys::default()));
                game.mobs[0].rider.as_mut().unwrap().client = true;
            } else {
                game.mobs[0].rider = None;
            }
            assert_eq!(
                equine::moved_by_client(&game.mobs[0]),
                !row["server_simulates"].as_bool().unwrap(),
                "{row}"
            );
            if controlled {
                let mut server_mount = game.mobs[0].clone();
                let before = server_mount.position;
                let mut test_level = sunrise_client::health::TestLevel::new(0);
                *test_level.random() = (*game.world.random).clone();
                let mut events = Vec::new();
                server_mount.tick(
                    &game.mob_world_base(&[]),
                    &mut test_level.level(),
                    &mut events,
                );
                assert_eq!(
                    server_mount.position, before,
                    "server traveled twice: {row}"
                );
            }
        }
    }

    /// A mob of the prepared resources' type `kind` (None without them).
    fn mob(kind: &str) -> Option<Mob> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        let types = MobTypes::load(&root).ok()?;
        let t = std::sync::Arc::new(types.types.get(kind)?.clone());
        Some(Mob::new(7, t, Vec3::new(1.5, 64.0, -2.5), 0.0))
    }

    /// #2920: on a vanilla server the rider's client moves a saddled horse,
    /// donkey, mule or camel; a pig without its stick, a llama, an unsaddled horse or a second
    /// passenger's ride is the server's.
    #[test]
    fn the_rider_moves_a_saddled_horse_on_its_client() {
        for kind in ["horse", "donkey", "mule", "camel", "pig", "llama"] {
            let Some(mut m) = mob(&format!("minecraft:{kind}")) else {
                return;
            };
            assert!(!client_moves(&m, true, false), "{kind} without a saddle");
            m.set_saddle(Some(sunrise_client::items::ItemStack::new(
                "minecraft:saddle",
                1,
            )));
            let horse = matches!(kind, "horse" | "donkey" | "mule" | "camel");
            assert_eq!(client_moves(&m, true, false), horse, "{kind}");
            assert!(
                !client_moves(&m, false, false),
                "{kind} behind another rider"
            );
        }
    }

    /// #4107: on a vanilla server the rider's client moves a harnessed adult
    /// Happy Ghast (`getControllingPassenger`); an unharnessed one, a baby
    /// or a second passenger's ride is the server's.
    #[test]
    fn the_rider_moves_a_harnessed_ghast_on_its_client() {
        let Some(mut m) = mob("minecraft:happy_ghast") else {
            return;
        };
        assert!(!client_moves(&m, true, false), "ghast without a harness");
        m.extra
            .set_body(Some("minecraft:white_happy_ghast_harness".into()));
        assert!(client_moves(&m, true, false), "harnessed adult");
        assert!(!client_moves(&m, false, false), "behind another rider");
        m.extra.age = -1;
        assert!(!client_moves(&m, true, false), "baby ghast");
    }

    /// The mount ticks before its rider (`ClientLevel.tickNonPassenger`): it
    /// moves on the input the rider's previous tick applied (`zza`, the key
    /// ×0.98 through `LocalPlayer.modifyInput`), so a press reaches it a tick
    /// late and a release a tick late too, as the server's anticheats expect.
    #[test]
    #[ignore = "daily"]
    fn a_driven_horse_moves_on_the_riders_last_tick_input() {
        use super::super::{
            Audio, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
        };
        use sunrise_client::{items::ItemStack, motion::PlayerMotion};
        use sunrise_protocol::connection::Pipe;
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!("no menu handoff"),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = ExecutionRole::Client;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "RiderTest").unwrap();
        game.remote.as_mut().unwrap().joined = true;
        game.remote.as_mut().unwrap().first_passenger = true;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 300.0, 0.5)).unwrap());
        let types = MobTypes::load(&root).unwrap();
        let t = std::sync::Arc::new(types.types["minecraft:horse"].clone());
        let mut horse = Mob::new(7, t, Vec3::new(0.5, 300.0, 0.5), 0.0);
        horse.set_saddle(Some(ItemStack::new("minecraft:saddle", 1)));
        game.mobs.push(horse);
        game.riding = Some(7);
        let forward = Keys {
            forward: true,
            ..Keys::default()
        };
        let zza = |game: &mut Game, keys| {
            assert!(game.remote_ride(keys));
            game.mobs[0].ai.zza
        };
        assert_eq!(zza(&mut game, forward), 0.0, "the press waits a tick");
        assert_eq!(zza(&mut game, forward), 0.98, "modifyInput's 0.98");
        assert_eq!(
            zza(&mut game, Keys::default()),
            0.98,
            "the release waits a tick"
        );
        assert_eq!(zza(&mut game, Keys::default()), 0.0);
    }

    #[test]
    #[ignore = "daily"]
    fn a_sprinting_rider_boards_a_camel_at_sprint_speed() {
        crate::game::in_own_process(
            "game::remote_vehicles::tests::a_sprinting_rider_boards_a_camel_at_sprint_speed",
            || {
                use sunrise_client::{items::ItemStack, mobs::camel, motion::PlayerMotion};
                crate::game::native_entity_refs::tests::with_client_game(|game| {
                    let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                    game.connect_local(client, "CamelRider").unwrap();
                    let remote = game.remote.as_mut().unwrap();
                    remote.joined = true;
                    remote.entity_id = 42;
                    game.world.game_time = 1_000;
                    let at = Vec3::new(4.5, 80.0, 4.5);
                    let mut player = PlayerMotion::new(game.motion, at).unwrap();
                    player.entity_id = 42;
                    player.applied_input = [0.0, 0.98];
                    player.set_sprinting(true).unwrap();
                    game.player = Some(player);
                    let root = crate::game::native_entity_refs::tests::fixture_root();
                    let types = MobTypes::load(&root).unwrap();
                    let kind = std::sync::Arc::new(types.types["minecraft:camel"].clone());
                    let mut mount = Mob::new(7, kind, at, 0.0);
                    mount.set_saddle(Some(ItemStack::new("minecraft:saddle", 1)));
                    camel::finish_load(&mut mount, game.world.game_time);
                    assert!(!camel::refuse_to_move(&mount));
                    assert_eq!(mount.extra.anim_ticks, 0);
                    let expected = mount.movement_speed() as f32 + 0.1_f32;
                    game.mobs.push(mount);
                    game.apply_remote_passengers(7, &[42]);
                    assert!(game.mobs[0].rider.is_none(), "before the first driven tick");
                    assert!(game.remote_ride(Keys::default()));
                    assert_eq!(game.mobs[0].ai.speed.to_bits(), expected.to_bits());
                    assert_eq!(game.mobs[0].ai.zza, 0.98, "the last walking input");
                    assert!(game.player.as_ref().unwrap().sprinting);
                });
            },
        );
    }

    /// #2920: the client reports where it moved the mount, and a released
    /// jump with the bar's percentage (`sendRidingJump`).
    #[test]
    fn the_rider_sends_its_mounts_moves_and_jumps() {
        let Some(mut m) = mob("minecraft:horse") else {
            return;
        };
        (m.yaw, m.head_pitch, m.ground.on_ground) = (90.0, 10.0, true);
        let at = PosRot {
            position: [m.position.x, m.position.y, m.position.z],
            yaw: m.yaw,
            pitch: m.head_pitch,
        };
        match mount_move(&m, at) {
            S::MoveVehicle(p) => {
                assert_eq!(p.position, [1.5, 64.0, -2.5]);
                assert_eq!((p.yaw, p.pitch, p.on_ground), (90.0, 10.0, true));
            }
            other => panic!("{other:?}"),
        }
        match riding_jump(42, 0.75) {
            S::PlayerCommand(p) => assert_eq!((p.id, p.action, p.data), (42, 3, 75)),
            other => panic!("{other:?}"),
        }
    }
}

#[cfg(test)]
mod plugin_seat_tests {
    #[test]
    #[ignore = "daily"]
    fn controlling_client_copies_sneak_press_and_release_into_rider() {
        use sunrise_client::sprint::Keys;
        crate::game::mounts::sneak_dismount_tests::with_horse(|game, horse, _, _| {
            game.riding = Some(horse);
            for shift in [true, false] {
                let i = game.mobs.iter().position(|m| m.id == horse).unwrap();
                // Exercise client prediction itself, not the server input assignment.
                game.drive_mount(
                    i,
                    Keys {
                        shift,
                        ..Keys::default()
                    },
                    0.0,
                );
                let input = game.mobs[i]
                    .rider
                    .expect("prediction stores next tick's input");
                assert_eq!(input.sneak, shift);
                assert!(!input.client, "the controlling client simulates this mount");
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn local_rider_uses_attachments_and_follows_a_moving_seat_chain() {
        use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
        use sunrise_client::{mobs::Mob, motion::PlayerMotion, sprint::Keys};
        use sunrise_core::geometry::Vec3;
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        let mut stand =
            sunrise_client::entities::ArmorStand::new(42, Vec3::new(2.0, 64.0, 3.0), 0.0);
        stand.marker = true;
        game.armor_stands.push(stand);
        game.riding = Some(42);
        assert!(game.remote_ride(Keys::default()));
        let close = |actual: f64, expected: f64| {
            assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}")
        };
        // EntityAttachments: zero-height marker minus Avatar's 0.6 attachment.
        close(game.player.as_ref().unwrap().position.y, 63.4);
        game.armor_stands[0].marker = false;
        game.armor_stands[0].small = true;
        game.remote_ride(Keys::default());
        close(game.player.as_ref().unwrap().position.y, 64.3875);
        game.armor_stands[0].small = false;
        game.remote_ride(Keys::default());
        close(game.player.as_ref().unwrap().position.y, 65.375);
        let kind = game.res.player_type().clone().unwrap();
        game.mobs
            .push(Mob::new(43, kind, Vec3::new(5.0, 70.0, 6.0), 0.0));
        game.armor_stands[0].marker = true;
        game.armor_stands[0].vehicle = Some(43);
        for at in [Vec3::new(5.0, 70.0, 6.0), Vec3::new(12.0, 72.0, -4.0)] {
            game.mobs[0].position = at;
            // No manual position_riders call: local ride must resolve the chain itself.
            game.remote_ride(Keys::default());
            let p = game.player.as_ref().unwrap().position;
            close(p.x, at.x);
            close(p.z, at.z);
            close(p.y, at.y + 1.2);
        }
        // Direct riding and a scaled avatar share the same attachment calculation.
        game.riding = Some(43);
        game.mobs[0].extra.model_scale = 1.5;
        game.remote_ride(Keys::default());
        close(game.player.as_ref().unwrap().position.y, 74.1);
        game.mobs.clear();
        game.armor_stands.clear();
        assert!(!game.remote_ride(Keys::default()));
        assert_eq!(game.riding, None);
    }
}
