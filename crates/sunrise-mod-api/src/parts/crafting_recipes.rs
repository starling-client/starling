//! Validate ownership and freeze crafting-kind registrations.
use super::*;
impl RecipeSerializersRegistrar for PartRegistrar {
    fn register_recipe_type(&mut self, name: &str) {
        let id = match self.owned_declaration_id(name) {
            Ok(id) => id,
            Err(error) => {
                self.refused.push(error);
                return;
            }
        };
        if self.parts.recipe_types.contains_key(id.as_str()) {
            self.refused
                .push(format!("{} registered recipe type {id} twice", self.mod_id));
        } else {
            self.parts.recipe_types.insert(id.into(), self.mod_index);
        }
    }

    fn add_recipe_serializer(
        &mut self,
        name: &str,
        recipe_type: &str,
        handler: RegisteredRecipeSerializer,
    ) {
        let id = match self.owned_declaration_id(name) {
            Ok(id) => id,
            Err(error) => {
                self.refused.push(error);
                return;
            }
        };
        if self.parts.recipe_serializers.contains_key(id.as_str()) {
            self.refused.push(format!(
                "{} registered recipe serializer {id} twice",
                self.mod_id
            ));
            return;
        }
        let handler = handler.bind(self.mod_id, self.mod_index, id.as_str());
        self.parts
            .recipe_serializers
            .insert(id.into(), (self.mod_index, recipe_type.into(), handler));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Context;
    impl crate::LoopContext for Context {
        fn is_in_tag(&self, _: crate::ItemKey, _: crate::TagKey) -> Option<bool> {
            None
        }
        fn items_in_tag(&self, _: crate::TagKey) -> Option<&[crate::ItemKey]> {
            None
        }
        fn side(&self) -> crate::LoopSide {
            crate::LoopSide::Server
        }
        fn registry(&self, _: &str) -> Option<&[String]> {
            None
        }
        fn setting(&self, _: &str, _: &str) -> Option<&str> {
            None
        }
        fn mods(&self) -> &crate::LoadedMods {
            unreachable!("not used by this fixture")
        }
        fn mod_recipe_property_contains(&self, _: &str, _: crate::ItemKey) -> bool {
            false
        }
    }
    impl crate::CraftingRemainders for Context {
        fn default_remainder(&self, _: u32) -> Option<crate::ItemStack> {
            None
        }
    }
    struct Count(u32);
    struct Literal {
        result: crate::ItemKey,
    }
    impl crate::CraftingRecipeKind for Literal {
        type Value = Count;
        fn decode(&self, json: &str) -> Result<Count, String> {
            let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
            value["count"]
                .as_u64()
                .filter(|n| *n == 3)
                .map(|n| Count(n as u32))
                .ok_or("count must be 3".into())
        }
        fn write(&self, value: &Count) -> Result<String, String> {
            Ok(format!("{{\"count\":{}}}", value.0))
        }
        fn matches(
            &self,
            _: &dyn crate::LoopContext,
            value: &Count,
            grid: crate::CraftingGrid,
        ) -> bool {
            value.0 == 3
                && grid.width == 3
                && grid.height == 3
                && grid
                    .cells
                    .iter()
                    .enumerate()
                    .all(|(i, c)| c.is_some() == [0, 4, 8].contains(&i))
        }
        fn assemble(
            &self,
            _: &dyn crate::LoopContext,
            value: &Count,
            _: crate::CraftingGrid,
        ) -> Option<crate::ItemStack> {
            crate::ItemStack::new(self.result, value.0)
        }
        fn remaining_items(
            &self,
            _: &dyn crate::CraftingRemainders,
            value: &Count,
            _: crate::CraftingGrid,
        ) -> Option<Vec<Option<crate::ItemStack>>> {
            let mut cells = vec![None; 9];
            cells[4] = crate::ItemStack::new(self.result, value.0 / 2);
            Some(cells)
        }
    }
    struct Recipes;
    #[mod_api::__subscriptions]
    impl crate::Hooks for Recipes {
        fn register(&self, r: &mut dyn crate::Registrar, _: &crate::RegistrationContext) {
            let key = r.item_key("test:result");
            r.register_crafting_recipe_serializer("diagonal", Literal { result: key });
        }
    }
    #[test]
    fn registered_kind_decodes_matches_assembles_and_places_literal_remainder() {
        crate::parts::tests::install_parts_fixture(&[crate::ModEntry::new(
            "test",
            Box::new(Recipes),
        )])
        .unwrap();
        mod_vocab::__private::resolve_items(["test:result"]).unwrap();
        let (kind, serializer) =
            mod_vocab::__private::parts_installation::recipe_serializer("test:diagonal").unwrap();
        assert_eq!(kind, "minecraft:crafting");
        let loaded = serializer.decode(r#"{"count":3}"#).unwrap();
        let key = mod_vocab::__private::item_key("test:result").unwrap();
        let mut cells = vec![None; 9];
        for i in [0, 4, 8] {
            cells[i] = crate::ItemStack::new(key, 1);
        }
        let grid = crate::CraftingGrid::new(3, 3, cells).unwrap();
        assert!(loaded.matches(&Context, grid.clone()));
        let out = loaded.assemble(&Context, grid.clone()).unwrap();
        assert_eq!(out.count(), 3);
        assert_eq!(out.item(), key);
        assert_eq!(loaded.write().unwrap().as_deref(), Some(r#"{"count":3}"#));
        let mut expected = vec![None; 9];
        expected[4] = crate::ItemStack::new(key, 1);
        assert_eq!(
            loaded.remaining_items(&Context, grid.clone()),
            Some(expected)
        );
        let mut wrong = grid;
        wrong.cells[4] = None;
        assert!(!loaded.matches(&Context, wrong));
        assert!(serializer.decode("{").is_err());
        assert!(serializer.decode(r#"{"count":2}"#).is_err());
    }
    struct Invalid;
    #[mod_api::__subscriptions]
    impl crate::Hooks for Invalid {
        fn register(&self, r: &mut dyn crate::Registrar, _: &crate::RegistrationContext) {
            let key = r.item_key("test:result");
            r.register_crafting_recipe_serializer("diagonal", Literal { result: key });
            r.register_crafting_recipe_serializer("diagonal", Literal { result: key });
        }
    }
    #[test]
    fn duplicate_kinds_refuse_by_name() {
        let errors = crate::parts::tests::install_parts_fixture(&[crate::ModEntry::new(
            "test",
            Box::new(Invalid),
        )])
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.contains("test:diagonal") && e.contains("twice")),
            "{errors:?}"
        );
    }
}

impl PartRegistrar {
    pub(super) fn validate_recipe_types(&mut self) {
        for (serializer, (_, recipe_type, _)) in &self.parts.recipe_serializers {
            if recipe_type != "minecraft:crafting"
                && !self.parts.recipe_types.contains_key(recipe_type.as_str())
            {
                self.refused.push(format!(
                    "recipe serializer {serializer} names unregistered recipe type {recipe_type}"
                ));
            }
        }
    }
}

#[cfg(test)]
mod identity_tests {
    struct Duplicate;
    #[mod_api::__subscriptions]
    impl crate::Hooks for Duplicate {
        fn register(&self, r: &mut dyn crate::Registrar, _: &crate::RegistrationContext) {
            r.register_recipe_type("machine:grinding");
            r.register_recipe_type("machine:grinding");
            r.register_raw_recipe_serializer("machine:grinder_json", "machine:grinding");
            r.register_raw_recipe_serializer("machine:grinder_json", "machine:grinding");
        }
    }
    #[test]
    fn duplicate_serializer_and_type_ids_refuse_independently_by_name() {
        let errors = crate::__private::part_tests::install_parts_fixture(&[crate::ModEntry::new(
            "machine",
            Box::new(Duplicate),
        )])
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.contains("recipe type machine:grinding twice")),
            "{errors:?}"
        );
        assert!(
            errors
                .iter()
                .any(|e| e.contains("machine:grinder_json twice")),
            "{errors:?}"
        );
    }
}
