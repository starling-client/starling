//! Unified crosshair entity picking (`LocalPlayer.pick` and `ProjectileUtil.getEntityHitResult`).
#[path = "pick_sections.rs"]
pub(in crate::game) mod pick_sections;

use super::Game;
use super::pick_accumulator::{PickAccumulator, PickCandidate, pick_ray_segment};
use mod_api::{BlockPos, CrosshairTarget, EntityRef};
use sunrise_client::{items::ItemStack, save::uuid_text};
use sunrise_core::geometry::{Aabb, Vec3};

#[derive(Clone, Copy)]
pub(in crate::game) enum AttackRangePickResult {
    Entity(PickCandidate),
    Block(super::Hit),
    Miss,
}

#[derive(Clone, Copy)]
pub(in crate::game) enum CrosshairDecision {
    Entity(PickCandidate),
    Block(super::Hit),
    Miss,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CrosshairEntityHit {
    pub id: i32,
    pub distance: f64,
    pub point: [f64; 3],
    pub pickable: bool,
    pub suggest_uuid: bool,
}

pub(super) fn aabb_extents((lo, hi): ([f64; 3], [f64; 3])) -> Aabb {
    Aabb::new(
        Vec3::new(lo[0], lo[1], lo[2]),
        Vec3::new(hi[0], hi[1], hi[2]),
    )
}

pub(in crate::game) fn knot_bounds(block: [i32; 3]) -> Aabb {
    let knot = sunrise_client::mobs::leash::knot_state(block);
    let p = knot.position;
    let half = knot.width / 2.0;
    Aabb::new(
        Vec3::new(p.x - half, p.y, p.z - half),
        Vec3::new(p.x + half, p.y + knot.height, p.z + half),
    )
}

impl Game<'_> {
    pub(super) fn crosshair_entity_hit(&self) -> Option<CrosshairEntityHit> {
        if let Some(hit) = self.cached_entity_pick() {
            let (entity, distance, point) = hit?;
            return Some(CrosshairEntityHit {
                id: entity.id() as i32,
                distance,
                point,
                pickable: self.client_pick.entity_metadata.0,
                suggest_uuid: self.client_pick.entity_metadata.1,
            });
        }
        let hit = self.pick_entity_hit_unified()?;
        let is_synthetic_dragon_part =
            (super::dragon::SPECIAL..super::dragon::SPECIAL + 8).contains(&hit.id);
        let id = hit.id as i32;
        let pickable = !self.interactions.contains_key(&id)
            && !is_synthetic_dragon_part
            && Self::has_pick_result(self.level_entity(id));
        let suggest_uuid = !is_synthetic_dragon_part && self.resident_dragon_part(hit.id).is_none();
        Some(CrosshairEntityHit {
            id,
            distance: hit.distance_sq.sqrt(),
            point: hit.point,
            pickable,
            suggest_uuid,
        })
    }

    pub(in crate::game) fn crosshair_block_target(
        &self,
        hit: super::Hit,
    ) -> Option<CrosshairTarget> {
        use crate::game::api_values::TryIntoApi;
        Some(CrosshairTarget::Block {
            pos: BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
            state: self
                .world
                .block(hit.pos[0], hit.pos[1], hit.pos[2])
                .try_into_api()
                .ok()?,
            hit: hit.native(),
        })
    }

    /// The stored frame decision on a window; original during dispatch.
    pub(super) fn crosshair_target(&self) -> Option<CrosshairTarget> {
        if !self.native_window() || self.title.is_some() {
            return None;
        }
        self.player.as_ref()?;
        if self.uses_stored_pick() {
            return self.stored_client_pick();
        }
        match self.compute_crosshair_decision() {
            CrosshairDecision::Entity(hit) => Some(CrosshairTarget::Entity {
                entity: EntityRef::new(self.native_lifecycle.epoch()?, hit.id),
                point: hit.point,
            }),
            CrosshairDecision::Block(hit) => self.crosshair_block_target(hit),
            CrosshairDecision::Miss => None,
        }
    }

