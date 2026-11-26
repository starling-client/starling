//! `ClientPacketListener.handleUpdateAttributes` for entity attributes that
//! affect the client-side nameplate.
use super::{Game, snapshot_value};
use sunrise_client::entities::nameplate::NameplateDistances;
use sunrise_protocol::packets::play;
use sunrise_world::attributes::entity::AttributeId;

impl Game<'_> {
    /// Returns a tracked remote entity's synced attribute or its vanilla type default.
    pub(in crate::game) fn remote_attribute(&self, id: i32, attribute: AttributeId) -> Option<f64> {
        let remote = self.remote.as_ref()?;
        let tracked = remote.entities.get(&id)?;
        if let Some(&value) = remote
            .synced_attributes
            .get(&id)
            .and_then(|attrs| attrs.get(&attribute))
        {
            return Some(value);
        }
        self.motion
            .attributes
            .default_base(&tracked.kind, attribute)
    }

    /// `LivingEntity.refreshDirtyAttributes`: clamp health to max health on living tick.
    #[cfg(test)]
    pub(in crate::game) fn refresh_remote_attributes(&mut self) {
        for m in &mut self.mobs {
            if m.extra.max_health_dirty {
                m.extra.max_health_dirty = false;
                m.health = m.health.min(m.attrs.max_health);
            }
        }
    }

    /// Computes one synced attribute using the catalog's vanilla range.
    fn remote_attribute_value(
        &self,
        name: &str,
        values: &[play::AttributeSnapshot],
    ) -> Option<f64> {
        let attribute = self.motion.attributes.id(name)?;
        let snapshot = values
            .iter()
            .find(|snapshot| snapshot.attribute == attribute.registry_id())?;
        Some(
            self.motion
                .attributes
                .attribute(attribute)
                .definition
                .sanitize(snapshot_value(snapshot)),
        )
    }

    /// The two attributes read by `LivingEntityRenderer` and
    /// `EntityRenderer.extractNameTags`.
    pub(super) fn remote_nameplate_values(
        &self,
        values: &[play::AttributeSnapshot],
    ) -> NameplateDistances {
        NameplateDistances {
            name_tag: self.remote_attribute_value("minecraft:name_tag_distance", values),
            below_name: self.remote_attribute_value("minecraft:below_name_distance", values),
        }
    }

    /// `ClientPacketListener.handleUpdateAttributes` for a mob. The same
    /// packet can target a player, mob or armor stand.
    pub(in crate::game) fn remote_mob_attributes(
        &mut self,
        id: i32,
        values: &[play::AttributeSnapshot],
    ) {
        if let Some(kind) = self
            .remote
            .as_ref()
            .and_then(|r| r.entities.get(&id))
            .map(|t| t.kind.clone())
        {
            let mut updates = Vec::new();
            for snapshot in values {
                if let Some((attr, reg)) = self
                    .motion
                    .attributes
                    .attributes()
                    .find(|(attr, _)| attr.registry_id() == snapshot.attribute)
                    && self.motion.attributes.default_base(&kind, attr).is_some()
                {
                    updates.push((attr, reg.definition.sanitize(snapshot_value(snapshot))));
                }
            }
            if !updates.is_empty()
                && let Some(r) = self.remote.as_mut()
            {
                let entry = r.synced_attributes.entry(id).or_default();
                for (attr, value) in updates {
                    entry.insert(attr, value);
                }
            }
        }
        let max_health = self.remote_attribute_value("minecraft:max_health", values);
        let armor = self.remote_attribute_value("minecraft:armor", values);
        let scale = self.remote_attribute_value("minecraft:scale", values);
        let camera_distance = self.remote_attribute_value("minecraft:camera_distance", values);
        let speed = self.remote_attribute_value("minecraft:movement_speed", values);
        let jump = self.remote_attribute_value("minecraft:jump_strength", values);
        let nameplate = self.remote_nameplate_values(values);
        let Some(m) = self.mob_index.get_mut(&mut self.mobs, id as u32) else {
            if let Some(a) = self.armor_stands.iter_mut().find(|a| a.id == id as u32) {
                if let Some(value) = nameplate.name_tag {
                    a.nameplate.name_tag = Some(value);
                }
                if let Some(value) = nameplate.below_name {
                    a.nameplate.below_name = Some(value);
                }
            }
            return;
        };
        if m.kind.id == "minecraft:player" {
            // A spectated RemotePlayer's FOV reads the synchronized attribute value,
            // already including any sprint modifier, not Mob::movement_speed().
            if let Some(speed) = speed {
                m.attrs.speed = speed;
            }
        } else if sunrise_client::mobs::equine::moved_by_rider(m.species()) {
            // A horse its rider's client moves goes at its own speed and jumps
            // its own height (`getSpeed`, `getCustomJump`).
            if let Some(speed) = speed {
                sunrise_client::mobs::finalize::set_base(
                    m,
                    sunrise_client::mobs::finalize::Attr::MovementSpeed,
                    speed,
                );
            }
            if let Some(jump) = jump {
                m.extra.vector[0] = jump;
            }
        }
        if let Some(max_health) = max_health {
            sunrise_client::mobs::finalize::set_base(
                m,
                sunrise_client::mobs::finalize::Attr::MaxHealth,
                max_health,
            );
            m.extra.max_health_dirty = true;
        }
        if let Some(armor) = armor {
            sunrise_client::mobs::finalize::set_base(
                m,
                sunrise_client::mobs::finalize::Attr::Armor,
                armor,
            );
        }
        if let Some(scale) = scale {
            m.extra.model_scale = scale as f32;
        }
        if let Some(value) = camera_distance {
            m.set_camera_distance(value);
        }
        if let Some(value) = nameplate.name_tag {
            m.set_name_tag_distance(value);
        }
        if let Some(value) = nameplate.below_name {
            m.set_below_name_distance(value);
        }
    }
}
