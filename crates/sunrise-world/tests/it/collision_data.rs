use {
    sunrise_core::nbt::Tag,
    sunrise_world::{
        DATA_VERSION,
        chunk::BlockSection,
        collision::CachedCollisionRegistry,
        registry::{BlockRegistry, BlockStateId},
    },
};

fn fixture() -> (Tag, Tag) {
    let states = vec![
        Tag::String("minecraft:air".into()),
        Tag::String("minecraft:stone".into()),
        Tag::String("minecraft:context".into()),
    ];
    let registry = Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        ("states", Tag::List(states.clone())),
        (
            "defaults",
            Tag::compound([
                ("minecraft:air", Tag::Int(0)),
                ("minecraft:stone", Tag::Int(1)),
                ("minecraft:context", Tag::Int(2)),
            ]),
        ),
    ]);
    let empty = Tag::compound([
        ("x", Tag::LongArray(vec![0])),
        ("y", Tag::LongArray(vec![0])),
        ("z", Tag::LongArray(vec![0])),
        ("occupied", Tag::ByteArray(vec![])),
        ("cube", Tag::Byte(0)),
        ("cube_axes", Tag::Byte(0)),
        ("discrete_min", Tag::IntArray(vec![0; 3])),
        ("discrete_max", Tag::IntArray(vec![0; 3])),
    ]);
    let coords = Tag::LongArray(vec![0, 1.0f64.to_bits() as i64]);
    let block = Tag::compound([
        ("x", coords.clone()),
        ("y", coords.clone()),
        ("z", coords),
        ("occupied", Tag::ByteArray(vec![1])),
        ("cube", Tag::Byte(1)),
        ("cube_axes", Tag::Byte(7)),
        ("discrete_min", Tag::IntArray(vec![0; 3])),
        ("discrete_max", Tag::IntArray(vec![1; 3])),
    ]);
    let collision = Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        ("states", Tag::List(states)),
        ("shapes", Tag::List(vec![empty, block])),
        ("cached_shapes", Tag::IntArray(vec![0, 1, -1])),
    ]);
    (registry, collision)
}

#[test]
fn empty_cached_and_context_states_are_distinct() {
    let (registry_tag, collision_tag) = fixture();
    let registry = BlockRegistry::from_nbt(&registry_tag).unwrap();
    let collision = CachedCollisionRegistry::from_nbt(&collision_tag, &registry).unwrap();
    assert_eq!(collision.cached_state_count(), 2);
    assert_eq!(collision.shape_count(), 2);
    assert!(
        collision
            .cached_shape(BlockStateId(0))
            .unwrap()
            .unwrap()
            .is_empty()
    );
    assert!(
        !collision
            .cached_shape(BlockStateId(1))
            .unwrap()
            .unwrap()
            .is_empty()
    );
    assert!(collision.cached_shape(BlockStateId(2)).unwrap().is_none());
    assert!(collision.cached_shape(BlockStateId(3)).is_err());
    let mut section = BlockSection::uniform(BlockStateId(0), &registry).unwrap();
    section.set(4095, BlockStateId(1)).unwrap();
    assert!(
        !collision
            .section_shape(&section, 4095)
            .unwrap()
            .unwrap()
            .is_empty()
    );
    assert!(collision.section_shape(&section, 4096).is_err());
    let different = BlockRegistry::from_nbt(&registry_tag).unwrap();
    let foreign_section = BlockSection::uniform(BlockStateId(0), &different).unwrap();
    assert!(collision.section_shape(&foreign_section, 0).is_err());
}

