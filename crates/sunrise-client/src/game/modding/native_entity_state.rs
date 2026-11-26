//! An entity's state flags for native server mods, through an `EntityRef`
//! (docs/modding/src/how-to/native-mods.md, "Entity and level handles", #7387): sneaking and
//! sprinting, water and lava, fire and air, silence, invisibility and pose.
//! Players and mobs have them all; the other kinds have the ones Sunrise
//! keeps for them (an armor stand's fire and invisibility, an item's fire),
//! and `None` for the rest, as for a stale handle. Secondary use also reads
//! the local client's live semantic sneak state; fire and underwater reads also
//! use the client's own cached state.
//! Ticks lived (`Entity.tickCount`) are read for players, mobs, items, orbs and minecarts.
use super::{Game, commands::Target, native_entity_base::BuiltinKind};
use mod_api::{EntityReadError, EntityRef};
use std::ops::ControlFlow;
use sunrise_client::join_seq::residency::table::EntityLookup;
use sunrise_client::mobs::{Mob, Species, sync};
use sunrise_client::motion::PlayerMotion;
use sunrise_client::pose::Pose;

/// `Pose`'s serialized names by id (`Pose.STANDING` is 0).
pub(super) const POSES: [&str; 18] = [
    "standing",
    "fall_flying",
    "sleeping",
    "swimming",
    "spin_attack",
    "crouching",
    "long_jumping",
    "dying",
    "croaking",
    "using_tongue",
    "sitting",
    "roaring",
    "sniffing",
    "emerging",
    "digging",
    "sliding",
    "shooting",
    "inhaling",
];

/// `Entity.getMaxAirSupply`: a dolphin's 4800, an axolotl's 6000, else 300.
pub(super) fn max_air(species: Option<Species>) -> i32 {
    match species {
        Some(Species::Dolphin) => 4800,
        Some(Species::Axolotl) => sunrise_client::mobs::axolotl::MAX_AIR,
        _ => 300,
    }
}

/// A player's pose as `Pose` names it.
fn player_pose(pose: Pose) -> &'static str {
    match pose {
        Pose::Standing => "standing",
        Pose::Crouching => "crouching",
        Pose::Swimming => "swimming",
        Pose::FallFlying => "fall_flying",
        Pose::SpinAttack => "spin_attack",
        Pose::Sleeping => "sleeping",
    }
}