    pub(super) fn pick_entity_candidate_within(
        &mut self,
        max_distance: f64,
        max_distance_sq: f64,
    ) -> Option<CrosshairTarget> {
        let subscribed = mod_api::__private::has_scoped_hook(mod_api::HookId::CanRiderInteract);
        let mut pick = if subscribed {
            PickAccumulator::recording(
                max_distance_sq,
                std::mem::take(&mut self.client_pick.rider_candidates),
            )
        } else {
            PickAccumulator::new(max_distance_sq)
        };
        self.collect_pick_candidates_within(max_distance, &mut pick);
        if subscribed {
            self.storage_scope(|game, _| {
                game.client_pick.rider_candidates = pick.dispatch(game);
                ((), true)
            });
        }
        let hit = pick.hovered?;
        Some(CrosshairTarget::Entity {
            entity: EntityRef::new(self.native_lifecycle.epoch()?, hit.id),
            point: hit.point,
        })
    }

    /// Runs the unified `LocalPlayer.raycastHitResult` entity result for unstored consumers.
    pub(super) fn pick_entity_hit_unified(&self) -> Option<PickCandidate> {
        match self.compute_crosshair_decision() {
            CrosshairDecision::Entity(hit) => Some(hit),
            _ => None,
        }
    }

    /// Full unstored `LocalPlayer.raycastHitResult`: active `AttackRange` (keeping both entity and
    /// in-range block results), then `LocalPlayer.pick` with nominal miss search bound and strict
    /// `<` reach filters so an out-of-reach entity occluding a block resolves to `Miss`.
    pub(in crate::game) fn compute_crosshair_decision(&self) -> CrosshairDecision {
        if let Some(range) = self.active_attack_range() {
            match self.attack_range_pick_result(&range) {
                AttackRangePickResult::Entity(hit) => return CrosshairDecision::Entity(hit),
                AttackRangePickResult::Block(hit) => return CrosshairDecision::Block(hit),
                AttackRangePickResult::Miss => {}
            }
        }
        let Some((eye, dir)) = self.aim() else {
            return CrosshairDecision::Miss;
        };
        let from = Vec3::new(eye[0], eye[1], eye[2]);
        let max_distance = self.block_reach.max(self.entity_reach);
        let block_hit = if self.client_pick.ray.is_some() {
            None
        } else {
            self.pick_unfiltered()
        };
        let (entity_max_distance, search_max_distance_sq, block_distance_sq) = match block_hit {
            Some(block) => {
                let d2 = from.distance_squared(Vec3::new(
                    block.point[0],
                    block.point[1],
                    block.point[2],
                ));
                (d2.sqrt(), d2, d2)
            }
            None => (
                max_distance,
                max_distance * max_distance,
                pick_ray_segment(eye, dir, max_distance)
                    .1
                    .distance_squared(from),
            ),
        };
        let mut pick = PickAccumulator::new(search_max_distance_sq);
        self.collect_pick_candidates_within(entity_max_distance, &mut pick);
        if let Some(hit) = pick.hovered
            && hit.distance_sq < block_distance_sq
        {
            return if hit.distance_sq < self.entity_reach * self.entity_reach {
                CrosshairDecision::Entity(hit)
            } else {
                CrosshairDecision::Miss
            };
        }
        if let Some(block) = block_hit
            && from.distance_squared(Vec3::new(block.point[0], block.point[1], block.point[2]))
                < self.block_reach * self.block_reach
        {
            CrosshairDecision::Block(block)
        } else {
            CrosshairDecision::Miss
        }
    }

    /// Single pickable-entity query matching `LocalPlayer.pick` and `Level.getEntities`:
    /// sweeps the camera entity's current bounding box, visits accessible sections in
    /// `EntitySectionStorage` order (filtering by raw box), then appends dragon parts.
    pub(super) fn collect_pick_candidates_within(
        &self,
        max_distance: f64,
        pick: &mut PickAccumulator,
    ) {
        let Some((eye, dir)) = self.aim() else {
            return;
        };
        let (from, to) = pick_ray_segment(eye, dir, max_distance);
        let search_box = self
            .pick_camera_bounds()
            .unwrap_or_else(|| Aabb::new(from, from))
            .expand_towards(Vec3::new(
                dir[0] * max_distance,
                dir[1] * max_distance,
                dir[2] * max_distance,
            ))
            .inflate(Vec3::new(1.0, 1.0, 1.0));
        self.for_each_pickable_in_bounds(search_box, |id, raw_bb, radius| {
            let bb = if radius == 0.0 {
                raw_bb
            } else {
                raw_bb.inflate(Vec3::new(radius, radius, radius))
            };
            pick.consider_box(self, id, from, to, bb);
        });
    }

