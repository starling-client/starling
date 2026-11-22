//! Validation at the listener boundary, before the next mod sees its edits.
use super::RecipePropertyAddition;

pub(crate) fn validate_recipe_properties(
    owner: &str,
    previous: &[RecipePropertyAddition],
    additions: &mut Vec<RecipePropertyAddition>,
) {
    validate_in(owner, &crate::LoadedMods::new(), previous, additions);
}

fn validate_in(
    owner: &str,
    packages: &crate::LoadedMods,
    previous: &[RecipePropertyAddition],
    additions: &mut Vec<RecipePropertyAddition>,
) {
    additions.retain(|addition| {
        let Some((namespace, _)) = addition.set.split_once(':') else {
            // Bare keys name vanilla sets; the engine checks that they exist.
            return true;
        };
        let existing = previous.iter().any(|old| old.set == addition.set);
        let owned = crate::parts::owns_namespace_in(packages, owner, namespace);
        let valid = crate::Identifier::parse_namespaced(&addition.set).is_some()
            && (namespace == "minecraft" || existing || owned);
        if !valid {
            log::warn!(
                "recipe_property_sets: {owner} cannot create set {} outside its owned namespaces",
                addition.set
            );
        }
        valid
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PackageInfo, PackageKind, Side};
    use std::sync::Arc;

    #[test]
    fn owned_manifest_namespace_can_create_but_foreign_new_key_cannot() {
        let packages = crate::LoadedMods::from_entries(Arc::from([PackageInfo::new(
            "ported_farmersdelight".into(),
            "1".into(),
            PackageKind::Code,
            Side::Both,
            vec!["ported_farmersdelight".into(), "farmersdelight".into()],
            vec![],
        )]));
        let previous = vec![RecipePropertyAddition::new("another:known", vec![])];
        let mut additions: Vec<_> = [
            "farmersdelight:cooking_pot_input",
            "ported_farmersdelight:input",
            "another:known",
            "another:new",
            "BAD:input",
            "minecraft:furnace_input",
        ]
        .into_iter()
        .map(|key| RecipePropertyAddition::new(key, vec![]))
        .collect();
        validate_in(
            "ported_farmersdelight",
            &packages,
            &previous,
            &mut additions,
        );
        assert_eq!(
            additions.iter().map(|a| a.set.as_str()).collect::<Vec<_>>(),
            [
                "farmersdelight:cooking_pot_input",
                "ported_farmersdelight:input",
                "another:known",
                "minecraft:furnace_input"
            ]
        );
    }

    #[test]
    fn served_content_namespace_grants_recipe_property_creation_only_to_its_host() {
        let packages = crate::LoadedMods::from_entries(Arc::from([
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
        ]));
        let keys = [
            "runes:altar_input",
            "runes_extra:catalyst",
            "horizon_java:own",
            "plain:input",
            "third:input",
        ];
        let mut host_additions: Vec<_> = keys
            .iter()
            .map(|key| RecipePropertyAddition::new(*key, vec![]))
            .collect();
        validate_in("horizon_java", &packages, &[], &mut host_additions);
        assert_eq!(
            host_additions
                .iter()
                .map(|a| a.set.as_str())
                .collect::<Vec<_>>(),
            [
                "runes:altar_input",
                "runes_extra:catalyst",
                "horizon_java:own"
            ]
        );

        let mut third_additions: Vec<_> = keys
            .iter()
            .map(|key| RecipePropertyAddition::new(*key, vec![]))
            .collect();
        validate_in("third", &packages, &[], &mut third_additions);
        assert_eq!(
            third_additions
                .iter()
                .map(|a| a.set.as_str())
                .collect::<Vec<_>>(),
            ["third:input"]
        );
    }
}
