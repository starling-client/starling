use super::*;

impl Game<'_> {
    /// `C::AddEntity`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_add_entity_packet(&mut self, p: play::AddEntity) {
        self.audio.remove_sound_entity(p.id as u32);
        let kind = self
            .remote
            .as_ref()
            .and_then(|r| {
                r.builtin
                    .get("minecraft:entity_type")?
                    .name(p.kind)
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        let rotation = [p.yaw, p.pitch, p.head_yaw].map(sunrise_protocol::buf::unpack_degrees);
        if let Some(r) = self.remote.as_mut() {
            // ClientLevel.addEntity: an id it holds already is removed
            // and added again, after the player.
            r.ticked_before_player.remove(&(p.id as u32));
            // A new entity looks its PlayerInfo up afresh.
            r.level_player_textures.remove(&p.id);
            if kind == "minecraft:player" {
                r.level_players.insert(p.id, p.uuid);
                // createEntityFromPacket: the player takes the profile of its list entry.
                match r.players.get(&p.uuid) {
                    Some(name) => r.level_player_names.insert(p.id, name.clone()),
                    None => r.level_player_names.remove(&p.id),
                };
            } else {
                r.level_players.remove(&p.id);
                r.level_player_names.remove(&p.id);
            }
        }
        // Entity.getTeam: a mob is on a team by its UUID, a player by
        // its profile name (`Game::scoreboard_name`).
        self.persist.uuids.insert(
            p.id as u32,
            sunrise_client::lan::playerdata::uuid_ints(p.uuid),
        );
        let holder = if kind == "minecraft:player" {
            Holder::Player(p.uuid)
        } else {
            Holder::Entity(p.id as u32)
        };
        self.initialize_entity(holder, &kind);
        let replace = self.tracks_remote_entity(p.id);
        self.spawn_entity(p.id, &kind, p.position, rotation);
        if let Some(r) = self.remote.as_mut() {
            r.cart_lerps.remove(&p.id);
            r.lerps.remove(&p.id);
        }
        if !self.spawn_remote_object(p.id, &kind, p.position, (rotation[0], rotation[1]), p.data) {
            self.spawn_added_projectile(&p, &kind, (rotation[0], rotation[1]), replace);
        }
        // Entity.recreateFromPacket: objects must exist before applying motion.
        self.remote_motion(p.id, p.velocity);
        if self.minecart_improvements()
            && self.carts.iter().any(|c| c.id == p.id as u32)
            && let Some(r) = self.remote.as_mut()
        {
            r.cart_lerps.insert(p.id, CartLerp::new());
        }
        self.move_remote_text_display(p.id, p.position, Some((rotation[0], rotation[1])));
        self.move_remote_interaction(p.id, p.position);
    }

    /// `C::RemoveEntities`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_remove_entities_packet(&mut self, p: play::RemoveEntities) {
        for &id in &p.ids {
            self.forget_remote_entity_construction(id);
            self.forget_client_collider(id);
            if let Some(r) = self.remote.as_mut() {
                if let Some(uuid) = r.level_players.remove(&id) {
                    r.attachments.remove(&Holder::Player(uuid));
                }
                r.level_player_names.remove(&id);
                r.level_player_textures.remove(&id);
                r.attachments.remove(&Holder::Entity(id as u32));
                Self::forget_remote_general(r, id);
            }
        }
        self.remove_entities(&p.ids);
    }

    /// `C::EntityPositionSync`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_entity_position_sync_packet(&mut self, p: play::EntityPositionSync) {
        let path = match p.path {
            play::PositionPath::Linear(at) => Path::Linear(at),
            play::PositionPath::Stepped(steps) => Path::Stepped(steps),
        };
        // handleEntityPositionSync: the position codec's base is set.
        if let Some(t) = self.remote.as_mut().and_then(|r| r.entities.get_mut(&p.id)) {
            t.codec_base = path.end();
        }
        self.remote_position_sync_and_file(p.id, path, p.yaw, p.pitch, p.on_ground);
    }

    /// `C::SetEntityLink`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_set_entity_link_packet(&mut self, p: play::SetEntityLink) {
        // ClientPacketListener.handleEntityLinkPacket: who holds a mob's
        // lead (this player, another player, a mob or a fence knot), for
        // the rope.
        let me = self.remote.as_ref().map(|r| r.entity_id);
        use sunrise_client::mobs::leash::Holder;
        let holder = match p.dest {
            0 => None,
            d if Some(d) == me => Some(Holder::Player),
            d => match self.remote_knots.get(&(d as u32)) {
                Some(&block) => Some(Holder::Knot(block)),
                // Not a known knot yet: kept by id until the knot
                // arrives (game/network/remote_objects.rs).
                None if self.boats.iter().any(|b| b.id == d as u32) => Some(Holder::Boat(d as u32)),
                None => Some(Holder::Mob(d as u32)),
            },
        };
        if let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.source as u32) {
            m.extra.leash = holder;
        } else if let Some(i) = self.boats.iter().position(|b| b.id == p.source as u32) {
            self.boats.update(i, |b| {
                // A leashed boat or raft (AbstractBoat is Leashable).
                b.leash = holder;
            });
        }
    }

    /// `C::MoveVehicle`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_move_vehicle_packet(&mut self, p: play::MoveVehicle) {
        // handleMoveVehicle: the vehicle this client moves goes where the server says,
        // unless it is within 1e-5 already, and is reported back at once.
        if let Some(at) = self.driven_client_position() {
            let to = Vec3::new(p.position[0], p.position[1], p.position[2]);
            if to.subtract(at).length() > f64::from(1.0e-5_f32) {
                self.remote_move_vehicle_and_file(p.position, (p.yaw, p.pitch));
            }
            if let Some(report) = self.remote_vehicle_packets().into_iter().next() {
                self.send(report);
            }
        }
    }

    /// `C::TakeItemEntity`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_take_item_entity_packet(&mut self, p: play::TakeItemEntity) {
        // ClientPacketListener.handleTakeItemEntity: the pickup sound at
        // the orb, item or arrow, whoever collects it (the server sends
        // none); an entity this client doesn't track sounds at this player.
        let orb_at = self
            .orbs
            .iter()
            .find(|o| o.id == p.item as u32)
            .map(|o| o.position);
        let orb = orb_at.is_some();
        let item = self.dropped.iter().find(|d| d.id == p.item as u32);
        let shot = self
            .creatures
            .projectiles
            .iter()
            .find(|s| s.id == p.item as u32);
        let shot = shot.map(|s| s.position).or_else(|| {
            self.tridents
                .iter()
                .find(|t| t.id == p.item as u32)
                .map(|t| t.position)
        });
        let at = match (item, shot) {
            (Some(d), _) => Some([d.position.x, d.position.y, d.position.z]),
            (None, Some(s)) => Some([s.x, s.y, s.z]),
            (None, None)
                if self
                    .remote
                    .as_ref()
                    .is_some_and(|r| r.entity_id == p.player) =>
            {
                self.eye_position()
            }
            (None, None) => None,
        };
        // `ClientPacketListener.random`, the listener's own stream: a
        // pickup pitch is never a draw of the level's.
        if let Some(o) = orb_at {
            let mut r = sunrise_client::random_source::create();
            let pitch = (r.next_float() - r.next_float()) * 0.35 + 0.9;
            self.audio.play(
                "minecraft:entity.experience_orb.pickup",
                Some([o.x, o.y, o.z]),
                0.1,
                pitch,
            );
        } else if let Some(at) = at {
            let mut r = sunrise_client::random_source::create();
            let pitch = ((r.next_float() - r.next_float()) * 0.7 + 1.0) * 2.0;
            self.audio
                .play("minecraft:entity.item.pickup", Some(at), 0.2, pitch);
        }
        // ItemPickupParticle toward the collector (this player, another one or a mob).
        let collector = if self
            .remote
            .as_ref()
            .is_some_and(|r| r.entity_id == p.player)
        {
            super::super::pickups::Collector::Player
        } else {
            super::super::pickups::Collector::Mob(p.player as u32)
        };
        if let Some(item) = self.dropped.iter().find(|d| d.id == p.item as u32).cloned() {
            self.add_pickup(
                super::super::pickups::Taken::Item(Box::new(item)),
                collector,
            );
        } else if let Some(orb) = self.orbs.iter().find(|o| o.id == p.item as u32).cloned() {
            self.add_pickup(super::super::pickups::Taken::Orb(Box::new(orb)), collector);
        } else if let Some(shot) = self
            .creatures
            .projectiles
            .iter()
            .find(|s| s.id == p.item as u32)
            .cloned()
        {
            // An arrow flies to its collector too, then goes.
            self.add_pickup(
                super::super::pickups::Taken::Arrow(Box::new(shot)),
                collector,
            );
        } else if let Some(i) = self.tridents.iter().position(|t| t.id == p.item as u32) {
            // So does a trident: a server's client keeps them apart.
            let trident = self.tridents.remove(i);
            self.add_pickup(
                super::super::pickups::Taken::Trident(Box::new(trident)),
                collector,
            );
        }
        // An orb stays: the server may still hold merged ones in it.
        if !orb {
            self.remove_entity(p.item);
        }
    }

    /// `C::ProjectilePower`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_projectile_power_packet(&mut self, p: play::ProjectilePower) {
        // ClientPacketListener.handleProjectilePowerPacket: the power a
        // hurting projectile accelerates by, sent bundled with its motion.
        super::super::remote_projectiles::remote_projectile_power(
            &mut self.creatures.projectiles,
            p.id,
            p.power,
        );
    }

    /// `C::SwingAnimation`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_swing_animation_packet(&mut self, p: play::SwingAnimation) {
        // ClientPacketListener.handleSwingAnimation: the entity swings
        // (LivingEntity.swing, which a swing in its first half ignores).
        let hand = if p.hand == 1 { Hand::Off } else { Hand::Main };
        let animation = sunrise_client::first_person::SwingAnimation {
            kind: p.kind.clamp(0, 255) as u8,
            duration: p.duration,
        };
        if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) {
            self.swing_hand(hand, animation);
        } else if let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32) {
            m.swing_hand_modified(hand == Hand::Off, animation);
        }
    }

    /// `C::HurtAnimation`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_hurt_animation_packet(&mut self, p: play::HurtAnimation) {
        if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) {
            // Player.animateHurt: the tilt turns around the yaw.
            if let Some(pl) = self.player.as_mut() {
                pl.health.animate_hurt_dir(p.yaw);
            }
        } else if let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32) {
            m.hurt_time = 10;
        } else {
            self.remote_dragon_hurt(p.id);
        }
    }

    /// `C::RotateHead`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_rotate_head_packet(&mut self, p: play::RotateHead) {
        // ClientPacketListener.handleRotateMob: lerpHeadTo(yHeadRot, 3).
        if let Some(r) = self.remote.as_mut() {
            let yaw = sunrise_protocol::buf::unpack_degrees(p.head_yaw);
            r.head_lerps.insert(p.id, (yaw, 3));
        }
    }

    /// `C::TeleportEntity`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_teleport_entity_packet(&mut self, p: play::TeleportEntity) -> Result<()> {
        // handleTeleportEntity: setValuesFromPositionPacket counts a relative part
        // from where the entity is on this client (PositionMoveRotation.of).
        if self.driven_vehicle(p.id as u32).is_some() {
            use play::relative as rel;
            let Some((at, yaw, pitch, velocity)) = self.driven_vehicle(p.id as u32) else {
                return Ok(());
            };
            let pick = |bit: i32, current: f64, change: f64| {
                if p.relatives & bit != 0 {
                    current + change
                } else {
                    change
                }
            };
            let position = [
                pick(rel::X, at.x, p.position[0]),
                pick(rel::Y, at.y, p.position[1]),
                pick(rel::Z, at.z, p.position[2]),
            ];
            let turn = |bit: i32, current: f32, change: f32| {
                if p.relatives & bit != 0 {
                    current + change
                } else {
                    change
                }
            };
            let rotation = (
                turn(rel::Y_ROT, yaw, p.yaw),
                turn(rel::X_ROT, pitch, p.pitch),
            );
            let delta = Vec3::new(
                pick(rel::DELTA_X, velocity.x, p.delta[0]),
                pick(rel::DELTA_Y, velocity.y, p.delta[1]),
                pick(rel::DELTA_Z, velocity.z, p.delta[2]),
            );
            // The tracked endpoint stays the server's latest, for the relative
            // teleports that come once nobody here drives it.
            if let Some(t) = self.remote.as_mut().and_then(|r| r.entities.get_mut(&p.id)) {
                t.position = position;
            }
            self.teleport_driven_vehicle(position, rotation, delta, p.on_ground);
            return Ok(());
        }
        let base = self
            .remote
            .as_ref()
            .and_then(|r| r.entities.get(&p.id))
            .map_or([0.0; 3], |t| t.position);
        let rel = |bit: i32, i: usize| {
            if p.relatives & bit != 0 {
                base[i] + p.position[i]
            } else {
                p.position[i]
            }
        };
        let position = [
            rel(play::relative::X, 0),
            rel(play::relative::Y, 1),
            rel(play::relative::Z, 2),
        ];
        // setValuesFromPositionPacket: interpolated, unless too far.
        self.move_entity(
            p.id,
            Some(Path::Linear(position)),
            Some((p.yaw, p.pitch)),
            p.on_ground,
            MovePacket::Teleport,
        );
        self.entity_section_moved_id(p.id as u32);
        Ok(())
    }

    /// `C::SetEntityMotion`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_set_entity_motion_packet(
        &mut self,
        p: play::SetEntityMotion,
    ) -> Result<()> {
        // ClientPacketListener.handleSetEntityMotion: knockback on this player.
        if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) {
            if let Some(pl) = self.player.as_mut() {
                pl.velocity = Vec3::new(p.velocity[0], p.velocity[1], p.velocity[2]);
            }
            return Ok(());
        }
        // The same packet for another entity: Entity.lerpMotion.
        self.remote_motion(p.id, p.velocity);
        Ok(())
    }
}
