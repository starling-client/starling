use super::*;
use sunrise_client::particles::TrackingBody;

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
            if kind == "minecraft:player"
                && let Some(r) = self.remote.as_ref()
            {
                mob.extra.spectator = r.remote_player_info_game_mode(id) == Some(SPECTATOR);
            }
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
            r.synced_attributes.remove(&id);
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
                        no_gravity: false,
                    },
                );
            }
            let tracked_yaw = if yaw.is_finite() { yaw } else { 0.0 };
            let tracked_pitch = if pitch.is_finite() {
                (pitch % 360.0).clamp(-90.0, 90.0)
            } else {
                0.0
            };
            r.entities.insert(
                id,
                Tracked {
                    kind: kind.to_owned(),
                    position,
                    previous: position,
                    yaw: tracked_yaw,
                    pitch: tracked_pitch,
                    previous_yaw: tracked_yaw,
                    previous_pitch: tracked_pitch,
                    codec_base: position,
                    on_fire: false,
                    mod_fire_immune: false,
                    silent: false,
                    vehicle: None,
                    tick_count: 0,
                    size_answer: None,
                },
            );
        }
    }

    /// The current entity position and dimensions used by `TrackingEmitter.tick`.
    /// Read body copies rather than the network position of an interpolated entity.
    pub(in crate::game) fn remote_entity_tracking_body(&self, id: i32) -> Option<TrackingBody> {
        let remote = self.remote.as_ref()?;
        if let Some(row) = remote.general.rows.get(&id) {
            let (width, height) =
                super::super::general::general_type_catalog().dimensions(row.type_slot);
            return Some(TrackingBody {
                feet: row.position,
                width,
                height,
            });
        }
        let (kind, last_position) = remote.tracked(id)?;
        let eid = id as u32;
        // The dragon's render and motion body is separate from its mob row.
        if kind != "minecraft:ender_dragon"
            && (kind == "minecraft:player" || self.res.mob_types().contains_key(kind))
            && let Some(i) = self.mob_index.find_ref(&self.mobs, eid)
        {
            return Some(mob_tracking_body(&self.mobs[i]));
        }
        if let Some(entity) = self.interactions.get(&id) {
            let Vec3 { x, y, z } = entity.position;
            return Some(TrackingBody {
                feet: [x, y, z],
                width: f64::from(entity.width),
                height: f64::from(entity.height),
            });
        }
        let (mut width, mut height) = self.res.rest().spawner_types.get(kind)?.entity_dimensions();
        let feet = match kind {
            "minecraft:ender_dragon" => {
                let dragon = self.dragon.as_ref().filter(|dragon| dragon.id == eid)?;
                let Vec3 { x, y, z } = dragon.position;
                [x, y, z]
            }
            "minecraft:armor_stand" => {
                let stand = self.armor_stands.iter().find(|stand| stand.id == eid)?;
                (width, height) = super::super::entity::armor_stand_size(stand);
                let Vec3 { x, y, z } = stand.position;
                [x, y, z]
            }
            "minecraft:area_effect_cloud" => {
                let cloud = self.potions.clouds.iter().find(|cloud| cloud.id == eid)?;
                width = f64::from(cloud.radius * 2.0);
                cloud.at
            }
            "minecraft:splash_potion"
            | "minecraft:lingering_potion"
            | "minecraft:experience_bottle" => {
                let (_, _, _, position, _) =
                    self.potions.thrown_tracked().find(|(id, ..)| *id == eid)?;
                [position.x, position.y, position.z]
            }
            "minecraft:fishing_bobber" => {
                let hook = self.remote_hooks.iter().find(|hook| hook.id == eid)?;
                let Vec3 { x, y, z } = hook.position;
                [x, y, z]
            }
            _ => self
                .text_displays
                .get(&id)
                .map(|display| display.position().to_array())
                .or_else(|| {
                    self.level_entity(id)
                        .and_then(|target| self.target_position(target))
                })
                .unwrap_or(last_position),
        };
        Some(TrackingBody {
            feet,
            width,
            height,
        })
    }

    pub(in crate::game) fn detach_remote_tracking_entity(&mut self, id: i32) {
        let target = TrackingTarget::Entity(id);
        if self.particles.has_tracking_target(target) {
            let last_body = self.remote_entity_tracking_body(id);
            self.particles.detach_tracking_target(target, || last_body);
        }
    }
}
