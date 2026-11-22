//! Bound, immutable item members for one published resource generation.
use crate::{ItemKey, TagKey};
use std::collections::{HashMap, HashSet};

// no wire: engine-owned lookup borrowed by an in-process loop context.
/// Item tags resolved against the owning generation and the completed API item table.
// hidden: the engine binds these members when publishing a resource generation.
// exhaustive: the backing map is private and can only be built by binding published item keys.
#[doc(hidden)]
// Keep tag names generation-owned: arbitrary server tags are not process-lifetime TagKeys.
pub struct BoundItemTags(HashMap<String, Option<Members>>);

struct Members {
    ordered: Box<[ItemKey]>,
    membership: HashSet<ItemKey>,
}

impl BoundItemTags {
    /// Bind an authoritative tag table at publication. `None` means the item keys
    /// have not been published; an empty input still yields an available table.
    /// If a member cannot bind, report and refuse that tag, not its neighbors.
    pub fn bind(tags: &HashMap<String, Vec<String>>, registry: &HashSet<&str>) -> Option<Self> {
        let items = crate::content::registry_keys::resolved_items().get()?;
        let tags = tags
            .iter()
            .map(|(tag, members)| {
                let ordered = members
                    .iter()
                    .map(|id| {
                        let name = id.as_str();
                        if !registry.contains(name) {
                            return Err(name);
                        }
                        items.slots.get(name).copied().map(ItemKey).ok_or(name)
                    })
                    .collect::<Result<Box<[_]>, _>>();
                let bound = match ordered {
                    Ok(ordered) => Some(Members {
                        membership: ordered.iter().copied().collect(),
                        ordered,
                    }),
                    Err(member) => {
                        log::warn!("Cannot bind loop item tag {tag}: unresolved member {member}");
                        None
                    }
                };
                (tag.clone(), bound)
            })
            .collect();
        Some(Self(tags))
    }

    pub(super) fn contains(&self, tag: TagKey, item: ItemKey) -> Option<bool> {
        Some(
            self.members(tag)?
                .is_some_and(|members| members.membership.contains(&item)),
        )
    }

    pub(super) fn items(&self, tag: TagKey) -> Option<&[ItemKey]> {
        Some(
            self.members(tag)?
                .map_or(&[], |members| members.ordered.as_ref()),
        )
    }

    // The outer option refuses an invalid key or an unbound tag; the inner one marks absence.
    fn members(&self, tag: TagKey) -> Option<Option<&Members>> {
        crate::world::tags::with_tag_name(tag, |name| match self.0.get(name?) {
            Some(bound) => bound.as_ref().map(Some),
            None => Some(None),
        })
    }
}
