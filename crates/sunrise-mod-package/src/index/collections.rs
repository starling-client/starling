//! Collections are curated lists of projects, not versioned packages.
use super::{Kind, page, valid_id};
use crate::manifest::RESERVED_IDS;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Bounds the selection presented by a collection card.
pub const MAX_COLLECTION_MEMBERS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub owners: Vec<String>,
    /// The registry's published HTTPS icon address.
    pub icon: String,
    pub members: Vec<Member>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub id: String,
    /// Initially selected on the collection page.
    #[serde(default = "by_default", skip_serializing_if = "is_default")]
    pub default: bool,
}

pub(super) fn by_default() -> bool {
    true
}

fn is_default(default: &bool) -> bool {
    *default
}

impl Collection {
    /// `taken` says whether the collection's id is a project's, whatever that project publishes;
    /// `project` gives the kind of a project with versions, which is what a member must be.
    pub(super) fn check<'a>(
        &self,
        taken: bool,
        project: impl Fn(&str) -> Option<&'a Kind>,
    ) -> Vec<String> {
        let mut errors = page::check_title(&self.name, &self.summary);
        errors.extend(page::check_description_limit(&self.description));
        if self.owners.is_empty() {
            errors.push("needs at least one owner (a GitHub login)".into());
        }
        if !valid_id(&self.id) || RESERVED_IDS.contains(&self.id.as_str()) {
            errors.push("pick a non-reserved id matching [a-z0-9_]{1,64}".into());
        }
        if taken {
            errors.push("its id is already a project id: pick another collection id".into());
        }
        if let Err(error) = super::https(&self.icon) {
            errors.push(format!(
                "its icon: {error}; use icons/{}.png in the registry",
                self.id
            ));
        }
        if !(2..=MAX_COLLECTION_MEMBERS).contains(&self.members.len()) {
            errors.push(format!("choose 2 to {MAX_COLLECTION_MEMBERS} members"));
        }
        if !self.members.iter().any(|member| member.default) {
            errors.push("choose at least one member installed by default".into());
        }
        let mut seen = HashSet::new();
        for member in &self.members {
            if !seen.insert(&member.id) {
                errors.push(format!(
                    "member {} is listed twice: keep one row",
                    member.id
                ));
            } else if !project(&member.id).is_some_and(Kind::is_mod) {
                errors.push(format!(
                    "member {} is not a listed mod or native mod: list the mod first or remove this member",
                    member.id
                ));
            }
        }
        errors
            .into_iter()
            .map(|e| format!("collection {}: {e}", self.id))
            .collect()
    }
}
