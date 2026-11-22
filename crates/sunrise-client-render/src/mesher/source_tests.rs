//! Original synthetic block/model/world controls; no prepared game resources.
use super::*;
use crate::assets::{Properties, SpritePixels};
use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf};
use sunrise_world::registry::BlockRegistry;

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn fixture() -> (Scratch, Assets, RenderAssets, MeshTables, Biomes) {
    let path = std::env::temp_dir().join(format!(
        "terrain-source-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).unwrap();
    let write = |name: &str, value: serde_json::Value| {
        let file = path.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, value.to_string()).unwrap();
    };
    let registry = json!({"defaults":{"minecraft:air":0,"fixture:signal":1},"states":[
        ["air",true,false,"", "minecraft:air"],
        ["signal",false,false,"",{"id":"fixture:signal","properties":{"active":"false","waterlogged":"false"}}],
        ["signal",false,true,"",{"id":"fixture:signal","properties":{"active":"true","waterlogged":"true"}}]
    ]});
    let registry = Box::leak(Box::new(
        BlockRegistry::from_facts_json(registry.to_string().as_bytes()).unwrap(),
    ));
    write(
        "assets/fixture/blockstates/signal.json",
        json!({"multipart":[
            {"apply":{"model":"fixture:block/low"}}, {"apply":{"model":"fixture:block/extended"}}
        ]}),
    );
    for (name, from, to) in [
        ("low", [0, 4, 0], [8, 4, 8]),
        ("extended", [20, 12, 0], [28, 12, 8]),
    ] {
        write(
            &format!("assets/fixture/models/block/{name}.json"),
            json!({
                "textures":{"all":"fixture:block/shared"},
                "elements":[{"from":from,"to":to,"faces":{"up":{"texture":"#all"}}}]
            }),
        );
    }
    // The same authored sprite is used by every part of both states.
    let png_path = path.join("assets/fixture/textures/block/shared.png");
    std::fs::create_dir_all(png_path.parent().unwrap()).unwrap();
    let encode = |file: PathBuf| {
        let mut encoder = png::Encoder::new(std::fs::File::create(file).unwrap(), 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[63, 127, 191, 255])
            .unwrap();
    };
    encode(png_path);
    let facts = BTreeMap::from([(
        "fixture:signal".into(),
        json!({
            "schema":{"active":{"named":["false","true"]},"waterlogged":{"named":["false","true"]}},
            "states":[{"active":"false","waterlogged":"false"},{"active":"true","waterlogged":"true"}]
        }),
    )]);
    let mut sprites = crate::sprites::Sprites::default();
    let catalog = crate::models::StateModels::from_facts(
        &sunrise_assets::resource_source::ResourceSource::directory(path.clone()),
        registry,
        &["fixture:signal"],
        &mut sprites,
        &facts,
    )
    .unwrap();
    let render = RenderAssets {
        sprite_size: 1,
        sprite_minimum_size: 1,
        pixels: SpritePixels::new(vec![255; 4]),
        texture_layers: 1,
        model_catalog: catalog,
        model_library: Err("unused synthetic item models".into()),
        model_placements: vec![],
        model_gallery: vec![],
        animations: vec![],
        sprite_layers: BTreeMap::new(),
        sprite_names: vec![None],
        file_layers: BTreeMap::new(),
        emissive: vec![],
        translucent_layers: vec![false],
        sprite_mips: vec![],
        pbr_normals: vec![],
        pbr_specular: vec![],
        pbr_size: 1,
        connected: Default::default(),
        fusion: Default::default(),
        mod_animations: Default::default(),
        mod_pipelines: Default::default(),
        generation: 1,
    };
    let assets = Assets {
        registry,
        properties: Properties::for_tests(&[
            (true, false, false, false),
            (false, false, false, false),
            (false, true, false, false),
        ]),
        material_states: [BlockStateId(0); 6],
        blocks_fluid_flow: Default::default(),
    };
    let air = Info {
        air: true,
        solid: false,
        full: false,
        permeable: true,
        shade: 1.0,
        emission: 0,
        tier: 0,
        emissive: false,
        fluid: FluidState::EMPTY,
        model: false,
        block_entity: false,
        tints: [Tint::None; 2],
        tint_count: 0,
        colormap: None,
        offset: None,
        legacy_solid: false,
        overlay: false,
        leaves: false,
        pane: false,
        opaque_cube: false,
        blocks_flow: false,
        ice: false,
        opaque_ice: false,
        sturdy: 0,
        face: [0; 6],
        cover: [0.0; 6],
    };
    let info = vec![
        air,
        Info {
            air: false,
            model: true,
            ..air
        },
        Info {
            air: false,
            model: true,
            emission: 13,
            fluid: FluidState::WATER,
            ..air
        },
    ];
    let tables = MeshTables {
        info,
        solid: vec![false; 3],
        full_faces: vec![0; 3],
        visibility: FaceVisibility::empty(),
        water_still: 0,
        water_flow: 0,
        water_overlay: 0,
        lava_still: 0,
        lava_flow: 0,
        animated_layers: vec![false],
        colormaps: Vec::new(),
        water_colormap: None,
        mod_fluids: vec![],
        mod_tints: Default::default(),
        section_geometry: std::sync::Arc::new([]),
    };
    let maps = path.join("assets/minecraft/textures/colormap");
    std::fs::create_dir_all(&maps).unwrap();
    for name in ["grass", "foliage", "dry_foliage"] {
        encode(maps.join(format!("{name}.png")));
    }
    write(
        "data/minecraft/worldgen/biome/plains.json",
        json!({"temperature":0.5,"downfall":0.5,"effects":{"water_color":4159204}}),
    );
    let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
        path.clone(),
    ))
    .unwrap();
    (Scratch(path), assets, render, tables, biomes)
}

