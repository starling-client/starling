//! Entity-type key reads share the completed registry and never format a type name.
use super::{Game, commands::Target};
use mod_api::{EntityRef, EntityTypeKey, LevelReadError, PushReaction};

impl Game<'_> {
    pub(super) fn native_entity_kind_key(&self, target: Target) -> Option<EntityTypeKey> {
        mod_api::__private::resolved_entity_type_key(self.native_entity_kind_name(target)?)
    }

    fn native_entity_kind_name(&self, target: Target) -> Option<&str> {
        Some(match target {
            Target::Player | Target::Guest(_) => "minecraft:player",
            Target::Mob(id) => &self.mobs.iter().find(|mob| mob.id == id)?.kind.id,
            Target::Item(_) => "minecraft:item",
            Target::Orb(_) => "minecraft:experience_orb",
            Target::Projectile(id) => self
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == id)?
                .entity_type(),
            Target::Stand(_) => "minecraft:armor_stand",
            Target::General(id) => self.general_kind(id)?,
            Target::Frame(id) => {
                if self
                    .item_frames
                    .iter()
                    .any(|frame| frame.id == id && frame.glow)
                {
                    "minecraft:glow_item_frame"
                } else {
                    "minecraft:item_frame"
                }
            }
            Target::Painting(_) => "minecraft:painting",
            Target::Boat(id) => self.boats.iter().find(|b| b.id == id)?.entity_type_name(),
            Target::Cart(id) => self.carts.iter().find(|cart| cart.id == id)?.kind.id(),
            Target::Other(id) => &self.others.iter().find(|other| other.id == id)?.kind,
            Target::Crystal(_) => "minecraft:end_crystal",
            Target::Dragon(_) | Target::DragonPart(_) => "minecraft:ender_dragon",
            Target::Tnt(_) => "minecraft:tnt",
            Target::Cushion(_) => "minecraft:cushion",
            Target::Falling(_) => "minecraft:falling_block",
            Target::Rocket(_) => "minecraft:firework_rocket",
            Target::Trident(_) => "minecraft:trident",
            Target::Fangs(_) => "minecraft:evoker_fangs",
            Target::Spawner(_) => "minecraft:ominous_item_spawner",
            Target::Cloud(_) => "minecraft:area_effect_cloud",
            Target::Hook(_) => "minecraft:fishing_bobber",
            Target::Knot(_) => "minecraft:leash_knot",
        })
    }

    /// `Entity.getPistonPushReaction`; only the armor stand's override depends on instance state.
    pub(super) fn entity_piston_push_reaction(
        &self,
        entity: EntityRef,
    ) -> Result<Option<PushReaction>, LevelReadError> {
        use super::api_values::api_push_reaction;
        use super::entity::{Entity, with_entity};
        use super::general::general_type_catalog;

        let target = self.native_entity_read_target(entity);
        if let Some(handle) = target.and_then(|_| self.native_walk_entity(entity.id())) {
            let reaction = with_entity!(self, handle, row => row.get_piston_push_reaction());
            return Ok(Some(api_push_reaction(reaction)));
        }
        if let Some(row) = self.native_client_general(entity) {
            // A tracked general row has no Entity body; both sides use its declared type.
            let reaction = general_type_catalog().piston_push_reaction(row.type_slot);
            return Ok(Some(api_push_reaction(reaction)));
        }
        if target.is_none()
            && self.native_lifecycle.epoch() == Some(entity.epoch())
            && let Some((_, mob)) = self.mob_in_other_level(entity.id())
        {
            return Ok(Some(api_push_reaction(mob.get_piston_push_reaction())));
        }
        Ok(self
            .native_kind_for_physical_read(entity, target)
            .map(|kind| match kind {
                // These kinds have no Entity body for the engine's piston to push.
                "minecraft:marker"
                | "minecraft:interaction"
                | "minecraft:block_display"
                | "minecraft:item_display"
                | "minecraft:text_display"
                | "minecraft:area_effect_cloud"
                | "minecraft:ominous_item_spawner" => PushReaction::IgnoreEntity,
                _ => PushReaction::PushPull,
            }))
    }

    pub(super) fn entity_is_hanging(
        &self,
        entity: EntityRef,
    ) -> Result<Option<bool>, LevelReadError> {
        Ok(self
            .native_kind_for_physical_read(entity, self.native_entity_read_target(entity))
            .map(|kind| {
                matches!(
                    kind,
                    "minecraft:painting" | "minecraft:item_frame" | "minecraft:glow_item_frame"
                )
            }))
    }

    fn native_kind_for_physical_read(
        &self,
        entity: EntityRef,
        target: Option<Target>,
    ) -> Option<&str> {
        match target {
            Some(target) => self.native_entity_kind_name(target),
            None => self
                .native_client_tracked(entity)
                .map(|(kind, _)| kind)
                .or_else(|| {
                    (self.native_lifecycle.epoch() == Some(entity.epoch())).then_some(())?;
                    self.mob_in_other_level(entity.id())
                        .map(|(_, mob)| mob.kind.id.as_str())
                }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::native_entity_refs::tests::{with_client_game, with_game};
    use sunrise_client::entities::ArmorStand;
    use sunrise_client::mobs::Mob;
    use sunrise_core::geometry::Vec3;

    #[test]
    #[ignore = "daily"]
    fn piston_reaction_and_hanging_class_follow_server_and_client_entities() {
        use mod_api::PushReaction::{IgnoreEntity, PushPull};

        let read = |game: &Game<'_>, id| {
            let entity = game.entity(id).unwrap();
            (
                entity.piston_push_reaction(game).unwrap().unwrap(),
                entity.is_hanging(game).unwrap().unwrap(),
            )
        };
        with_game(|game| {
            game.build_registries();
            let pig_ty = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(10, pig_ty, Vec3::ZERO, 0.0));
            let mut stand = ArmorStand::new(11, Vec3::ZERO, 0.0);
            stand.marker = true;
            game.armor_stands.push(stand);
            assert!(game.as_console(|game| {
                game.run_command("summon minecraft:item_frame 0 64 0 {Facing:2b,Fixed:1b}")
            }));
            assert_eq!(read(game, 10), (PushPull, false));
            assert_eq!(read(game, 11), (IgnoreEntity, false));
            assert_eq!(read(game, game.item_frames[0].id), (PushPull, true));
        });
        with_client_game(|game| {
            let (client, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "reaction").unwrap();
            game.remote.as_mut().unwrap().logged_in = true;
            use sunrise_protocol::packets::play::{self, Clientbound as C};

            for (id, key) in [
                (10, "minecraft:pig"),
                (11, "minecraft:armor_stand"),
                (12, "minecraft:item_frame"),
                (13, "minecraft:leash_knot"),
                (14, "minecraft:block_display"),
            ] {
                let kind = game
                    .remote
                    .as_ref()
                    .unwrap()
                    .builtin
                    .get("minecraft:entity_type")
                    .unwrap()
                    .id(key)
                    .unwrap();
                game.handle_packet(C::AddEntity(play::AddEntity {
                    id,
                    uuid: id as u128,
                    kind,
                    position: [0.0; 3],
                    velocity: [0.0; 3],
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: 0,
                }))
                .unwrap();
            }
            assert_eq!(read(game, 11), (PushPull, false));
            game.handle_packet(C::SetEntityData(play::SetEntityData {
                id: 11,
                values: vec![(15, 0, sunrise_protocol::entity_data::Value::Byte(16))],
            }))
            .unwrap();
            assert_eq!(read(game, 10), (PushPull, false));
            assert_eq!(read(game, 11), (IgnoreEntity, false));
            assert_eq!(read(game, 12), (PushPull, true));
            assert_eq!(read(game, 13), (PushPull, false));
            assert_eq!(read(game, 14), (IgnoreEntity, false));

            let stale = EntityRef::new(mod_api::WorldEpoch::new(0), 11);
            assert_eq!(stale.piston_push_reaction(game), Ok(None));
            assert_eq!(stale.is_hanging(game), Ok(None));
        });
    }
}