#[test]
fn invalid_imports_fail_before_collision_queries() {
    let (registry_tag, valid) = fixture();
    let registry = BlockRegistry::from_nbt(&registry_tag).unwrap();
    for (field, tag) in [
        ("DataVersion", Tag::Int(0)),
        ("cached_shapes", Tag::IntArray(vec![0, 1])),
        ("cached_shapes", Tag::IntArray(vec![0, 2, -1])),
        ("cached_shapes", Tag::IntArray(vec![0, 1, -2])),
    ] {
        let mut bad = valid.clone();
        *bad.get_mut(field).unwrap() = tag;
        assert!(CachedCollisionRegistry::from_nbt(&bad, &registry).is_err());
    }
    let mut bad = valid.clone();
    let Some(Tag::List(states)) = bad.get_mut("states") else {
        unreachable!()
    };
    states.swap(0, 1);
    assert!(CachedCollisionRegistry::from_nbt(&bad, &registry).is_err());
    for (field, tag) in [
        ("cube", Tag::Byte(2)),
        ("cube_axes", Tag::Byte(8)),
        ("cube_axes", Tag::Byte(0)),
        ("occupied", Tag::ByteArray(vec![2])),
        ("discrete_min", Tag::IntArray(vec![0, 0])),
        ("discrete_min", Tag::IntArray(vec![-1, 0, 0])),
        ("discrete_max", Tag::IntArray(vec![1, 0, 1])),
        ("occupied", Tag::ByteArray(vec![])),
        ("x", Tag::LongArray(vec![0, 0])),
        ("x", Tag::LongArray(vec![0, f64::NAN.to_bits() as i64])),
        ("x", Tag::LongArray(vec![0, 0.5f64.to_bits() as i64])),
    ] {
        let mut bad = valid.clone();
        let Some(Tag::List(shapes)) = bad.get_mut("shapes") else {
            unreachable!()
        };
        *shapes[1].get_mut(field).unwrap() = tag;
        assert!(CachedCollisionRegistry::from_nbt(&bad, &registry).is_err());
    }
}

#[test]
fn equal_grids_share_storage_without_collapsing_shape_identities() {
    let (registry_tag, mut collision_tag) = fixture();
    let registry = BlockRegistry::from_nbt(&registry_tag).unwrap();
    let Some(Tag::List(shapes)) = collision_tag.get_mut("shapes") else {
        unreachable!()
    };
    shapes.push(shapes[1].clone());
    *collision_tag.get_mut("cached_shapes").unwrap() = Tag::IntArray(vec![0, 1, 2]);
    let collision = CachedCollisionRegistry::from_nbt(&collision_tag, &registry).unwrap();
    assert_eq!(collision.shape_count(), 3);
    assert_eq!(collision.unique_grid_count(), 2);
    let first = collision.cached_shape(BlockStateId(1)).unwrap().unwrap();
    let second = collision.cached_shape(BlockStateId(2)).unwrap().unwrap();
    assert!(!first.same_identity(second));
    assert!(std::ptr::eq(
        first.boxes().as_ptr(),
        second.boxes().as_ptr()
    ));
    assert!(first.same_identity(collision.cached_shape(BlockStateId(1)).unwrap().unwrap()));
}

fn canonical_marker(shape: &mut Tag, value: Tag) {
    let Tag::Compound(fields) = shape else {
        unreachable!()
    };
    fields.insert("canonical".into(), value);
}

#[test]
fn canonical_shapes_restore_global_identity_without_normalizing_equal_grids() {
    use sunrise_core::geometry::VoxelShape;
    let (registry_tag, mut collision_tag) = fixture();
    let registry = BlockRegistry::from_nbt(&registry_tag).unwrap();
    assert!(
        !CachedCollisionRegistry::from_nbt(&collision_tag, &registry)
            .unwrap()
            .has_canonical_identities()
    );
    let Some(Tag::List(shapes)) = collision_tag.get_mut("shapes") else {
        unreachable!()
    };
    canonical_marker(&mut shapes[0], Tag::Byte(1));
    canonical_marker(&mut shapes[1], Tag::Byte(2));
    let mut equal_block = shapes[1].clone();
    canonical_marker(&mut equal_block, Tag::Byte(0));
    shapes.push(equal_block);
    *collision_tag.get_mut("cached_shapes").unwrap() = Tag::IntArray(vec![0, 1, 2]);
    let collision = CachedCollisionRegistry::from_nbt(&collision_tag, &registry).unwrap();
    assert!(collision.has_canonical_identities());
    assert_eq!(collision.unique_grid_count(), 2);
    let empty = collision.cached_shape(BlockStateId(0)).unwrap().unwrap();
    let block = collision.cached_shape(BlockStateId(1)).unwrap().unwrap();
    let separate = collision.cached_shape(BlockStateId(2)).unwrap().unwrap();
    assert!(empty.same_identity(&VoxelShape::empty()));
    assert!(block.same_identity(&VoxelShape::block()));
    assert!(!separate.same_identity(&VoxelShape::block()));
    assert_eq!(block.boxes(), separate.boxes());
    assert!(std::ptr::eq(
        block.boxes().as_ptr(),
        separate.boxes().as_ptr()
    ));
}

