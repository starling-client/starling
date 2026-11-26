use super::{DataRef, Target};
use crate::game::native_entity_refs::tests::with_game;
use std::path::PathBuf;
use sunrise_core::nbt::{Tag, parse_snbt};

fn load_witness() -> String {
    let raw = std::env::var_os("SUNRISE_EYE_ITEM_WITNESS").unwrap_or_else(|| {
        panic!(
            "SUNRISE_EYE_ITEM_WITNESS is unset; run tools/test_eye_item_parity.py, which records the witness"
        )
    });
    let path = PathBuf::from(raw);
    let path = if path.is_dir() {
        path.join("eye_item.tsv")
    } else {
        path
    };
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read eye_item witness {} (run tools/test_eye_item_parity.py): {e}",
            path.display()
        )
    })
}

fn parse_exact_snbt(text: &str) -> Tag {
    let (tag, used) = parse_snbt(text).expect("valid recorded 26.3 SNBT");
    assert_eq!(
        used,
        text.len(),
        "trailing bytes in recorded 26.3 SNBT: {text}"
    );
    tag
}

fn saved_item(game: &mut crate::game::Game<'_>, id: u32) -> Tag {
    let tag = game.data_of(&DataRef::Entity(Target::Other(id))).unwrap();
    tag.get("Item")
        .cloned()
        .expect("an eye of ender always saves Item")
}

/// Matches the real vanilla 26.3 witness recorded by `tools/test_eye_item_parity.py`.
#[test]
#[ignore = "run by tools/test_eye_item_parity.py"]
fn a_summoned_eye_saves_its_normalised_item_live_and_after_a_reload() {
    run_a_summoned_eye_saves_its_normalised_item_live_and_after_a_reload();
}

pub(in crate::game) fn run_a_summoned_eye_saves_its_normalised_item_live_and_after_a_reload() {
    let witness = load_witness();
    assert!(
        witness.contains("# minecraft_version\t26.3"),
        "witness must be tagged with 26.3"
    );
    with_game(|game| {
        let mut checked = 0usize;
        for line in witness.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut cols = line.split('\t');
            let case = cols.next().expect("case column");
            let nbt = cols.next().expect("summon_nbt column");
            let live_expected = parse_exact_snbt(cols.next().expect("live_item_snbt column"));
            let reloaded_expected =
                parse_exact_snbt(cols.next().expect("reloaded_item_snbt column"));
            assert_eq!(cols.next(), None, "unexpected extra column in {case}");

            game.summon_with_nbt("minecraft:eye_of_ender", [5.5, 64.0, 5.5], nbt)
                .unwrap();
            let eye = game.others.last().unwrap().id;
            assert_eq!(
                saved_item(game, eye),
                live_expected,
                "26.3 live, {case}: {nbt}"
            );

            let other = game.others.last().unwrap().clone();
            let uuid = game.uuid_for(eye);
            let saved = game.other_to_nbt(&other, uuid);
            game.others.retain(|o| o.id != eye);
            let reloaded = game
                .load_other_transferred(saved, "minecraft:eye_of_ender", None)
                .unwrap();
            assert_eq!(
                saved_item(game, reloaded),
                reloaded_expected,
                "26.3 reloaded, {case}: {nbt}"
            );
            game.others.retain(|o| o.id != reloaded);
            checked += 1;
        }
        assert!(checked > 0, "expected at least one 26.3 witness row");
    });
}
