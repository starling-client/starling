//! The mesh visibility design's plan rules that need no resources: combining answers, the
//! border from the neighbours, the cells that stop occluding, the raw input and the no-mod path.
use super::*;
use crate::chunk::{Chunk, MIN_Y};
use std::sync::Arc;

fn answer(edit: impl FnOnce(&mut SectionPlanBuilder)) -> Option<SectionAnswer> {
    let mut builder = SectionPlanBuilder::new();
    edit(&mut builder);
    builder.finish()
}

#[test]
fn answers_that_change_nothing_keep_the_no_plan_path() {
    assert_eq!(answer(|_| {}), None);
    assert_eq!(answer(|b| b.restrict([3, 4, 5], true, true, 63)), None);
    assert!(plan_section(None, |_| None).is_none());
    let none = answer(|_| {});
    assert!(plan_section(none.as_ref(), |_| None).is_none());
}

#[test]
fn answers_combine_by_intersection_and_never_restore() {
    let combined = answer(|b| {
        b.restrict([1, 2, 3], true, true, 0b11_1101);
        // A later mod keeps everything: the up face stays removed.
        b.restrict([1, 2, 3], true, true, 63);
        b.restrict([1, 2, 3], false, true, 0b11_1110);
    })
    .unwrap();
    let plan = plan_section(Some(&combined), |_| None).unwrap();
    assert_eq!(plan.kept([1, 2, 3]), FLUID | 0b11_1100);
    assert_eq!(plan.kept([0, 0, 0]), ALL);
    assert!(!plan.hides_terrain());
}

#[test]
fn hide_terrain_wins_over_cell_answers() {
    let hidden = answer(|b| {
        b.restrict([0, 0, 0], true, true, 1);
        b.hide_terrain();
    })
    .unwrap();
    assert!(hidden.hides_terrain());
    assert!(
        plan_section(Some(&hidden), |_| None)
            .unwrap()
            .hides_terrain()
    );
}

#[test]
fn the_border_reads_the_neighbours_answers() {
    // The east neighbour hides its cell (0, 4, 7); the section above hides all its terrain.
    let east = answer(|b| b.restrict([0, 4, 7], false, false, 0)).unwrap();
    let above = answer(SectionPlanBuilder::hide_terrain).unwrap();
    let plan = plan_section(None, |d| match d {
        [1, 0, 0] => Some(&east),
        [0, 1, 0] => Some(&above),
        _ => None,
    })
    .unwrap();
    assert!(!plan.hides_terrain());
    assert_eq!(plan.kept([16, 4, 7]), 0);
    assert_eq!(plan.kept([16, 4, 8]), ALL);
    assert_eq!(
        plan.kept([5, 16, 9]),
        0,
        "a hidden section above occludes nothing"
    );
    assert_eq!(
        plan.kept([16, 16, 7]),
        ALL,
        "the diagonal above-east has no answer"
    );
    for local in [[0, 4, 7], [15, 4, 7], [5, 15, 9]] {
        assert_eq!(plan.kept(local), ALL, "{local:?}: the section's own cells");
    }
}

#[test]
fn only_outer_cells_change_the_neighbours() {
    let inner = answer(|b| b.restrict([7, 7, 7], false, false, 0)).unwrap();
    let outer = answer(|b| b.restrict([15, 7, 7], false, false, 0)).unwrap();
    let corner = answer(|b| b.restrict([0, 0, 0], true, true, 0)).unwrap();
    assert!(!SectionAnswer::border_differs(Some(&inner), None));
    assert!(SectionAnswer::border_differs(Some(&outer), None));
    assert!(SectionAnswer::border_differs(None, Some(&corner)));
    assert!(SectionAnswer::border_differs(Some(&outer), Some(&corner)));
    assert!(!SectionAnswer::border_differs(
        Some(&outer),
        Some(&outer.clone())
    ));
}

#[test]
fn a_cell_with_anything_removed_stops_being_solid() {
    let partial = answer(|b| b.restrict([4, 0, 15], true, true, 0b11_1110)).unwrap();
    let plan = plan_section(Some(&partial), |_| None).unwrap();
    let mut rows = vec![u32::MAX; P * P];
    open_masked(&plan, &mut rows);
    let row = |y: i32, z: i32| rows[((y + B) as usize) * P + (z + B) as usize];
    assert_eq!(row(0, 15), !(1u32 << (4 + B)));
    assert_eq!(row(0, 14), u32::MAX);
    assert!(occludes(ALL, 3));
    assert!(
        !occludes(ALL & !MODEL, 3),
        "a model not drawn occludes nothing"
    );
    assert!(
        !occludes(ALL & !(1 << 3), 3),
        "a removed face occludes nothing"
    );
    assert!(occludes(ALL & !(1 << 2), 3));
    let mut visible = [true; 6];
    keep_faces(0b11_1101, &mut visible);
    assert_eq!(visible, [true, false, true, true, true, true]);
    let mut culled = [false; 6];
    keep_faces(63, &mut culled);
    assert_eq!(culled, [false; 6], "a mask never shows a culled face");
}