#[test]
fn canonical_identity_claims_require_exact_geometry_and_unique_exports() {
    let (registry_tag, valid) = fixture();
    let registry = BlockRegistry::from_nbt(&registry_tag).unwrap();
    for (index, marker) in [
        (0, Tag::Byte(2)),
        (1, Tag::Byte(1)),
        (1, Tag::Byte(3)),
        (1, Tag::Int(2)),
    ] {
        let mut data = valid.clone();
        let Some(Tag::List(shapes)) = data.get_mut("shapes") else {
            unreachable!()
        };
        canonical_marker(&mut shapes[index], marker);
        assert!(CachedCollisionRegistry::from_nbt(&data, &registry).is_err());
    }
    let mut duplicate = valid.clone();
    let Some(Tag::List(shapes)) = duplicate.get_mut("shapes") else {
        unreachable!()
    };
    canonical_marker(&mut shapes[1], Tag::Byte(2));
    shapes.push(shapes[1].clone());
    assert!(CachedCollisionRegistry::from_nbt(&duplicate, &registry).is_err());
    let mut noncanonical_zero = valid;
    let Some(Tag::List(shapes)) = noncanonical_zero.get_mut("shapes") else {
        unreachable!()
    };
    canonical_marker(&mut shapes[1], Tag::Byte(2));
    let Some(Tag::LongArray(coords)) = shapes[1].get_mut("x") else {
        unreachable!()
    };
    coords[0] = (-0.0f64).to_bits() as i64;
    assert!(CachedCollisionRegistry::from_nbt(&noncanonical_zero, &registry).is_err());
}

#[test]
fn displaced_collision_declines_static_rays_and_reaches_the_next_face_cell() {
    use std::{collections::BTreeSet, convert::Infallible, sync::Arc};
    use sunrise_core::{
        geometry::{Aabb, Vec3, VoxelShape},
        position::{BlockPos, ChunkPos},
    };
    use sunrise_world::{
        block_collision::{BlockCollisionRegistry, DynamicCollisionWorld, ShulkerCollisionState},
        block_collision_scan::BlockCollisionScan,
        collision_context::CollisionContext,
        dynamic_support::DynamicSupportWorld,
        entity_obstruction::EntityIdentity,
        fluid::FluidState,
        liquid_collision::LiquidCollisionWorld,
        powder_snow_collision::{PowderSnowEntity, PowderSnowWorld},
        registry_collision_scan::{CollisionChunks, RegistryCollisionWorld},
        suffocation::{SuffocationDefinition, SuffocationRegistry},
    };
    const AT: BlockPos = BlockPos::new(1, 64, 2);
    const SOLID: BlockStateId = BlockStateId(1);
    struct Scene;
    impl DynamicSupportWorld for Scene {
        type Error = Infallible;
        fn shulker_is_closed(&mut self, _: [i32; 3]) -> Result<Option<bool>, Infallible> {
            Ok(None)
        }
        fn moving_piston_shape(&mut self, _: [i32; 3]) -> Result<Option<VoxelShape>, Infallible> {
            Ok(None)
        }
    }
    impl DynamicCollisionWorld for Scene {
        fn shulker_collision_state(
            &mut self,
            _: [i32; 3],
        ) -> Result<Option<ShulkerCollisionState>, Infallible> {
            Ok(None)
        }
    }
    impl LiquidCollisionWorld for Scene {
        type Error = Infallible;
        fn entity_liquid_shape(&mut self, _: EntityIdentity) -> Result<VoxelShape, Infallible> {
            unreachable!()
        }
        fn fluid(&mut self, _: BlockPos) -> Result<FluidState, Infallible> {
            unreachable!()
        }
        fn entity_can_stand(
            &mut self,
            _: EntityIdentity,
            _: FluidState,
        ) -> Result<bool, Infallible> {
            unreachable!()
        }
    }
    impl PowderSnowWorld for Scene {
        type Error = Infallible;
        fn powder_snow_entity(
            &mut self,
            _: EntityIdentity,
        ) -> Result<PowderSnowEntity, Infallible> {
            unreachable!()
        }
        fn powder_snow_walkable_tag(&mut self, _: EntityIdentity) -> Result<bool, Infallible> {
            unreachable!()
        }
        fn feet_are_leather_boots(&mut self, _: EntityIdentity) -> Result<bool, Infallible> {
            unreachable!()
        }
    }
    impl CollisionChunks for Scene {
        type Chunk = ();
        type ChunkWorld = Self;
        fn collision_chunk(&mut self, _: ChunkPos) -> Result<Option<()>, Infallible> {
            Ok(Some(()))
        }
        fn block_state(&mut self, (): (), position: BlockPos) -> Result<BlockStateId, Infallible> {
            Ok(if position == AT {
                SOLID
            } else {
                BlockStateId(0)
            })
        }
        fn chunk_world(&mut self, (): ()) -> &mut Self {
            self
        }
    }
    let (registry_tag, mut collision_tag) = fixture();
    let registry = BlockRegistry::from_nbt(&registry_tag).unwrap();
    let Some(Tag::List(shapes)) = collision_tag.get_mut("shapes") else {
        unreachable!()
    };
    canonical_marker(&mut shapes[0], Tag::Byte(1));
    canonical_marker(&mut shapes[1], Tag::Byte(2));
    let Tag::Compound(root) = &mut collision_tag else {
        unreachable!()
    };
    root.insert(
        "shape_offsets".into(),
        Tag::List(vec![Tag::compound([
            ("state", Tag::Int(1)),
            ("channels", Tag::Byte(1)),
            ("horizontal", Tag::Float(0.25f32.to_bits())),
            ("vertical", Tag::Float(0.2f32.to_bits())),
        ])]),
    );
    let cached = Arc::new(CachedCollisionRegistry::from_nbt(&collision_tag, &registry).unwrap());
    let binding = Arc::new(BlockCollisionRegistry::new(cached).unwrap());
    assert!(binding.static_shape(SOLID).unwrap().is_none());
    assert!(
        binding
            .static_shape(BlockStateId(0))
            .unwrap()
            .unwrap()
            .is_empty()
    );
    let mut scene = Scene;
    let shape = binding
        .collision_shape(&mut scene, SOLID, [AT.x, AT.y, AT.z])
        .unwrap();
    // A downward ray through the narrow strip of the unshifted east edge must miss;
    // the centre ray must hit the independently known height of the displaced cube.
    let clip = |x| shape.clip(Vec3::new(x, 66.0, 2.5), Vec3::new(x, 63.0, 2.5), AT);
    assert!(clip(1.95).is_none());
    assert!((clip(1.5).unwrap().location.y - 64.83999999821186).abs() < 1.0e-12);
    let definitions = ["minecraft:air", "minecraft:stone", "minecraft:context"]
        .map(|name| (name.to_owned(), SuffocationDefinition::Never))
        .into_iter()
        .collect();
    let binding = SuffocationRegistry::new(binding, &definitions, &BTreeSet::new()).unwrap();
    let mut scene = RegistryCollisionWorld::new(&binding, &mut scene).unwrap();
    // The box is wholly in the cell west of AT: only the face halo can find this block.
    let probe = Aabb::new(Vec3::new(0.92, 64.2, 2.3), Vec3::new(0.98, 64.4, 2.6));
    let hits =
        BlockCollisionScan::new(&mut scene, CollisionContext::empty(false), probe, false, 27)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].position, AT);
}

