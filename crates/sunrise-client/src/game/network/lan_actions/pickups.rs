use super::*;

impl Game<'_> {
    /// Dropped items touching a guest go into their inventory
    /// (`ItemEntity.playerTouch`, then `ServerPlayer.onItemPickup`).
    pub(in crate::game) fn guest_pickups(&mut self, gi: usize) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let (conn, mode, position, id, uuid) = {
            let g = &host.hub.guests[gi];
            (
                g.conn,
                g.game_mode,
                g.position,
                g.entity_id,
                sunrise_client::lan::playerdata::uuid_ints(g.uuid),
            )
        };
        if mode == SPECTATOR || self.dropped.is_empty() {
            return;
        }
        // The test below takes items whose position is less than 1.5 from
        // the player's on x and z: their boxes reach these columns.
        if let Some(columns) = self.pickup_columns.as_mut()
            && !columns.may_reach_area(
                &self.dropped,
                (
                    (position[0] - 1.5).floor() as i32,
                    (position[0] + 1.5).floor() as i32,
                ),
                (
                    (position[2] - 1.5).floor() as i32,
                    (position[2] + 1.5).floor() as i32,
                ),
            )
        {
            return;
        }
        let listens = mod_api::__private::listened(mod_api::HookId::ItemPickedUp);
        let near: Vec<_> = self
            .dropped
            .iter()
            .filter(|d| {
                (listens
                    || Self::pickup_decision(
                        d.pickup_delay,
                        d.target,
                        uuid,
                        mod_api::TriState::Default,
                    ))
                    && (d.position.x - position[0]).abs() < 1.5
                    && (d.position.z - position[2]).abs() < 1.5
                    && d.position.y > position[1] - 0.6
                    && d.position.y < position[1] + 2.4
            })
            .map(|d| d.id)
            .collect();
        let mut taken = Vec::new();
        for item_id in near.into_iter().rev() {
            self.native_guest_pickup(conn, item_id, uuid, &mut taken);
        }
        // Entity.take: the clients play the pickup sound themselves
        // (handleTakeItemEntity).
        if let Some(host) = self.lan.as_ref() {
            for (item, count) in taken {
                host.hub.take_item(item as i32, id, count as i32, position);
            }
        }
    }

    /// `Player.aiStep` touching arrows in the guest's box grown by (1, 0.5, 1):
    /// `AbstractArrow.playerTouch` for a guest, as `touch_arrows` does for the
    /// host's player. The guest's own client plays the pickup sound
    /// (`handleTakeItemEntity`).
    pub(super) fn guest_arrows(&mut self, gi: usize) {
        use sunrise_client::arrows::Touch;
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let (conn, id, at, creative, unable, dims, player, name) = {
            let g = &host.hub.guests[gi];
            (
                g.conn,
                g.entity_id,
                g.position,
                g.creative(),
                g.spectator() || g.dead || g.health <= 0.0,
                g.dimensions(),
                g.uuid,
                g.name.clone(),
            )
        };
        if unable || self.creatures.projectiles.is_empty() {
            return;
        }
        let w = f64::from(dims.width) / 2.0;
        let reach = Aabb::new(
            Vec3::new(at[0] - w, at[1], at[2] - w),
            Vec3::new(at[0] + w, at[1] + f64::from(dims.height), at[2] + w),
        )
        .inflate(Vec3::new(1.0, 0.5, 1.0));
        let mut projectiles = self.creatures.projectiles.lift();
        #[cfg(test)]
        crate::game::entity::taken_pass_tests::after_take(self);
        let mut picked = Vec::new();
        projectiles.retain(|shot| {
            let Some(a) = shot.arrow.as_ref().filter(|a| !a.removed) else {
                return true;
            };
            let p = shot.position;
            let bounds = Aabb::new(
                Vec3::new(p.x - 0.25, p.y, p.z - 0.25),
                Vec3::new(p.x + 0.25, p.y + 0.5, p.z + 0.25),
            );
            if !bounds.intersects(reach) {
                return true;
            }
            let taken = match a.player_touch(creative) {
                Touch::Ignore => false,
                Touch::Take => true,
                // Inventory.add: a creative player takes what does not fit.
                Touch::Give(stack) => {
                    self.as_guest(conn, |g| g.add_stack_to_inventory(stack))
                        .flatten()
                        .is_none()
                        || creative
                }
            };
            if taken {
                picked.push(shot.clone());
            }
            !taken
        });
        self.creatures.projectiles.restore(
            projectiles,
            sunrise_client::join_seq::RestoreOrder::SurvivorsFirst,
        );
        for shot in picked {
            if let Some(arrow) = shot.arrow.as_ref() {
                self.native_player_pickup_arrow(player, &name, shot.id, &arrow.pickup_item.item);
            }
            if let Some(host) = self.lan.as_ref() {
                host.hub.take_item(shot.id as i32, id, 1, at);
            }
        }
    }

    /// `Player.tick`'s takeXpDelay and `Player.aiStep`'s touch of the
    /// experience orbs in the guest's box grown by 1, 0.5, 1: one of them at
    /// random, at most every 2 ticks (`ExperienceOrb.playerTouch`), repairs
    /// with Mending and gives the rest as points. Everyone near hears it.
    pub(super) fn guest_orbs(&mut self, gi: usize) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let g = &host.hub.guests[gi];
        let own = g.bounds();
        let vehicle = g.vehicle;
        let (conn, id, at, player, name) =
            (g.conn, g.entity_id, g.position, g.uuid, g.name.clone());
        let unable = g.spectator() || g.dead || g.health <= 0.0;
        let play = host.play.entry(conn).or_default();
        let cooldown = if play.xp_delay > 0 {
            play.xp_delay -= 1;
            Some(play.xp_delay)
        } else {
            None
        };
        if let Some(cooldown) = cooldown {
            self.native_player_exp_cooldown_change(player, &name, cooldown);
        }
        let delayed = self
            .lan
            .as_ref()
            .is_none_or(|host| host.play.get(&conn).is_some_and(|play| play.xp_delay > 0));
        if unable || delayed {
            return;
        }
        let area = match vehicle.and_then(|v| self.guest_target_box(gi, v as i32)) {
            Some(vehicle) => own.union_bounds(vehicle).inflate(Vec3::new(1.0, 0.0, 1.0)),
            None => own.inflate(Vec3::new(1.0, 0.5, 1.0)),
        };
        let (min, max) = (
            [
                area.min_position().x,
                area.min_position().y,
                area.min_position().z,
            ],
            [
                area.max_position().x,
                area.max_position().y,
                area.max_position().z,
            ],
        );
        let touching: Vec<usize> = self
            .orbs
            .iter()
            .enumerate()
            .filter(|(_, o)| {
                o.position.x + 0.25 > min[0]
                    && o.position.x - 0.25 < max[0]
                    && o.position.y + 0.5 > min[1]
                    && o.position.y < max[1]
                    && o.position.z + 0.25 > min[2]
                    && o.position.z - 0.25 < max[2]
            })
            .map(|(i, _)| i)
            .collect();
        if touching.is_empty() {
            return;
        }
        // Util.getRandom(orbs, random).
        let pick = self
            .world
            .random
            .next_int_bound(touching.len() as i32)
            .unwrap_or(0) as usize;
        // A merged orb gives its value once per count and goes at zero.
        let index = touching[pick];
        let join = self.orbs[index].join;
        let orb_id = self.orbs[index].id;
        // Swapping the guest in costs: only when a mod hears the pickup.
        let listened = (self.serving.is_some() || self.options.dedicated.is_some())
            && mod_api::__private::listened(mod_api::HookId::PickupXp);
        if listened && let Some(epoch) = self.native_lifecycle.epoch() {
            let orb = mod_api::ExperienceOrbRef::new(mod_api::EntityRef::new(epoch, orb_id));
            let mut canceled = false;
            let dimension = self.dim.id();
            let called = self.as_guest(conn, |game| {
                let player = game.native_acting_id();
                game.native_pickup_xp(player, dimension, orb, &mut canceled);
            });
            if called.is_none() || canceled {
                return;
            }
        }
        // A callback may have committed a discard: the row stays until removals flush.
        let live = |game: &Self| {
            !game.held_out.contains(&orb_id)
                && game.resident_source_target_visible(
                    game.dim.key(),
                    crate::game::commands::Target::Orb(orb_id),
                )
        };
        if !live(self) {
            return;
        }
        let Some(index) = self.orbs.locate(join) else {
            return;
        };
        let (orb_value, orb_pos) = {
            let orb = &self.orbs[index];
            (orb.value, orb.position)
        };
        // ExperienceOrb.playerTouch: delay, take, repair/award positive value,
        // then decrement count and discard at zero.
        if let Some(host) = self.lan.as_mut() {
            host.play.entry(conn).or_default().xp_delay = 2;
        }
        self.native_player_exp_cooldown_change(player, &name, 2);
        if let Some(host) = self.lan.as_ref() {
            host.hub.take_item(orb_id as i32, id, 1, at);
        }
        // ClientPacketListener.handleTakeItemEntity's sound, at the orb.
        let mut random = sunrise_client::random_source::create(); // the listener's own stream
        let pitch = (random.next_float() - random.next_float()) * 0.35 + 0.9;
        self.play_audio(
            "minecraft:entity.experience_orb.pickup",
            Some([orb_pos.x, orb_pos.y, orb_pos.z]),
            0.1,
            pitch,
        );
        self.as_guest(conn, |game| {
            let rest = game.mending(orb_value);
            if rest > 0 {
                game.give_experience_points(rest);
            }
        });
        if !live(self) {
            return;
        }
        if let Some(index) = self.orbs.locate(join) {
            if self.orbs[index].count <= 1 {
                self.remove_orb(index);
            } else {
                self.orbs[index].count -= 1;
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/pickup_post_tests.rs"]
mod pickup_post_tests;

#[cfg(test)]
#[path = "tests/orb_merge_contract_tests.rs"]
mod orb_merge_contract_tests;
