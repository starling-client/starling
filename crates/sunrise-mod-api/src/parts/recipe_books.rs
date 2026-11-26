//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

impl RecipeBooksRegistrar for PartRegistrar {
    fn recipe_book_type(&mut self, id: &str) -> Result<RecipeBookTypeKey, String> {
        self.register_book(id, None)
    }

    fn recipe_book_type_with_fields(
        &mut self,
        id: &str,
        open_field: &str,
        filtering_field: &str,
    ) -> Result<RecipeBookTypeKey, String> {
        self.register_book(id, Some((open_field, filtering_field)))
    }

    fn recipe_book_filter_look(
        &mut self,
        book: &RecipeBookTypeKey,
        look: crate::RecipeBookFilterLook,
    ) -> Result<(), String> {
        let result = (|| {
            if mod_vocab::__private::parts_installation::parts_installed() {
                return Err("registry is frozen".to_owned());
            }
            if self
                .recipe_book_types
                .get(book.as_str())
                .map(String::as_str)
                != Some(self.mod_id)
            {
                return Err("book must be registered by the declaring mod".to_owned());
            }
            if self.recipe_book_filters.contains_key(book.as_str()) {
                return Err("filter look already declared".to_owned());
            }
            for sprite in [
                &look.enabled,
                &look.disabled,
                &look.enabled_highlighted,
                &look.disabled_highlighted,
            ] {
                if crate::Identifier::parse_namespaced(sprite).is_none() {
                    return Err(format!(
                        "sprite {sprite:?}: expected a namespaced identifier"
                    ));
                }
            }
            if look.tooltip.is_empty() {
                return Err("tooltip key must be nonempty".into());
            }
            self.recipe_book_filters
                .insert(book.as_str().to_owned(), look);
            Ok(())
        })();
        result.map_err(|reason: String| {
            let message = format!(
                "mod {} recipe book {:?} filter look: {reason}",
                self.mod_id,
                book.as_str()
            );
            self.refused.push(message.clone());
            message
        })
    }
}

/// Vanilla 26.3 `RecipeBookType` enum values.
const VANILLA_RECIPE_BOOK_TYPES: &[&str] = &[
    "minecraft:crafting",
    "minecraft:furnace",
    "minecraft:blast_furnace",
    "minecraft:smoker",
];

