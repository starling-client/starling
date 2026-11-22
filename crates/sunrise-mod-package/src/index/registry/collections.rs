//! Collection entries and their published pages.
use super::{
    Checkout, MAX_ICON, ReviewChange, ReviewStatus, Sources, has_versions, image, owned_by,
    owner_ids_error, read_regular,
};
use crate::index::{Collection, Member, collections::by_default, page, valid_id};
use serde::{Deserialize, Deserializer, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub const COLLECTIONS: &str = "collections";

/// `collections/<id>.toml`. Optional `owner_ids` are checkout metadata, not published.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollectionEntry {
    pub id: String,
    pub name: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub owners: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub owner_ids: Vec<u64>,
    pub icon: String,
    #[serde(deserialize_with = "checkout_members")]
    pub members: Vec<Member>,
}

fn checkout_members<'de, D: Deserializer<'de>>(input: D) -> Result<Vec<Member>, D::Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct CheckoutMember {
        id: String,
        #[serde(default = "by_default")]
        default: bool,
    }
    Ok(Vec::<CheckoutMember>::deserialize(input)?
        .into_iter()
        .map(|CheckoutMember { id, default }| Member { id, default })
        .collect())
}

impl Checkout {
    /// Validate collection metadata even when `registry review` has no package files yet.
    pub fn check_collections(&self, src: &Sources<'_>) -> Vec<String> {
        let (collections, mut errors) = self.collection_pages(src);
        let listed: BTreeMap<_, _> = self
            .listed()
            .filter(|(entry, cache)| has_versions(entry, *cache))
            .map(|(entry, _)| (entry.id.as_str(), &entry.kind))
            .collect();
        for collection in collections {
            errors.extend(collection.check(false, |id| listed.get(id).copied()));
        }
        errors
    }

    pub(super) fn collection_pages(&self, src: &Sources<'_>) -> (Vec<Collection>, Vec<String>) {
        let mut collections = Vec::new();
        let mut errors = Vec::new();
        for entry in &self.collections {
            let id = &entry.id;
            let start = errors.len();
            if !valid_id(id) || crate::manifest::RESERVED_IDS.contains(&id.as_str()) {
                errors.push(format!(
                    "collection {id}: pick a non-reserved id matching [a-z0-9_]{{1,64}}"
                ));
                continue;
            }
            if self.entries.iter().any(|e| e.id == *id) {
                errors.push(format!(
                    "collection {id}: its id is already a project id: pick another collection id"
                ));
            }
            if let Some(error) = owner_ids_error(&entry.owners, &entry.owner_ids) {
                errors.push(format!("collection {id}: {error}"));
            }
            let icon = if let Some(error) = page::check_icon_path(id, Some(&entry.icon)) {
                errors.push(format!("collection {id}: {error}"));
                None
            } else {
                let mut invalid = Vec::new();
                let icon = image(src, id, &entry.icon, "icons/", &mut invalid);
                errors.extend(
                    invalid
                        .into_iter()
                        .map(|error| format!("collection {id}: {error}")),
                );
                icon
            };
            if errors.len() == start
                && let Some(icon) = icon
            {
                collections.push(Collection {
                    id: entry.id.clone(),
                    name: entry.name.clone(),
                    summary: entry.summary.clone(),
                    description: entry.description.clone(),
                    owners: entry.owners.clone(),
                    icon,
                    members: entry.members.clone(),
                });
            }
        }
        (collections, errors)
    }
}

/// Classify what changed about collections. Use the base's owners, not any ownership a
/// submission may add alongside the change. A routine result is not an approval or a check.
pub fn review_collections(base: &Checkout, head: &Checkout, author: &str) -> Vec<ReviewChange> {
    let before: BTreeMap<_, _> = base
        .collections
        .iter()
        .map(|c| (c.id.as_str(), c))
        .collect();
    let after: BTreeMap<_, _> = head
        .collections
        .iter()
        .map(|c| (c.id.as_str(), c))
        .collect();
    let ids: BTreeSet<_> = before.keys().chain(after.keys()).copied().collect();
    let owners: BTreeMap<_, _> = base.entries.iter().map(|p| (p.id.as_str(), p)).collect();
    let mut changes = Vec::new();
    for id in ids {
        let (status, reason) = match (before.get(id), after.get(id)) {
            (None, Some(_)) => (
                ReviewStatus::ReviewRequired,
                "new collection: review its page and members".into(),
            ),
            (Some(_), None) => (
                ReviewStatus::ReviewRequired,
                "removed collection: review the removal".into(),
            ),
            (Some(old), Some(new)) => {
                let icon_changed = old.icon != new.icon
                    || (valid_id(id)
                        && page::check_icon_path(id, Some(&new.icon)).is_none()
                        && read_regular(&base.directory, Path::new(&old.icon), MAX_ICON).ok()
                            != read_regular(&head.directory, Path::new(&new.icon), MAX_ICON).ok());
                if old == new && !icon_changed {
                    continue;
                }
                use ReviewStatus::{ReviewRequired, Routine};
                if !owned_by(&old.owners, author) {
                    (
                        ReviewRequired,
                        format!("{author} is not an owner of this collection: ask a reviewer"),
                    )
                } else if old.owners != new.owners || old.owner_ids != new.owner_ids {
                    (
                        ReviewRequired,
                        "collection owners changed: review the new owners".into(),
                    )
                } else {
                    let old_ids: BTreeSet<_> = old.members.iter().map(|m| m.id.as_str()).collect();
                    let new_ids: BTreeSet<_> = new.members.iter().map(|m| m.id.as_str()).collect();
                    // Both iterators preserve the order and default flags of unchanged rows.
                    let old_kept = old
                        .members
                        .iter()
                        .filter(|m| new_ids.contains(m.id.as_str()));
                    let new_kept = new
                        .members
                        .iter()
                        .filter(|m| old_ids.contains(m.id.as_str()));
                    if old.name != new.name
                        || old.summary != new.summary
                        || old.description != new.description
                        || icon_changed
                        || !old_kept.eq(new_kept)
                    {
                        (ReviewRequired, "collection page fields, existing member order or defaults changed: review the page".into())
                    } else if let Some(member) = old_ids
                        .symmetric_difference(&new_ids)
                        .find(|id| !owners.get(*id).is_some_and(|p| owned_by(&p.owners, author)))
                    {
                        (
                            ReviewRequired,
                            format!(
                                "member {member} is not an existing project owned by this author: ask a reviewer"
                            ),
                        )
                    } else {
                        (
                            Routine,
                            "the owner changed only membership of their own projects".into(),
                        )
                    }
                }
            }
            _ => continue,
        };
        changes.push(ReviewChange {
            id: id.into(),
            status,
            reason,
        });
    }
    changes
}

#[cfg(test)]
mod tests;
