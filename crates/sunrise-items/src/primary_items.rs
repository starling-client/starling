//! Tests for the primary-selection reader of the shared answer table.
#[cfg(test)]
mod tests {
    use crate::enchant::tests::{sample, sample_items};
    use std::{collections::HashMap, sync::Arc};
    use sunrise_world::enchantment_answers::Answers;

    fn answers(sharpness: &Arc<str>, item: &str, primary: bool) -> Answers {
        HashMap::from([(item.into(), HashMap::from([(sharpness.clone(), primary)]))])
    }

    #[test]
    fn selection_reads_the_snapshots_own_answers() {
        const PROBE: &str = "test:primary_probe";
        const SWORD: &str = "minecraft:iron_sword";
        let (e, items) = (sample(), sample_items());
        let sharpness: Arc<str> = "minecraft:sharpness".into();
        let at_30 = |e: &crate::enchant::Enchantments, item: &str| {
            e.available_results(&items, 30, item, std::slice::from_ref(&sharpness))
        };
        let level_3 = [(sharpness.clone(), 3)];
        // Vanilla: the probe item isn't primary for sharpness; an iron sword gets level 3,
        // the highest whose window (23 to 43) holds 30.
        assert!(at_30(&e, PROBE).is_empty());
        assert_eq!(at_30(&e, SWORD), level_3);
        e.primary_answers()
            .install(answers(&sharpness, PROBE, true));
        assert_eq!(at_30(&e, PROBE), level_3);
        assert_eq!(at_30(&e, SWORD), level_3);
        // A reload installs the new answers in place of the old.
        e.primary_answers()
            .install(answers(&sharpness, SWORD, false));
        assert!(at_30(&e, SWORD).is_empty());
        assert!(at_30(&e, PROBE).is_empty());
        // Retired: vanilla again.
        e.primary_answers().clear();
        assert_eq!(at_30(&e, SWORD), level_3);
        assert!(e.primary_answers().is_empty());
    }

    #[test]
    fn two_snapshots_keep_their_own_answers() {
        let items = sample_items();
        let (a, b) = (sample(), sample());
        let sharpness: Arc<str> = "minecraft:sharpness".into();
        let at_30 = |e: &crate::enchant::Enchantments| {
            e.available_results(
                &items,
                30,
                "minecraft:iron_sword",
                std::slice::from_ref(&sharpness),
            )
        };
        a.primary_answers()
            .install(answers(&sharpness, "minecraft:iron_sword", false));
        // b never resolved: vanilla. Installing into b leaves a's answer alone.
        assert!(at_30(&a).is_empty());
        assert_eq!(at_30(&b), [(sharpness.clone(), 3)]);
        b.primary_answers().install(Answers::new());
        assert!(at_30(&a).is_empty());
        // A clone carries the answers it was copied with, and is its own table after.
        let copy = a.clone();
        a.primary_answers().clear();
        assert!(at_30(&copy).is_empty());
        assert_eq!(at_30(&a), [(sharpness.clone(), 3)]);
    }
}
