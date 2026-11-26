//! Owned advancement progress captured at a criterion's grant or revoke, and a loaded
//! advancement's definition.

use crate::{Identifier, ItemStack, NoBackend, Text};

/// One criterion in an advancement, including criteria not yet obtained.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct CriterionProgress {
    /// The criterion's name within the advancement.
    pub name: String,
    /// The stored grant date, or `None` when this criterion is not obtained.
    /// Dates use the advancement save format (`yyyy-MM-dd HH:mm:ss Z`);
    /// capturing progress preserves the original date rather than reading a clock.
    pub obtained_at: Option<String>,
}

impl CriterionProgress {
    /// Build a criterion snapshot from its name and existing grant date.
    pub fn new(name: String, obtained_at: Option<String>) -> Self {
        Self { name, obtained_at }
    }
}

/// An owned, read-only snapshot of one advancement's complete progress.
///
/// [`crate::PlayerAdvancementProgressEvent::progress`] captures this after the
/// criterion changes and before completion rewards, and
/// [`crate::PlayerRef::advancement_progress`] reads a server player's live
/// progress on demand. Retaining or changing a clone does not change the
/// player's progress.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct AdvancementProgress {
    /// Every criterion in definition order, including unobtained criteria.
    pub criteria: Vec<CriterionProgress>,
    /// Requirement groups: at least one criterion in each group must be obtained.
    /// All groups must be satisfied; an empty list is not complete.
    pub requirements: Vec<Vec<String>>,
    /// Number of requirement groups satisfied, not the number of obtained criteria.
    pub completed_requirements: u32,
    /// Whether the owning progress considers this advancement complete.
    pub done: bool,
}

impl AdvancementProgress {
    /// Build a snapshot from the owning progress's criteria and requirement results.
    pub fn new(
        criteria: Vec<CriterionProgress>,
        requirements: Vec<Vec<String>>,
        completed_requirements: u32,
        done: bool,
    ) -> Self {
        Self {
            criteria,
            requirements,
            completed_requirements,
            done,
        }
    }
}

/// A loaded advancement's definition (vanilla's `AdvancementHolder`), read with
/// `ServerHost::advancement`. A snapshot: it does not follow a later `/reload`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct AdvancementInfo {
    /// The advancement's id.
    pub id: Identifier,
    /// The parent's id; `None` for a root.
    pub parent: Option<Identifier>,
    /// How the advancement screen and toasts show it; `None` for a hidden recipe advancement.
    pub display: Option<AdvancementDisplay>,
    /// The criterion names in definition order.
    pub criteria: Vec<String>,
    /// Requirement groups: at least one criterion in each group completes the advancement.
    pub requirements: Vec<Vec<String>>,
}

impl AdvancementInfo {
    /// A definition from its parts.
    pub fn new(
        id: Identifier,
        parent: Option<Identifier>,
        display: Option<AdvancementDisplay>,
        criteria: Vec<String>,
        requirements: Vec<Vec<String>>,
    ) -> Self {
        Self {
            id,
            parent,
            display,
            criteria,
            requirements,
        }
    }
}

/// An advancement's `DisplayInfo`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct AdvancementDisplay {
    /// The title.
    pub title: Text,
    /// The description.
    pub description: Text,
    /// The icon: the stack the file names, with its count and components.
    pub icon: ItemStack,
    /// The frame.
    pub frame: AdvancementFrame,
    /// Whether completing it shows a toast.
    pub show_toast: bool,
    /// Whether completing it is announced in chat.
    pub announce_to_chat: bool,
    /// Whether it stays hidden until done.
    pub hidden: bool,
}

impl AdvancementDisplay {
    /// A display from its parts.
    pub fn new(
        title: Text,
        description: Text,
        icon: ItemStack,
        frame: AdvancementFrame,
        show_toast: bool,
        announce_to_chat: bool,
        hidden: bool,
    ) -> Self {
        Self {
            title,
            description,
            icon,
            frame,
            show_toast,
            announce_to_chat,
            hidden,
        }
    }
}

/// An advancement's frame (`AdvancementType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum AdvancementFrame {
    /// `task`.
    Task,
    /// `goal`.
    Goal,
    /// `challenge`.
    Challenge,
}

/// Why awarding or revoking an advancement for a player could not complete.
// no wire: returned directly to the native caller on the server thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AdvancementError {
    /// This game is a remote client's copy, not the authoritative server.
    NotServer,
    /// The player handle is stale, offline, displaced by a nested turn, or their level could not be entered.
    PlayerUnavailable,
    /// No loaded advancement has this identifier.
    UnknownAdvancement,
    /// The advancement has no criterion with this name.
    UnknownCriterion,
    /// The host has no game backend (for example, a mod's test double).
    NoBackend,
}

impl std::fmt::Display for AdvancementError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotServer => "advancements can only be changed on the logical server",
            Self::PlayerUnavailable => "player is unavailable for advancement changes",
            Self::UnknownAdvancement => "unknown advancement identifier",
            Self::UnknownCriterion => "unknown advancement criterion",
            Self::NoBackend => "host has no game backend",
        })
    }
}

impl std::error::Error for AdvancementError {}

impl NoBackend for AdvancementError {
    fn no_backend() -> Self {
        Self::NoBackend
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EntityRef, Host, PlayerRef, ServerHost, WorldEpoch};

    #[test]
    fn award_and_revoke_without_a_game_refuse_with_no_backend() {
        struct TestHost;
        impl Host for TestHost {}
        mod_api::impl_host_areas!(TestHost);
        impl ServerHost for TestHost {}
        mod_api::impl_server_host_areas!(TestHost);

        let player = PlayerRef {
            entity: EntityRef::new(WorldEpoch::new(1), 1),
            uuid: 1,
        };
        let id = Identifier::parse("test:adv").unwrap();
        assert_eq!(
            player.award_advancement(&mut TestHost, &id, None),
            Err(AdvancementError::NoBackend)
        );
        assert_eq!(
            player.revoke_advancement(&mut TestHost, &id, Some("step")),
            Err(AdvancementError::NoBackend)
        );
    }
}
