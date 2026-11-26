//! A data pack's file of each worldgen kind changes what the generator makes, as it does in
//! vanilla. Each test needs the prepared worldgen bundles, so each is a daily test.
use super::super::{
    biome_source::FixedBiomeSource,
    generator_decoration::Decoration,
    overrides::PackFile,
    structure::{
        jigsaw::{FeatureKey, PoolElement},
        processor::ProcessorList,
        start::Piece,
    },
};
use super::tests::{PieceWorld, prepared_root, prepared_with};
use super::*;
use serde_json::json;

/// A prepared bundle (`worldgen/<name>.json`): each entry's id and JSON text.
fn bundle(name: &str) -> BTreeMap<String, String> {
    let path = prepared_root()
        .expect("prepared resources")
        .join("worldgen")
        .join(format!("{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The prepared entry `id` of the bundle `name`, as JSON.
fn entry(name: &str, id: &str) -> Value {
    serde_json::from_str(&bundle(name)[id]).unwrap()
}

/// The prepared data under the pack files `(path, text)`.
fn with_pack(files: &[(impl AsRef<str>, String)]) -> Arc<GeneratorData> {
    let mut overrides = WorldgenOverrides::default();
    for (file, text) in files {
        let added = overrides.add_file("file/test", file.as_ref(), || Some(text.clone()));
        assert_eq!(added, PackFile::Read, "{}", file.as_ref());
    }
    let data = prepared_with(&overrides).expect("prepared resources");
    assert!(data.warnings().is_empty(), "{:?}", data.warnings());
    data
}

/// The surface height and top block of 64 overworld columns of seed 1234; None where the
/// column holds no solid block.
fn surface(data: Arc<GeneratorData>) -> Vec<Option<(i32, BlockStateId)>> {
    let mut worker = OverworldGenerator::new(data, 1234)
        .expect("generator")
        .worker();
    (0..64)
        .map(|i| {
            worker
                .surface_sample(i * 137, i * 91)
                .map(|sample| (sample.height, sample.state))
        })
        .collect()
}

fn vanilla_surface() -> Vec<Option<(i32, BlockStateId)>> {
    surface(prepared_with(&WorldgenOverrides::default()).expect("prepared resources"))
}

/// Places `feature` at one position of an empty world, as a structure piece does; whether it
/// placed, and the blocks.
fn placed(
    data: &Arc<GeneratorData>,
    feature: FeatureKey<'_>,
) -> (bool, BTreeMap<[i32; 3], BlockStateId>) {
    let plains = ResolvedBiomeSource::Fixed(FixedBiomeSource {
        biome: "minecraft:plains".into(),
    });
    let decoration = Decoration::new(data, &plains).unwrap().expect("decoration");
    let mut world = PieceWorld(BTreeMap::new());
    let mut random = LegacyRandom::new(5);
    let done = decoration
        .place_piece_feature(&mut world, &mut random, feature, [3, 70, 4])
        .unwrap();
    (done, world.0)
}

#[test]
#[ignore = "daily"]
fn a_packs_noise_settings_cap_the_terrain_at_their_noise_range() {
    let mut settings = entry("settings", "minecraft:overworld");
    settings["noise"]["height"] = 128.into();
    let packed = surface(with_pack(&[(
        "data/minecraft/worldgen/noise_settings/overworld.json",
        settings.to_string(),
    )]));
    // The pack's noise reaches from y -64 to 64.
    assert!(packed.iter().flatten().all(|(height, _)| *height <= 64));
    assert!(
        vanilla_surface()
            .iter()
            .flatten()
            .any(|(height, _)| *height > 64)
    );
}

#[test]
#[ignore = "daily"]
fn a_packs_density_function_empties_the_terrain() {
    let packed = surface(with_pack(&[(
        "data/minecraft/worldgen/density_function/overworld/final_density.json",
        // Aquifer barriers can still hold stone where the density is only slightly negative;
        // the lowest value a constant takes leaves none.
        json!({"type": "minecraft:constant", "value": -1000000.0}).to_string(),
    )]));
    assert!(packed.iter().all(Option::is_none));
    assert!(vanilla_surface().iter().any(Option::is_some));
}

#[test]
#[ignore = "daily"]
fn a_packs_noise_changes_the_terrain_that_samples_it() {
    let mut noise = entry("noises", "minecraft:continentalness");
    noise["base_octave"] = (-3).into();
    let packed = surface(with_pack(&[(
        "data/minecraft/worldgen/noise/continentalness.json",
        noise.to_string(),
    )]));
    assert_ne!(packed, vanilla_surface());
}

#[test]
#[ignore = "daily"]
fn a_packs_material_rule_names_the_surface_block() {
    let data = with_pack(&[(
        "data/minecraft/worldgen/material_rule/overworld.json",
        json!({"type": "minecraft:block", "result_state": "minecraft:obsidian"}).to_string(),
    )]);
    let obsidian = data.registry().default_state("minecraft:obsidian").unwrap();
    let packed = surface(data);
    assert!(packed.iter().flatten().count() > 0);
    assert!(packed.iter().flatten().all(|(_, state)| *state == obsidian));
}

#[test]
#[ignore = "daily"]
fn a_packs_material_condition_decides_where_the_surface_rule_applies() {
    // `on_floor` that no height satisfies: the surface rule never runs.
    let data = with_pack(&[(
        "data/minecraft/worldgen/material_condition/on_floor.json",
        json!({
            "type": "minecraft:y_above",
            "add_stone_depth": false,
            "anchor": {"absolute": 2000},
            "surface_depth_multiplier": 0,
        })
        .to_string(),
    )]);
    let grass = data
        .registry()
        .default_state("minecraft:grass_block")
        .unwrap();
    assert!(
        surface(data)
            .iter()
            .flatten()
            .all(|(_, state)| *state != grass)
    );
    assert!(
        vanilla_surface()
            .iter()
            .flatten()
            .any(|(_, state)| *state == grass)
    );
}

#[test]
#[ignore = "daily"]
fn a_packs_carvers_that_never_run_leave_the_noise_chunk_uncarved() {
    let files = ["cave", "cave_extra_underground", "canyon"].map(|name| {
        let mut carver = entry("carvers", &format!("minecraft:{name}"));
        carver["probability"] = 0.0.into();
        (
            format!("data/minecraft/worldgen/carver/{name}.json"),
            carver.to_string(),
        )
    });
    // Whether carving changed any of the sixteen chunks next to the origin.
    let sections = |chunk: &ProtoChunk| {
        (0..chunk.terrain.height() as usize / 16)
            .map(|index| chunk.terrain.section(index).unwrap().to_nbt().unwrap())
            .collect::<Vec<_>>()
    };
    let carved = |data: Arc<GeneratorData>| {
        let mut worker = OverworldGenerator::new(data, 1234)
            .expect("generator")
            .worker();
        let mut changed = false;
        for (x, z) in (0..4).flat_map(|x| (0..4).map(move |z| (x, z))) {
            let pos = ChunkPos::new(x, z);
            let before = worker
                .generate_proto_stage(pos, ProtoStage::Surface)
                .unwrap();
            let after = worker
                .generate_proto_stage(pos, ProtoStage::Carvers)
                .unwrap();
            changed |= sections(&before) != sections(&after);
        }
        changed
    };
    assert!(!carved(with_pack(&files)));
    assert!(carved(
        prepared_with(&WorldgenOverrides::default()).unwrap()
    ));
}

/// The prepared igloo under the pack's `biomes`, in a pack structure set that gives every
/// second chunk a start; whether any of 36 chunks starts it.
fn igloo_set_starts(biomes: &str) -> bool {
    let mut igloo = entry("structures", "minecraft:igloo");
    igloo["biomes"] = biomes.into();
    let data = with_pack(&[
        (
            "data/sunrise/worldgen/structure/igloo.json",
            igloo.to_string(),
        ),
        (
            "data/sunrise/worldgen/structure_set/igloos.json",
            json!({
                "placement": {
                    "type": "minecraft:random_spread",
                    "salt": 7,
                    "separation": 1,
                    "spacing": 2,
                },
                "structures": [{"structure": "sunrise:igloo", "weight": 1}],
            })
            .to_string(),
        ),
    ]);
    let mut worker = OverworldGenerator::new(data, 1234)
        .expect("generator")
        .worker();
    (0..6)
        .flat_map(|x| (0..6).map(move |z| ChunkPos::new(x, z)))
        .flat_map(|pos| worker.structure_starts(pos))
        .any(|start| start.name == "sunrise:igloo")
}

#[test]
#[ignore = "daily"]
fn a_packs_structure_set_places_its_structure_where_the_structures_biomes_allow() {
    assert!(igloo_set_starts("#minecraft:is_overworld"));
    assert!(!igloo_set_starts("minecraft:the_void"));
}

#[test]
#[ignore = "daily"]
fn a_packs_template_pool_and_processor_list_make_the_start_piece() {
    // The outpost's start pool, with one element: the watchtower under the pack's own list.
    let mut outpost = entry("structures", "minecraft:pillager_outpost");
    outpost["biomes"] = "#minecraft:is_overworld".into();
    let data = with_pack(&[
        (
            "data/sunrise/worldgen/structure/outpost_anywhere.json",
            outpost.to_string(),
        ),
        (
            "data/minecraft/worldgen/template_pool/pillager_outpost/base_plates.json",
            json!({
                "fallback": "minecraft:empty",
                "elements": [{
                    "weight": 1,
                    "element": {
                        "element_type": "minecraft:legacy_single_pool_element",
                        "location": "minecraft:pillager_outpost/watchtower",
                        "processors": "sunrise:stone_to_obsidian",
                        "projection": "rigid",
                    },
                }],
            })
            .to_string(),
        ),
        (
            "data/sunrise/worldgen/processor_list/stone_to_obsidian.json",
            json!({"processors": [{
                "processor_type": "minecraft:rule",
                "rules": [{
                    "input_predicate": {
                        "predicate_type": "minecraft:block_match",
                        "block": "minecraft:stone",
                    },
                    "location_predicate": {"predicate_type": "minecraft:always_true"},
                    "output_state": "minecraft:obsidian",
                }],
            }]})
            .to_string(),
        ),
    ]);
    let start = OverworldGenerator::new(data, 1234)
        .expect("generator")
        .worker()
        .structure_start("sunrise:outpost_anywhere", ChunkPos::new(0, 0))
        .expect("the pack's structure starts");
    let Piece::Pool(piece) = &start.pieces[0] else {
        panic!("a jigsaw structure starts with a pool piece");
    };
    let PoolElement::Single {
        processors_source, ..
    } = &*piece.element
    else {
        panic!("the pack's pool holds a single element");
    };
    // The vanilla pool's element names its processors inline.
    assert_eq!(processors_source, &json!("sunrise:stone_to_obsidian"));
}

#[test]
#[ignore = "daily"]
fn a_packs_processor_list_replaces_a_vanilla_one() {
    let decode = |data: Arc<GeneratorData>| {
        let resources = data.structures.as_ref().expect("structure resources");
        let bindings = data.feature_block_bindings().unwrap();
        ProcessorList::decode(r#""minecraft:zombie_plains""#, resources, &bindings).unwrap()
    };
    let packed = decode(with_pack(&[(
        "data/minecraft/worldgen/processor_list/zombie_plains.json",
        json!({"processors": []}).to_string(),
    )]));
    assert!(packed.0.is_empty());
    let vanilla = decode(prepared_with(&WorldgenOverrides::default()).unwrap());
    assert!(!vanilla.0.is_empty());
}

#[test]
#[ignore = "daily"]
fn a_packs_block_state_provider_names_what_a_feature_places() {
    let data = with_pack(&[(
        "data/minecraft/worldgen/block_state_provider/flower_meadow.json",
        json!({"id": "minecraft:obsidian"}).to_string(),
    )]);
    let obsidian = data.registry().default_state("minecraft:obsidian").unwrap();
    let (done, blocks) = placed(&data, FeatureKey::Configured("minecraft:flower_meadow"));
    assert!(done);
    assert_eq!(blocks, BTreeMap::from([([3, 70, 4], obsidian)]));
}

/// The pack's configured feature: one obsidian block.
fn obsidian_feature() -> (&'static str, String) {
    (
        "data/sunrise/worldgen/feature/obsidian.json",
        json!({"type": "minecraft:simple_block", "to_place": {"id": "minecraft:obsidian"}})
            .to_string(),
    )
}

#[test]
#[ignore = "daily"]
fn a_packs_configured_feature_places_its_block() {
    let data = with_pack(&[obsidian_feature()]);
    let obsidian = data.registry().default_state("minecraft:obsidian").unwrap();
    let (done, blocks) = placed(&data, FeatureKey::Configured("sunrise:obsidian"));
    assert!(done);
    assert_eq!(blocks, BTreeMap::from([([3, 70, 4], obsidian)]));
}

#[test]
#[ignore = "daily"]
fn a_packs_placed_feature_places_its_configured_feature() {
    let data = with_pack(&[
        obsidian_feature(),
        (
            "data/sunrise/worldgen/placed_feature/obsidian.json",
            json!({"feature": "sunrise:obsidian", "placement": []}).to_string(),
        ),
    ]);
    let obsidian = data.registry().default_state("minecraft:obsidian").unwrap();
    let (done, blocks) = placed(&data, FeatureKey::Placed("sunrise:obsidian"));
    assert!(done);
    assert_eq!(blocks, BTreeMap::from([([3, 70, 4], obsidian)]));
}
