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
                resource_id(sprite).map_err(|reason| format!("sprite {sprite:?}: {reason}"))?;
            }
            if look.tooltip.is_empty() || look.tooltip.len() > 256 {
                return Err("tooltip key must be nonempty and at most 256 bytes".into());
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

/// Split a lowercase `namespace:path` id of at most 256 bytes.
fn resource_id(id: &str) -> Result<(&str, &str), String> {
    let (namespace, path) = id.split_once(':').ok_or("explicit namespace required")?;
    if id.len() > 256 {
        return Err("id exceeds 256 bytes".into());
    }
    if namespace.is_empty()
        || path.is_empty()
        || !namespace
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b))
        || !path
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-/".contains(&b))
    {
        return Err("nonempty lowercase resource-id namespace and path required".into());
    }
    Ok((namespace, path))
}

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
            let (namespace, _) = resource_id(id)?;
            if let Some(owner) = self.recipe_book_types.get(id) {
                return Err(format!(
                    "duplicate key declared by {owner} and {}",
                    self.mod_id
                ));
            }
            if namespace == "minecraft" || !owns_namespace(self.mod_id, namespace) {
                return Err("namespace must be owned by the registering mod, not minecraft".into());
            }
            if self.recipe_book_types.len() >= 1024 {
                return Err("registry exceeds 1024 custom books".into());
            }
            if let Some((open, filtering)) = fields {
                use crate::item::recipe_book::saved_fields;
                if open == filtering {
                    return Err("open and filtering fields must differ".into());
                }
                for field in [open, filtering] {
                    if field.is_empty() || field.len() > 256 {
                        return Err("save fields must be nonempty and at most 256 bytes".into());
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
        assert!(
            r.recipe_book_type_with_fields("demo:bad", &"é".repeat(129), "unused")
                .is_err()
        );
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
    fn registration_rejects_invalid_and_duplicate_declarations() {
        let mut registrar = PartRegistrar::new("demo");
        let key = registrar.recipe_book_type("demo:cooking_pot").unwrap();
        assert_eq!(key.as_str(), "demo:cooking_pot");
        assert!(
            registrar
                .recipe_book_type("demo:cooking_pot")
                .unwrap_err()
                .contains("duplicate")
        );
        for id in [
            "pot",
            "demo:",
            ":pot",
            "demo:Pot",
            "minecraft:pot",
            "other:pot",
            "demo:a:b",
        ] {
            assert!(registrar.recipe_book_type(id).is_err(), "{id}");
        }
        assert!(
            registrar
                .recipe_book_type(&format!("demo:{}", "a".repeat(252)))
                .is_err()
        );
        assert_eq!(registrar.recipe_book_types.len(), 1);
        assert_eq!(registrar.refused.len(), 9);
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
    fn registry_capacity_does_not_evict_existing_books() {
        let mut registrar = PartRegistrar::new("demo");
        for index in 0..1024 {
            registrar
                .recipe_book_type(&format!("demo:book_{index}"))
                .unwrap();
        }
        assert!(
            registrar
                .recipe_book_type("demo:overflow")
                .unwrap_err()
                .contains("1024")
        );
        assert_eq!(registrar.recipe_book_types.len(), 1024);
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
                disabled_highlighted: "demo:Upper".into(),
                ..look()
            },
            crate::RecipeBookFilterLook {
                tooltip: String::new(),
                ..look()
            },
            crate::RecipeBookFilterLook {
                tooltip: "k".repeat(257),
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