#[test]
fn baked_model_particle_uses_the_published_sprite_name_at_its_layer() {
    let (_scratch, _, mut render, _, _) = fixture();
    let key = render
        .block_state_models(BlockStateId(1))
        .iter()
        .next()
        .unwrap();
    // Not the quad's texture, the model slot, or the first name in the atlas.
    render.sprite_names = vec![
        Some("fixture:block/other".into()),
        None,
        Some("fixture:block/particle".into()),
    ];
    for (_, model) in render.model_catalog.models_mut() {
        model.particle = Some(2);
    }
    let name = render.baked_model(key).unwrap().particle.unwrap();
    assert_eq!(name, "fixture:block/particle");
    assert!(std::ptr::eq(
        name,
        render.sprite_names[2].as_deref().unwrap()
    ));

    render.sprite_names[2] = None;
    assert_eq!(render.baked_model(key).unwrap().particle, None);
    render.sprite_names.truncate(2);
    assert_eq!(render.baked_model(key).unwrap().particle, None);
    for (_, model) in render.model_catalog.models_mut() {
        model.particle = None;
    }
    assert_eq!(render.baked_model(key).unwrap().particle, None);
}

#[test]
fn terrain_source_cpu_multipart_and_waterlogged_faces_keep_their_owner() {
    let (_scratch, assets, render, tables, biomes) = fixture();
    let mut chunk = Chunk::with_layout(3, -2, crate::chunk::ColumnLayout::new(0, 16).unwrap());
    chunk.set(2, 3, 4, 1);
    chunk.set(6, 3, 4, 2);
    let region = [
        [None, None, None],
        [None, Some(&chunk), None],
        [None, None, None],
    ];
    let mut mesher = Mesher {
        hidden_states: &[],
        part_tables: Default::default(),
        nether_shading: false,
        shader_pack_lighting: false,
        separate_ao: false,
        shader_pack: true,
        assets: &assets,
        render: &render,
        tables: &tables,
        biomes: &biomes,
        visibility: FaceVisibility::empty(),
        zoom: BiomeZoom::new(0),
        blend_radius: 0,
        smooth_lighting: false,
        cutout_leaves: true,
        opaque_ice: false,
        translucent_layers: &[],
        leaf_culling: LeafCullingMode::Off,
    };
    let mut mesh = mesher.mesh(&region, 0);
    assert_eq!(mesh.solid.len(), 16, "two parts per source state");
    assert!(
        !mesh.translucent.is_empty(),
        "waterlogged fluid geometry was emitted"
    );
    assert_eq!(
        mesh.solid_sources,
        [
            TerrainSource::new(1, [2, 3, 4], 0, false),
            TerrainSource::new(1, [2, 3, 4], 0, false),
            TerrainSource::new(2, [6, 3, 4], 13, false),
            TerrainSource::new(2, [6, 3, 4], 13, false)
        ]
    );
    assert_eq!(mesh.translucent_sources.len(), mesh.translucent.len() / 4);
    assert!(
        mesh.translucent_sources
            .iter()
            .all(|s| *s == TerrainSource::new(2, [6, 3, 4], 13, true))
    );
    let before: Vec<_> = mesh
        .solid
        .as_chunks::<4>()
        .0
        .iter()
        .zip(&mesh.solid_sources)
        .map(|(q, s)| (bytemuck::cast_slice::<_, u8>(q).to_vec(), *s))
        .collect();
    mesh.sort_solid();
    for (q, s) in mesh
        .solid
        .as_chunks::<4>()
        .0
        .iter()
        .zip(&mesh.solid_sources)
    {
        assert!(
            before.contains(&(bytemuck::cast_slice::<_, u8>(q).to_vec(), *s)),
            "sort changed face ownership"
        );
    }
    // Reusing the same mesh for native/default rendering clears metadata and keeps
    // native model vertices byte-for-byte (fluid splitting already differs with packs).
    mesher.shader_pack = false;
    let native = mesher.mesh_reusing(&region, 0, mesh, &mut MeshScratch::default());
    assert!(native.solid_sources.is_empty() && native.translucent_sources.is_empty());
    for q in native.solid.as_chunks::<4>().0.iter() {
        assert!(
            before
                .iter()
                .any(|(bytes, _)| bytes == bytemuck::cast_slice::<_, u8>(q)),
            "source tracking altered native vertices"
        );
    }
}

