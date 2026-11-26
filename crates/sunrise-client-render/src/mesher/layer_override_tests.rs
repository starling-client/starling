//! A `shaders/` pack's `layer.*` rules (spec 4.2) through the real mesher: the state the table
//! moves (`game::pack_layers` keeps full opaque cubes out of it) moves its model faces to that
//! layer, the other states of the same block keep theirs, and with no state moved the mesh is
//! the one the mesher made before, to the byte. Needs prepared resources, like the other mesher
//! geometry tests.
use super::opaque_ice_tests::{Fixture, fixture};
use super::*;
use crate::chunk::Chunk;
use sunrise_world::registry::BlockStateId;

const BASE: i32 = MIN_Y + 4;

/// The state of `name` with every `properties` pair.
fn state(f: &Fixture, name: &str, properties: &[(&str, &str)]) -> u32 {
    (0..f.assets.registry.len() as u32)
        .find(|&i| {
            f.assets.registry.state(BlockStateId(i)).is_some_and(|s| {
                s.name == name
                    && properties
                        .iter()
                        .all(|(k, v)| s.properties.get(*k).map(String::as_str) == Some(*v))
            })
        })
        .unwrap_or_else(|| panic!("{name} {properties:?}"))
}

fn mesh(
    f: &Fixture,
    chunk: &Chunk,
    layer_overrides: &[Option<RenderLayer>],
    cutout_leaves: bool,
) -> SectionMesh {
    let region = [[Some(chunk); 3]; 3];
    let mut mesher = Mesher {
        nether_shading: false,
        shader_pack_lighting: true,
        separate_ao: false,
        shader_pack: true,
        assets: &f.assets,
        render: &f.render,
        tables: &f.tables,
        biomes: &f.biomes,
        visibility: f.visibility.clone(),
        zoom: BiomeZoom::new(0),
        blend_radius: 0,
        smooth_lighting: true,
        cutout_leaves,
        opaque_ice: false,
        translucent_layers: &[],
        layer_overrides,
        leaf_culling: LeafCullingMode::Off,
        hidden_states: &[],
        part_tables: Default::default(),
    };
    mesher.mesh(&region, 0)
}

/// A table moving `moved` states to their layer, every other state kept.
fn table(f: &Fixture, moved: &[(u32, RenderLayer)]) -> Vec<Option<RenderLayer>> {
    let mut table = vec![None; f.assets.registry.len()];
    for &(id, layer) in moved {
        table[id as usize] = Some(layer);
    }
    table
}

/// The quads with a corner west of x = 8 (the west block) or not, as their vertices.
fn west(vertices: &[TerrainVertex], west: bool) -> Vec<TerrainVertex> {
    vertices
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|q| (q[0].pos[0] < 8.0) == west)
        .flatten()
        .copied()
        .collect()
}

fn bytes(vertices: &[TerrainVertex]) -> &[u8] {
    bytemuck::cast_slice(vertices)
}

/// `vertices` with their layer's opaque bit set or cleared.
fn opaque_bit(vertices: &[TerrainVertex], on: bool) -> Vec<TerrainVertex> {
    vertices
        .iter()
        .map(|v| TerrainVertex {
            layer: if on {
                v.layer | OPAQUE_LAYER
            } else {
                v.layer & !OPAQUE_LAYER
            },
            ..*v
        })
        .collect()
}

/// Two isolated oak slabs, bottom west and top east: they draw the same sprites.
fn slabs(f: &Fixture) -> (Chunk, u32, u32) {
    let slab = |kind| {
        state(
            f,
            "minecraft:oak_slab",
            &[("type", kind), ("waterlogged", "false")],
        )
    };
    let (bottom, top) = (slab("bottom"), slab("top"));
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(4, BASE, 8, bottom);
    chunk.set(12, BASE, 8, top);
    (chunk, bottom, top)
}

/// The issue's control on a block that may move (4.2 keeps full opaque cubes, the issue's
/// oak log, on their layer): `layer.translucent=oak_slab:type=bottom` moves the bottom slab's
/// six faces, and only them, to the translucent pass, as the same quads; the top slab keeps
/// its solid ones, opaque bit and all.
#[test]
#[ignore = "daily"]
fn translucent_moves_only_the_named_state() {
    let f = fixture();
    let (chunk, bottom, _) = slabs(&f);
    let before = mesh(&f, &chunk, &[], true);
    assert!(before.translucent.is_empty());
    assert_eq!(before.solid.len(), 48, "two slabs in the solid pass");
    assert!(before.solid.iter().all(|v| v.layer & OPAQUE_LAYER != 0));
    let after = mesh(
        &f,
        &chunk,
        &table(&f, &[(bottom, RenderLayer::Translucent)]),
        true,
    );
    assert_eq!(
        bytes(&after.translucent),
        bytes(&opaque_bit(&west(&before.solid, true), false)),
        "the bottom slab's quads, blended"
    );
    assert_eq!(
        bytes(&after.solid),
        bytes(&west(&before.solid, false)),
        "the top slab's quads, unchanged"
    );
    // The moved faces keep their sources (the pack's `mc_Entity` of each quad).
    assert_eq!(after.translucent_sources.len(), 6);
    assert_eq!(after.solid_sources.len(), 6);
}

