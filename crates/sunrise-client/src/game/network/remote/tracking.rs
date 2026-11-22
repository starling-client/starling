use super::*;

impl Game<'_> {
    /// `recreateFromPacket`: yRot and xRot from the packet, the body and head
    /// both at its head yaw (LivingEntity).
    pub(in crate::game) fn client_attachment_entity_exists(&self, holder: Holder) -> bool {
        self.remote.as_ref().is_some_and(|remote| match holder {
            Holder::Entity(id) => remote.entities.contains_key(&(id as i32)),
            Holder::Player(uuid) => {
                remote.profile == uuid || remote.level_players.values().any(|p| *p == uuid)
            }
            _ => false,
        })
    }

    pub(in crate::game) fn spawn_entity(
        &mut self,
        id: i32,
        kind: &str,
        position: [f64; 3],
        rotation: [f32; 3],
    ) {
        self.detach_remote_tracking_entity(id);
        let [yaw, pitch, head] = rotation;
        let at = Vec3::new(position[0], position[1], position[2]);
        let mob_type = if kind == "minecraft:player" {
            self.remote.as_ref().and_then(|r| r.player_type.clone())
        } else {
            self.res.mob_types().get(kind).cloned()
        };
        if kind == "minecraft:ender_dragon" {
            self.spawn_remote_dragon(id, position, yaw);
        }
        if kind == "minecraft:experience_orb" {
            // Its value (ExperienceOrb.DATA_VALUE) follows in SetEntityData.
            self.orbs.retain(|o| o.id != id as u32);
            self.orbs
                .push(ExperienceOrb::new(id as u32, at, Vec3::ZERO, 1));
        }
        if let Some(t) = mob_type {
            // The window's copy of the server's mob: no mod part goals —
            // their `register_goals` ran on the authoritative side (#1077).
            let mut mob = Mob::new_remote(id as u32, t, at, head);
            super::super::remote_mob_data::client_defaults(&mut mob);
            (mob.yaw, mob.previous_yaw) = (yaw, yaw);
            (mob.head_pitch, mob.previous_head_pitch) = (pitch, pitch);
            mob.persistent = true;
            self.mobs.retain(|m| m.id != id as u32);
            self.mobs.push(mob);
        }
        if let Some(r) = self.remote.as_mut() {
            // A new entity has a new interpolation handler (a mob's or an
            // armor stand's), even under an id the server used before.
            r.stepped.remove(&id);
            if r.entities
                .get(&id)
                .is_some_and(|old| old.kind == "minecraft:item")
            {
                self.dropped.retain(|item| item.id != id as u32);
                r.pending_items.remove(&id);
            }
            if kind == "minecraft:item" {
                r.pending_items.insert(
                    id,
                    PendingItem {
                        join: JoinSeq::next(),
                        position,
                        velocity: Vec3::ZERO,
                    },
                );
            }
            r.entities.insert(
                id,
                Tracked {
                    kind: kind.to_owned(),
                    position,
                    codec_base: position,
                    on_fire: false,
                    mod_fire_immune: false,
                    silent: false,
                    vehicle: None,
                },
            );
        }
    }

    pub(in crate::game) fn detach_remote_tracking_entity(&mut self, id: i32) {
        let mobs = &self.mobs;
        let index = &mut self.mob_index;
        self.particles
            .detach_tracking_target(TrackingTarget::Entity(id), || {
                index.get(mobs, id as u32).map(mob_tracking_body)
            });
    }
}
