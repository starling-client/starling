//! Shape queries for a supplied state without replacing the block in the world.
use super::{Game, perspective};
use mod_api::BlockShapeKind;
use sunrise_client::{
    block_entity_anim::BlockEntityAnims, collisions::SceneCollisions, world::World,
};
use sunrise_core::{geometry::VoxelShape, position::BlockPos};
use sunrise_world::{
    block_collision::BlockCollisionRegistry,
    collision::ShapeChannel,
    collision_context::{CollisionContext, EntityContextKind},
    registry::BlockStateId,
};

/// A state's empty-context collision boxes at `pos`, block-local: a block entity's published
/// shape where its block publishes one, else the declared or vanilla shape. Collision errors
/// return no boxes. The one read behind `LevelRef::block_shape` and `LevelHost::collision_shape`.
pub(super) fn collision_boxes(
    collision: &BlockCollisionRegistry<'_>,
    world: &World,
    state: BlockStateId,
    pos: [i32; 3],
) -> Vec<[f64; 6]> {
    let mut scene = SceneCollisions {
        world,
        binding: collision,
        subject: None,
    };
    collision
        .collision_shape_with_context(
            &mut scene,
            state,
            BlockPos::new(pos[0], pos[1], pos[2]),
            &CollisionContext::empty(false),
        )
        .map(|shape| {
            shape
                .boxes()
                .iter()
                .map(|bounds| {
                    let min = bounds.min_position();
                    let max = bounds.max_position();
                    [min.x, min.y, min.z, max.x, max.y, max.z]
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The geometry chosen by selection, before its column offset is applied.
#[derive(Clone, Copy)]
pub(super) enum Outline<'a> {
    Published(&'a VoxelShape),
    Declared(&'a VoxelShape, EntityContextKind),
    Dynamic([f64; 6]),
    Default(&'a sunrise_client::blocks::StateFacts),
}

impl Game<'_> {
    pub(super) fn selected_outline<'a>(
        &'a self,
        world: &'a World,
        state: BlockStateId,
        anims: &BlockEntityAnims,
        pos: [i32; 3],
        player_context: bool,
    ) -> (Outline<'a>, [f64; 3]) {
        let facts = self.res.facts().get(state);
        let default_offset = || {
            perspective::outline_offset(
                &self.pack.context.tables,
                state,
                pos[0],
                pos[2],
                facts.outline_offset,
            )
        };
        if let Some(shape) =
            sunrise_client::collisions::published_outline(world, self.collision, state, pos)
        {
            // Published outlines already followed the template's outline offset before
            // declarations added their own channels.
            return (Outline::Published(shape), default_offset());
        }
        let cached = self.collision.cached_shapes();
        let kind = if player_context {
            self.outline_context_kind()
        } else {
            EntityContextKind::Other
        };
        if let Some(shape) = cached.context_outline_shape(state, kind) {
            return (
                Outline::Declared(shape, kind),
                cached.shape_offset(state, ShapeChannel::ContextOutline, pos),
            );
        }
        if let Some(bounds) = self.context_outline(pos, state, player_context, anims) {
            return (Outline::Dynamic(bounds), [0.0; 3]);
        }
        (Outline::Default(facts), default_offset())
    }

    pub(super) fn outline_edges_iter(
        &self,
        state: BlockStateId,
        pos: [i32; 3],
    ) -> impl Iterator<Item = [f64; 6]> + '_ {
        let (outline, offset) =
            self.selected_outline(&self.world, state, &self.be_anims, pos, true);
        let (owned, cached) = match outline {
            Outline::Published(shape) => (shape.edges(), &[][..]),
            Outline::Declared(_, kind) => (
                Vec::new(),
                self.collision
                    .cached_shapes()
                    .context_outline_edges(state, kind)
                    .unwrap_or_default(),
            ),
            Outline::Dynamic(_) => (sunrise_client::blocks::cube_edges(), &[][..]),
            Outline::Default(facts) => (Vec::new(), facts.outline_edges.as_ref()),
        };
        owned
            .into_iter()
            .chain(cached.iter().copied())
            .map(move |edge| {
                std::array::from_fn(|i| {
                    let axis = i % 3;
                    let local = match outline {
                        Outline::Dynamic(b) => b[axis] + edge[i] * (b[axis + 3] - b[axis]),
                        _ => edge[i],
                    };
                    f64::from(pos[axis]) + offset[axis] + local
                })
            })
    }

    /// The same outline traversal without allocating a box list.
    pub(super) fn outline_boxes_iter<'a>(
        &'a self,
        world: &'a World,
        state: BlockStateId,
        anims: &BlockEntityAnims,
        pos: [i32; 3],
        player_context: bool,
    ) -> impl Iterator<Item = [f64; 6]> + 'a {
        let (outline, offset) = self.selected_outline(world, state, anims, pos, player_context);
        let base = std::array::from_fn(|i| f64::from(pos[i]) + offset[i]);
        outline_boxes(outline, base)
    }

    pub(super) fn state_shape_boxes(
        &self,
        world: &World,
        anims: &BlockEntityAnims,
        state: BlockStateId,
        pos: [i32; 3],
        kind: BlockShapeKind,
    ) -> Vec<[f64; 6]> {
        match kind {
            BlockShapeKind::Outline => {
                let (outline, offset) = self.selected_outline(world, state, anims, pos, false);
                // Stay block-local: adding world coordinates and subtracting them again
                // would lose precision in offsets used to sample debris.
                outline_boxes(outline, offset).collect()
            }
            BlockShapeKind::Collision => collision_boxes(self.collision, world, state, pos),
            _ => unreachable!("BlockShapeKind: unhandled variant"),
        }
    }
}

fn outline_boxes(outline: Outline<'_>, base: [f64; 3]) -> impl Iterator<Item = [f64; 6]> + '_ {
    let (voxel, dynamic, defaults) = match outline {
        Outline::Published(shape) | Outline::Declared(shape, _) => (shape.boxes(), None, &[][..]),
        Outline::Dynamic(bounds) => (&[][..], Some(bounds), &[][..]),
        Outline::Default(facts) => (&[][..], None, &facts.outline[..]),
    };
    voxel
        .iter()
        .map(|b| {
            let min = b.min_position();
            let max = b.max_position();
            [min.x, min.y, min.z, max.x, max.y, max.z]
        })
        .chain(dynamic)
        .chain(defaults.iter().copied())
        .map(move |b| std::array::from_fn(|i| base[i % 3] + b[i]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::api_values::TryIntoApi;

    #[test]
    #[ignore = "daily"]
    fn supplied_shape_state_does_not_replace_world_state() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            let pos = [0, 64, 0];
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let slab = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone_slab")
                .unwrap();
            let before = game.world.block(pos[0], pos[1], pos[2]);
            assert_ne!(before, stone);
            for kind in [BlockShapeKind::Outline, BlockShapeKind::Collision] {
                assert_eq!(
                    level.block_shape(game, stone.try_into_api().unwrap(), pos, kind),
                    Some(vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]])
                );
                assert_eq!(
                    level.block_shape(game, slab.try_into_api().unwrap(), pos, kind),
                    Some(vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]])
                );
                assert_eq!(
                    level.block_shape(game, before.try_into_api().unwrap(), pos, kind),
                    Some(Vec::new())
                );
            }
            assert_eq!(game.world.block(pos[0], pos[1], pos[2]), before);
        });
    }
}