#[test]
fn the_raw_input_reads_the_world_and_names_missing_columns() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        mod_api::__private::resolve_block_keys(["minecraft:air"], |_| {
            sunrise_mod_runtime::block_definitions::BlockDefinition::new(
                &[&std::collections::BTreeMap::new()],
                0,
            )
            .map_err(|error| format!("{error:?}"))
        })
        .unwrap();
        let block = mod_api::__private::block_key("minecraft:air").unwrap();
        let state = |raw: u32| mod_api::__private::block_state_from_ordinal(block, raw as u16);
        let mut chunk = Chunk::empty(0, 0);
        chunk.set(15, MIN_Y + 31, 15, 7);
        let mut east = Chunk::empty(1, 0);
        east.set(0, MIN_Y + 16, 0, 9);
        let biomes = Biomes::from_jsons(&[
            ("minecraft:plains", serde_json::json!({})),
            ("minecraft:desert", serde_json::json!({})),
        ]);
        chunk.set_biome_cell(0, 4, 0, biomes.id("minecraft:plains"));
        chunk.set_biome_cell(1, 4, 0, u16::MAX);
        east.set_biome_cell(0, 4, 0, biomes.id("minecraft:desert"));
        let mut region: Region = Default::default();
        region[1][1] = Some(Arc::new(chunk));
        region[1][2] = Some(Arc::new(east));
        let desert = mod_api::BiomeKey::parse("minecraft:desert").unwrap();
        let plains = mod_api::BiomeKey::parse("minecraft:plains").unwrap();
        let input = capture_input(&region, 1, state, &biomes);
        assert_eq!(input.cell([15, 15, 15]), Ok(SectionCell::State(state(7))));
        assert_eq!(input.cell([16, 0, 0]), Ok(SectionCell::State(state(9))));
        assert_eq!(input.biome([16, 0, 0]), Ok(Some(&desert)));
        assert_eq!(input.cell([0, 0, 0]), Ok(SectionCell::State(state(0))));
        assert_eq!(input.biome([0, 0, 0]), Ok(Some(&plains)));
        assert_eq!(input.cell([4, 0, 0]), Ok(SectionCell::State(state(0))));
        assert_eq!(input.biome([4, 0, 0]), Ok(None));
        assert_eq!(input.cell([-1, 0, 0]), Ok(SectionCell::Missing));
        assert_eq!(input.cell([16, 0, 16]), Ok(SectionCell::Missing));
        assert_eq!(input.light([16, 0, 16]), Ok([0, 0]));
        assert_eq!(input.biome([16, 0, 16]), Ok(None));
        assert_eq!(
            input.cell([17, 0, 0]),
            Err(SectionVisibilityRefusal::OutsideInput)
        );
        assert_ne!(input.model_seed([15, 0, 0]), input.model_seed([16, 0, 0]));
    });
}

#[test]
fn the_api_checks_answers_whole() {
    use mod_api::{CellVisibility, SectionVisibility as V, SectionVisibilityRefusal as R};
    let cell = |pos, faces| CellVisibility::new(pos, true, true, faces);
    assert_eq!(V::keep([cell([0, 0, 0], 63)]).unwrap().check(), Ok(()));
    assert_eq!(V::hide_terrain().check(), Ok(()));
    let invalid = V::keep([cell([1, 1, 1], 61), cell([2, 2, 2], 64)]).unwrap();
    assert_eq!(
        invalid.check(),
        Err(R::FaceMask {
            pos: [2, 2, 2],
            faces: 64
        })
    );
    assert_eq!(
        V::keep([cell([1, 1, 1], 0), cell([1, 1, 1], 1)])
            .unwrap()
            .check(),
        Err(R::Duplicate { pos: [1, 1, 1] })
    );
    assert_eq!(
        V::keep([cell([16, 0, 0], 0)]).unwrap().check(),
        Err(R::OutsideSection { pos: [16, 0, 0] })
    );
    let mut plan = SectionPlanBuilder::new();
    plan.apply(&V::keep([cell([0, 0, 0], 61)]).unwrap())
        .unwrap();
    assert!(plan.apply(&invalid).is_err());
    let answer = plan.finish().unwrap();
    let plan = plan_section(Some(&answer), |_| None).unwrap();
    assert_eq!(plan.kept([0, 0, 0]) & FACES, 61);
    assert_eq!(
        plan.kept([1, 1, 1]),
        ALL,
        "a bad record refuses the valid prefix"
    );
    assert_eq!(plan.kept([2, 2, 2]), ALL);
}

/// Cached answers and frozen plans are charged to the workers' shared in-flight bytes: past the
/// limit a charge is refused, and a dropped lease gives its bytes back.
#[test]
fn leases_charge_the_shared_in_flight_bytes_and_give_them_back() {
    use super::super::mod_geometry::MAX_IN_FLIGHT_BYTES;
    static SHARED: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(MAX_IN_FLIGHT_BYTES - 2 * PLAN_BYTES - ANSWER_BYTES);
    let plan = Lease::charge_in(&SHARED, PLAN_BYTES).unwrap();
    let other = Lease::charge_in(&SHARED, PLAN_BYTES).unwrap();
    let answer = Lease::charge_in(&SHARED, ANSWER_BYTES).unwrap();
    assert!(
        Lease::charge_in(&SHARED, 1).is_none(),
        "the limit is reached"
    );
    drop(plan);
    assert!(Lease::charge_in(&SHARED, PLAN_BYTES + 1).is_none());
    let again = Lease::charge_in(&SHARED, PLAN_BYTES).expect("a dropped lease's bytes are free");
    drop((other, answer, again));
    assert_eq!(
        SHARED.load(std::sync::atomic::Ordering::Acquire),
        MAX_IN_FLIGHT_BYTES - 2 * PLAN_BYTES - ANSWER_BYTES
    );
}