    /// Entity under the crosshair before any block (`Some(None)` when `getPickResult` is empty).
    pub(super) fn pick_entity_hit_at(&self) -> Option<(f64, Option<i32>)> {
        self.crosshair_entity_hit()
            .map(|hit| (hit.distance, hit.pickable.then_some(hit.id)))
    }
    pub(super) fn pick_entity_hit(&self) -> Option<Option<i32>> {
        self.pick_entity_hit_at().map(|hit| hit.1)
    }
    /// Returns the wire entity id under the crosshair before any block.
    #[cfg(test)]
    pub(in crate::game) fn crosshair_entity_id(&self) -> Option<i32> {
        self.crosshair_entity_hit().map(|hit| hit.id)
    }

    /// `ClientSuggestionProvider.getSelectedEntities`: UUID of the crosshair entity hit.
    pub(super) fn crosshair_entity_uuid(&self) -> Option<String> {
        self.crosshair_entity_hit()
            .filter(|hit| hit.suggest_uuid)
            .and_then(|hit| self.persist.uuids.get(&(hit.id as u32)).copied())
            .map(uuid_text)
    }

    /// `Entity.getPickResult` of the entity `id`.
    pub(super) fn pick_result_of(&self, id: i32) -> Option<ItemStack> {
        let id = id as u32;
        // EnderDragonPart.getPickResult delegates to its owner's Mob.getPickResult.
        let id = self
            .resident_dragon_part(id)
            .map_or(id, |(dragon, _)| dragon.id);
        let mob_kind = self
            .mobs
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.kind.id.as_str())
            .or_else(|| {
                matches!(
                    self.level_entity(id as i32),
                    Some(super::commands::Target::Dragon(_))
                )
                .then_some("minecraft:ender_dragon")
            });
        let item = if let Some(kind) = mob_kind {
            // Mob.getPickResult: SpawnEggItem.byId.
            let egg = format!("{kind}_spawn_egg");
            return self.items().get(&egg).map(|_| ItemStack::new(&egg, 1));
        } else if let Some(b) = self.boats.iter().find(|b| b.id == id) {
            b.entity_type()
        } else if let Some(c) = self.carts.iter().find(|c| c.id == id) {
            c.kind.item().to_owned()
        } else if self.armor_stands.iter().any(|a| a.id == id) {
            "minecraft:armor_stand".to_owned()
        } else if let Some(c) = self.cushions.iter().find(|c| c.id == id) {
            c.item()
        } else if let Some(f) = self.item_frames.iter().find(|f| f.id == id) {
            // ItemFrame.getPickResult: what it frames, else the frame itself.
            return Some(
                f.item
                    .clone()
                    .unwrap_or_else(|| ItemStack::new(f.kind(), 1)),
            );
        } else if self.paintings.iter().any(|p| p.id == id) {
            "minecraft:painting".to_owned()
        } else if self.crystals.iter().any(|c| c.id == id) {
            "minecraft:end_crystal".to_owned()
        } else if self.pick_knot_fence(id).is_some() {
            // LeashFenceKnotEntity.getPickResult.
            "minecraft:lead".to_owned()
        } else {
            return None;
        };
        Some(ItemStack::new(&item, 1))
    }

    /// The fence of a natural knot or an explicitly summoned/loaded leash knot.
    fn pick_knot_fence(&self, id: u32) -> Option<[i32; 3]> {
        self.natural_knot(id).or_else(|| {
            let knot = self
                .others
                .iter()
                .find(|o| o.id == id && o.knot_check.is_some())?;
            if self.level_entity(id as i32) != Some(super::commands::Target::Other(id)) {
                return None;
            }
            let block = sunrise_core::position::BlockPos::containing(knot.position);
            Some([block.x, block.y, block.z])
        })
    }

    /// `Player.isWithinEntityInteractionRange(entity, 3.0)`.
    pub(super) fn within_entity_range(&self, id: i32) -> bool {
        let Some(eye) = self.eye_position() else {
            return false;
        };
        let Some(b) = self
            .object_target(id)
            .map(|(lo, hi, _)| (lo, hi))
            .or_else(|| {
                let uid = id as u32;
                let bounds = self
                    .mobs
                    .iter()
                    .find(|m| m.id == uid)
                    .map(|m| m.bounds())
                    .or_else(|| self.tracked_special_box(id))
                    .or_else(|| self.pick_knot_fence(uid).map(knot_bounds))?;
                let (lo, hi) = (bounds.min_position(), bounds.max_position());
                Some(([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]))
            })
        else {
            return false;
        };
        let d2: f64 = (0..3)
            .map(|a| (eye[a] - eye[a].clamp(b.0[a], b.1[a])).powi(2))
            .sum();
        let range = self.entity_reach + 3.0;
        d2 < range * range
    }
}