impl PartRegistrar {
    fn register_book(
        &mut self,
        id: &str,
        fields: Option<(&str, &str)>,
    ) -> Result<RecipeBookTypeKey, String> {
        let result = (|| {
            if mod_vocab::__private::parts_installation::parts_installed() {
                return Err("registry is frozen".to_owned());
            }
            crate::Identifier::parse_namespaced(id).ok_or("expected a namespaced identifier")?;
            if VANILLA_RECIPE_BOOK_TYPES.contains(&id) {
                return Err(format!("built-in recipe book type {id}"));
            }
            if let Some(owner) = self.recipe_book_types.get(id) {
                return Err(format!(
                    "duplicate key declared by {owner} and {}",
                    self.mod_id
                ));
            }
            // CompoundTag.write names each field with DataOutput.writeUTF. Check only
            // the names this declaration will actually write, before recording it.
            let saved_names = fields.map_or(
                [Some(("extension book ID", id)), None],
                |(open, filtering)| {
                    [
                        Some(("open field", open)),
                        Some(("filtering field", filtering)),
                    ]
                },
            );
            for (role, name) in saved_names.into_iter().flatten() {
                if u16::try_from(sunrise_core::nbt::JavaString::from(name).modified_utf8_len())
                    .is_err()
                {
                    return Err(format!(
                        "{role} exceeds the NBT field name limit of 65,535 modified UTF-8 bytes"
                    ));
                }
            }
            if let Some((open, filtering)) = fields {
                use crate::item::recipe_book::saved_fields;
                if open == filtering {
                    return Err("open and filtering fields must differ".into());
                }
                for field in [open, filtering] {
                    if field.is_empty() {
                        return Err("save fields must be nonempty".into());
                    }
                    if saved_fields::VANILLA
                        .iter()
                        .flat_map(|&(a, b)| [a, b])
                        .chain([
                            saved_fields::RECIPES,
                            saved_fields::TO_BE_DISPLAYED,
                            saved_fields::EXTENSION,
                        ])
                        .any(|reserved| field == reserved)
                    {
                        return Err(format!("reserved save field {field:?}"));
                    }
                    if let Some((other, _)) = self
                        .recipe_book_fields
                        .iter()
                        .find(|(_, (a, b))| field == a || field == b)
                    {
                        return Err(format!(
                            "save field {field:?} already belongs to book {other:?}"
                        ));
                    }
                }
                self.recipe_book_fields
                    .insert(id.into(), (open.into(), filtering.into()));
            }
            self.recipe_book_types.insert(id.into(), self.mod_id.into());
            Ok(mod_vocab::__private::parts_installation::recipe_book_type(
                id.into(),
            ))
        })();
        result.map_err(|reason: String| {
            let message = format!("mod {} recipe book {id:?}: {reason}", self.mod_id);
            self.refused.push(message.clone());
            message
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_fields_are_validated_atomically_and_refuse_installation() {
        let mut r = PartRegistrar::new("demo");
        r.recipe_book_type_with_fields(
            "demo:pot",
            "isModCookingGuiOpen",
            "isModCookingFilteringCraftable",
        )
        .unwrap();
        r.recipe_book_type_with_fields("demo:unicode", "Ouvert é / !", "絞り込み")
            .unwrap();
        let reserved = [
            "isGuiOpen",
            "isFilteringCraftable",
            "isFurnaceGuiOpen",
            "isFurnaceFilteringCraftable",
            "isBlastingFurnaceGuiOpen",
            "isBlastingFurnaceFilteringCraftable",
            "isSmokerGuiOpen",
            "isSmokerFilteringCraftable",
            "recipes",
            "toBeDisplayed",
            "sunrise:book_settings",
        ];
        for field in reserved {
            assert!(
                r.recipe_book_type_with_fields("demo:bad", field, "unused")
                    .is_err()
            );
            assert!(
                r.recipe_book_type_with_fields("demo:bad", "unused", field)
                    .is_err()
            );
        }
        for (a, b) in [("", "valid"), ("valid", ""), ("equal", "equal")] {
            assert!(r.recipe_book_type_with_fields("demo:bad", a, b).is_err());
        }
        r.mod_id = "second";
        for field in ["isModCookingGuiOpen", "isModCookingFilteringCraftable"] {
            assert!(
                r.recipe_book_type_with_fields("second:bad", field, "unused")
                    .is_err()
            );
            assert!(
                r.recipe_book_type_with_fields("second:bad", "unused", field)
                    .is_err()
            );
        }
        assert!(!r.refused.is_empty(), "ignored errors refuse installation");
        assert_eq!(r.recipe_book_fields.len(), 2);
        assert_eq!(r.recipe_book_types.len(), 2);
        r.recipe_book_type_with_fields("second:ok", "unused", &"é".repeat(128))
            .unwrap();
    }

    #[test]
    fn only_persisted_names_must_fit_the_modified_utf8_field_width() {
        let mut registrar = PartRegistrar::new("demo");
        let longest_id = format!("demo:{}", "a".repeat(65_530));
        assert!(registrar.recipe_book_type(&longest_id).is_ok());
        let oversized_id = longest_id + "a";
        let error = registrar.recipe_book_type(&oversized_id).unwrap_err();
        assert!(error.contains("mod demo") && error.contains("extension book ID"));

        // A supplementary character takes two UTF-16 surrogates (six bytes), and NUL two bytes.
        let longest_field = "😀".repeat(10_922) + "\0a";
        let oversized_field = longest_field.clone() + "a";
        let error = registrar
            .recipe_book_type_with_fields(&oversized_id, "open", &oversized_field)
            .unwrap_err();
        assert!(error.contains("mod demo") && error.contains("filtering field"));
        // The oversized ID is not written as an NBT field when flat fields are declared.
        assert!(
            registrar
                .recipe_book_type_with_fields(&oversized_id, "open", &longest_field)
                .is_ok()
        );
        assert_eq!(registrar.recipe_book_types.len(), 2);
        assert_eq!(registrar.recipe_book_fields.len(), 1);
    }

    #[test]
    fn registration_rejects_invalid_and_duplicate_declarations() {
        let mut registrar = PartRegistrar::new("demo");
        let key = registrar
            .recipe_book_type("demo:kitchen/cooking_pot")
            .unwrap();
        assert_eq!(key.as_str(), "demo:kitchen/cooking_pot");
        for id in ["other:pot", "minecraft:pot"] {
            assert_eq!(registrar.recipe_book_type(id).unwrap().as_str(), id);
            assert_eq!(registrar.recipe_book_types[id], "demo");
        }
        assert!(
            registrar
                .recipe_book_type("demo:kitchen/cooking_pot")
                .unwrap_err()
                .contains("duplicate")
        );
        let malformed = ["pot", "demo:", ":pot", "demo:Pot", "..:pot", "demo:a:b"];
        for id in malformed {
            let error = registrar.recipe_book_type(id).unwrap_err();
            assert!(
                error.contains("expected a namespaced identifier"),
                "{error}"
            );
        }
        let built_in = [
            "minecraft:crafting",
            "minecraft:furnace",
            "minecraft:blast_furnace",
            "minecraft:smoker",
        ];
        for id in built_in {
            let error = registrar.recipe_book_type(id).unwrap_err();
            assert!(error.contains("built-in recipe book type"), "{error}");
        }
        assert_eq!(registrar.recipe_book_types.len(), 3);
        assert_eq!(
            registrar.refused.len(),
            1 + malformed.len() + built_in.len()
        );
    }

    #[test]
    fn cross_mod_duplicate_names_both_declarers() {
        let mut registrar = PartRegistrar::new("first");
        registrar.recipe_book_type("first:book").unwrap();
        registrar.mod_id = "second";
        let error = registrar.recipe_book_type("first:book").unwrap_err();
        assert!(error.contains("duplicate"));
        assert!(error.contains("first:book"));
        assert!(error.contains("first and second"));
        assert_eq!(registrar.recipe_book_types.len(), 1);
    }

    #[test]
    fn registration_keeps_more_than_1024_books_and_long_declarations() {
        let mut registrar = PartRegistrar::new("demo");
        for index in 0..1024 {
            registrar
                .recipe_book_type(&format!("demo:book_{index}"))
                .unwrap();
        }
        let id = format!("demo:{}", "cook/".repeat(80) + "pot");
        let open = "é".repeat(150);
        let filtering = "絞".repeat(150);
        let book = registrar
            .recipe_book_type_with_fields(&id, &open, &filtering)
            .unwrap();
        let appearance = crate::RecipeBookFilterLook {
            enabled: format!("demo:{}", "sprite/".repeat(50) + "enabled"),
            tooltip: "demo.".repeat(80) + "cookable",
            ..look()
        };
        registrar
            .recipe_book_filter_look(&book, appearance.clone())
            .unwrap();

        assert_eq!(book.as_str(), id.as_str());
        assert_eq!(registrar.recipe_book_types.len(), 1025);
        assert_eq!(registrar.recipe_book_types["demo:book_0"], "demo");
        assert_eq!(registrar.recipe_book_fields[&id], (open, filtering));
        assert_eq!(registrar.recipe_book_filters[&id], appearance);
        assert!(registrar.refused.is_empty());
    }

    fn look() -> crate::RecipeBookFilterLook {
        crate::RecipeBookFilterLook {
            enabled: "other:book/on".into(),
            disabled: "other:book/off".into(),
            enabled_highlighted: "other:book/on_hover".into(),
            disabled_highlighted: "other:book/off_hover".into(),
            tooltip: "demo.book.cookable".into(),
        }
    }

    #[test]
    fn a_filter_look_is_declared_once_by_the_books_own_mod() {
        let mut r = PartRegistrar::new("demo");
        let book = r.recipe_book_type("demo:pot").unwrap();
        r.recipe_book_filter_look(&book, look()).unwrap();
        assert_eq!(r.recipe_book_filters["demo:pot"], look());
        assert!(
            r.recipe_book_filter_look(&book, look())
                .unwrap_err()
                .contains("already declared")
        );
        let second = r.recipe_book_type("demo:other").unwrap();
        for bad in [
            crate::RecipeBookFilterLook {
                enabled: "no_namespace".into(),
                ..look()
            },
            crate::RecipeBookFilterLook {
                enabled: "..:pot".into(),
                ..look()
            },
            crate::RecipeBookFilterLook {
                disabled_highlighted: "demo:Upper".into(),
                ..look()
            },
            crate::RecipeBookFilterLook {
                tooltip: String::new(),
                ..look()
            },
        ] {
            assert!(r.recipe_book_filter_look(&second, bad).is_err());
        }
        assert!(!r.recipe_book_filters.contains_key("demo:other"));
        r.mod_id = "stranger";
        assert!(
            r.recipe_book_filter_look(&second, look())
                .unwrap_err()
                .contains("declaring mod")
        );
        assert_eq!(r.refused.len(), 6, "each refusal also refuses installation");
    }
}
