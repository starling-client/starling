#![cfg(test)]

#[test]
#[ignore = "daily"]
fn enchantment_registry_returns_records_and_unknown_cost() {
    super::super::native_entity_refs::tests::with_game(|game| {
        let all = game.enchantments();
        let sharpness = all.iter().find(|e| e.id == "minecraft:sharpness").unwrap();
        assert_eq!((sharpness.max_level, sharpness.curse), (5, false));
        let binding = all
            .iter()
            .find(|e| e.id == "minecraft:binding_curse")
            .unwrap();
        assert!(binding.curse);
        assert!(
            game.enchantment_min_cost("minecraft:sharpness", 1)
                .is_some()
        );
        assert_eq!(game.enchantment_min_cost("missing:unknown", 1), None);
    });
}

#[test]
#[ignore = "daily"]
fn translate_gives_the_text_of_a_defined_key_and_none_otherwise() {
    use sunrise_client::lang::{Language, ResourceSource, scoped};
    struct Empty;
    impl ResourceSource for Empty {
        fn read(&self, _: &str) -> Option<Vec<u8>> {
            None
        }
    }
    super::super::native_entity_refs::tests::with_game(|game| {
        let (got, _) = scoped(Language::load(&[&Empty], "en_us"), || {
            (
                game.translate("sunrise.options.controller"),
                game.translate("missing.key"),
            )
        });
        assert_eq!(got, (Some("Controller".to_owned()), None));
    });
}

#[test]
#[ignore = "daily"]
fn a_trait_mod_measures_text_through_its_draw_context() {
    use super::super::host::draw::GameDrawContext;
    use super::super::native_entity_refs::tests::with_game;
    use mod_api::DrawContext;

    with_game(|game| {
        let mut batch = sunrise_client::hud::HudBatch::scratch([320, 240]);
        let draw = GameDrawContext::new(game, &mut batch);
        // A server's HUD atlas is metrics-only (upstream 2e5a2629a): no sprite sheets,
        // while an upper-case A still advances 6 GUI pixels.
        assert_eq!(draw.text_width("AA"), 12.0);
        assert_eq!(draw.text_width(""), 0.0);
    });
}