/// What a handle names, as the flags read it.
enum Body<'a> {
    Player(&'a PlayerMotion),
    Mob(&'a Mob),
    Other(Target),
}

impl Game<'_> {
    /// The player's body or the mob a handle names, or its target.
    fn native_entity_body(&self, entity: EntityRef) -> Option<Body<'_>> {
        self.native_target_body(self.native_entity_target(entity)?)
    }

    fn native_target_body(&self, target: Target) -> Option<Body<'_>> {
        Some(match target {
            Target::Player => Body::Player(self.player.as_ref()?),
            Target::Guest(conn) => Body::Player(self.command_guest(conn)?.motion.as_deref()?),
            Target::Mob(id) => Body::Mob(self.mobs.iter().find(|m| m.id == id)?),
            t => Body::Other(t),
        })
    }
    /// `isSecondaryUseActive`: the local client's live semantic sneak state (not the
    /// physical key binding or last tick's crouching pose); the server's shift-key flag.
    /// False for non-player entities, None for a stale or unreachable handle.
    pub(super) fn entity_is_shift_key_down(&self, entity: EntityRef) -> Option<bool> {
        if !self.is_logical_server() {
            return Some(match self.native_client_entity_target(entity)? {
                Target::Player => self.held_mapping(sunrise_client::actions::Action::Sneak),
                _ => false,
            });
        }
        Some(match self.native_entity_body(entity)? {
            Body::Player(p) => p.shift_key_down,
            _ => false,
        })
    }
    /// `LivingEntity.getMainArm`: `Avatar`'s `DATA_PLAYER_MAIN_HAND` (a server sets it from the
    /// player's `ClientInformation`; a client's own player draws with its Main Hand option, and
    /// another player is a mob that keeps what the server sent), `Mob`'s left-handed flag and
    /// `ArmorStand`'s right arm. The dragon's left-handed draw is not kept, so it reads right.
    /// The mob read uses its resident row without a separate pass over the mob list.
    pub(super) fn entity_main_arm(&self, entity: EntityRef) -> Option<mod_api::MainHand> {
        let (target, location) = if self.is_logical_server() {
            self.native_entity_target_with_location(entity)?
        } else {
            let target = self.native_client_entity_target(entity)?;
            let location = match (target, self.server_resident_entity(entity.id())) {
                (Target::Mob(_), EntityLookup::Found { location, .. }) => Some(location),
                _ => None,
            };
            (target, location)
        };
        let left = match target {
            Target::Player if !self.is_logical_server() => self.settings.main_hand_left,
            Target::Player => return self.guest_main_arm(self.lan_acting?),
            Target::Guest(conn) => return self.guest_main_arm(conn),
            Target::Mob(_) => self.mobs[self.mobs.locate(location?.join)?].main_arm_left(),
            Target::Stand(_) | Target::Dragon(_) => false,
            _ => return None,
        };
        Some(if left {
            mod_api::MainHand::Left
        } else {
            mod_api::MainHand::Right
        })
    }
    /// The armor stand an entity handle names, on either side.
    fn native_armor_stand(
        &self,
        entity: EntityRef,
    ) -> Option<&sunrise_client::entities::ArmorStand> {
        let Target::Stand(id) = self.native_entity_read_target(entity)? else {
            return None;
        };
        self.armor_stands.iter().find(|s| s.id == id)
    }
    /// `ArmorStand.getHeadPose` ... `getRightLegPose`, in `POSE_KEYS` order.
    pub(super) fn entity_armor_stand_pose(
        &self,
        entity: EntityRef,
    ) -> Option<mod_api::ArmorStandPose> {
        let [head, body, left_arm, right_arm, left_leg, right_leg] =
            self.native_armor_stand(entity)?.pose;
        Some(mod_api::ArmorStandPose {
            head,
            body,
            left_arm,
            right_arm,
            left_leg,
            right_leg,
        })
    }
    /// `ArmorStand.DATA_CLIENT_FLAGS`.
    pub(super) fn entity_armor_stand_flags(
        &self,
        entity: EntityRef,
    ) -> Option<mod_api::ArmorStandFlags> {
        let stand = self.native_armor_stand(entity)?;
        Some(mod_api::ArmorStandFlags {
            small: stand.small,
            show_arms: stand.show_arms,
            show_base_plate: !stand.no_base_plate,
            marker: stand.marker,
        })
    }
    /// A server's player's main arm, from the Main Hand its client last reported.
    fn guest_main_arm(&self, conn: u32) -> Option<mod_api::MainHand> {
        Some(super::lan::main_hand(
            self.command_guest(conn)?.info.main_hand,
        ))
    }
    /// Whether the entity crouches (`Entity.isCrouching`: the crouching
    /// pose).
    pub(super) fn entity_is_crouching(&self, entity: EntityRef) -> Option<bool> {
        // LocalPlayer.isCrouching: the client player's own flag, set in aiStep before its input.
        if !self.is_logical_server() && self.native_entity_read_target(entity)? == Target::Player {
            return Some(self.player.as_ref()?.local_crouching);
        }
        Some(self.entity_pose(entity)? == "crouching")
    }
    /// Whether the entity sprints (`Entity.isSprinting`): a player, or a
    /// cat or ocelot dashing after its target (their shared sprinting flag);
    /// false for the other kinds.
    pub(super) fn entity_is_sprinting(&self, entity: EntityRef) -> Option<bool> {
        Some(
            match self.native_target_body(self.native_entity_read_target(entity)?)? {
                Body::Player(p) => p.sprinting,
                Body::Mob(m) => m.extra.sprinting,
                _ => false,
            },
        )
    }
    /// `Entity.setSprinting` for a player (the flag and the speed modifier) or a mob (the
    /// flag, sent to the clients). False for other kinds, a stale handle and a client.
    pub(super) fn entity_set_sprinting(&mut self, entity: EntityRef, sprinting: bool) -> bool {
        if !self.is_logical_server() {
            return false;
        }
        match self.native_entity_target(entity) {
            // The acting body: a guest's turn writes its flag back from the guest row, so the
            // row (`Guest::sprinting`, what metadata publishes) is set with it.
            Some(Target::Player) => {
                let Some(player) = self.player.as_mut() else {
                    return false;
                };
                if player.set_sprinting(sprinting).is_err() {
                    return false;
                }
                if let (Some(&conn), Some(lan)) = (self.lan_guest_swaps.last(), self.lan.as_mut()) {
                    lan.hub
                        .update_guest(conn, |guest| guest.sprinting = sprinting);
                }
                true
            }
            // A parked guest: its row and, with it, the body's flag and speed modifier.
            Some(Target::Guest(conn)) => self
                .lan
                .as_mut()
                .and_then(|lan| {
                    lan.hub.update_guest(conn, |guest| {
                        let body = guest.motion.as_mut()?;
                        body.set_sprinting(sprinting).ok()?;
                        guest.sprinting = sprinting;
                        Some(())
                    })
                })
                .flatten()
                .is_some(),
            Some(Target::Mob(id)) => self.mobs.iter_mut().find(|m| m.id == id).is_some_and(|m| {
                m.extra.sprinting = sprinting;
                m.needs_sync = true;
                true
            }),
            _ => false,
        }
    }
    /// Whether the entity touched water at its last fluid update
    /// (`Entity.isInWater`): players and mobs.
    pub(super) fn entity_is_in_water(&self, entity: EntityRef) -> Option<bool> {
        match self.native_target_body(self.native_entity_read_target(entity)?)? {
            Body::Player(p) => Some(p.fluids.was_touching_water),
            Body::Mob(m) => Some(m.fluids.in_water),
            Body::Other(Target::General(id)) => {
                Some(self.resident_entities.general.row(id)?.fluids.in_water)
            }
            Body::Other(_) => None,
        }
    }
    /// The player's swimming flag, distinct from its pose (which also covers crawling).
    pub(super) fn entity_is_swimming(&self, entity: EntityRef) -> Option<bool> {
        match self.native_target_body(self.native_entity_read_target(entity)?)? {
            Body::Player(p) => Some(p.swimming),
            _ => None,
        }
    }
    /// Whether the entity is in lava (`Entity.isInLava`): players and mobs.
    pub(super) fn entity_is_in_lava(&self, entity: EntityRef) -> Option<bool> {
        match self.native_target_body(self.native_entity_read_target(entity)?)? {
            Body::Player(p) => Some(p.is_in_lava()),
            Body::Mob(m) => Some(m.fluids.in_lava),
            Body::Other(Target::General(id)) => {
                Some(self.resident_entities.general.row(id)?.fluids.in_lava)
            }
            Body::Other(_) => None,
        }
    }
    /// Visit the cached fluid contacts of the player or mob, highest first.
    pub(super) fn entity_visit_fluid_heights(
        &self,
        entity: EntityRef,
        visit: &mut dyn FnMut(&str, f64) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, EntityReadError> {
        use sunrise_client::motion::{LAVA, WATER};
        let body = self.native_target_body(self.native_entity_visit_target(entity)?);
        let (water, lava) = match body {
            Some(Body::Player(p)) => (
                (p.fluid_height(WATER), p.fluids.was_touching_water),
                (p.fluid_height(LAVA), p.is_in_lava()),
            ),
            Some(Body::Mob(m)) => (
                (m.fluids.water, m.fluids.in_water),
                (m.fluids.lava, m.fluids.in_lava),
            ),
            Some(Body::Other(Target::General(id))) => {
                let fluids = self
                    .resident_entities
                    .general
                    .row(id)
                    .ok_or(EntityReadError::Unavailable)?
                    .fluids;
                (
                    (fluids.water, fluids.in_water),
                    (fluids.lava, fluids.in_lava),
                )
            }
            _ => return Err(EntityReadError::Unavailable),
        };
        Ok(visit_touched_fluids(water, lava, visit))
    }
    /// The fluid at the entity's eyes from its last fluid update
    /// (`IEntityExtension.getFirstEyeInFluidType`): water or lava, none in air. Players and mobs.
    /// On a client this reads its own cached fluid update, without sampling the world again.
    pub(super) fn entity_eye_fluid(&self, entity: EntityRef) -> Option<Option<&'static str>> {
        use sunrise_client::motion::{LAVA, WATER};
        Some(
            match self.native_target_body(self.native_entity_read_target(entity)?)? {
                Body::Player(p) => eye_fluid(p.is_eye_in_fluid(WATER), p.is_eye_in_fluid(LAVA)),
                Body::Mob(m) => eye_fluid(m.fluids.eye_in_water, m.fluids.eye_in_lava),
                Body::Other(Target::General(id)) => {
                    let fluids = self.resident_entities.general.row(id)?.fluids;
                    eye_fluid(fluids.eye_in_water, fluids.eye_in_lava)
                }
                Body::Other(_) => return None,
            },
        )
    }
    /// Entity.isUnderWater uses eye sample AND contact; LocalPlayer's override uses the sample alone.
    pub(super) fn entity_is_under_water(&self, entity: EntityRef) -> Option<bool> {
        match self.native_target_body(self.native_entity_read_target(entity)?)? {
            Body::Player(p) if !self.is_logical_server() => Some(p.is_under_water()),
            Body::Player(p) => Some(p.fluids.was_eye_in_water && p.fluids.was_touching_water),
            Body::Mob(m) => Some(m.was_eye_in_water && m.fluids.in_water),
            Body::Other(Target::General(id)) => {
                Some(self.resident_entities.general.row(id)?.under_water)
            }
            Body::Other(_) => None,
        }
    }

    /// `Entity.isOnFire`: remaining ticks or the client's synced shared flag 0, unless fire
    /// immune. Every kind, and on a client every tracked copy.
    pub(super) fn entity_is_on_fire(&self, entity: EntityRef) -> Option<bool> {
        let (ticks, immune, kind) = match self.native_entity_read_target(entity) {
            Some(target) => {
                let view = self.read_entity_base(target)?;
                (view.base.fire_ticks, view.fire_immune, view.kind)
            }
            None => (
                0,
                self.client_tracked_fire_immunity(entity)?,
                self.native_client_tracked(entity)
                    .and_then(|(id, _)| BuiltinKind::from_type_id(id)),
            ),
        };
        if kind.is_some_and(BuiltinKind::is_on_fire_overridden_false) {
            return Some(false);
        }
        let synced = !self.is_logical_server()
            && self
                .remote
                .as_ref()?
                .entity_on_fire(i32::try_from(entity.id()).ok()?)?;
        Some(!immune && (ticks > 0 || synced))
    }

    /// `Entity.fireImmune`: effective immunity on the side of the host, for every kind.
    pub(super) fn entity_fire_immune(&self, entity: EntityRef) -> Option<bool> {
        match self.native_entity_read_target(entity) {
            Some(target) => Some(self.read_entity_base(target)?.fire_immune),
            None => self.client_tracked_fire_immunity(entity),
        }
    }

    /// The immunity of a copy the client only tracks (no body of its own): a player's synced
    /// grant, a general row's declared `fire_immune`, a mob type's flag, and the one table of
    /// [`BuiltinKind`] the server answers from for every other kind.
    fn client_tracked_fire_immunity(&self, entity: EntityRef) -> Option<bool> {
        if let Some(row) = self.native_client_general(entity) {
            return Some(super::general::general_type_catalog().fire_immune(row.type_slot));
        }
        let (kind, _) = self.native_client_tracked(entity)?;
        if kind == "minecraft:player" {
            return self.client_tracked_player_fire_immunity(entity);
        }
        Some(
            BuiltinKind::from_type_id(kind).is_some_and(BuiltinKind::fire_immune)
                || self
                    .res
                    .mob_types()
                    .get(kind)
                    .is_some_and(|mob| mob.flags.fire_immune),
        )
    }

    fn client_tracked_player_fire_immunity(&self, entity: EntityRef) -> Option<bool> {
        let (kind, _) = self.native_client_tracked(entity)?;
        (kind == "minecraft:player").then_some(())?;
        self.remote
            .as_ref()?
            .tracked_fire_immunity(i32::try_from(entity.id()).ok()?)
    }

    pub(super) fn entity_set_fire_immunity(&mut self, entity: EntityRef, immune: bool) -> bool {
        self.native_entity_target(entity)
            .is_some_and(|target| self.set_fire_immunity_for(target, immune))
    }

    pub(super) fn entity_set_client_fire_immunity(
        &mut self,
        entity: EntityRef,
        immune: bool,
    ) -> bool {
        if let Some(target) = self.native_client_entity_target(entity) {
            return self.set_fire_immunity_for(target, immune);
        }
        if self.client_tracked_player_fire_immunity(entity).is_none() {
            return false;
        }
        self.remote
            .as_mut()
            .is_some_and(|remote| remote.set_tracked_fire_immunity(entity.id() as i32, immune))
    }

    fn set_fire_immunity_for(&mut self, target: Target, immune: bool) -> bool {
        let server = self.is_logical_server();
        let (grant, ticks) = match target {
            Target::Mob(id) => {
                let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id && !m.removed) else {
                    return false;
                };
                mob.needs_sync |= server && mob.mod_fire_immune != immune;
                (&mut mob.mod_fire_immune, &mut mob.fire_ticks)
            }
            Target::Stand(id) => {
                let Some(stand) = self.armor_stands.iter_mut().find(|s| s.id == id) else {
                    return false;
                };
                (&mut stand.mod_fire_immune, &mut stand.fire_ticks)
            }
            Target::Player | Target::Guest(_) => {
                let Some(player) = self.native_player_body_mut(target) else {
                    return false;
                };
                (&mut player.mod_fire_immune, &mut player.fire_ticks)
            }
            _ => return false,
        };
        *grant = immune;
        // Remote mobs and stands keep the server's fire bit in this field. Mask it
        // while rendering, but keep the copy so revocation needs no new packet.
        if immune && (server || matches!(target, Target::Player)) {
            *ticks = (*ticks).min(0);
        }
        true
    }

    /// The ticks the entity burns for (`Entity.getRemainingFireTicks`), for every kind.
    pub(super) fn entity_fire_ticks(&self, entity: EntityRef) -> Option<i32> {
        let target = self.native_entity_target(entity)?;
        Some(self.read_entity_base(target)?.base.fire_ticks)
    }
    /// `Entity.getPortalCooldown`: a player's own portal state, the falling
    /// block's, a pearl's End gateway state, or the level's for the other
    /// kinds its portal walk carries (zero when it keeps no state for one).
    pub(super) fn entity_portal_cooldown(&self, entity: EntityRef) -> Option<i32> {
        let id = match self.native_entity_target(entity)? {
            // The acting guest's state is in place for its turn (the entry of `guest_turn`).
            Target::Player => return Some(self.portals.player.cooldown),
            Target::Guest(conn) => {
                let play = self.lan.as_ref()?.play.get(&conn);
                return Some(play.map_or(0, |p| p.portal.cooldown));
            }
            Target::Falling(id) => {
                return Some(self.falling.iter().find(|f| f.id == id)?.portal.cooldown);
            }
            Target::Projectile(id) => {
                let p = self.creatures.projectiles.iter().find(|p| p.id == id)?;
                if matches!(
                    p.kind,
                    sunrise_client::projectiles::ProjectileKind::EnderPearl
                ) {
                    return Some(self.pearl_portal_cooldown(id));
                }
                if !super::nether_portal::is_arrow(p)
                    && !super::nether_portal::is_portal_projectile(p)
                {
                    return None;
                }
                id
            }
            Target::Mob(id)
            | Target::Item(id)
            | Target::Orb(id)
            | Target::Boat(id)
            | Target::Cart(id)
            | Target::Tnt(id)
            | Target::Rocket(id)
            | Target::Trident(id) => id,
            _ => return None,
        };
        Some(self.portals.entities.get(&id).map_or(0, |p| p.cooldown))
    }
    /// `Entity.setPortalCooldown`: writes the value `entity_portal_cooldown`
    /// reads, where that read keeps it — the level's portal state for the
    /// kinds its portal walk carries (mobs, items, experience orbs, boats,
    /// minecarts, primed TNT, fireworks, tridents, arrows and the thrown
    /// items and fireballs a chunk saves). Server-only; a negative value or a
    /// client-side write is refused. A positive value blocks portal entry on
    /// the next tick (and is refilled while the entity stands in a portal);
    /// 0 lets it enter at once. The write refuses a player, an acting guest,
    /// a falling block and an ender pearl: their cooldown lives in a store of
    /// its own (the player and guest portal states, the falling block's saved
    /// field, an End gateway traveller) that this write does not own, so it
    /// needs its own owner decision before it can reach them. False for a kind
    /// the read refuses or a stale handle.
    pub(super) fn entity_set_portal_cooldown(&mut self, entity: EntityRef, ticks: i32) -> bool {
        if ticks < 0 || !self.is_logical_server() {
            return false;
        }
        match self.native_entity_target(entity) {
            Some(Target::Projectile(id)) => {
                let Some(projectile) = self.creatures.projectiles.iter().find(|p| p.id == id)
                else {
                    return false;
                };
                if matches!(
                    projectile.kind,
                    sunrise_client::projectiles::ProjectileKind::EnderPearl
                ) || (!super::nether_portal::is_arrow(projectile)
                    && !super::nether_portal::is_portal_projectile(projectile))
                {
                    return false;
                }
                self.portals.entities.entry(id).or_default().cooldown = ticks;
                true
            }
            Some(Target::Mob(id))
            | Some(Target::Item(id))
            | Some(Target::Orb(id))
            | Some(Target::Boat(id))
            | Some(Target::Cart(id))
            | Some(Target::Tnt(id))
            | Some(Target::Rocket(id))
            | Some(Target::Trident(id)) => {
                self.portals.entities.entry(id).or_default().cooldown = ticks;
                true
            }
            _ => false,
        }
    }
    /// Sets the remaining fire ticks (`Entity.setRemainingFireTicks`), for every kind.
    pub(super) fn entity_set_remaining_fire_ticks(
        &mut self,
        entity: EntityRef,
        ticks: i32,
    ) -> bool {
        self.native_entity_target(entity)
            .is_some_and(|target| self.update_entity_base(target, |b| b.fire_ticks = ticks))
    }
    /// Clears positive fire ticks while retaining fire immunity (`Entity.clearFire`).
    pub(super) fn entity_clear_fire(&mut self, entity: EntityRef) -> bool {
        let Some(ticks) = self.entity_fire_ticks(entity) else {
            return false;
        };
        self.entity_set_remaining_fire_ticks(entity, ticks.min(0))
    }
    /// The entity's air (`Entity.getAirSupply`): players and mobs.
    pub(super) fn entity_air_supply(&self, entity: EntityRef) -> Option<i32> {
        match self.native_target_body(self.native_entity_read_target(entity)?)? {
            Body::Player(p) => Some(p.air),
            Body::Mob(m) => Some(m.air),
            Body::Other(_) => None,
        }
    }
    /// Sets the entity's air (`Entity.setAirSupply`).
    pub(super) fn entity_set_air_supply(&mut self, entity: EntityRef, air: i32) -> bool {
        match self.native_entity_target(entity) {
            Some(Target::Player) => match self.player.as_mut() {
                Some(player) => {
                    player.air = air;
                    true
                }
                None => false,
            },
            Some(Target::Guest(conn)) => match self
                .lan
                .as_mut()
                .and_then(|lan| lan.hub.guests.motion_mut(conn))
            {
                Some(player) => {
                    player.air = air;
                    true
                }
                None => false,
            },
            Some(Target::Mob(id)) => match self.mobs.iter_mut().find(|mob| mob.id == id) {
                Some(mob) => {
                    mob.air = air;
                    mob.needs_sync = true;
                    true
                }
                None => false,
            },
            _ => false,
        }
    }
    /// The most air the entity holds (`Entity.getMaxAirSupply`): 300, a
    /// dolphin's 4800, an axolotl's 6000.
    pub(super) fn entity_max_air_supply(&self, entity: EntityRef) -> Option<i32> {
        match self.native_target_body(self.native_entity_read_target(entity)?)? {
            Body::Mob(m) => Some(max_air(Some(m.species()))),
            _ => Some(max_air(None)),
        }
    }
    /// Whether the entity makes no sounds (`Entity.isSilent`, its `Silent` tag), for every kind.
    pub(super) fn entity_is_silent(&self, entity: EntityRef) -> Option<bool> {
        let target = self.native_entity_target(entity)?;
        Some(self.read_entity_base(target)?.base.silent)
    }
    /// Silences the entity or lets it make sounds again (`Entity.setSilent`), for every kind.
    pub(super) fn entity_set_silent(&mut self, entity: EntityRef, silent: bool) -> bool {
        self.native_entity_target(entity)
            .is_some_and(|target| self.update_entity_base(target, |b| b.silent = silent))
    }
    /// Whether the entity's model is hidden (`Entity.isInvisible`): the
    /// invisibility effect on a player or a mob, the flag of a mob, an armor
    /// stand or an item frame.
    pub(super) fn entity_is_invisible(&self, entity: EntityRef) -> Option<bool> {
        match self.native_entity_target(entity)? {
            Target::Player => Some(self.potions.effects.get("invisibility").is_some()),
            Target::Guest(conn) => Some(
                self.command_guest(conn)?
                    .effects
                    .get("invisibility")
                    .is_some(),
            ),
            Target::Mob(id) => {
                let m = self.mobs.iter().find(|m| m.id == id)?;
                Some(m.is_invisible())
            }
            Target::Stand(id) => Some(self.armor_stands.iter().find(|s| s.id == id)?.invisible),
            Target::Frame(id) => Some(self.item_frames.iter().find(|f| f.id == id)?.invisible),
            _ => None,
        }
    }
    /// Sets the entity's frozen ticks (`Entity.setTicksFrozen`).
    pub(super) fn entity_set_ticks_frozen(&mut self, entity: EntityRef, ticks: i32) -> bool {
        match self.native_entity_target(entity) {
            Some(Target::Player) => self.player.as_mut().is_some_and(|p| {
                p.ticks_frozen = ticks;
                true
            }),
            Some(Target::Guest(conn)) => self
                .lan
                .as_mut()
                .and_then(|lan| lan.hub.guests.motion_mut(conn))
                .is_some_and(|p| {
                    p.ticks_frozen = ticks;
                    true
                }),
            Some(Target::Mob(id)) => self.mobs.iter_mut().find(|m| m.id == id).is_some_and(|m| {
                m.ticks_frozen = ticks;
                m.needs_sync = true;
                true
            }),
            _ => false,
        }
    }
    /// The persistent glowing tag (`Entity.hasGlowingTag`), for every kind.
    pub(super) fn entity_glowing(&self, entity: EntityRef) -> Option<bool> {
        let target = self.native_entity_target(entity)?;
        Some(self.read_entity_base(target)?.base.glowing)
    }
    /// The no-gravity flag (`Entity.isNoGravity`), for every kind.
    pub(super) fn entity_no_gravity(&self, entity: EntityRef) -> Option<bool> {
        let target = self.native_entity_target(entity)?;
        Some(self.read_entity_base(target)?.base.no_gravity)
    }
    /// Returns the entity's frozen ticks (`Entity.getTicksFrozen`).
    pub(super) fn entity_ticks_frozen(&self, entity: EntityRef) -> Option<i32> {
        match self.native_entity_body(entity)? {
            Body::Player(p) => Some(p.ticks_frozen),
            Body::Mob(m) => Some(m.ticks_frozen),
            _ => None,
        }
    }
    /// Ticks the entity has lived (`Entity.tickCount`): on a server, living entities, item
    /// entities, experience orbs and minecarts; on a client, every entity of the level it
    /// shows ([`Self::client_entity_tick_count`]). Wraps as vanilla's `int` does.
    pub(super) fn entity_tick_count(&self, entity: EntityRef) -> Option<i32> {
        if !self.is_logical_server() {
            return self.client_entity_tick_count(entity);
        }
        Some(match self.native_entity_target(entity)? {
            Target::Player => self.player.as_ref()?.health.tick_count,
            Target::Guest(conn) => {
                let guest = self.command_guest(conn)?;
                guest
                    .motion
                    .as_deref()
                    .map(|motion| motion.health.tick_count)
                    .or(guest.carried_tick_count)
                    .unwrap_or(0)
            }
            Target::Mob(id) => self.mobs.iter().find(|m| m.id == id)?.age as i32,
            Target::Stand(id) => self.armor_stands.iter().find(|s| s.id == id)?.tick_count,
            Target::Dragon(id) => self.dragon.as_ref().filter(|d| d.id == id)?.ticks,
            Target::Item(id) => self.dropped.iter().find(|i| i.id == id)?.ticks as i32,
            Target::Orb(id) => self.orbs.iter().find(|o| o.id == id)?.ticks as i32,
            Target::Cart(id) => self.carts.iter().find(|c| c.id == id)?.tick_count,
            Target::General(id) => self.resident_entities.general.row(id)?.tick_count,
            _ => return None,
        })
    }
    /// `Entity.tickCount` of a client's own copy: the local player's, or that of an entity the
    /// server added to the level the client shows, whatever its kind, counted by the client's
    /// entity pass (`Entity.commonTick`) since the client added it.
    fn client_entity_tick_count(&self, entity: EntityRef) -> Option<i32> {
        let (remote, id) = self.native_client_entity_context(entity).ok()?;
        // The local player from its admission (Login, Respawn), before the server's first
        // teleport gives it a body.
        if id == remote.entity_id {
            return remote.has_logical_player.then_some(remote.own_tick_count);
        }
        remote
            .entities
            .get(&id)
            .map(|tracked| tracked.tick_count)
            .or_else(|| remote.general.rows.get(&id).map(|row| row.age as i32))
    }
    /// Sets the persistent glowing tag (`Entity.setGlowingTag`), for every kind.
    pub(super) fn entity_set_glowing(&mut self, entity: EntityRef, glowing: bool) -> bool {
        self.native_entity_target(entity)
            .is_some_and(|target| self.update_entity_base(target, |b| b.glowing = glowing))
    }
    /// Sets the no-gravity flag (`Entity.setNoGravity`), for every kind.
    pub(super) fn entity_set_no_gravity(&mut self, entity: EntityRef, no_gravity: bool) -> bool {
        self.native_entity_target(entity)
            .is_some_and(|target| self.update_entity_base(target, |b| b.no_gravity = no_gravity))
    }
    /// Sets the entity's shared invisibility flag (`Entity.setInvisible`) on a mob, an armor
    /// stand or an item frame. Returns false for players, guests and other kinds.
    pub(super) fn entity_set_invisible(&mut self, entity: EntityRef, invisible: bool) -> bool {
        match self.native_entity_target(entity) {
            Some(Target::Mob(id)) => self.mobs.iter_mut().find(|m| m.id == id).is_some_and(|m| {
                m.invisible = invisible;
                m.needs_sync = true;
                true
            }),
            Some(Target::Stand(id)) => self
                .armor_stands
                .iter_mut()
                .find(|s| s.id == id)
                .is_some_and(|s| {
                    s.invisible = invisible;
                    true
                }),
            // `ItemFrame` saves the flag with its chunk.
            Some(Target::Frame(id)) => {
                let Some(frame) = self.item_frames.iter_mut().find(|f| f.id == id) else {
                    return false;
                };
                frame.invisible = invisible;
                let pos = frame.pos;
                self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
                true
            }
            _ => false,
        }
    }
    /// The entity's pose (`Entity.getPose`) as `Pose` names it: `standing`,
    /// `crouching`, `swimming`, `sleeping`, a warden's `roaring`... Kinds
    /// with no pose stand.
    pub(super) fn entity_pose(&self, entity: EntityRef) -> Option<&'static str> {
        Some(
            match self.native_target_body(self.native_entity_read_target(entity)?)? {
                Body::Player(p) => player_pose(p.pose),
                // A client keeps a server's mob as its synched DATA_POSE.
                Body::Mob(m) if !self.is_logical_server() => {
                    POSES[super::entity_sizes::synced_pose(m) as usize]
                }
                Body::Mob(m) if &*m.kind.id == "minecraft:mannequin" => {
                    player_pose(m.extra.avatar_pose)
                }
                Body::Mob(m) => POSES.get(usize::from(sync::pose_id(m))).copied()?,
                Body::Other(_) => POSES[0],
            },
        )
    }
    /// Each mob's own sitting test (`Mob::sitting`), on either side.
    pub(super) fn entity_is_sitting(&self, entity: EntityRef) -> Option<bool> {
        match self.native_target_body(self.native_entity_read_target(entity)?)? {
            Body::Mob(m) => m.sitting(),
            Body::Player(_) | Body::Other(_) => None,
        }
    }
}