#[test]
fn registry_scan_binding_requires_complete_canonical_metadata() {
    use std::{collections::BTreeSet, sync::Arc};
    use sunrise_world::{
        block_collision::BlockCollisionRegistry,
        registry_collision_scan::RegistryCollisionWorld,
        suffocation::{SuffocationDefinition, SuffocationRegistry},
    };
    let (registry_tag, mut collision_tag) = fixture();
    let registry = BlockRegistry::from_nbt(&registry_tag).unwrap();
    let definitions = ["minecraft:air", "minecraft:stone", "minecraft:context"]
        .map(|name| (name.to_owned(), SuffocationDefinition::Never))
        .into_iter()
        .collect();
    for marked in [false, true] {
        if marked {
            let Some(Tag::List(shapes)) = collision_tag.get_mut("shapes") else {
                unreachable!()
            };
            canonical_marker(&mut shapes[0], Tag::Byte(1));
            canonical_marker(&mut shapes[1], Tag::Byte(2));
        }
        let cached =
            Arc::new(CachedCollisionRegistry::from_nbt(&collision_tag, &registry).unwrap());
        let collisions = Arc::new(BlockCollisionRegistry::new(cached).unwrap());
        assert!(collisions.collision_block(BlockStateId(3)).is_err());
        let binding = SuffocationRegistry::new(collisions, &definitions, &BTreeSet::new()).unwrap();
        assert_eq!(
            RegistryCollisionWorld::new(&binding, &mut ()).is_ok(),
            marked
        );
    }
}