#[test]
fn a_folder_without_connected_texture_rules_meshes_exactly_as_before() {
    let (scratch, assets, mut render, tables, biomes) = fixture();
    let mut chunk = Chunk::with_layout(3, -2, crate::chunk::ColumnLayout::new(0, 16).unwrap());
    chunk.set(2, 3, 4, 1);
    chunk.set(3, 3, 4, 1);
    chunk.set(6, 3, 4, 2);
    let region = [
        [None, None, None],
        [None, Some(&chunk), None],
        [None, None, None],
    ];
    let bytes = |render: &RenderAssets| {
        let mut mesher = Mesher {
            hidden_states: &[],
            part_tables: Default::default(),
            nether_shading: false,
            shader_pack_lighting: false,
            separate_ao: false,
            shader_pack: false,
            assets: &assets,
            render,
            tables: &tables,
            biomes: &biomes,
            visibility: FaceVisibility::empty(),
            zoom: BiomeZoom::new(0),
            blend_radius: 0,
            smooth_lighting: false,
            cutout_leaves: true,
            opaque_ice: false,
            translucent_layers: &[],
            leaf_culling: LeafCullingMode::Off,
        };
        let mesh = mesher.mesh(&region, 0);
        let both = [&mesh.solid, &mesh.translucent];
        both.map(|v| bytemuck::cast_slice::<_, u8>(v).to_vec())
    };
    let before = bytes(&render);
    assert!(!before[0].is_empty());
    let source = sunrise_assets::resource_source::ResourceSource::directory(scratch.0.clone());
    render.connected = crate::connected::ConnectedTextures::load(
        &source,
        crate::connected::read_rules(&source),
        assets.registry,
        &mut crate::sprites::Sprites::default(),
        |_, _| None,
        |_| false,
    );
    assert!(render.connected.is_empty(), "nothing declares a rule here");
    assert_eq!(bytes(&render), before);
    // A rule that matches another block leaves the mesh the same.
    let sheet = sunrise_connected::TableSheet {
        sprite: "fixture:block/low".into(),
        table: std::sync::Arc::new(
            sunrise_connected::Table::new(Vec::new(), false, (1, 1), vec![0]).unwrap(),
        ),
    };
    let mut other = sunrise_connected::Rule::table("other.json".into(), sheet);
    other.method = sunrise_connected::Method::Fixed;
    other.table = None;
    other.tiles = vec![sunrise_connected::Tile::Default];
    other.match_blocks = vec![sunrise_connected::BlockSel {
        id: "fixture:other".into(),
        states: Vec::new(),
    }];
    render.connected = crate::connected::ConnectedTextures::load(
        &source,
        vec![other],
        assets.registry,
        &mut crate::sprites::Sprites::default(),
        |_, _| None,
        |_| false,
    );
    assert!(!render.connected.is_empty());
    assert_eq!(bytes(&render), before);
}

#[test]
fn terrain_source_cpu_partial_metadata_never_inherits_a_previous_face() {
    let mut mesh = SectionMesh::default();
    let quad = |y| {
        [[0.0, y, 0.0], [0.0, y, 1.0], [1.0, y, 1.0], [1.0, y, 0.0]]
            .map(|pos| TerrainVertex::new(pos, [0.0; 2], 0, 0, 0))
    };
    mesh.solid.extend(quad(1.0));
    let start = [mesh.solid.len(), 0];
    mesh.solid.extend(quad(3.0));
    mesh.solid.extend(quad(2.0));
    mesh.record_source(start, TerrainSource::new(71, [15, 0, 8], 7, false));
    assert_eq!(mesh.solid_sources[0], TerrainSource::default());
    assert_eq!(
        bytemuck::cast_slice::<_, u32>(&mesh.solid_sources[1..]),
        [71, 124943, 71, 124943]
    );
    mesh.sort_solid();
    assert_eq!(
        mesh.solid
            .as_chunks::<4>()
            .0
            .iter()
            .map(|q| q[0].pos[1])
            .collect::<Vec<_>>(),
        [3.0, 2.0, 1.0]
    );
    assert_eq!(
        mesh.solid_sources,
        [
            TerrainSource::new(71, [15, 0, 8], 7, false),
            TerrainSource::new(71, [15, 0, 8], 7, false),
            TerrainSource::default()
        ]
    );
    mesh.clear();
    assert!(mesh.solid_sources.is_empty());
}
