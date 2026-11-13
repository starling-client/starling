//! A blockstate-driven inspection gallery, separate from gameplay simulation.
pub mod catalog;
mod cube;
use crate::{Result, sprites::Sprites};
pub use catalog::StateModels;
use catalog::descriptor;
pub use cube::CubeStyle;
use horizon_core::position::BlockPos;
use horizon_world::registry::{BlockRegistry, BlockStateId};
use serde_json::{Value, json};
use std::path::Path;

/// The standalone models native mods asked for (`native_standalone_models`).
static STANDALONE: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
/// Test fixture shared with the resource-bake worker.
static STANDALONE_FIXTURE: std::sync::Mutex<Option<(Vec<String>, usize)>> =
    std::sync::Mutex::new(None);

#[cfg(feature = "test-support")]
static FIXTURE_CHANGED: std::sync::Condvar = std::sync::Condvar::new();

fn standalone_names(mut names: Vec<String>) -> Vec<String> {
    names.retain(|n| {
        n.split_once(':')
            .is_some_and(|(ns, path)| !ns.is_empty() && !path.is_empty())
    });
    names.sort();
    names.dedup();
    names
}

/// Sets the models no blockstate names that every resource load bakes
/// beside the block models ([`StateModels::standalone`]): once, at start,
/// as vanilla's `ModelEvent.RegisterStandalone` is posted once per model
/// discovery. Names are sorted and deduplicated; one without a namespace
/// (`mymod:block/shaft_half`) is dropped. False when already set.
pub fn register_standalone(names: Vec<String>) -> bool {
    STANDALONE.set(standalone_names(names)).is_ok()
}

/// Names baked beside block models. An active test fixture wins over process registration.
pub fn standalone() -> Vec<String> {
    if let Some((names, _)) = STANDALONE_FIXTURE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
    {
        return names.clone();
    }
    STANDALONE.get().cloned().unwrap_or_default()
}

/// Installs standalone names until dropped, including on the bake worker, without sealing
/// process registration. Other fixture holders wait until the active fixture finishes.
#[cfg(feature = "test-support")]
pub struct StandaloneFixture;

#[cfg(feature = "test-support")]
pub fn standalone_fixture(names: Vec<String>) -> StandaloneFixture {
    let names = standalone_names(names);
    let mut fixture = STANDALONE_FIXTURE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    while fixture.is_some() {
        fixture = FIXTURE_CHANGED
            .wait(fixture)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }
    *fixture = Some((names, 1));
    StandaloneFixture
}

#[cfg(feature = "test-support")]
impl Drop for StandaloneFixture {
    fn drop(&mut self) {
        let mut fixture = STANDALONE_FIXTURE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((_, holders)) = fixture.as_mut() {
            *holders -= 1;
            if *holders == 0 {
                *fixture = None;
                FIXTURE_CHANGED.notify_all();
            }
        }
    }
}