/// `layer.cutout=oak_slab:type=bottom` keeps the bottom slab in the solid pass but drops the
/// opaque bit its texels gave it (the alpha test runs); the top slab keeps the bit.
#[test]
#[ignore = "daily"]
fn cutout_clears_the_opaque_bit_of_the_named_state_only() {
    let f = fixture();
    let (chunk, bottom, _) = slabs(&f);
    let before = mesh(&f, &chunk, &[], true);
    let after = mesh(
        &f,
        &chunk,
        &table(&f, &[(bottom, RenderLayer::Cutout)]),
        true,
    );
    assert!(after.translucent.is_empty());
    assert_eq!(
        bytes(&west(&after.solid, true)),
        bytes(&opaque_bit(&west(&before.solid, true), false))
    );
    assert_eq!(
        bytes(&west(&after.solid, false)),
        bytes(&west(&before.solid, false))
    );
}

/// `layer.solid` sets the opaque bit (no alpha test) on leaves, which Fancy leaves cut out by
/// their sprite, and moves stained glass out of the translucent pass; `layer.translucent` on
/// leaves wins over Fast leaves' opaque pass. Each block's unmoved twin keeps its own pass.
#[test]
#[ignore = "daily"]
fn rules_win_over_the_sprite_and_the_options() {
    let f = fixture();
    let leaves = |persistent| {
        state(
            &f,
            "minecraft:oak_leaves",
            &[
                ("persistent", persistent),
                ("distance", "7"),
                ("waterlogged", "false"),
            ],
        )
    };
    let (moved, kept) = (leaves("true"), leaves("false"));
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(4, BASE, 8, moved);
    chunk.set(12, BASE, 8, kept);
    let before = mesh(&f, &chunk, &[], true);
    assert!(before.translucent.is_empty());
    assert_eq!(before.solid.len(), 48);
    assert!(before.solid.iter().all(|v| v.layer & OPAQUE_LAYER == 0));
    let after = mesh(&f, &chunk, &table(&f, &[(moved, RenderLayer::Solid)]), true);
    assert_eq!(
        bytes(&west(&after.solid, true)),
        bytes(&opaque_bit(&west(&before.solid, true), true))
    );
    assert_eq!(
        bytes(&west(&after.solid, false)),
        bytes(&west(&before.solid, false))
    );

    let stained = state(&f, "minecraft:red_stained_glass", &[]);
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(4, BASE, 8, stained);
    let before = mesh(&f, &chunk, &[], true);
    assert!(before.solid.is_empty());
    assert_eq!(before.translucent.len(), 24);
    let after = mesh(
        &f,
        &chunk,
        &table(&f, &[(stained, RenderLayer::Solid)]),
        true,
    );
    assert!(after.translucent.is_empty());
    assert_eq!(
        bytes(&after.solid),
        bytes(&opaque_bit(&before.translucent, true))
    );

    // Fast leaves (cutoutLeaves off) are opaque; the rule blends the persistent ones only.
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(4, BASE, 8, moved);
    chunk.set(12, BASE, 8, kept);
    let before = mesh(&f, &chunk, &[], false);
    assert!(before.translucent.is_empty());
    assert!(before.solid.iter().all(|v| v.layer & OPAQUE_LAYER != 0));
    let after = mesh(
        &f,
        &chunk,
        &table(&f, &[(moved, RenderLayer::Translucent)]),
        false,
    );
    assert_eq!(
        bytes(&after.translucent),
        bytes(&opaque_bit(&west(&before.solid, true), false))
    );
    assert_eq!(bytes(&after.solid), bytes(&west(&before.solid, false)));
}

/// A table moving no state, an empty table and a table moving another block's states mesh a
/// mixed section exactly as the mesher does without a pack's rules (it passes before the rules
/// exist, and guards that the empty table stays the no-pack path).
#[test]
#[ignore = "daily"]
fn no_moved_state_meshes_to_the_byte() {
    let f = fixture();
    let mut chunk = Chunk::empty(0, 0);
    for (i, name) in [
        "minecraft:oak_log",
        "minecraft:glass",
        "minecraft:red_stained_glass",
        "minecraft:oak_leaves",
        "minecraft:grass_block",
        "minecraft:water",
        "minecraft:stone",
    ]
    .into_iter()
    .enumerate()
    {
        let id = f.assets.registry.default_state(name).unwrap().0;
        chunk.set(2 * i as i32, BASE, 8, id);
        chunk.set(2 * i as i32 + 1, BASE, 8, id);
    }
    let dirt = state(&f, "minecraft:dirt", &[]);
    for cutout_leaves in [true, false] {
        let plain = mesh(&f, &chunk, &[], cutout_leaves);
        for overrides in [
            table(&f, &[]),
            table(&f, &[(dirt, RenderLayer::Translucent)]),
        ] {
            let with = mesh(&f, &chunk, &overrides, cutout_leaves);
            assert_eq!(bytes(&with.solid), bytes(&plain.solid));
            assert_eq!(bytes(&with.translucent), bytes(&plain.translucent));
            assert_eq!(with.solid_tints, plain.solid_tints);
            assert_eq!(with.solid_sources.len(), plain.solid_sources.len());
            assert_eq!(
                with.translucent_sources.len(),
                plain.translucent_sources.len()
            );
        }
    }
}
