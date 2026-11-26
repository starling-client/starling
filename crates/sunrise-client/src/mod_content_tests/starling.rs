//! Starling's part of mod_content_tests.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// A block with a shape of its own on a full template (a disc on a log): each state's outline and
/// collision are the boxes turned to its axis, and every table still loads.
#[test]
pub(super) fn a_declared_shape_is_each_states_outline_and_collision() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-shape-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let toml = manifest("cogs", "");
    let m = folder_mod(
        &scratch.join("cogs"),
        &[
            ("sunrise-mod.toml", &toml),
            (
                "data/cogs/sunrise/block/disc.json",
                r#"{"template": "minecraft:oak_log",
                    "properties": {"shape": [[0, 6, 0, 16, 10, 16]]}}"#,
            ),
        ],
    );
    let root = scratch.join("root");
    build_fixture(&base, &base.join("jars"), &[m], &root).unwrap();
    let assets = Assets::load(&root).unwrap();
    let registry = assets.registry;
    let facts = BlockFacts::load(&root, registry).unwrap();
    let state = |axis: &str| {
        let mut s = registry
            .state(registry.default_state("cogs:disc").unwrap())
            .unwrap()
            .clone();
        s.properties.insert("axis".into(), axis.into());
        registry.id(&s).unwrap()
    };
    // Made lying flat (along y), standing up across x and across z.
    assert_eq!(
        &*facts.get(state("y")).outline,
        [[0.0, 0.375, 0.0, 1.0, 0.625, 1.0]]
    );
    assert_eq!(
        &*facts.get(state("x")).outline,
        [[0.375, 0.0, 0.0, 0.625, 1.0, 1.0]]
    );
    assert_eq!(
        &*facts.get(state("z")).outline,
        [[0.0, 0.0, 0.375, 1.0, 1.0, 0.625]]
    );
    assert_eq!(facts.get(state("y")).outline_edges.len(), 12);
    // It occludes nothing, and its template still collides as a whole block.
    assert!(!facts.get(state("y")).can_occlude);
    let log = facts.get(registry.default_state("minecraft:oak_log").unwrap());
    assert!(log.full_collision);
    let _ = fs::remove_dir_all(&scratch);
}