/// A table read from its bytes checks its state identities while reading and
/// keeps none of them, yet loads the same shapes as the tree; a nested table
/// (`block-properties.nbt`'s `occlusion`) is checked the same way.
#[test]
fn streamed_tables_check_states_while_reading() {
    use {
        sunrise_core::{
            nbt,
            nbt::{Limits, NamedTag},
        },
        sunrise_world::registry::{BlockTable, BlockTableFile},
    };
    let (registry_tag, collision_tag) = fixture();
    let registry = BlockRegistry::from_nbt(&registry_tag).unwrap();
    let bytes = |tag: &Tag| {
        let root = NamedTag {
            name: "".into(),
            tag: tag.clone(),
        };
        nbt::write_named(&root, Limits::tables()).unwrap()
    };
    let file = BlockTableFile::read(&bytes(&collision_tag), &registry).unwrap();
    let table = file.table("collision").unwrap();
    assert_eq!(table.get("states"), Some(&Tag::List(Vec::new())));
    let streamed = CachedCollisionRegistry::from_table(table).unwrap();
    let tree = CachedCollisionRegistry::from_nbt(&collision_tag, &registry).unwrap();
    for id in 0..3 {
        let id = BlockStateId(id);
        assert_eq!(
            streamed.cached_shape(id).unwrap().map(|s| s.boxes()),
            tree.cached_shape(id).unwrap().map(|s| s.boxes())
        );
    }

    let mut outer = Tag::compound([("occlusion", collision_tag.clone())]);
    let Tag::Compound(map) = &mut outer else {
        unreachable!()
    };
    map.insert(
        "states".into(),
        collision_tag.get("states").unwrap().clone(),
    );
    let file = BlockTableFile::read(&bytes(&outer), &registry).unwrap();
    let nested = file
        .table("outer")
        .unwrap()
        .nested("occlusion", "occlusion")
        .unwrap();
    assert_eq!(nested.get("states"), Some(&Tag::List(Vec::new())));
    CachedCollisionRegistry::from_table(nested).unwrap();
    // Checked only as a tree, an emptied list is refused.
    assert!(BlockTable::check(nested.tag(), &registry, "occlusion").is_err());

    // Other states, a different count or no states at all are refused.
    let with_states = |edit: &dyn Fn(&mut Vec<Tag>)| {
        let mut tag = collision_tag.clone();
        let Some(Tag::List(states)) = tag.get_mut("states") else {
            unreachable!()
        };
        edit(states);
        tag
    };
    let swapped = with_states(&|s| s.swap(1, 2));
    assert!(BlockTableFile::read(&bytes(&swapped), &registry).is_err());
    let short = with_states(&|s| drop(s.pop()));
    assert!(BlockTableFile::read(&bytes(&short), &registry).is_err());
    let mut swapped = swapped;
    let Tag::Compound(map) = &mut swapped else {
        unreachable!()
    };
    map.remove(&"states".into());
    let file = BlockTableFile::read(&bytes(&swapped), &registry).unwrap();
    assert!(file.table("collision").is_err());
}

