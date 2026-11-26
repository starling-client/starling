//! A mob riding a server player (`Entity.startRiding` with a player as the
//! vehicle), for `EntityRef::ride_player` only (#4456): Tinkers' Construct's
//! piggyback pack seats a mob on its wearer. `EntityRef::start_riding` keeps
//! refusing a player vehicle.
//!
//! Vanilla never seats anything on a player (`/ride` refuses with
//! `cant_ride_players`), so every path here is reached only after a mod's
//! call left a mob's `extra.mount` naming a player: the mount checks, the
//! seat (`seat_player_passenger`), the validity (`player_vehicle`) and the
//! vehicle player's own `SetPassengers` (`tell_vehicle_players`) all start by
//! finding a player behind that id, and find none in a vanilla game.
use super::{Game, commands::Target};
use mod_api::{EntityRef, RidePlayerRefusal};
use sunrise_client::lan::entity::Riding;
use sunrise_client::mobs::{Species, riders};
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_protocol::packets::play;

impl Game<'_> {
    /// The server player whose entity id is `id`, in the level in place: the
    /// player in place or a guest here. None on a client, or for any other id.
    pub(in crate::game) fn player_vehicle(&self, id: u32) -> Option<Target> {
        let entity = EntityRef::new(self.native_lifecycle.epoch()?, id);
        match self.native_entity_target(entity)? {
            Target::Player => self.player.as_ref().map(|_| Target::Player),
            Target::Guest(conn) => self
                .command_guest(conn)
                .filter(|g| g.dimension == self.dim.id())
                .map(|_| Target::Guest(conn)),
            _ => None,
        }
    }

    /// The entity id a player target has: the vehicle id its passengers keep.
    pub(in crate::game) fn player_vehicle_id(&self, player: Target) -> Option<u32> {
        match player {
            Target::Player => self
                .player
                .as_ref()
                .map(|_| self.own_entity_id() as u32)
                .filter(|&id| id != 0),
            Target::Guest(conn) => self
                .command_guest(conn)
                .filter(|g| g.dimension == self.dim.id())
                .map(|g| g.entity_id as u32),
            _ => None,
        }
    }

    /// What `id` rides, when `id` may be a player: the next link of a
    /// vehicle walk that can pass through a player carrying a mob.
    pub(in crate::game) fn vehicle_of_any(&self, id: u32) -> Option<u32> {
        match self.player_vehicle(id) {
            Some(player) => self.native_player_vehicle_id(player),
            None => self
                .native_vehicle_id(Target::Mob(id))
                .or_else(|| self.native_vehicle_id(Target::General(id))),
        }
    }

    fn native_player_vehicle_id(&self, player: Target) -> Option<u32> {
        match player {
            Target::Player => self.riding,
            Target::Guest(conn) => self.command_guest(conn)?.vehicle,
            _ => None,
        }
    }

    /// `Entity.startRiding(player, force)` for the mob `entity`, with each
    /// refusal named. The mount itself is `start_riding_target`'s, the path
    /// every other vehicle takes.
    pub(super) fn entity_ride_player(
        &mut self,
        entity: EntityRef,
        player: EntityRef,
        force: bool,
    ) -> Result<(), RidePlayerRefusal> {
        let rider = self
            .native_entity_target(entity)
            .ok_or(RidePlayerRefusal::NoEntity)?;
        let vehicle = self
            .native_entity_target(player)
            .ok_or(RidePlayerRefusal::NoEntity)?;
        if !matches!(vehicle, Target::Player | Target::Guest(_)) {
            return Err(RidePlayerRefusal::NotAPlayer);
        }
        let vehicle_id = self
            .player_vehicle_id(vehicle)
            .ok_or(RidePlayerRefusal::NoEntity)?;
        if entity.id() == player.id() {
            return Err(RidePlayerRefusal::SameEntity);
        }
        let id = match rider {
            Target::Mob(id) => id,
            Target::Player | Target::Guest(_) => return Err(RidePlayerRefusal::PlayerRider),
            _ => return Err(RidePlayerRefusal::UnsupportedRider),
        };
        let mob = self
            .mobs
            .iter()
            .find(|m| m.id == id && !m.removed)
            .ok_or(RidePlayerRefusal::NoEntity)?;
        if mob.extra.mount == Some(vehicle_id) {
            return Err(RidePlayerRefusal::AlreadyRiding);
        }
        let cannot_ride = mob.extra.boarding_cooldown > 0
            || matches!(mob.species(), Species::Warden | Species::Wither);
        if self.player_rides(vehicle, id) {
            return Err(RidePlayerRefusal::Cycle);
        }
        if !force {
            if !self.seated_passengers(vehicle_id).is_empty() {
                return Err(RidePlayerRefusal::Full);
            }
            if cannot_ride {
                return Err(RidePlayerRefusal::CannotRide);
            }
        }
        if self
            .try_start_riding_with(rider, vehicle, force, true)
            .is_ok()
        {
            Ok(())
        } else {
            Err(RidePlayerRefusal::Canceled)
        }
    }

    /// Gets every mob off the player whose entity id is `vehicle`, or off every player when
    /// None: no save holds a player vehicle, and a player leaving, dying or changing level
    /// carries nothing with it. Each mob gets off as `stop_riding` takes it off (`entity_mount`
    /// hears it); one a listener kept on is taken off anyway. Nothing to do, and nothing
    /// allocated, while no mob rides a player, which is every vanilla game.
    pub(in crate::game) fn eject_player_riders(&mut self, vehicle: Option<u32>) {
        let own = self.player.as_ref().map(|_| self.own_entity_id() as u32);
        let is_player = |game: &Self, v: u32| match vehicle {
            Some(only) => v == only,
            None => {
                own == Some(v)
                    || game
                        .lan
                        .as_ref()
                        .is_some_and(|host| host.hub.guests.iter().any(|g| g.entity_id as u32 == v))
            }
        };
        if !self
            .mobs
            .iter()
            .any(|m| !m.removed && m.extra.mount.is_some_and(|v| is_player(self, v)))
        {
            return;
        }
        let riders: Vec<(u32, u32)> = self
            .mobs
            .iter()
            .filter(|m| !m.removed)
            .filter_map(|m| {
                m.extra
                    .mount
                    .filter(|&v| is_player(self, v))
                    .map(|v| (m.id, v))
            })
            .collect();
        for (id, v) in riders {
            self.dismount_mob(id);
            if self
                .mob_object(id)
                .is_some_and(|m| m.extra.mount == Some(v))
            {
                self.dismount_mob_reentrant(id);
            }
        }
    }

    /// Whether `player` rides the mob `rider`, through any number of vehicles.
    fn player_rides(&self, player: Target, rider: u32) -> bool {
        let mut current = self.native_player_vehicle_id(player);
        for _ in 0..=self.riding_link_limit() {
            let Some(id) = current else {
                return false;
            };
            if id == rider {
                return true;
            }
            current = self.vehicle_of_any(id);
        }
        true
    }

    /// `Entity.positionRider` for a mob on a player: the top of the player's
    /// box for its pose (`Player`'s default passenger attachment, unrotated
    /// as it lies on the axis), less the mob's own vehicle attachment. False,
    /// changing nothing, when `vehicle` is no player of this level.
    pub(in crate::game) fn seat_player_passenger(&mut self, index: usize, vehicle: u32) -> bool {
        if self.player_vehicle(vehicle).is_none() {
            return false;
        }
        let Some(player) = self.entity(vehicle) else {
            return false;
        };
        let (Some([x, _, z]), Some([low, high])) = (
            self.entity_position(player),
            self.entity_bounding_box(player),
        ) else {
            return false;
        };
        let width = (high[0] - low[0]) as f32;
        let player_box = Aabb::new(
            Vec3::new(low[0], low[1], low[2]),
            Vec3::new(high[0], high[1], high[2]),
        );
        let m = &mut self.mobs[index];
        let offset = riders::vehicle_attachment(m, width);
        m.seated(vehicle, offset, player_box);
        m.previous = m.position;
        sunrise_client::mobs::living::teleports::set_pos(m, Vec3::new(x, high[1] - offset, z));
        m.velocity = Vec3::ZERO;
        m.fall_distance = 0.0;
        m.ground.on_ground = false;
        m.extra.passenger = true;
        m.ai.nav.stop();
        true
    }

    /// The vehicle players' own `SetPassengers`: no tracker sends a client its
    /// own entity, so a guest carrying a mob is told here when its passengers
    /// change, as the piggyback pack sends `ClientboundSetPassengersPacket` to
    /// its wearer. Nothing is sent while no guest carries anything and none
    /// was told it did, which is every vanilla tick.
    pub(in crate::game) fn tell_vehicle_players(&self, riding: &mut Riding) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let guests = &host.hub.guests;
        if riding.told_self.is_empty()
            && guests
                .iter()
                .all(|g| riding.passengers(g.entity_id).is_empty())
        {
            return;
        }
        for g in guests {
            let now = riding.passengers(g.entity_id);
            let told = riding
                .told_self
                .get(&g.entity_id)
                .map_or(&[][..], Vec::as_slice);
            if now == told {
                continue;
            }
            g.send(&play::Clientbound::SetPassengers(play::SetPassengers {
                vehicle: g.entity_id,
                passengers: now.to_vec(),
            }));
            let now = now.to_vec();
            if now.is_empty() {
                riding.told_self.remove(&g.entity_id);
            } else {
                riding.told_self.insert(g.entity_id, now);
            }
        }
        riding
            .told_self
            .retain(|id, _| guests.iter().any(|g| g.entity_id == *id));
    }
}

