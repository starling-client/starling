use super::{Game, to_loot};

impl Game<'_> {
    /// The killing player as a loot entity: their worn and held items, read
    /// from the guest who struck the last blow when a server's player did
    /// (on the server thread that is every player, the window's too), so
    /// Looting on their weapon counts.
    pub(in crate::game) fn killer_loot_entity(
        &mut self,
        guest: Option<u32>,
    ) -> sunrise_world::loot::LootEntity {
        let capture_nbt = self
            .server_loot
            .get()
            .is_some_and(|data| data.copies_entity_nbt());
        let own = |g: &mut Self| {
            let mut e = sunrise_world::loot::LootEntity::new("minecraft:player");
            let slots = [
                ("mainhand", g.stack_in(g.selected)),
                ("offhand", g.stack_in(crate::game::shield::OFFHAND_SLOT)),
                ("head", g.armor[0].as_ref()),
                ("chest", g.armor[1].as_ref()),
                ("legs", g.armor[2].as_ref()),
                ("feet", g.armor[3].as_ref()),
            ];
            for (slot, stack) in slots {
                if let Some(stack) = stack {
                    e.equipment.push((slot.into(), to_loot(stack)));
                }
            }
            if capture_nbt {
                e.nbt = g
                    .command_target_nbt(crate::game::commands::Target::Player)
                    .map(std::sync::Arc::new);
            }
            e
        };
        if let Some(conn) = guest.filter(|_| self.lan_acting.is_none())
            && let Some(entity) = self.as_guest(conn, |g| own(g))
        {
            return entity;
        }
        own(self)
    }
    /// The acting player as `INTERACTING_ENTITY`: [`Self::killer_loot_entity`]
    /// with the fire and vehicle state the loot predicates test.
    pub(in crate::game) fn interacting_loot_entity(&mut self) -> sunrise_world::loot::LootEntity {
        let mut entity = self.killer_loot_entity(None);
        entity.on_fire = self
            .player
            .as_ref()
            .is_some_and(sunrise_client::motion::PlayerMotion::is_on_fire);
        entity.vehicle = self
            .riding
            .and_then(|id| self.vehicle_loot_entity(id))
            .map(Box::new);
        entity
    }
    /// The vehicle `id` (`Entity.getVehicle`) as loot predicates read it: a
    /// mob with all its facts and its own vehicle, a boat or a minecart (the
    /// rideable one sets the player's riding link) by type and fire. Neither
    /// is a `LivingEntity`, so no age, equipment or mob state is read from it.
    fn vehicle_loot_entity(&self, id: u32) -> Option<sunrise_world::loot::LootEntity> {
        use sunrise_world::loot::LootEntity;
        if let Some(mob) = self.mobs.iter().find(|mob| mob.id == id) {
            let vehicle = mob
                .extra
                .mount
                .and_then(|mount| self.vehicle_loot_entity(mount));
            return Some(sunrise_client::mobs::drops::loot_entity(mob, vehicle));
        }
        if let Some(boat) = self.boats.iter().find(|boat| boat.id == id) {
            return Some(LootEntity {
                kind: boat.entity_type(),
                on_fire: boat.fire_ticks > 0,
                ..LootEntity::default()
            });
        }
        // A minecart has no `baseTick`, so nothing lights it.
        self.carts
            .iter()
            .find(|cart| cart.id == id)
            .map(|cart| LootEntity {
                kind: cart.kind.id().into(),
                ..LootEntity::default()
            })
    }
    /// `Player.getLuck`: the luck attribute (the Luck and Bad Luck effects).
    pub(in crate::game) fn acting_luck(&mut self) -> f32 {
        self.player
            .as_mut()
            .and_then(|p| p.attribute("minecraft:luck").ok())
            .unwrap_or(0.0) as f32
    }
}