#[test]
fn declared_shape_offsets_match_vanilla_xz_xyz_and_clamp_without_moving_the_cache() {
    use std::sync::Arc;
    use sunrise_core::geometry::{Axis, VoxelShape};
    use sunrise_world::{
        block_collision::{BlockCollisionRegistry, DynamicCollisionWorld, ShulkerCollisionState},
        collision::ShapeChannel,
        dynamic_support::DynamicSupportWorld,
    };
    struct World {
        published: bool,
    }
    impl DynamicSupportWorld for World {
        type Error = std::convert::Infallible;
        fn shulker_is_closed(&mut self, _: [i32; 3]) -> Result<Option<bool>, Self::Error> {
            Ok(None)
        }
        fn moving_piston_shape(&mut self, _: [i32; 3]) -> Result<Option<VoxelShape>, Self::Error> {
            Ok(None)
        }
        fn published_collision_shape(
            &mut self,
            _: [i32; 3],
            _: BlockStateId,
        ) -> Result<Option<VoxelShape>, Self::Error> {
            Ok(self.published.then(VoxelShape::block))
        }
    }
    impl DynamicCollisionWorld for World {
        fn shulker_collision_state(
            &mut self,
            _: [i32; 3],
        ) -> Result<Option<ShulkerCollisionState>, Self::Error> {
            Ok(None)
        }
    }
    // Independently read from 26.3 POPPY (XZ), SHORT_GRASS (XYZ), and POINTED_DRIPSTONE
    // (horizontal clamp) through their actual BlockState.getOffset at these columns.
    for (limits, expected) in [
        (
            (0.25f32, 0.0f32),
            [-0.11666665971279144, 0.0, -0.18333332985639572],
        ),
        (
            (0.25, 0.2),
            [
                -0.11666665971279144,
                -0.16000000178813933,
                -0.18333332985639572,
            ],
        ),
        ((0.125, 0.0), [-0.11666665971279144, 0.0, -0.125]),
    ] {
        let (registry_tag, mut tag) = fixture();
        let registry = BlockRegistry::from_nbt(&registry_tag).unwrap();
        let Tag::Compound(root) = &mut tag else {
            unreachable!()
        };
        root.insert(
            "shape_offsets".into(),
            Tag::List(vec![Tag::compound([
                ("state", Tag::Int(1)),
                ("channels", Tag::Byte(15)),
                ("horizontal", Tag::Float(limits.0.to_bits())),
                ("vertical", Tag::Float(limits.1.to_bits())),
            ])]),
        );
        let cached = Arc::new(CachedCollisionRegistry::from_nbt(&tag, &registry).unwrap());
        let state = BlockStateId(1);
        let local = cached.cached_shape(state).unwrap().unwrap();
        for channel in [
            ShapeChannel::Collision,
            ShapeChannel::ContextCollision,
            ShapeChannel::ContextOutline,
            ShapeChannel::Interaction,
        ] {
            let shape = cached.positioned_shape(state, channel, [1, 64, 2], local);
            for (axis, offset) in Axis::ALL.into_iter().zip(expected) {
                assert_eq!(shape.min(axis), offset);
                assert_eq!(shape.max(axis), 1.0 + offset);
            }
            let second = cached.positioned_shape(state, channel, [-7, 3, 13], local);
            assert_eq!(second.min(Axis::X), -0.04999999701976776);
            assert_eq!(second.min(Axis::Z), -0.11666665971279144);
            assert_eq!(cached.shape_offset(state, channel, [1, -99, 2]), expected);
        }
        assert_eq!(local.min(Axis::X), 0.0);
        assert_eq!(local.max(Axis::Z), 1.0);
        let binding = BlockCollisionRegistry::new(cached).unwrap();
        assert!(binding.reaches_out(state));
        let declared = binding
            .collision_shape(&mut World { published: false }, state, [1, 64, 2])
            .unwrap();
        assert_eq!(declared.min(Axis::X), expected[0]);
        let suffocation = sunrise_world::suffocation::SuffocationRegistry::new(
            Arc::new(binding),
            &["minecraft:air", "minecraft:stone", "minecraft:context"]
                .into_iter()
                .map(|name| {
                    (
                        name.to_string(),
                        sunrise_world::suffocation::SuffocationDefinition::TaggedFullCollision,
                    )
                })
                .collect(),
            &["minecraft:stone".to_string()].into_iter().collect(),
        )
        .unwrap();
        assert!(
            !suffocation
                .is_suffocating(&mut World { published: false }, state, [1, 64, 2])
                .unwrap()
        );
        let Tag::Compound(root) = &mut tag else {
            unreachable!()
        };
        root.insert("published_shapes".into(), Tag::IntArray(vec![1]));
        let published_binding = BlockCollisionRegistry::new(Arc::new(
            CachedCollisionRegistry::from_nbt(&tag, &registry).unwrap(),
        ))
        .unwrap();
        let published = published_binding
            .collision_shape(&mut World { published: true }, state, [1, 64, 2])
            .unwrap();
        assert_eq!(published.min(Axis::X), 0.0);
        assert_eq!(published.max(Axis::Z), 1.0);
    }
    let (registry_tag, mut tag) = fixture();
    let registry = BlockRegistry::from_nbt(&registry_tag).unwrap();
    for horizontal in [f32::NAN, f32::INFINITY, -0.1, 0.3] {
        let Tag::Compound(root) = &mut tag else {
            unreachable!()
        };
        root.insert(
            "shape_offsets".into(),
            Tag::List(vec![Tag::compound([
                ("state", Tag::Int(1)),
                ("channels", Tag::Byte(1)),
                ("horizontal", Tag::Float(horizontal.to_bits())),
                ("vertical", Tag::Float(0.0f32.to_bits())),
            ])]),
        );
        assert!(CachedCollisionRegistry::from_nbt(&tag, &registry).is_err());
    }
}
