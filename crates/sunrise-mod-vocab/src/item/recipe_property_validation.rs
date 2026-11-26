//! Scoped insertion and namespace validation for [`crate::RecipePropertySetsEvent`].
use super::RecipePropertyAddition;
use crate::Answered;

impl crate::RecipePropertySetsEvent<'_> {
    /// Add items to a recipe property set. A valid explicit key can create a custom set,
    /// including an unused `minecraft:` key. Bare names refer to built-in sets.
    /// Contributions are unioned without changing the first contributor's attribution.
    pub fn register(&mut self, addition: RecipePropertyAddition) {
        let mod_id = self.mod_id;
        if addition.set.contains(':')
            && crate::Identifier::parse_namespaced(&addition.set).is_none()
        {
            log::warn!(
                "recipe_property_sets: {mod_id}: invalid set key {}",
                addition.set
            );
            return;
        }
        self.additions.push(Answered {
            answer: addition,
            mod_id,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::phase::with_test_package_inventory;
    use crate::{PackageInfo, PackageKind, Side};

    #[test]
    fn any_valid_namespace_can_create_sets_and_keeps_contributor_attribution() {
        let packages = vec![PackageInfo::new(
            "ported_farmersdelight".into(),
            "1".into(),
            PackageKind::Code,
            Side::Both,
            vec!["ported_farmersdelight".into(), "farmersdelight".into()],
            vec![],
        )];
        let mut additions = Vec::new();
        with_test_package_inventory(packages, || {
            let mut event = crate::RecipePropertySetsEvent {
                mod_id: "another",
                additions: &mut additions,
            };
            event.register(RecipePropertyAddition::new("another:known", vec![]));
            event.mod_id = "ported_farmersdelight";
            for key in [
                "farmersdelight:cooking_pot_input",
                "ported_farmersdelight:input",
                "another:known",
                "another:new",
                "BAD:input",
                "minecraft:furnace_input",
            ] {
                event.register(RecipePropertyAddition::new(key, vec![]));
            }
        });
        assert_eq!(
            additions
                .iter()
                .map(|a| (a.answer.set.as_str(), a.mod_id))
                .collect::<Vec<_>>(),
            [
                ("another:known", "another"),
                ("farmersdelight:cooking_pot_input", "ported_farmersdelight"),
                ("ported_farmersdelight:input", "ported_farmersdelight"),
                ("another:known", "ported_farmersdelight"),
                ("another:new", "ported_farmersdelight"),
                ("minecraft:furnace_input", "ported_farmersdelight"),
            ]
        );
    }

    #[test]
    fn served_content_namespace_does_not_change_property_set_admission() {
        let packages = vec![
            PackageInfo::new(
                "horizon_java".into(),
                "1".into(),
                PackageKind::Code,
                Side::Both,
                vec!["horizon_java".into()],
                vec![],
            ),
            PackageInfo::new(
                "third".into(),
                "1".into(),
                PackageKind::Code,
                Side::Both,
                vec!["third".into()],
                vec![],
            ),
            PackageInfo::new(
                "runes".into(),
                "1".into(),
                PackageKind::Content,
                Side::Both,
                vec!["runes".into(), "runes_extra".into()],
                vec![],
            )
            .with_served_by(Some("horizon_java".into())),
            PackageInfo::new(
                "plain".into(),
                "1".into(),
                PackageKind::Content,
                Side::Both,
                vec!["plain".into()],
                vec![],
            ),
        ];
        let keys = [
            "runes:altar_input",
            "runes_extra:catalyst",
            "horizon_java:own",
            "plain:input",
            "third:input",
        ];
        with_test_package_inventory(packages, || {
            for mod_id in ["third", "horizon_java"] {
                let mut additions = Vec::new();
                let mut event = crate::RecipePropertySetsEvent {
                    mod_id,
                    additions: &mut additions,
                };
                for key in keys {
                    event.register(RecipePropertyAddition::new(key, vec![]));
                }
                assert_eq!(
                    additions
                        .iter()
                        .map(|a| a.answer.set.as_str())
                        .collect::<Vec<_>>(),
                    keys
                );
            }
        });
    }
}
