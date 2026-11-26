//! Engine-owned tag reads passed only while evaluating custom loot kinds.
use crate::tag_layers::TagLayers;
use std::collections::{HashMap, HashSet};
use std::{io, sync::Arc};
use sunrise_core::identifier::{Identifier, IdentifierError};

pub(super) type BoundTags = HashMap<String, HashMap<String, Vec<String>>>;

/// Typed names for one generation of bound item tags. The original spelling
/// lets enumeration reuse that generation's membership lookup without formatting.
#[derive(Clone)]
pub struct ItemTagNames(Arc<[(String, Result<Identifier, IdentifierError>)]>);

impl ItemTagNames {
    pub fn bind(tags: &super::RegistryTags) -> Self {
        Self(
            tags.keys()
                .map(|name| (name.clone(), Identifier::parse(name)))
                .collect(),
        )
    }

    /// Clone the matching names, or refuse the query if a matching name is invalid.
    pub fn matching(&self, mut matches: impl FnMut(&str) -> bool) -> Option<Vec<Identifier>> {
        self.0
            .iter()
            .filter(|(name, _)| matches(name))
            .map(|(_, id)| id.as_ref().ok().cloned())
            .collect()
    }
}

/// Authoritative tag reads without a game or registry borrow.
/// Registry names are vanilla directory names (`item`, `block`, `fluid`).
pub trait LootTags {
    /// Members of a loaded tag in its resolved order, or `None` if absent.
    fn members(&self, registry: &str, tag: &str) -> Option<Vec<String>>;
    /// The bound item tags, including empty tags, or `None` before the registry binds.
    fn item_tag_names(&self) -> Option<&ItemTagNames>;
    /// Whether one registry value belongs to a tag.
    fn contains(&self, registry: &str, tag: &str, id: &str) -> bool {
        self.members(registry, tag)
            .is_some_and(|members| members.iter().any(|member| member == id))
    }
    /// `DamageSource.is(TagKey)`: whether `damage_type` is in the bound damage type tag `tag`.
    /// An unbound tag is false; a damage type the bound registry lacks, or no bound damage
    /// type registry, answer None.
    fn damage_type_in_tag(&self, _damage_type: &str, _tag: &str) -> Option<bool> {
        None
    }
}

impl super::LootData {
    /// One registry's bound tags in their resolved order (`TagLoader.build`), if it was bound.
    pub fn registry_tags(&self, registry: &str) -> Option<&super::RegistryTags> {
        self.tags.get(registry)
    }
}

impl LootTags for super::LootData {
    fn members(&self, registry: &str, tag: &str) -> Option<Vec<String>> {
        self.tags.get(registry)?.get(tag).cloned()
    }

    fn item_tag_names(&self) -> Option<&ItemTagNames> {
        self.item_tag_names.as_ref()
    }

    fn contains(&self, registry: &str, tag: &str, id: &str) -> bool {
        self.tags
            .get(registry)
            .and_then(|tags| tags.get(tag))
            .is_some_and(|members| members.iter().any(|member| member == id))
    }

    fn damage_type_in_tag(&self, damage_type: &str, tag: &str) -> Option<bool> {
        self.damage_types
            .binary_search_by(|id| id.as_str().cmp(damage_type))
            .ok()?;
        Some(self.contains("damage_type", tag, damage_type))
    }
}

/// Unlike registry tags, cycles in a loot predicate or modifier tag are invalid.
pub(super) fn check_cycles(registry: &str, tags: &HashMap<String, TagLayers>) -> io::Result<()> {
    let mut active = HashSet::new();
    let mut done = HashSet::new();
    let mut pending = Vec::new();
    for name in tags.keys().map(String::as_str) {
        pending.push((name, false));
        while let Some((name, ready)) = pending.pop() {
            if ready {
                active.remove(name);
                done.insert(name);
            } else if !done.contains(name)
                && let Some(layers) = tags.get(name)
            {
                if !active.insert(name) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("tags/{registry} {name}: reference cycle"),
                    ));
                }
                pending.push((name, true));
                pending.extend(layers.tags().map(|tag| (tag, false)));
            }
        }
    }
    Ok(())
}