type Placement = (&'static str, &'static [(&'static str, &'static str)]);
const GALLERY: [Placement; 16] = [
    (
        "oak_stairs",
        &[
            ("facing", "north"),
            ("half", "top"),
            ("shape", "inner_left"),
        ],
    ),
    ("stone_slab", &[("type", "top")]),
    ("anvil", &[("facing", "east")]),
    ("torch", &[]),
    ("lantern", &[]),
    ("oak_fence", &[("north", "true"), ("east", "true")]),
    ("oak_fence_gate", &[("facing", "west"), ("open", "true")]),
    ("iron_bars", &[("north", "true"), ("south", "true")]),
    ("cobweb", &[]),
    ("grindstone", &[("face", "wall"), ("facing", "west")]),
    ("ladder", &[("facing", "west")]),
    ("amethyst_cluster", &[("facing", "east")]),
    ("stone", &[]),
    ("stone", &[]),
    ("stone", &[]),
    ("stone", &[]),
];

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ModelInstance {
    pub positions: [[f32; 3]; 4],
    pub uv: [[f32; 2]; 4],
    pub layer: u32,
    pub shade: f32,
}
#[derive(Default)]
pub struct PreparedModels {
    pub catalog: StateModels,
    pub placements: Vec<(BlockPos, BlockStateId)>,
    pub states: Vec<Value>,
}
pub fn prepare(
    root: &crate::resource_source::ResourceRoot,
    registry: &BlockRegistry,
    sprites: &mut Sprites,
) -> Result<PreparedModels> {
    prepare_with_models(
        root,
        registry,
        sprites,
        &crate::model_library::ParsedModels::default(),
    )
}
pub(crate) fn prepare_with_models(
    root: &crate::resource_source::ResourceRoot,
    registry: &BlockRegistry,
    sprites: &mut Sprites,
    models: &crate::model_library::ParsedModels,
) -> Result<PreparedModels> {
    let facts: std::collections::BTreeMap<String, Value> =
        horizon_world::json_file::read(&root.prepared().join("model-states.json"))?;
    let blocks: Vec<&str> = facts
        .keys()
        .filter(|name| registry.default_state(name).is_some())
        .map(|name| name.strip_prefix("minecraft:").unwrap_or(name))
        .collect();
    let mut catalog = {
        let _step = crate::spans::startup::step("core.bake_states");
        StateModels::from_facts_with_models(
            root.source(),
            registry,
            &blocks,
            sprites,
            &facts,
            models,
        )?
    };
    {
        let _step = crate::spans::startup::step("core.bake_standalone");
        let names = standalone();
        catalog.bake_standalone(root.source(), sprites, models, &names)?;
    }
    let mut placements = Vec::new();
    let mut states = Vec::new();
    for (index, (name, properties)) in GALLERY.iter().enumerate() {
        let state = registry.resolve(&descriptor(name, properties))?;
        let actual = registry
            .state(state)
            .ok_or("missing gallery registry state")?;
        let position = BlockPos::new((index % 4) as i32 * 4 - 7, 4, (index / 4) as i32 * 4 + 14);
        // A state no definition binds (a pack's partial blockstate file
        // without vanilla's below it) is left out, as the mesher skips it,
        // instead of failing the whole pack.
        let selected = match catalog.collect(state, position) {
            Ok(selected) => selected,
            Err(e) => {
                log::warn!("Horizon: gallery block {name} has no model: {e}");
                continue;
            }
        };
        states.push(json!({"block":actual.name,"properties":actual.properties,"state_id":state.0,
            "position":[position.x,position.y,position.z],
            "models":selected.iter().map(|(v,_)| json!({"model":v.model,"rotations":v.rotations,"uv_lock":v.uv_lock})).collect::<Vec<_>>()}));
        placements.push((position, state));
    }
    Ok(PreparedModels {
        catalog,
        placements,
        states,
    })
}
pub fn instance(face: &catalog::BakedFace, position: BlockPos) -> ModelInstance {
    let origin = [position.x as f32, position.y as f32, position.z as f32];
    ModelInstance {
        positions: face
            .quad
            .positions
            .map(|v| std::array::from_fn(|i| v[i] + origin[i])),
        uv: face.quad.uv,
        layer: face.layer,
        shade: cube::shade(face),
    }
}

pub fn load_visibility(
    root: &Path,
    registry: &BlockRegistry,
) -> Result<horizon_world::block_render::FaceVisibility> {
    use horizon_world::{
        block_render::{FaceVisibility, RenderDefinition, SkipRendering},
        collision::CachedCollisionRegistry,
        registry::BlockTableFile,
    };
    use std::collections::BTreeMap;
    let file = BlockTableFile::open(&root.join("occlusion.nbt"), registry)?;
    let occlusion = CachedCollisionRegistry::from_table(file.table("occlusion")?)?;
    let raw: BTreeMap<String, Value> =
        horizon_world::json_file::read(&root.join("render-definitions.json"))?;
    let definitions = raw
        .into_iter()
        .map(|(name, v)| {
            let skip = match v["rule"].as_str().ok_or("missing render rule")? {
                "never" => SkipRendering::Never,
                "same_block" => SkipRendering::SameBlock,
                "bars" => SkipRendering::Bars,
                "leaves" => SkipRendering::Leaves,
                "liquid" => SkipRendering::Liquid,
                "mangrove_roots" => SkipRendering::MangroveRoots,
                _ => return Err("unrecognized render rule"),
            };
            Ok((
                name,
                RenderDefinition {
                    skip,
                    can_occlude: v["can_occlude"].as_bool().ok_or("missing occlusion flag")?,
                    bars_tag: v["bars_tag"].as_bool().ok_or("missing bars tag")?,
                },
            ))
        })
        .collect::<std::result::Result<_, &str>>()?;
    Ok(FaceVisibility::new(&occlusion, &definitions)?)
}

#[cfg(test)]
mod tests {
    #[test]
    fn standalone_names_are_namespaced_sorted_and_set_once() {
        assert!(super::standalone().is_empty());
        let names = [
            "lanef:block/b",
            "nonamespace",
            ":x",
            "lanef:",
            "lanef:block/a",
            "lanef:block/b",
        ];
        assert!(super::register_standalone(names.map(str::to_owned).into()));
        assert_eq!(
            super::standalone(),
            ["lanef:block/a".to_owned(), "lanef:block/b".to_owned()]
        );
        assert!(!super::register_standalone(vec!["other:block/c".into()]));
        assert_eq!(
            super::standalone(),
            ["lanef:block/a".to_owned(), "lanef:block/b".to_owned()]
        );
    }
}
