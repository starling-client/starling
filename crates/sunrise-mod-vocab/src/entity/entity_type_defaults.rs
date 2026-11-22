//! Entity types' default attributes (`DefaultAttributes`), as the game publishes them once its
//! attribute catalog is built: every host reads the same table, a world open or not, so the read
//! never needs a game behind the host (`ItemsHost::entity_type_default_attribute`).
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

/// Each entity type's attribute supplier: its attributes' values, by namespaced name, as the
/// supplier gives them (`AttributeSupplier.getValue`: the base inside the attribute's range).
pub(crate) type Suppliers = HashMap<String, HashMap<String, f64>>;

/// The game's entity types' attribute suppliers (`DefaultAttributes.SUPPLIERS`, mods' declared
/// rows and additions included), replacing the last: published whenever the attribute catalog is
/// built. Only a registered entity type's row is ever answered.
pub fn publish_entity_type_defaults(
    rows: impl IntoIterator<Item = (String, HashMap<String, f64>)>,
) {
    let Some(installed) = crate::parts::installed() else {
        return;
    };
    let rows = Arc::new(rows.into_iter().collect::<Suppliers>());
    if let Ok(mut published) = installed.entity_type_defaults.write() {
        *published = Some(rows);
    }
}

/// `DefaultAttributes.getSupplier(type).getValue(attribute)`: what every host answers for
/// `ItemsHost::entity_type_default_attribute` and the typed reload/setup contexts. None before
/// a catalog is published, or where the type or attribute has no value; never unwinds.
pub fn entity_type_default_attribute(entity_type: &str, attribute: &str) -> Option<f64> {
    let installed = crate::parts::installed()?;
    let suppliers = installed.entity_type_defaults.read().ok()?.clone()?;
    let keys = installed.resolved_entity_types.get()?;
    lookup(&suppliers, entity_type, attribute, |id| {
        keys.names().slots.contains_key(id)
    })
}

/// The supplier value of `attribute` for `entity_type` in `suppliers`, both names read as
/// `Identifier.parse` reads them. None for a type `registered` does not know (a mod's row for a
/// type no mod registered), a type with no supplier, or an attribute it lacks.
fn lookup(
    suppliers: &Suppliers,
    entity_type: &str,
    attribute: &str,
    registered: impl Fn(&str) -> bool,
) -> Option<f64> {
    let entity_type = identifier(entity_type);
    if !registered(&entity_type) {
        return None;
    }
    suppliers
        .get(&*entity_type)?
        .get(&*identifier(attribute))
        .copied()
}

/// `Identifier.parse`'s namespace rule: a name with no namespace, or an empty one (`:zombie`), is
/// `minecraft:`'s; a qualified name is borrowed as it is.
fn identifier(name: &str) -> Cow<'_, str> {
    match name.split_once(':') {
        Some(("", path)) => Cow::Owned(format!("minecraft:{path}")),
        Some(_) => Cow::Borrowed(name),
        None => Cow::Owned(format!("minecraft:{name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suppliers() -> Suppliers {
        let row = |pairs: &[(&str, f64)]| {
            pairs
                .iter()
                .map(|(name, base)| (name.to_string(), *base))
                .collect()
        };
        HashMap::from([
            (
                "minecraft:zombie".into(),
                row(&[
                    ("minecraft:max_health", 20.0),
                    ("minecraft:armor", 2.0),
                    ("minecraft:attack_damage", 3.0),
                ]),
            ),
            ("ruby:ghoul".into(), row(&[("minecraft:max_health", 40.0)])),
            ("ruby:orphan".into(), row(&[("minecraft:max_health", 7.0)])),
        ])
    }

    fn registered(id: &str) -> bool {
        ["minecraft:zombie", "minecraft:item", "ruby:ghoul"].contains(&id)
    }

    /// #3944: names read as `Identifier.parse` reads them (a bare or empty namespace is
    /// `minecraft:`'s); a type with no supplier (`minecraft:item`), an attribute it lacks, an
    /// unknown attribute and a row for a type no one registered all have none.
    #[test]
    fn a_registered_types_supplier_answers() {
        let suppliers = suppliers();
        let read = |t, a| lookup(&suppliers, t, a, registered);
        assert_eq!(read("minecraft:zombie", "minecraft:max_health"), Some(20.0));
        assert_eq!(read("zombie", "armor"), Some(2.0));
        assert_eq!(read(":zombie", ":attack_damage"), Some(3.0));
        assert_eq!(read("ruby:ghoul", "max_health"), Some(40.0));
        assert_eq!(read("minecraft:zombie", "minecraft:luck"), None);
        assert_eq!(read("minecraft:zombie", "minecraft:no_such"), None);
        assert_eq!(read("minecraft:item", "minecraft:max_health"), None);
        assert_eq!(read("ruby:orphan", "minecraft:max_health"), None);
        assert_eq!(read("minecraft:no_such", "minecraft:max_health"), None);
        assert!(matches!(identifier("minecraft:zombie"), Cow::Borrowed(_)));
    }
}