const WATER_ID: &str = "minecraft:water";
const LAVA_ID: &str = "minecraft:lava";

/// Visit cached contacts in descending total order, keeping water before lava on a tie.
fn visit_touched_fluids(
    water: (f64, bool),
    lava: (f64, bool),
    visit: &mut dyn FnMut(&str, f64) -> ControlFlow<()>,
) -> ControlFlow<()> {
    let mut fluids = [(WATER_ID, water), (LAVA_ID, lava)];
    if lava.0.total_cmp(&water.0).is_gt() {
        fluids.swap(0, 1);
    }
    fluids
        .into_iter()
        .filter(|(_, (_, touched))| *touched)
        .try_for_each(|(name, (height, _))| visit(name, height))
}

/// The first fluid at the eyes: water, then lava, as the trackers are kept.
fn eye_fluid(water: bool, lava: bool) -> Option<&'static str> {
    if water {
        Some(WATER_ID)
    } else {
        lava.then_some(LAVA_ID)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reads' order: highest first, water before lava in a tie, an untouched fluid left
    /// out even with a height, and nothing for an entity in neither.
    #[test]
    fn fluid_reads_list_the_touched_fluids_highest_first() {
        let (water, lava) = ("minecraft:water", "minecraft:lava");
        let touched_fluids = |water, lava| {
            let mut fluids = Vec::new();
            let result = visit_touched_fluids(water, lava, &mut |id, depth| {
                fluids.push((id.to_owned(), depth));
                ControlFlow::Continue(())
            });
            assert_eq!(result, ControlFlow::Continue(()));
            fluids
        };
        assert_eq!(
            touched_fluids((0.5, true), (1.25, true)),
            [(lava.to_owned(), 1.25), (water.to_owned(), 0.5)]
        );
        assert_eq!(
            touched_fluids((0.75, true), (0.75, true)),
            [(water.to_owned(), 0.75), (lava.to_owned(), 0.75)]
        );
        assert_eq!(
            touched_fluids((0.4, true), (0.2, false)),
            [(water.to_owned(), 0.4)]
        );
        assert!(touched_fluids((0.0, false), (0.0, false)).is_empty());

        // A break visits only the first contact. Total order treats +0.0 as higher than -0.0.
        let mut first = Vec::new();
        assert_eq!(
            visit_touched_fluids((-0.0, true), (0.0, true), &mut |id, depth| {
                first.push((id.to_owned(), depth.to_bits()));
                ControlFlow::Break(())
            }),
            ControlFlow::Break(())
        );
        assert_eq!(first, [(lava.to_owned(), 0.0_f64.to_bits())]);
        assert_eq!(eye_fluid(true, true), Some(water));
        assert_eq!(eye_fluid(false, true), Some(lava));
        assert_eq!(eye_fluid(false, false), None);
    }

    #[test]
    fn poses_keep_vanillas_ids() {
        assert_eq!(POSES[usize::from(sync::SITTING)], "sitting");
        assert_eq!(POSES[usize::from(sync::INHALING)], "inhaling");
        for pose in [
            Pose::Standing,
            Pose::Crouching,
            Pose::Swimming,
            Pose::FallFlying,
            Pose::SpinAttack,
            Pose::Sleeping,
        ] {
            let id = POSES.iter().position(|p| *p == player_pose(pose));
            assert_eq!(id.map(|i| Pose::from_id(i as i32)), Some(pose));
        }
    }

    #[test]
    #[ignore = "daily"]
    fn mob_setters_update_stored_state() {
        use super::super::native_entity_refs::tests::with_resident_game;
        use sunrise_core::geometry::Vec3;
        with_resident_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(42, kind, Vec3::ZERO, 0.0));
            let entity = game.entity(42).unwrap();
            assert!(entity.set_air_supply(game, -7));
            assert_eq!(game.mobs[0].air, -7);
            assert!(entity.set_remaining_fire_ticks(game, 37));
            assert_eq!(game.mobs[0].fire_ticks, 37);
            assert!(entity.clear_fire(game));
            assert_eq!(game.mobs[0].fire_ticks, 0);
            assert!(game.entity_set_remaining_fire_ticks(entity, -12));
            assert!(game.entity_clear_fire(entity));
            assert_eq!(game.mobs[0].fire_ticks, -12);
            assert!(entity.set_ticks_frozen(game, 31));
            assert_eq!(game.mobs[0].ticks_frozen, 31);
            assert!(entity.set_invisible(game, true));
            assert!(game.mobs[0].invisible);
            game.mobs[0].add_effect("minecraft:invisibility", 100, 0);
            assert!(entity.set_invisible(game, false));
            assert_eq!(entity.is_invisible(game), Some(false));
            assert!(entity.set_glowing(game, true));
            assert!(game.mobs[0].glowing);
            assert!(entity.set_no_gravity(game, true));
            assert!(game.mobs[0].no_gravity);
            assert!(game.mobs[0].needs_sync);
        });
    }

    /// `Entity.setNoGravity` and `Entity.isNoGravity` reach a live dropped item, not only a
    /// mob; a kind that is neither still refuses both, as does a stale handle (#5011, #3938
    /// item 47).
    #[test]
    #[ignore = "daily"]
    fn no_gravity_reads_and_writes_a_live_dropped_item() {
        use super::super::native_entity_refs::tests::with_resident_game;
        use mod_api::WorldEpoch;
        use sunrise_client::entities::ArmorStand;
        use sunrise_client::items::ItemStack;
        use sunrise_core::geometry::Vec3;

        with_resident_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.spawn_item(
                ItemStack {
                    item: "minecraft:stone".into(),
                    count: 1,
                    damage: 0,
                    extra: None,
                },
                [0.0; 3],
                [0.0; 3],
                0,
            );
            let item = game.entity(game.dropped[0].id).unwrap();
            assert_eq!(item.no_gravity(game), Some(false));

            assert!(item.set_no_gravity(game, true));
            assert!(game.dropped[0].no_gravity);
            assert!(game.dropped[0].needs_sync);
            assert_eq!(item.no_gravity(game), Some(true));

            // Every kind keeps the flag (#5082): a stand too.
            game.armor_stands.push(ArmorStand::new(43, Vec3::ZERO, 0.0));
            let stand = game.entity(43).unwrap();
            assert_eq!(stand.no_gravity(game), Some(false));
            assert!(stand.set_no_gravity(game, true));
            assert_eq!(stand.no_gravity(game), Some(true));

            let stale = EntityRef::new(WorldEpoch::new(item.epoch().get() + 1), item.id());
            assert_eq!(stale.no_gravity(game), None);
            assert!(!stale.set_no_gravity(game, true));
        });
    }

    fn with_player(test: impl FnOnce(&mut Game<'_>, EntityRef)) {
        use super::super::native_entity_refs::tests::with_resident_game;
        use sunrise_core::geometry::Vec3;
        with_resident_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.entity(20).unwrap();
            test(game, player);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_granted_player_does_not_burn_or_take_damage_in_lava() {
        use std::sync::Arc;
        use sunrise_client::{chunk::Chunk, motion::MotionWorld};
        use sunrise_core::geometry::Vec3;

        with_player(|game, player| {
            // `EntityFluidInteraction.update` needs the chunks around the box loaded as well.
            for chunk in [(-1, -1), (-1, 0), (0, -1), (0, 0)] {
                game.world
                    .chunks
                    .insert(chunk, Arc::new(Chunk::empty(chunk.0, chunk.1)));
            }
            let lava = game
                .pack
                .assets
                .registry
                .default_state("minecraft:lava")
                .unwrap();
            assert!(game.world.set_stored(0, 64, 0, lava));
            game.player.as_mut().unwrap().position = Vec3::new(0.5, 64.0, 0.5);
            let tick_lava = |game: &mut Game<'_>| {
                let side = game_side!(game);
                let mut world = MotionWorld {
                    side: &side,
                    entities: &[],
                    player: game.player.as_mut().unwrap(),
                    scene: &mut game.world,
                    assets: &game.pack.assets,
                    collision_binding: game.collision,
                    resources: game.motion,
                    health: &game.health_resources,
                    tag_items: None,
                    loop_context: &game.loop_context,
                    damage_hooks: &game.damage_hooks,
                    block_registries: None,
                };
                world.fluid_base_tick().unwrap();
                assert!(world.player.is_in_lava());
                world.tick_environment_damage().unwrap();
            };
            let health = |game: &Game<'_>| game.player.as_ref().unwrap().health.health();
            let before = health(game);
            assert_eq!(player.fire_immune(game), Some(false));
            assert!(player.set_fire_immunity(game, true));
            assert_eq!(player.fire_immune(game), Some(true));
            game.hurt_player_by("minecraft:on_fire", 2.0);
            assert_eq!(health(game), before, "direct fire damage is also refused");
            tick_lava(game);
            assert_eq!(player.is_on_fire(game), Some(false));
            assert_eq!(health(game), before);

            assert!(player.set_fire_immunity(game, false));
            tick_lava(game);
            assert_eq!(player.is_on_fire(game), Some(true));
            assert!(health(game) < before);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn fire_ticks_reads_every_kind_and_refuses_a_stale_handle() {
        use mod_api::WorldEpoch;
        use sunrise_client::entities::ArmorStand;
        use sunrise_client::items::ItemStack;
        use sunrise_core::geometry::Vec3;

        with_player(|game, player| {
            game.player.as_mut().unwrap().fire_ticks = 0;
            assert_eq!(player.fire_ticks(game), Some(0));
            game.player.as_mut().unwrap().fire_ticks = -12;
            assert_eq!(player.fire_ticks(game), Some(-12));

            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(42, kind, Vec3::ZERO, 0.0));
            game.mobs[0].fire_ticks = 37;
            let mob = game.entity(42).unwrap();
            assert_eq!(mob.fire_ticks(game), Some(37));

            game.armor_stands.push(ArmorStand::new(43, Vec3::ZERO, 0.0));
            game.armor_stands[0].fire_ticks = 23;
            let stand = game.entity(43).unwrap();
            assert_eq!(stand.fire_ticks(game), Some(23));

            game.spawn_item(
                ItemStack {
                    item: "minecraft:stone".into(),
                    count: 1,
                    damage: 0,
                    extra: None,
                },
                [0.0; 3],
                [0.0; 3],
                0,
            );
            game.dropped[0].fire_ticks = 19;
            let item = game.entity(game.dropped[0].id).unwrap();
            assert_eq!(item.fire_ticks(game), Some(19));

            let mut snowball = sunrise_client::projectiles::Projectile::new(
                sunrise_client::projectiles::ProjectileKind::Snowball,
                Vec3::ZERO,
                None,
            );
            snowball.id = 45;
            snowball.fire_ticks = 159;
            game.creatures.projectiles.push(snowball);
            let proj = game.entity(45).unwrap();
            assert_eq!(proj.fire_ticks(game), Some(159));
            assert_eq!(proj.is_on_fire(game), Some(true));
            assert!(proj.set_remaining_fire_ticks(game, 40));
            assert_eq!(proj.fire_ticks(game), Some(40));

            let mut arrow = sunrise_client::projectiles::Projectile::new(
                sunrise_client::projectiles::ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                Vec3::ZERO,
                None,
            );
            arrow.id = 46;
            arrow.arrow.as_mut().unwrap().fire_ticks = 80;
            game.creatures.projectiles.push(arrow);
            let arrow_ref = game.entity(46).unwrap();
            assert_eq!(arrow_ref.fire_ticks(game), Some(80));
            assert_eq!(arrow_ref.is_on_fire(game), Some(true));
            assert!(arrow_ref.clear_fire(game));
            assert_eq!(arrow_ref.fire_ticks(game), Some(0));

            game.boats.push(super::super::boats::Boat::new(
                44,
                "oak".into(),
                Vec3::ZERO,
                0.0,
            ));
            let boat = game.entity(44).unwrap();
            // A boat keeps the base state every `Entity` has (#5082).
            assert_eq!(boat.fire_ticks(game), Some(0));
            assert!(boat.set_remaining_fire_ticks(game, 5));
            assert_eq!(boat.fire_ticks(game), Some(5));
            let stale = EntityRef::new(WorldEpoch::new(mob.epoch().get() + 1), mob.id());
            assert_eq!(stale.fire_ticks(game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn tick_count_reads_supported_entities_and_refuses_others() {
        use super::super::minecarts::{CartKind, Minecart};
        use super::super::native_entity_refs::tests::bind_local_player;
        use super::super::native_guest_turn_tests::with_resident_guests;
        use mod_api::WorldEpoch;
        use sunrise_client::entities::ExperienceOrb;
        use sunrise_client::items::ItemStack;
        use sunrise_client::motion::PlayerMotion;
        use sunrise_core::geometry::Vec3;

        with_resident_guests(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            bind_local_player(game, 20, 123);
            let player = game.entity(20).unwrap();
            game.player.as_mut().unwrap().health.tick_count = i32::MAX;
            game.player.as_mut().unwrap().health.advance_entity_clock();
            assert_eq!(player.tick_count(game), Some(i32::MIN));

            let acting_guest = game.entity(12).unwrap();
            let parked_guest = game.entity(13).unwrap();
            assert_eq!(parked_guest.tick_count(game), Some(0));
            game.as_guest(13, |game| {
                game.player.as_mut().unwrap().health.tick_count = 314;
            })
            .unwrap();
            assert_eq!(parked_guest.tick_count(game), Some(314));

            game.as_guest(12, |game| {
                game.player.as_mut().unwrap().health.tick_count = 2718;
                assert_eq!(acting_guest.tick_count(game), Some(2718));
            })
            .unwrap();

            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(42, kind, Vec3::ZERO, 0.0));
            game.mobs[0].age = 2401;
            game.mobs[0].extra.age = -24000;
            let mob = game.entity(42).unwrap();
            assert_eq!(mob.tick_count(game), Some(2401));

            game.spawn_item(
                ItemStack {
                    item: "minecraft:stone".into(),
                    count: 1,
                    damage: 0,
                    extra: None,
                },
                [0.0; 3],
                [0.0; 3],
                0,
            );
            game.dropped[0].ticks = 77;
            game.dropped[0].age = 5000;
            let item = game.entity(game.dropped[0].id).unwrap();
            assert_eq!(item.tick_count(game), Some(77));

            game.orbs.push(ExperienceOrb {
                size_answer: None,
                attachment_lifetime: Default::default(),
                fall_distance: 0.0,
                rotation: [0.0, 0.0],
                join: Default::default(),
                id: 43,
                position: Vec3::ZERO,
                previous: Vec3::ZERO,
                velocity: Vec3::ZERO,
                value: 7,
                age: 600,
                ground: sunrise_world::ground_support::GroundState::from_ground_flag(true),
                count: 1,
                fire_ticks: 0,
                ticks: 21,
                health: 5,
                air: 300,
                no_gravity: true,
                following: None,
            });
            let orb = game.entity(43).unwrap();
            assert_eq!(orb.tick_count(game), Some(21));

            game.carts
                .push(Minecart::new(44, CartKind::Rideable, Vec3::ZERO));
            game.carts.update(0, |c| c.tick_count = 5);
            let cart = game.entity(44).unwrap();
            assert_eq!(cart.tick_count(game), Some(5));

            game.boats.push(super::super::boats::Boat::new(
                45,
                "oak".into(),
                Vec3::ZERO,
                0.0,
            ));
            let boat = game.entity(45).unwrap();
            assert_eq!(boat.tick_count(game), None);
            let stale = EntityRef::new(WorldEpoch::new(mob.epoch().get() + 1), mob.id());
            assert_eq!(stale.tick_count(game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn portal_cooldown_reads_simulated_kinds_and_refuses_others() {
        use super::super::native_guest_turn_tests::with_resident_guests;
        use super::super::{
            Dim, FallingBlock, boats::Boat, fireworks::Rocket, minecarts::CartKind,
            minecarts::Minecart, trident::ThrownTrident,
        };
        use mod_api::WorldEpoch;
        use sunrise_client::entities::{ArmorStand, ExperienceOrb};
        use sunrise_client::items::ItemStack;
        use sunrise_client::projectiles::{Projectile, ProjectileKind};
        use sunrise_core::{geometry::Vec3, position::BlockPos};

        with_resident_guests(|game| {
            // Players keep their own state: in place during their turn, beside the hub otherwise.
            let host = game.lan.as_mut().unwrap();
            host.play.entry(12).or_default().portal.cooldown = 7;
            host.play.entry(13).or_default().portal.cooldown = 4;
            game.portals.player.cooldown = 99;
            let (acting, other) = (game.entity(12).unwrap(), game.entity(13).unwrap());
            assert_eq!(acting.portal_cooldown(game), Some(7));
            game.as_guest(12, |game| {
                assert_eq!(acting.portal_cooldown(game), Some(7));
                assert_eq!(other.portal_cooldown(game), Some(4));
            })
            .unwrap();
            assert_eq!(acting.portal_cooldown(game), Some(7));
            assert_eq!(other.portal_cooldown(game), Some(4));

            // A mob counts down from `setPortalCooldown`'s 300 as the level ticks.
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            game.mobs.push(Mob::new(40, kind, Vec3::ZERO, 0.0));
            let mob = game.entity(40).unwrap();
            assert_eq!(mob.portal_cooldown(game), Some(0));
            game.set_portal_cooldown(40);
            assert_eq!(mob.portal_cooldown(game), Some(300));
            game.handle_entity_portals();
            assert_eq!(mob.portal_cooldown(game), Some(299));

            // The other kinds the level carries through portals.
            game.spawn_item(
                ItemStack {
                    item: "minecraft:stone".into(),
                    count: 1,
                    damage: 0,
                    extra: None,
                },
                [0.0; 3],
                [0.0; 3],
                0,
            );
            let item = game.dropped[0].id;
            game.orbs.push(ExperienceOrb {
                size_answer: None,
                attachment_lifetime: Default::default(),
                fall_distance: 0.0,
                rotation: [0.0, 0.0],
                join: Default::default(),
                no_gravity: false,
                id: 41,
                position: Vec3::ZERO,
                previous: Vec3::ZERO,
                velocity: Vec3::ZERO,
                value: 1,
                age: 0,
                ground: Default::default(),
                count: 1,
                fire_ticks: 0,
                ticks: 0,
                health: 5,
                air: 300,
                following: None,
            });
            game.boats
                .push(Boat::new(42, "oak".into(), Vec3::ZERO, 0.0));
            game.carts
                .push(Minecart::new(43, CartKind::Rideable, Vec3::ZERO));
            game.add_primed_tnt(BlockPos::new(0, 64, 0), 0.0, 80, None, None);
            let tnt = game.tnts[0].id;
            let arrow = Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                Vec3::ZERO,
                None,
            );
            game.add_projectile(arrow);
            let arrow = game.creatures.projectiles[0].id;
            game.add_projectile(Projectile::new(ProjectileKind::Snowball, Vec3::ZERO, None));
            let snowball = game.creatures.projectiles[1].id;
            game.rockets
                .push(Rocket::remote(44, Vec3::ZERO, Vec3::ZERO));
            game.tridents
                .push(ThrownTrident::remote(45, Vec3::ZERO, 0.0, 0.0));
            // Entity defaults to 300; VehicleEntity overrides it to 10 and
            // Projectile to 2 (`getDimensionChangingDelay` in vanilla 26.3).
            for (id, delay) in [
                (item, 300),
                (41, 300),
                (42, 10),
                (43, 10),
                (tnt, 300),
                (arrow, 2),
                (snowball, 2),
                (44, 2),
                (45, 2),
            ] {
                let entity = game.entity(id).unwrap();
                assert_eq!(entity.portal_cooldown(game), Some(0), "entity {id}");
                game.set_portal_cooldown(id);
                assert_eq!(entity.portal_cooldown(game), Some(delay), "entity {id}");
            }

            // A falling block keeps its own, as its save does.
            let state = game
                .pack
                .assets
                .registry
                .default_state("minecraft:sand")
                .unwrap();
            game.next_entity += 1;
            let falling = game.next_entity;
            game.falling.push(FallingBlock {
                size_answer: None,
                ground: Default::default(),
                fire_ticks: 0,
                sync_velocity: false,
                attachment_lifetime: Default::default(),
                rotation: [0.0, 0.0],
                join: Default::default(),
                id: falling,
                state,
                blocks_building: true,
                start_pos: BlockPos::default(),
                position: Vec3::ZERO,
                previous: Vec3::ZERO,
                velocity: Vec3::ZERO,
                age: 0,
                no_gravity: false,
                portal: sunrise_client::nether_portal::PortalState {
                    cooldown: 25,
                    process: None,
                },
                drop_item: false,
                fall_distance: 0.0,
                stuck: Vec3::ZERO,
                damage: sunrise_client::falling_block::Damage::falling("minecraft:sand"),
                saved: None,
            });
            let falling = game.entity(falling).unwrap();
            assert_eq!(falling.portal_cooldown(game), Some(25));

            // No portal cooldown: an armor stand, a wind charge, a stale handle, a parked level.
            game.armor_stands.push(ArmorStand::new(46, Vec3::ZERO, 0.0));
            let stand = game.entity(46).unwrap();
            assert_eq!(stand.portal_cooldown(game), None);
            game.add_projectile(Projectile::new(
                ProjectileKind::WindCharge,
                Vec3::ZERO,
                None,
            ));
            let charge = game.entity(game.creatures.projectiles[2].id).unwrap();
            assert_eq!(charge.portal_cooldown(game), None);
            let stale = EntityRef::new(WorldEpoch::new(mob.epoch().get() + 1), mob.id());
            assert_eq!(stale.portal_cooldown(game), None);
            game.enter_level(Dim::Nether).unwrap();
            assert_eq!(mob.portal_cooldown(game), None);
            game.enter_level(Dim::Overworld).unwrap();
            assert_eq!(mob.portal_cooldown(game), Some(299));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn portal_cooldown_write_blocks_entry_and_counts_down() {
        use super::super::native_entity_refs::tests::with_resident_game;
        use std::sync::Arc;
        use sunrise_client::chunk::Chunk;
        use sunrise_client::motion::PlayerMotion;
        use sunrise_core::geometry::Vec3;

        with_resident_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.settings.simulation_distance = 4;
            game.settings.render_distance = 12;
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let portal = registry.default_state("minecraft:nether_portal").unwrap();
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 65.0, 8.5)).unwrap());
            for z in [8, 12] {
                game.world.set(0, 63, z, stone).unwrap();
            }
            game.world.set(0, 64, 8, portal).unwrap();
            game.world.set(0, 65, 8, portal).unwrap();
            game.mobs
                .push(Mob::new(40, zombie, Vec3::new(0.5, 64.0, 8.5), 0.0));
            game.load_ticking_neighbourhood(0, 0);
            game.update_chunk_tickets();
            assert!(
                game.entity_ticking_area()(0, 8),
                "the mob's chunk ticks entities"
            );
            let mob = game.entity(40).unwrap();

            // A written positive value blocks the portal: the mob stays, and
            // standing in the portal refills the countdown.
            assert!(mob.set_portal_cooldown(game, 40));
            assert_eq!(mob.portal_cooldown(game), Some(40));
            game.find_entity_portals();
            game.handle_entity_portals();
            assert!(
                game.mobs.iter().any(|m| m.id == 40),
                "the mob stays in its portal"
            );
            let held = mob.portal_cooldown(game).unwrap();
            assert_eq!(
                held,
                sunrise_client::nether_portal::ENTITY_DELAY - 1,
                "standing in the portal refills the cooldown before the tick"
            );

            // Out of the portal, the value counts down one per tick.
            game.mobs.iter_mut().find(|m| m.id == 40).unwrap().position =
                Vec3::new(0.5, 64.0, 12.5);
            game.handle_entity_portals();
            assert_eq!(mob.portal_cooldown(game), Some(held - 1));

            // A negative value is refused and leaves the value alone.
            assert!(!mob.set_portal_cooldown(game, -1));
            assert_eq!(mob.portal_cooldown(game), Some(held - 1));

            // 0 lets it enter on the next tick.
            game.mobs.iter_mut().find(|m| m.id == 40).unwrap().position = Vec3::new(0.5, 64.0, 8.5);
            assert!(mob.set_portal_cooldown(game, 0));
            assert_eq!(mob.portal_cooldown(game), Some(0));
            game.find_entity_portals();
            game.handle_entity_portals();
            assert!(
                !game.mobs.iter().any(|m| m.id == 40),
                "0 lets the mob through"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn portal_cooldown_write_is_refused_for_a_player() {
        with_player(|game, player| {
            game.portals.player.cooldown = 7;
            assert!(!player.set_portal_cooldown(game, 40));
            assert_eq!(game.portals.player.cooldown, 7);
            assert_eq!(player.portal_cooldown(game), Some(7));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn portal_cooldown_write_is_refused_for_an_acting_guest() {
        use super::super::native_guest_turn_tests::with_resident_guests;

        with_resident_guests(|game| {
            let host = game.lan.as_mut().unwrap();
            host.play.entry(12).or_default().portal.cooldown = 7;
            let guest = game.entity(12).unwrap();
            assert!(!guest.set_portal_cooldown(game, 40));
            assert_eq!(
                game.lan
                    .as_ref()
                    .unwrap()
                    .play
                    .get(&12)
                    .unwrap()
                    .portal
                    .cooldown,
                7
            );
            assert_eq!(guest.portal_cooldown(game), Some(7));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn portal_cooldown_write_is_refused_for_a_falling_block() {
        use super::super::FallingBlock;
        use super::super::native_entity_refs::tests::with_resident_game;
        use sunrise_client::nether_portal::PortalState;
        use sunrise_core::{geometry::Vec3, position::BlockPos};

        with_resident_game(|game| {
            let state = game
                .pack
                .assets
                .registry
                .default_state("minecraft:sand")
                .unwrap();
            game.next_entity += 1;
            let id = game.next_entity;
            game.falling.push(FallingBlock {
                size_answer: None,
                ground: Default::default(),
                fire_ticks: 0,
                sync_velocity: false,
                attachment_lifetime: Default::default(),
                rotation: [0.0, 0.0],
                join: Default::default(),
                id,
                state,
                blocks_building: true,
                start_pos: BlockPos::default(),
                position: Vec3::ZERO,
                previous: Vec3::ZERO,
                velocity: Vec3::ZERO,
                age: 0,
                no_gravity: false,
                portal: PortalState {
                    cooldown: 25,
                    process: None,
                },
                drop_item: false,
                fall_distance: 0.0,
                stuck: Vec3::ZERO,
                damage: sunrise_client::falling_block::Damage::falling("minecraft:sand"),
                saved: None,
            });
            let falling = game.entity(id).unwrap();
            assert!(!falling.set_portal_cooldown(game, 40));
            assert_eq!(
                game.falling
                    .iter()
                    .find(|f| f.id == id)
                    .unwrap()
                    .portal
                    .cooldown,
                25
            );
            assert_eq!(falling.portal_cooldown(game), Some(25));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn portal_cooldown_write_is_refused_on_a_client() {
        use super::super::execution_role::ExecutionRole;
        use super::super::native_entity_refs::tests::with_resident_game;
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::connection::Pipe;

        with_resident_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            game.options.dedicated = None;
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "PortalWriteTest").unwrap();
            let remote = game.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.spawn_entity(15, "minecraft:pig", [2.0, 0.0, 0.0], [0.0; 3]);
            // A handle this client really shows, so the refusal is the client
            // side's and not a fabricated id that never resolved.
            let entity = game.entity(15).unwrap();
            assert!(
                entity.position(game).is_some(),
                "the handle is live on this client"
            );
            assert!(!entity.set_portal_cooldown(game, 5));
            assert!(!entity.set_portal_cooldown(game, 0));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn portal_cooldown_write_reaches_an_item() {
        use super::super::native_entity_refs::tests::with_resident_game;
        use sunrise_client::items::ItemStack;

        with_resident_game(|game| {
            game.spawn_item(
                ItemStack {
                    item: "minecraft:stone".into(),
                    count: 1,
                    damage: 0,
                    extra: None,
                },
                [0.0; 3],
                [0.0; 3],
                0,
            );
            let item = game.entity(game.dropped[0].id).unwrap();
            assert!(item.set_portal_cooldown(game, 40));
            assert_eq!(item.portal_cooldown(game), Some(40));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn portal_cooldown_write_reaches_an_arrow() {
        use super::super::native_entity_refs::tests::with_resident_game;
        use sunrise_client::projectiles::{Projectile, ProjectileKind};
        use sunrise_core::geometry::Vec3;

        with_resident_game(|game| {
            game.add_projectile(Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                Vec3::ZERO,
                None,
            ));
            let arrow = game.entity(game.creatures.projectiles[0].id).unwrap();
            assert!(arrow.set_portal_cooldown(game, 40));
            assert_eq!(arrow.portal_cooldown(game), Some(40));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn portal_cooldown_write_is_refused_for_a_non_portal_projectile() {
        use super::super::native_entity_refs::tests::with_resident_game;
        use sunrise_client::projectiles::{Projectile, ProjectileKind};
        use sunrise_core::geometry::Vec3;

        with_resident_game(|game| {
            game.add_projectile(Projectile::new(
                ProjectileKind::WindCharge,
                Vec3::ZERO,
                None,
            ));
            let charge = game.entity(game.creatures.projectiles[0].id).unwrap();
            assert!(!charge.set_portal_cooldown(game, 40));
            assert_eq!(charge.portal_cooldown(game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn player_invisibility_write_is_refused() {
        with_player(|game, player| assert!(!player.set_invisible(game, true)));
    }

    #[test]
    #[ignore = "daily"]
    fn player_glowing_write_is_kept_in_its_saved_data() {
        with_player(|game, player| {
            assert!(player.set_glowing(game, true));
            assert_eq!(player.glowing(game), Some(true));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn player_no_gravity_write_is_kept_in_its_saved_data() {
        with_player(|game, player| {
            assert!(player.set_no_gravity(game, true));
            assert_eq!(player.no_gravity(game), Some(true));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn player_frozen_write_updates_server_state() {
        with_player(|game, player| {
            assert!(player.set_ticks_frozen(game, 53));
            assert_eq!(game.player.as_ref().unwrap().ticks_frozen, 53);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn on_fire_reads_timers_immunity_and_client_shared_flag() {
        mod_platform::testing::with_mods(Vec::new().into(), || {
            with_player(|g, player| {
                for ticks in [-1, 0, 1] {
                    g.player.as_mut().unwrap().fire_ticks = ticks;
                    assert_eq!(player.is_on_fire(g), Some(ticks > 0));
                }
                for (id, kind, expected) in [
                    (40, "minecraft:zombie", true),
                    (41, "minecraft:blaze", false),
                ] {
                    let kind = g.res.mob_types().get(kind).unwrap().clone();
                    g.mobs
                        .push(Mob::new(id, kind, sunrise_core::geometry::Vec3::ZERO, 0.0));
                    g.mobs.last_mut().unwrap().fire_ticks = 20;
                    assert_eq!(g.entity(id).unwrap().is_on_fire(g), Some(expected));
                }
                assert_eq!(
                    EntityRef::new(mod_api::WorldEpoch::new(0), 40).is_on_fire(g),
                    None
                );
            });
            super::super::native_entity_refs::tests::with_client_game(|g| {
                let _server = super::super::native_client_session::tests::logged_client(g);
                g.player =
                    Some(PlayerMotion::new(g.motion, sunrise_core::geometry::Vec3::ZERO).unwrap());
                let local = g.entity(7).unwrap();
                // Spawn actual renderable client copies through the normal packet path.
                for (id, name) in [(15, "minecraft:armor_stand"), (16, "minecraft:zombie")] {
                    let kind = g
                        .remote
                        .as_ref()
                        .unwrap()
                        .builtin
                        .get("minecraft:entity_type")
                        .unwrap()
                        .id(name)
                        .unwrap();
                    g.handle_packet(sunrise_protocol::packets::play::Clientbound::AddEntity(
                        sunrise_protocol::packets::play::AddEntity {
                            id,
                            uuid: id as u128,
                            kind,
                            position: [0.0; 3],
                            velocity: [0.0; 3],
                            pitch: 0,
                            yaw: 0,
                            head_yaw: 0,
                            data: 0,
                        },
                    ))
                    .unwrap();
                }
                let stand = g.entity(15).unwrap();
                let zombie = g.entity(16).unwrap();
                // A tracked copy answers `EntityType.fireImmune()` as the server does.
                for (id, name, immune) in [
                    (17, "minecraft:tnt", true),
                    (18, "minecraft:end_crystal", true),
                    (19, "minecraft:blaze", true),
                    (20, "minecraft:pig", false),
                ] {
                    let kind = g
                        .remote
                        .as_ref()
                        .unwrap()
                        .builtin
                        .get("minecraft:entity_type")
                        .unwrap()
                        .id(name)
                        .unwrap();
                    g.handle_packet(sunrise_protocol::packets::play::Clientbound::AddEntity(
                        sunrise_protocol::packets::play::AddEntity {
                            id,
                            uuid: id as u128,
                            kind,
                            position: [0.0; 3],
                            velocity: [0.0; 3],
                            pitch: 0,
                            yaw: 0,
                            head_yaw: 0,
                            data: 0,
                        },
                    ))
                    .unwrap();
                    assert_eq!(
                        g.entity(id as u32)
                            .unwrap()
                            .fire_immune(&*g as &dyn mod_api::ClientHost),
                        Some(immune),
                        "{name}"
                    );
                }
                for flag in [1, 0] {
                    for id in [7, 15, 16] {
                        g.handle_packet(
                            sunrise_protocol::packets::play::Clientbound::SetEntityData(
                                sunrise_protocol::packets::play::SetEntityData {
                                    id,
                                    values: vec![(
                                        0,
                                        0,
                                        sunrise_protocol::entity_data::Value::Byte(flag),
                                    )],
                                },
                            ),
                        )
                        .unwrap();
                    }
                    assert_eq!(
                        local.is_on_fire(&*g as &dyn mod_api::ClientHost),
                        Some(flag != 0)
                    );
                    assert_eq!(
                        stand.is_on_fire(&*g as &dyn mod_api::ClientHost),
                        Some(flag != 0)
                    );
                    assert!(local.set_client_fire_immunity(g, true));
                    assert_eq!(local.fire_immune(g), Some(true));
                    assert_eq!(local.is_on_fire(g), Some(false));
                    assert!(local.set_client_fire_immunity(g, false));
                    assert_eq!(local.is_on_fire(g), Some(flag != 0));
                    // Revoking while the server flag is unchanged must reveal the
                    // existing flame again, both in the handle and in the draw state.
                    for (grant, burning) in [(true, false), (false, flag != 0)] {
                        assert!(zombie.set_client_fire_immunity(g, grant));
                        assert!(stand.set_client_fire_immunity(g, grant));
                        assert_eq!(zombie.is_on_fire(g), Some(burning));
                        assert_eq!(stand.is_on_fire(g), Some(burning));
                        assert_eq!(
                            g.mobs.iter().find(|m| m.id == 16).unwrap().is_on_fire(),
                            burning
                        );
                        assert_eq!(
                            g.armor_stands
                                .iter()
                                .find(|s| s.id == 15)
                                .unwrap()
                                .is_on_fire(),
                            burning
                        );
                    }
                }
                g.player.as_mut().unwrap().fire_ticks = 2;
                assert_eq!(
                    local.is_on_fire(&*g as &dyn mod_api::ClientHost),
                    Some(true)
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn under_water_reads_previous_eye_sample_and_current_contact_on_both_sides() {
        mod_platform::testing::with_mods(Vec::new().into(), || {
            for client in [false, true] {
                super::super::native_entity_refs::tests::with_resident_game(|g| {
                    if client {
                        g.options.execution_role =
                            super::super::execution_role::ExecutionRole::Client;
                        g.options.dedicated = None;
                    }
                    let _server = client
                        .then(|| super::super::native_client_session::tests::logged_client(g));
                    g.player = Some(
                        PlayerMotion::new(g.motion, sunrise_core::geometry::Vec3::ZERO).unwrap(),
                    );
                    if !client {
                        g.options.dedicated = Some(Vec::new());
                        super::super::native_entity_refs::tests::bind_local_player(g, 7, 123);
                    }
                    let player = g.entity(7).unwrap();
                    if client {
                        g.spawn_entity(15, "minecraft:zombie", [0.0; 3], [0.0; 3]);
                    } else {
                        let kind = g.res.mob_types().get("minecraft:zombie").unwrap().clone();
                        g.mobs
                            .push(Mob::new(15, kind, sunrise_core::geometry::Vec3::ZERO, 0.0));
                    }
                    let mob = g.entity(15).unwrap();
                    for old_eye in [false, true] {
                        for contact in [false, true] {
                            let p = g.player.as_mut().unwrap();
                            p.fluids.was_eye_in_water = old_eye;
                            p.fluids.was_touching_water = contact;
                            assert_eq!(
                                g.entity_is_under_water(player),
                                Some(if client { old_eye } else { old_eye && contact })
                            );
                            let m = g.mobs.iter_mut().find(|m| m.id == 15).unwrap();
                            m.was_eye_in_water = old_eye;
                            m.fluids.eye_in_water = !old_eye;
                            m.fluids.in_water = contact;
                            assert_eq!(g.entity_is_under_water(mob), Some(old_eye && contact));
                            assert_eq!(
                                g.entity_is_under_water(EntityRef::new(
                                    mod_api::WorldEpoch::new(0),
                                    15
                                )),
                                None
                            );
                        }
                    }
                });
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn secondary_use_reads_reachable_client_handles_and_keeps_server_flags() {
        use super::super::execution_role::ExecutionRole;
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::connection::Pipe;

        use winit::keyboard::KeyCode;
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            game.options.dedicated = None;
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "SneakReadTest").unwrap();
            // The joined client's state; the local-player and entity constructors must both
            // resolve it, not just a hand-built entity id accepted by an unreachable stub.
            let remote = game.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            let local = game.local_player().unwrap();
            let entity = game.entity(7).unwrap();
            assert_eq!(local.entity(), entity);
            game.insert_device_key(KeyCode::ShiftLeft);
            assert!(!game.player.as_ref().unwrap().shift_key_down);
            assert_eq!(entity.is_shift_key_down(game), Some(true));
            game.remove_device_key(&KeyCode::ShiftLeft);
            game.player.as_mut().unwrap().shift_key_down = true;
            assert_eq!(
                entity.is_shift_key_down(game),
                Some(false),
                "live secondary use, not last tick's body flag"
            );
            let stale = EntityRef::new(mod_api::WorldEpoch::new(0), 7);
            assert_eq!(stale.is_shift_key_down(game), None);
            game.remote.as_mut().unwrap().logged_in = false;
            assert!(game.local_player().is_none());
            assert_eq!(entity.is_shift_key_down(game), None);
        });
        with_player(|game, player| {
            game.clear_device_keys();
            game.player.as_mut().unwrap().shift_key_down = true;
            assert_eq!(
                player.is_shift_key_down(game),
                Some(true),
                "server keeps its semantic entity flag"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn client_fluid_reads_use_cached_local_and_tracked_bodies() {
        use super::super::{
            native_client_session::tests::logged_client,
            native_entity_refs::tests::with_client_game,
        };
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::packets::play;

        with_client_game(|game| {
            let _server = logged_client(game);
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            let local = game.entity(7).unwrap();
            let host = &*game as &dyn mod_api::ClientHost;
            assert_eq!(
                local.visit_fluid_heights(host, &mut |_, _| panic!("dry player visited")),
                Ok(ControlFlow::Continue(()))
            );
            assert_eq!(local.eye_fluid(host), Some(None));

            game.spawn_entity(15, "minecraft:zombie", [2.0, 0.0, 0.0], [0.0; 3]);
            let zombie = game.entity(15).unwrap();
            // No water or lava in the world: these answers must come from the last
            // client fluid update, without a fresh world query or a server body.
            for (water, lava, eye_water, eye_lava, expected, eye) in [
                (
                    0.75,
                    0.0,
                    true,
                    false,
                    vec![("minecraft:water".to_owned(), 0.75)],
                    Some("minecraft:water"),
                ),
                (
                    0.0,
                    0.5,
                    false,
                    false,
                    vec![("minecraft:lava".to_owned(), 0.5)],
                    None,
                ),
                (
                    0.25,
                    1.5,
                    false,
                    true,
                    vec![
                        ("minecraft:lava".to_owned(), 1.5),
                        ("minecraft:water".to_owned(), 0.25),
                    ],
                    Some("minecraft:lava"),
                ),
                (0.0, 0.0, false, false, Vec::new(), None),
            ] {
                let fluids = &mut game.mobs.iter_mut().find(|m| m.id == 15).unwrap().fluids;
                fluids.water = water;
                fluids.lava = lava;
                fluids.in_water = water > 0.0;
                fluids.in_lava = lava > 0.0;
                fluids.eye_in_water = eye_water;
                fluids.eye_in_lava = eye_lava;
                let host = &*game as &dyn mod_api::ClientHost;
                let mut fluids = Vec::new();
                assert_eq!(
                    zombie.visit_fluid_heights(host, &mut |id, depth| {
                        fluids.push((id.to_owned(), depth));
                        ControlFlow::Continue(())
                    }),
                    Ok(ControlFlow::Continue(()))
                );
                assert_eq!(fluids, expected);
                assert_eq!(zombie.eye_fluid(host), Some(eye));
            }
            game.spawn_entity(16, "minecraft:armor_stand", [2.0, 0.0, 0.0], [0.0; 3]);
            let stand = game.entity(16).unwrap();
            let host = &*game as &dyn mod_api::ClientHost;
            assert_eq!(
                stand.visit_fluid_heights(host, &mut |_, _| panic!("absent fluid visit")),
                Err(EntityReadError::Unavailable)
            );
            assert_eq!(stand.eye_fluid(host), None);
            let stale = EntityRef::new(mod_api::WorldEpoch::new(0), 15);
            assert_eq!(
                stale.visit_fluid_heights(host, &mut |_, _| panic!("absent fluid visit")),
                Err(EntityReadError::Stale)
            );
            assert_eq!(stale.eye_fluid(host), None);
            game.handle_packet(play::Clientbound::RemoveEntities(play::RemoveEntities {
                ids: vec![15],
            }))
            .unwrap();
            assert_eq!(
                zombie.visit_fluid_heights(&*game as &dyn mod_api::ClientHost, &mut |_, _| panic!(
                    "absent fluid visit"
                )),
                Err(EntityReadError::Stale)
            );
            assert_eq!(zombie.eye_fluid(&*game as &dyn mod_api::ClientHost), None);
            game.remote.as_mut().unwrap().logged_in = false;
            assert_eq!(
                local.visit_fluid_heights(&*game as &dyn mod_api::ClientHost, &mut |_, _| panic!(
                    "absent fluid visit"
                )),
                Err(EntityReadError::Unavailable)
            );
            assert_eq!(local.eye_fluid(&*game as &dyn mod_api::ClientHost), None);
        });
    }

    #[test]
    fn max_air_follows_the_species() {
        assert_eq!(max_air(None), 300);
        assert_eq!(max_air(Some(Species::Dolphin)), 4800);
        assert_eq!(max_air(Some(Species::Axolotl)), 6000);
        assert_eq!(max_air(Some(Species::Zombie)), 300);
    }

    // `EntityRef::main_arm` (`LivingEntity.getMainArm`) on a client and on a server.
    use super::super::native_entity_refs::tests::{run_fresh_cases, with_client_game};
    use super::super::native_guest_turn_tests::with_resident_guests;
    use mod_api::MainHand;
    use sunrise_protocol::entity_data::{Value, serializer};
    use sunrise_protocol::packets::play::{self, Clientbound as C};

    /// A client reads every entity it tracks as the server sent it: a mob's left-handed flag
    /// (`Mob.DATA_MOB_FLAGS_ID`) and another player's main hand (`Avatar.DATA_PLAYER_MAIN_HAND`);
    /// its own player follows its Main Hand option. An entity that isn't living, and one the
    /// client doesn't track, has none.
    #[test]
    #[ignore = "daily"]
    fn a_client_reads_the_main_arm_the_server_sent() {
        if run_fresh_cases(
            "game::native_entity_state::tests::a_client_reads_the_main_arm_the_server_sent",
            "SUNRISE_TEST_CLIENT_MAIN_ARM_CASE",
            &["client"],
        ) {
            return;
        }
        use super::super::native_client_session::tests::logged_client;
        use sunrise_client::motion::PlayerMotion;
        use sunrise_core::geometry::Vec3;
        mod_platform::testing::with_mods(Default::default(), || {
            with_client_game(|game| {
                let _server = logged_client(game);
                game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
                let types = game.remote.as_ref().unwrap().builtin.clone();
                let kind = |name: &str| {
                    types
                        .get("minecraft:entity_type")
                        .unwrap()
                        .id(name)
                        .unwrap()
                };
                game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                    actions: 1,
                    entries: vec![play::PlayerInfo {
                        uuid: 132,
                        name: Some("Remote".into()),
                        ..Default::default()
                    }],
                }))
                .unwrap();
                for (id, name) in [
                    (31, "minecraft:zombie"),
                    (32, "minecraft:player"),
                    (33, "minecraft:experience_orb"),
                ] {
                    game.handle_packet(C::AddEntity(play::AddEntity {
                        id,
                        uuid: 100 + id as u128,
                        kind: kind(name),
                        position: [2.0, 64.0, 3.0],
                        velocity: [0.0; 3],
                        pitch: 0,
                        yaw: 0,
                        head_yaw: 0,
                        data: 0,
                    }))
                    .unwrap();
                }
                let (zombie, player, orb) = (
                    game.entity(31).unwrap(),
                    game.entity(32).unwrap(),
                    game.entity(33).unwrap(),
                );
                let local = game.local_player().unwrap().entity();
                let read = |game: &Game<'_>, entity: EntityRef| {
                    let host: &dyn mod_api::ClientHost = game;
                    entity.main_arm(host)
                };
                assert_eq!(read(game, zombie), Some(MainHand::Right));
                assert_eq!(read(game, player), Some(MainHand::Right));
                for left in [true, false] {
                    // No AI 1, left-handed 2, aggressive 4; HumanoidArm 0 is the left arm.
                    let flags = if left { 2 | 4 } else { 4 };
                    game.handle_packet(C::SetEntityData(play::SetEntityData {
                        id: 31,
                        values: vec![(15, serializer::BYTE, Value::Byte(flags))],
                    }))
                    .unwrap();
                    game.handle_packet(C::SetEntityData(play::SetEntityData {
                        id: 32,
                        values: vec![(15, serializer::HUMANOID_ARM, Value::Id(i32::from(!left)))],
                    }))
                    .unwrap();
                    game.settings.main_hand_left = left;
                    let arm = if left {
                        MainHand::Left
                    } else {
                        MainHand::Right
                    };
                    assert_eq!(read(game, zombie), Some(arm), "the mob");
                    assert_eq!(read(game, player), Some(arm), "the other player");
                    assert_eq!(read(game, local), Some(arm), "the local player");
                }
                assert_eq!(read(game, orb), None, "not a living entity");
                assert_eq!(read(game, EntityRef::new(local.epoch(), 99)), None);
            });
        });
    }

    /// A server reads a player's main arm from the Main Hand its client reported, and a mob's
    /// from its left-handed flag.
    #[test]
    #[ignore = "daily"]
    fn a_server_reads_the_reported_main_hand_and_the_left_handed_flag() {
        if run_fresh_cases(
            "game::native_entity_state::tests::a_server_reads_the_reported_main_hand_and_the_left_handed_flag",
            "SUNRISE_TEST_SERVER_MAIN_ARM_CASE",
            &["server"],
        ) {
            return;
        }
        with_resident_guests(|game| {
            let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
            let host = game.lan.as_mut().unwrap();
            host.hub.update_guest(12, |g| g.info.main_hand = 0).unwrap();
            let read = |game: &Game<'_>, entity: EntityRef| {
                let host: &dyn mod_api::ServerHost = game;
                entity.main_arm(host)
            };
            assert_eq!(read(game, a.entity()), Some(MainHand::Left));
            assert_eq!(read(game, b.entity()), Some(MainHand::Right));
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let at = sunrise_core::geometry::Vec3::ZERO;
            game.mobs
                .push(sunrise_client::mobs::Mob::new(500, zombie, at, 0.0));
            let mob = game.entity(500).unwrap();
            assert_eq!(read(game, mob), Some(MainHand::Right));
            game.mobs.last_mut().unwrap().extra.left_handed = true;
            assert_eq!(read(game, mob), Some(MainHand::Left));
        });
    }
}