#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::{collect_entity_visit, fixture_resident_column};
    use mod_api::RidePlayerRefusal as Refusal;
    use mod_api::{EntityMountEvent, Flow, Hooks, ServerHost};
    use std::sync::{Arc, Mutex};
    use sunrise_client::lan::entity::Riding;
    use sunrise_client::mobs::{Mob, riders};
    use sunrise_core::geometry::Vec3;

    /// The `entity_mount` calls a test heard: rider, vehicle, mounting.
    type Heard = Arc<Mutex<Vec<(u32, u32, bool)>>>;

    /// Hears `entity_mount`, and cancels every dismount when `keep` is set.
    struct Mounts(Heard, bool);
    #[mod_api::__subscriptions]
    impl Hooks for Mounts {
        fn entity_mount(&self, _: &mut dyn ServerHost, event: &mut EntityMountEvent<'_>) -> Flow {
            self.0
                .lock()
                .unwrap()
                .push((event.entity, event.vehicle, event.mounting));
            if self.1 && !event.mounting {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    /// A pig seated on a guest, then `case` run on the game: afterwards the pig rides nothing
    /// and `entity_mount` heard it get off the guest, a listener that cancels it or not.
    fn ejected(case: impl Fn(&mut super::Game<'_>, u32)) {
        for keep in [false, true] {
            let heard = Heard::default();
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "mounts",
                Box::new(Mounts(heard.clone(), keep)),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                super::super::native_entity_refs::tests::with_game(|game| {
                    // `entity_mount` is heard on a server thread.
                    game.options.dedicated = Some(Vec::new());
                    let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                    game.mobs.push(Mob::new(80, pig, Vec3::ZERO, 0.0));
                    let player = super::super::native_player_death::tests::add_guest(game).entity();
                    let pig = game.entity(80).unwrap();
                    assert_eq!(game.entity_ride_player(pig, player, false), Ok(()));
                    heard.lock().unwrap().clear();
                    let conn = game.lan.as_ref().unwrap().hub.guests[0].conn;
                    case(game, conn);
                    // The unload save takes the pig away with its chunk; every other case keeps it.
                    let mount = |game: &super::Game<'_>| {
                        game.mobs
                            .iter()
                            .find(|m| m.id == 80 && !m.removed)
                            .map(|m| m.extra.mount)
                    };
                    assert!(
                        matches!(mount(game), None | Some(None)),
                        "ejected, a cancelling listener or not"
                    );
                    assert_eq!(game.seated_passengers(player.id()), Vec::new());
                    // It stays off: a later save and a tick of the world do not seat it again.
                    let _ = game.save_entities(false);
                    let _ = game.tick_creature_entities();
                    assert!(matches!(mount(game), None | Some(None)), "stays dismounted");
                    assert_eq!(game.seated_passengers(player.id()), Vec::new());
                    let heard = heard.lock().unwrap();
                    assert!(heard.contains(&(80, player.id(), false)), "{heard:?}");
                    assert!(!heard.iter().any(|&(_, _, mounting)| mounting), "{heard:?}");
                });
            });
        }
    }

    #[test]
    #[ignore = "daily"]
    fn a_save_ejects_a_mob_riding_a_player() {
        ejected(|game, _| {
            let _ = game.save_entities(false);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_level_save_ejects_a_mob_riding_a_player() {
        ejected(|game, _| {
            let _ = game.save_level(true);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_autosave_ejects_a_mob_riding_a_player() {
        ejected(|game, _| {
            let _ = game.save_game_with(false);
        });
    }

    /// `save-all` from the control channel: the save staged across event-loop turns,
    /// whose entity stage snapshots one chunk at a time.
    #[test]
    #[ignore = "daily"]
    fn a_staged_save_ejects_a_mob_riding_a_player() {
        ejected(|game, _| {
            // The entity stage runs once per loaded chunk, as the unload save's fixture has one.
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            let (reply, _answer) = std::sync::mpsc::channel();
            game.begin_control_save(true, reply).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            while game.pending_save.is_some() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the save did not finish"
                );
                let _ = game.poll_closing_save();
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_unload_save_ejects_a_mob_riding_a_player() {
        ejected(|game, _| {
            let keys = [(0, 0)].into_iter().collect();
            let (records, _) = game.unloading_entity_snapshots(&keys);
            game.save.save_entities_later(records).unwrap();
            game.remove_unloading_entities(&keys);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_logout_ejects_a_mob_riding_the_player() {
        ejected(|game, _| {
            let host = game.lan.as_mut().unwrap();
            host.hub.guests.update(0, |g| g.placed = true);
            // The hub takes a guest out of its list before it reports the leave.
            let guest = host.hub.guests.remove(0);
            let event = sunrise_client::lan::hub::HostEvent::Left(Box::new(guest));
            let host = game.lan.take().unwrap();
            game.lan = game.lan_process_events(host, [event], &[], [0.5, 64.0, 0.5]);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_death_ejects_a_mob_riding_the_player() {
        ejected(|game, conn| {
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.health = 0.0);
            game.guest_die(0, conn);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_dimension_change_ejects_a_mob_riding_the_player() {
        ejected(|game, conn| {
            game.start_server_trip(conn, super::super::Dim::Nether, None);
        });
    }

    /// The piggyback pack's cases on a guest: the refusals, the seat on top of
    /// the player as it moves, the guest's own `SetPassengers`, getting off,
    /// and the player leaving the level.
    #[test]
    #[ignore = "daily"]
    fn a_mob_rides_a_guest_sits_on_it_and_the_guest_is_told() {
        super::super::native_entity_refs::tests::with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            for id in [80, 81] {
                game.mobs.push(Mob::new(id, pig.clone(), Vec3::ZERO, 0.0));
            }
            let player = super::super::native_player_death::tests::add_guest(game).entity();
            let (first, second) = (game.entity(80).unwrap(), game.entity(81).unwrap());
            let mut riding = Riding::default();
            // Nothing rides a player: no one is told anything.
            game.lan_riding(&[], &mut riding);
            assert!(riding.told_self.is_empty());

            assert_eq!(
                game.entity_ride_player(player, first, false),
                Err(Refusal::NotAPlayer)
            );
            assert_eq!(
                game.entity_ride_player(player, player, false),
                Err(Refusal::SameEntity)
            );
            assert_eq!(game.entity_ride_player(first, player, false), Ok(()));
            assert_eq!(game.mobs[0].extra.mount, Some(player.id()));
            assert_eq!(game.entity_vehicle(first), Some(player));
            assert_eq!(
                collect_entity_visit(|visit| game.entity_visit_passengers(player, visit)),
                Ok(vec![first])
            );
            assert_eq!(
                game.entity_ride_player(first, player, false),
                Err(Refusal::AlreadyRiding)
            );
            assert_eq!(
                game.entity_ride_player(second, player, false),
                Err(Refusal::Full)
            );

            // The seat: the top of the player's box, less the pig's attachment.
            game.sync_riders();
            let [x, _, z] = game.entity_position(player).unwrap();
            let [low, high] = game.entity_bounding_box(player).unwrap();
            let m = &game.mobs[0];
            let offset = riders::vehicle_attachment(m, (high[0] - low[0]) as f32);
            assert_eq!(
                m.extra.mount,
                Some(player.id()),
                "a player vehicle stays valid"
            );
            assert_eq!(
                [m.position.x, m.position.y, m.position.z],
                [x, high[1] - offset, z]
            );
            assert!(m.extra.passenger);

            // The guest's own client is told, once.
            game.lan_riding(&[], &mut riding);
            let id = player.id() as i32;
            assert_eq!(riding.passengers(id), [80]);
            assert_eq!(riding.told_self.get(&id), Some(&vec![80]));

            // Forced past the one-passenger limit, as vanilla's force is.
            assert_eq!(game.entity_ride_player(second, player, true), Ok(()));
            assert!(game.entity_stop_riding(first));
            assert_eq!(game.mobs[0].extra.mount, None);
            assert!(game.entity_eject_passengers(player));
            game.lan_riding(&[], &mut riding);
            assert!(riding.told_self.is_empty(), "told it carries nothing now");

            // Off a vehicle, the boarding cooldown runs; ride_player forces it on. The plain
            // start_riding keeps refusing a player vehicle, forced or not.
            assert_eq!(
                game.entity_ride_player(first, player, false),
                Err(Refusal::CannotRide)
            );
            assert_eq!(
                game.entity_start_riding(first, player, true),
                Err(mod_api::StartRidingRefusal::PlayerVehicle)
            );
            assert_eq!(
                game.entity_start_riding(first, player, false),
                Err(mod_api::StartRidingRefusal::PlayerVehicle)
            );
            assert_eq!(game.entity_vehicle(first), None);
            assert_eq!(game.entity_ride_player(first, player, true), Ok(()));
            assert_eq!(game.entity_vehicle(first), Some(player));

            // The player riding the second pig: that pig can't ride the player.
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| guest.vehicle = Some(81));
            assert_eq!(
                game.entity_ride_player(second, player, true),
                Err(Refusal::Cycle)
            );
            game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                guest.vehicle = None;
                guest.dimension = "minecraft:the_nether".into();
            });
            // The player left the level: its passenger gets off.
            game.sync_riders();
            assert_eq!(game.mobs[0].extra.mount, None);
        });
    }
}