#[cfg(test)]
mod tests {
    use super::Game;
    use crate::game::commands::Target;
    use crate::game::creatures::SpawnSite;
    use crate::game::dragon::dragon_part_id;
    use crate::game::native_entity_refs::tests::with_game;
    use sunrise_client::{items::ItemStack, motion::PlayerMotion};
    use sunrise_core::geometry::Vec3;

    #[test]
    fn tnt_and_falling_blocks_have_no_entity_pick_result() {
        assert!(!Game::has_pick_result(Some(Target::Tnt(10))));
        assert!(!Game::has_pick_result(Some(Target::Falling(20))));
        assert!(!Game::has_pick_result(Some(Target::Projectile(30))));
        assert!(!Game::has_pick_result(Some(Target::DragonPart(40))));
        assert!(Game::has_pick_result(Some(Target::Boat(50))));
        assert!(Game::has_pick_result(Some(Target::Mob(60))));
    }

    /// `EntityRef::pick_result` gives a mod what `Entity.getPickResult` gives:
    /// a mob's spawn egg, an armor stand, a boat, an item frame's item, and
    /// nothing for a projectile or the player.
    #[test]
    #[ignore = "daily"]
    fn an_entitys_pick_result_reads_through_its_handle() {
        crate::game::in_own_process(
            "game::pick_entity::tests::an_entitys_pick_result_reads_through_its_handle",
            || {
                with_game(|game| {
                    game.player =
                        Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 65.0, 0.5)).unwrap());
                    crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
                    // The summoned entities resolve in a loaded chunk.
                    game.world.chunks.insert(
                        (0, 0),
                        std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
                    );
                    game.block_ticks
                        .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
                        .unwrap();
                    let mut summon = |kind: &str, nbt: &str| match game
                        .summon_with_nbt(kind, [2.5, 65.0, 2.5], nbt)
                        .unwrap()
                    {
                        Some(
                            Target::Mob(id)
                            | Target::Stand(id)
                            | Target::Boat(id)
                            | Target::Frame(id)
                            | Target::Projectile(id),
                        ) => id,
                        other => panic!("{kind} summoned as {other:?}"),
                    };
                    let cases = [
                        (
                            summon("minecraft:zombie", "{}"),
                            Some("minecraft:zombie_spawn_egg"),
                        ),
                        (
                            summon("minecraft:armor_stand", "{}"),
                            Some("minecraft:armor_stand"),
                        ),
                        (
                            summon("minecraft:oak_boat", "{}"),
                            Some("minecraft:oak_boat"),
                        ),
                        (
                            summon(
                                "minecraft:item_frame",
                                r#"{Item:{id:"minecraft:diamond",count:1}}"#,
                            ),
                            Some("minecraft:diamond"),
                        ),
                        (summon("minecraft:arrow", "{}"), None),
                        (20, None),
                    ];
                    for (id, want) in cases {
                        let entity = game
                            .entity(id)
                            .unwrap_or_else(|| panic!("no handle for {id}"));
                        let picked = entity.pick_result(game).map(|stack| {
                            mod_api::__private::item_key_id(stack.item())
                                .unwrap()
                                .to_owned()
                        });
                        assert_eq!(picked.as_deref(), want, "entity {id}");
                    }
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn server_entity_pick_returns_lead_and_delegates_dragon_parts() {
        crate::game::in_own_process(
            "game::pick_entity::tests::server_entity_pick_returns_lead_and_delegates_dragon_parts",
            || {
                with_game(|game| {
                    game.player =
                        Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 65.0, 0.5)).unwrap());
                    let knot = game.tie_knot([0, 65, 2]);
                    let far_knot = game.tie_knot([0, 65, 10]);
                    let Some(Target::Other(explicit_knot)) = game
                        .summon_with_nbt("minecraft:leash_knot", [1.5, 65.0, 2.5], "{}")
                        .unwrap()
                    else {
                        panic!("expected an explicit leash knot");
                    };
                    for id in [knot, explicit_knot] {
                        assert!(
                            game.within_entity_range(id as i32),
                            "knot {id} is reachable"
                        );
                        assert_eq!(
                            game.pick_result_of(id as i32),
                            Some(ItemStack::new("minecraft:lead", 1))
                        );
                    }
                    assert!(
                        !game.within_entity_range(far_knot as i32),
                        "far knot outside reach+3 must fail server reach"
                    );
                    // Cross the signed i32 boundary so later dragon parts carry negative wire IDs.
                    game.next_entity = (i32::MAX as u32) - 3;
                    let dragon = game
                        .spawn_dragon_facing([0.5, 65.0, 3.5], SpawnSite::Plain, Some(0.0))
                        .unwrap();
                    let parent_result = game.pick_result_of(dragon as i32);
                    assert_eq!(
                        parent_result,
                        Some(ItemStack::new("minecraft:ender_dragon_spawn_egg", 1))
                    );
                    let body_part_id = dragon_part_id(dragon, 2) as i32;
                    assert!(body_part_id < 0, "expected negative signed part wire ID");
                    assert!(game.within_entity_range(body_part_id));
                    for part in 0..8 {
                        assert_eq!(
                            game.pick_result_of(dragon_part_id(dragon, part) as i32),
                            parent_result
                        );
                    }

                    // Exercise the real PickItemFromEntity server packet handler through a creative guest.
                    let (mut hub, _rx, _other) =
                        crate::game::lan_particles::tests::test_hub("PickEntity", Vec::new(), None);
                    let gi = hub.guests.connection_index(1).unwrap();
                    hub.guests.update(gi, |g| {
                        g.game_mode = 1;
                        g.position = [0.5, 65.0, 0.5];
                        g.placed = true;
                        g.dimension = "minecraft:overworld".into();
                        g.inventory[0] = None;
                        g.selected = 0;
                    });
                    game.lan = Some(crate::game::lan::LanHost::for_test(hub));

                    for (id, expected) in [
                        (far_knot as i32, None),
                        (knot as i32, Some("minecraft:lead")),
                        (explicit_knot as i32, Some("minecraft:lead")),
                        (body_part_id, Some("minecraft:ender_dragon_spawn_egg")),
                    ] {
                        game.lan.as_mut().unwrap().hub.guests.update(gi, |g| {
                            g.inventory[0] = None;
                            g.selected = 0;
                        });
                        game.lan_guest_packet(
                            1,
                            sunrise_protocol::packets::play::Serverbound::PickItemFromEntity(
                                sunrise_protocol::packets::play::PickItemFromEntity {
                                    id,
                                    include_data: false,
                                },
                            ),
                        );
                        let held = game.lan.as_ref().unwrap().hub.guests[gi].inventory[0]
                            .as_ref()
                            .map(|s| &*s.item);
                        assert_eq!(held, expected, "PickItemFromEntity({id})");
                    }
                });
            },
        );
    }
}
