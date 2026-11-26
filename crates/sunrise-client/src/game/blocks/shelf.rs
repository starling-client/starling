//! The displayed items of `ShelfBlockEntity`.
use super::{
    Game, Hit,
    item_draw::{BlockItemDraw, block_entity_pose},
    machines::dir_from_name,
};
use sunrise_assets::item_model::DisplayContext;
use sunrise_client::{
    block_entity_render::vertex_light, camera::rotate_degrees, containers::BlockEntity,
    mesher::Vertex,
};
use sunrise_client_render::block_entity_reach::Reach;
use sunrise_core::{
    geometry::{Direction, Vec3},
    position::BlockPos,
};
use sunrise_world::level::hit_slot;

fn accepts(limit: Option<u32>, stack: Option<&sunrise_client::items::ItemStack>) -> bool {
    limit.is_none_or(|limit| stack.is_none_or(|stack| stack.count <= limit))
}

impl Game<'_> {
    /// Slot and facing for a click on a shelf's front face.
    pub(super) fn shelf_hit(&self, hit: &Hit) -> Option<(usize, Direction, bool)> {
        let p = hit.pos;
        if !matches!(
            self.world.block_entities.get(&(p[0], p[1], p[2])),
            Some(BlockEntity::Shelf(_))
        ) {
            return None;
        }
        let state = self.world.block(p[0], p[1], p[2]);
        let data = self.pack.assets.registry.state(state)?;
        let facing = data
            .properties
            .get("facing")
            .and_then(|s| dir_from_name(s))?;
        let face = Direction::ALL
            .into_iter()
            .find(|d| d.normal() == hit.face)?;
        let slot = hit_slot(
            facing,
            face,
            BlockPos::new(p[0], p[1], p[2]),
            Vec3::new(hit.point[0], hit.point[1], hit.point[2]),
            1,
            3,
        )?;
        Some((
            slot,
            facing,
            data.properties.get("powered").is_some_and(|s| s == "true"),
        ))
    }

    /// `SideChainPartBlock.getAllBlocksConnectedTo`: left to right, at most three.
    fn shelf_chain(&self, pos: [i32; 3], facing: Direction) -> Vec<[i32; 3]> {
        let left = match facing {
            Direction::North => Direction::East,
            Direction::East => Direction::South,
            Direction::South => Direction::West,
            Direction::West => Direction::North,
            _ => return Vec::new(),
        };
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let behaviours = self.res.behaviours();
        if !behaviours
            .tag("minecraft:wooden_shelves")
            .is_some_and(|tag| tag.contains(behaviours.block(state)))
        {
            return Vec::new();
        }
        let mut result = vec![pos];
        for (dir, end, prepend) in [(left, "left", true), (left.opposite(), "right", false)] {
            let normal = dir.normal();
            for step in 1..3 {
                let p = [pos[0] + normal[0] * step, pos[1], pos[2] + normal[2] * step];
                let state = self.world.block(p[0], p[1], p[2]);
                let Some(info) = self.pack.assets.registry.state(state) else {
                    break;
                };
                let behaviours = self.res.behaviours();
                let connectable = behaviours
                    .tag("minecraft:wooden_shelves")
                    .is_some_and(|tag| tag.contains(behaviours.block(state)))
                    && info.properties.get("powered").is_some_and(|s| s == "true")
                    && info.properties.get("facing").and_then(|s| dir_from_name(s)) == Some(facing);
                if !connectable {
                    break;
                }
                let part = info
                    .properties
                    .get("side_chain")
                    .map(String::as_str)
                    .unwrap_or("unconnected");
                if part == "center" || part == end {
                    if prepend {
                        result.insert(0, p);
                    } else {
                        result.push(p);
                    }
                }
                if part != "center" {
                    break;
                }
            }
        }
        result
    }

    /// `ShelfBlock.useItemOn`: a single swap, or a connected hotbar swap while powered.
    pub(super) fn use_shelf(&mut self, hit: &Hit) -> Option<bool> {
        use mod_api::client::first_person::Hand;
        if self.hand != Hand::Main {
            return None;
        }
        let (slot, facing, powered) = self.shelf_hit(hit)?;
        let pos = hit.pos;
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if powered {
            let chain = self.shelf_chain(pos, facing);
            let mut swapped = false;
            let count = chain.len();
            for (i, p) in chain.into_iter().enumerate() {
                let key = (p[0], p[1], p[2]);
                let limit = self.world.container_stack_limit(key);
                let Some(BlockEntity::Shelf(shelf)) = self.world.block_entities.get_mut(&key)
                else {
                    continue;
                };
                for n in 0..3 {
                    let index = 9_i32 - (count as i32 - i as i32) * 3 + n as i32;
                    if !(0..9).contains(&index) {
                        continue;
                    }
                    let index = index as usize;
                    if !accepts(limit, self.inventory[index].as_ref()) {
                        continue;
                    }
                    swapped |= self.inventory[index].is_some() || shelf.items[n].is_some();
                    std::mem::swap(&mut self.inventory[index], &mut shelf.items[n]);
                }
                self.container_changed(key);
                let s = self.world.block(p[0], p[1], p[2]);
                self.post_block_game_event("minecraft:entity_interact", p, None, Some(s));
            }
            if !swapped {
                return Some(false);
            }
            self.play_audio(
                "minecraft:block.shelf.multi_swap",
                Some(pos.map(|n| f64::from(n) + 0.5)),
                1.0,
                1.0,
            );
            Some(true)
        } else {
            let held = self.inventory[self.selected].clone();
            let key = (pos[0], pos[1], pos[2]);
            if !accepts(self.world.container_stack_limit(key), held.as_ref()) {
                return Some(false);
            }
            let Some(BlockEntity::Shelf(shelf)) = self.world.block_entities.get_mut(&key) else {
                return None;
            };
            let removed = std::mem::replace(&mut shelf.items[slot], held.clone());
            self.inventory[self.selected] = if self.creative && removed.is_none() {
                held.clone()
            } else {
                removed.clone()
            };
            self.container_changed(key);
            let vibrations = self.inventory[self.selected].as_ref().is_none_or(|stack| {
                let default = self
                    .res
                    .items()
                    .get(&stack.item)
                    .is_none_or(|item| item.interact_vibrations);
                let Some(data) = stack.data() else {
                    return default;
                };
                if data
                    .preserved_components()
                    .iter()
                    .any(|(key, _)| key == "!minecraft:use_effects")
                {
                    return true;
                }
                data.preserved_components()
                    .iter()
                    .find(|(key, _)| key == "minecraft:use_effects")
                    .map_or(default, |(_, tag)| {
                        sunrise_client::save::number(tag.get("interact_vibrations")).unwrap_or(1)
                            != 0
                    })
            });
            if vibrations {
                self.post_block_game_event(
                    "minecraft:item_interact_finish",
                    pos,
                    None,
                    Some(state),
                );
            }
            let sound = match (held.is_some(), removed.is_some()) {
                (false, false) => return None,
                (true, false) => "minecraft:block.shelf.place_item",
                (false, true) => "minecraft:block.shelf.take_item",
                (true, true) => "minecraft:block.shelf.single_swap",
            };
            self.play_audio(sound, Some(pos.map(|n| f64::from(n) + 0.5)), 1.0, 1.0);
            Some(true)
        }
    }

    /// `ShelfRenderer`: three items face out, aligned to the bottom or centre.
    pub(super) fn shelf_geometry(&self, camera: glam::DVec3, out: &mut Vec<Vertex>) {
        self.shelf_items(camera, |d| {
            self.submit_item_world(d.state, d.pose, d.light, out);
        });
    }

    /// The items [`Self::shelf_geometry`] draws, slot by slot.
    pub(super) fn shelf_items(&self, camera: glam::DVec3, mut draw: impl FnMut(BlockItemDraw)) {
        let _items = sunrise_client::spans::raw_flight_guard("block_entity_items");
        let reach = Reach::Default;
        sunrise_client::spans::note_block_entity_item_visits(|| {
            self.world.block_entities.len() as u64
        });
        for (key, entity) in &self.world.block_entities {
            let BlockEntity::Shelf(shelf) = entity else {
                continue;
            };
            if !reach.should_render([key.0, key.1, key.2], camera, 0) {
                continue;
            }
            let block = self.world.block(key.0, key.1, key.2);
            let facing = self
                .pack
                .assets
                .registry
                .state(block)
                .and_then(|s| s.properties.get("facing"))
                .map(String::as_str)
                .unwrap_or("north");
            // `-Direction.toYRot()` of a horizontal facing (south's is -0), else 180.
            let yaw: f32 = match facing {
                "south" => -0.0,
                "west" => -90.0,
                "north" => -180.0,
                "east" => -270.0,
                _ => 180.0,
            };
            let packed = BlockPos::new(key.0, key.1, key.2).pack() as u64;
            let seed = (packed ^ (packed >> 32)) as i32;
            let light = vertex_light(self.world.light(key.0, key.1, key.2));
            // `ShelfRenderer.extractRenderState` hands the items the shelf as their
            // `ItemOwner`, the only block entity renderer that does:
            // `ShelfBlockEntity.position()` is the block's centre and
            // `getVisualRotationYInDegrees()` the facing's opposite yaw, so a compass
            // on a shelf points from the shelf's block.
            let owner = super::item_draw::owner_at(
                [
                    f64::from(key.0) + 0.5,
                    f64::from(key.1) + 0.5,
                    f64::from(key.2) + 0.5,
                ],
                shelf_owner_yaw(facing),
                "",
            );
            for (slot, stack) in shelf.items.iter().enumerate() {
                let Some(stack) = stack else { continue };
                let seed = seed.wrapping_add(slot as i32);
                let item = self.resolve_item(stack, DisplayContext::OnShelf, Some(&owner), seed);
                if item.is_empty() {
                    continue;
                }
                // `ShelfRenderer.submitItem`; the model's bounds are an `AABB` of doubles.
                let (lo, hi) = item.bounding_box();
                let (lo, hi) = (f64::from(lo.y), f64::from(hi.y));
                let mut offset_y = -lo;
                if !shelf.align_to_bottom {
                    offset_y += -(hi - lo) / 2.0;
                }
                let mut pose = block_entity_pose(*key, camera);
                pose.translate(0.5, 0.5, 0.5);
                rotate_degrees(&mut pose, 1, yaw);
                pose.translate(
                    (slot as i32 - 1) as f32 * 0.3125,
                    if shelf.align_to_bottom { -0.25 } else { 0.0 },
                    -0.25,
                );
                pose.scale(0.25, 0.25, 0.25);
                pose.translate(0.0, offset_y as f32, 0.0);
                draw(BlockItemDraw {
                    #[cfg(test)]
                    stack,
                    #[cfg(test)]
                    display: DisplayContext::OnShelf,
                    #[cfg(test)]
                    seed,
                    state: &item,
                    pose: pose.to_glam(),
                    light,
                });
            }
        }
    }
}

#[cfg(test)]
mod stack_limit_tests {
    #[test]
    fn shelf_swap_refuses_an_oversized_offer_without_capping_unmodified_shelves() {
        let stack = sunrise_client::items::ItemStack::new("minecraft:stick", 99);
        assert!(super::accepts(None, Some(&stack)));
        assert!(!super::accepts(Some(10), Some(&stack)));
        assert!(super::accepts(Some(10), None));
        let stack = sunrise_client::items::ItemStack::new("minecraft:stick", 10);
        assert!(super::accepts(Some(10), Some(&stack)));
    }
}

/// `ShelfBlockEntity.getVisualRotationYInDegrees`: `facing.getOpposite().toYRot()`,
/// where `Direction.toYRot` is `(data2d & 3) * 90` and `data2d` is north 2, south 0,
/// west 1, east 3. A shelf's facing is always horizontal; anything else reads north.
fn shelf_owner_yaw(facing: &str) -> f32 {
    match facing {
        "south" => 180.0,
        "west" => 270.0,
        "east" => 90.0,
        _ => 0.0,
    }
}

#[cfg(test)]
#[path = "tests/shelf_compass_tests.rs"]
mod shelf_compass_tests;
