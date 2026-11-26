//! Item storage queries, client item-use parts, consume effects and recipe panels.

pub mod items_by_kind {
    // ANCHOR: items_by_kind
    use mod_api::{ItemResource, ItemStorage, ServerHost, StorageError};
    use std::ops::ControlFlow;

    pub fn first_at_least(
        storage: &dyn ItemStorage,
        host: &mut dyn ServerHost,
        minimum: u64,
    ) -> Result<Option<(ItemResource, u64)>, StorageError> {
        let mut found = None;
        storage.visit_entries(host, &mut |kind, amount| {
            if amount >= minimum {
                found = Some((kind.clone(), amount));
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })?;
        Ok(found)
    }
    // ANCHOR_END: items_by_kind
}

pub mod client_item_use {
    // ANCHOR: client_item_use
    use std::sync::Arc;

    use mod_api::{
        ClientHost, ClientItemUseBehavior, Hooks, ItemStack, JavaString, Registrar,
        RegistrationContext,
    };

    #[derive(Default)]
    struct ToolMod;
    struct Behavior;

    #[mod_api::sunrise_mod]
    impl Hooks for ToolMod {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let key = r.item_key("mymod:tool");
            r.register_client_item_use(key, Arc::new(Behavior));
        }
    }

    impl ClientItemUseBehavior for Behavior {
        fn use_item(&self, _host: &mut dyn ClientHost, stack: &ItemStack, slot: u32) {
            let data = stack
                .components()
                .and_then(|c| c.as_nbt().get(&JavaString::from("minecraft:custom_data")));
            eprintln!(
                "Tool input started in slot {slot}: {} item(s), custom data present: {}",
                stack.count(),
                data.is_some(),
            );
        }

        fn release_using(
            &self,
            _host: &mut dyn ClientHost,
            _stack: &ItemStack,
            slot: u32,
            ticks_held: u32,
        ) {
            eprintln!("Tool input released in slot {slot} after {ticks_held} ticks");
        }

        fn on_stop_using(
            &self,
            _host: &mut dyn ClientHost,
            _stack: &ItemStack,
            slot: u32,
            _ticks_held: u32,
        ) {
            eprintln!("Tool input ended in slot {slot}");
        }
    }
    // ANCHOR_END: client_item_use
}

pub mod consume_effects {
    // ANCHOR: register_consume_effect
    use mod_api::{Compound, ConsumeEffectType, Identifier, Registrar, Tag, Wire};

    #[derive(Wire)]
    struct Charge {
        points: u16,
    }

    pub fn register(r: &mut dyn Registrar) -> Result<(), String> {
        r.register_consume_effect_type(
            Identifier::parse("example:charge").map_err(|e| e.to_string())?,
            ConsumeEffectType::new::<Charge>(
                |fields| match fields.get(&"points".into()) {
                    Some(Tag::Int(n)) => u16::try_from(*n)
                        .map(|points| Charge { points })
                        .map_err(|e| e.to_string()),
                    _ => Err("expected integer points".into()),
                },
                |effect| {
                    Ok(Compound::from([(
                        "points".into(),
                        Tag::Int(effect.points.into()),
                    )]))
                },
            ),
        )
    }
    // ANCHOR_END: register_consume_effect
}

pub mod panels {
    // ANCHOR: show_planks
    use mod_api::{RecipeLayout, RecipePage, RecipeResult, RecipeView};

    pub fn show_planks(
        host: &mut dyn mod_api::ClientHost,
        log: mod_api::Stack,
        planks: mod_api::Stack,
    ) {
        let craft = RecipeView::grid(1, 1)
            .with_slots(vec![vec![log.clone()]])
            .with_results(vec![RecipeResult::new(vec![planks])]);
        let smelt = RecipeView::new(RecipeLayout::Cooking)
            .with_slots(vec![vec![log]])
            .with_note("10 s")
            .with_action(true);
        let page = RecipePage::new("Oak Planks", vec![craft, smelt]);
        host.panel_recipes("my_viewer", Some(page));
    }
    // ANCHOR_END: show_planks

    pub fn clear_panel_search(host: &mut dyn mod_api::ClientHost) {
        // ANCHOR: clear_panel_search
        // In the viewer's own panel_event hook (its mod id is "jei"): the player
        // pressed its "clear" key, so the viewer empties its own search box.
        host.panel_search("jei", "");
        // ANCHOR_END: clear_panel_search
    }
}
