//! The server scoreboard's teams, and the client's view of them.

use crate::{TeamColor, Text, TextFormError};
use core::ops::ControlFlow;

/// `Team.Visibility`: who sees a team's name tags or death messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum TeamVisibility {
    /// Everyone (`always`).
    Always,
    /// Nobody (`never`).
    Never,
    /// Only the team's own members (`hideForOtherTeams`).
    HideForOtherTeams,
    /// Everyone but the team's own members (`hideForOwnTeam`).
    HideForOwnTeam,
}

impl TeamVisibility {
    /// Every visibility, in vanilla's order.
    pub const ALL: [Self; 4] = [
        Self::Always,
        Self::Never,
        Self::HideForOtherTeams,
        Self::HideForOwnTeam,
    ];
    /// The name `/team modify` writes (`hideForOtherTeams`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Never => "never",
            Self::HideForOtherTeams => "hideForOtherTeams",
            Self::HideForOwnTeam => "hideForOwnTeam",
        }
    }
    /// The visibility with that name, or None for one vanilla doesn't have.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.name() == name)
    }
}

/// `Team.CollisionRule`: whether a team's members push each other and other entities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum CollisionRule {
    /// Everyone pushes everyone (`always`).
    Always,
    /// Nobody pushes (`never`).
    Never,
    /// Members are pushed only by other teams (`pushOtherTeams`).
    PushOtherTeams,
    /// Members are pushed only by their own team (`pushOwnTeam`).
    PushOwnTeam,
}

impl CollisionRule {
    /// Every rule, in vanilla's order.
    pub const ALL: [Self; 4] = [
        Self::Always,
        Self::Never,
        Self::PushOtherTeams,
        Self::PushOwnTeam,
    ];
    /// The name `/team modify` writes (`pushOtherTeams`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Never => "never",
            Self::PushOtherTeams => "pushOtherTeams",
            Self::PushOwnTeam => "pushOwnTeam",
        }
    }
    /// The rule with that name, or None for one vanilla doesn't have.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.name() == name)
    }
}

/// Why a team call changed nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum TeamError {
    /// The handle is stale: its team was removed (whatever was added under its name since), or
    /// it is from an earlier open, or there is no server world.
    Stale,
    /// A team of that name exists (`/team add` refuses it).
    NameTaken,
    /// A holder that does not resolve: a player or entity that is not on this server. The team
    /// is fine. [`ScoreHolder::Name`] always resolves.
    UnknownHolder,
    /// A text was too deep to apply.
    Text(TextFormError),
}

impl From<TextFormError> for TeamError {
    fn from(error: TextFormError) -> Self {
        Self::Text(error)
    }
}

impl crate::NoBackend for TeamError {
    /// No game: as for a stale handle.
    fn no_backend() -> Self {
        Self::Stale
    }
}

/// Why a visit to a server team's members did not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: the refusal is returned by a scoped in-process visitor, not encoded.
pub enum TeamMembersReadError {
    /// The team's handle is from an earlier open, or the team was removed.
    Stale,
    /// The server scoreboard is not readable from this side or phase.
    Unavailable,
}

impl crate::NoBackend for TeamMembersReadError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// A received team for one visitor call. The server sends all teams, but not their
/// death message visibility.
// no wire: in-process scoped scoreboard view.
pub trait ClientTeam {
    /// The team's key.
    fn name(&self) -> &str;

    /// The team's color, or `None` if it has no color.
    fn color(&self) -> Option<TeamColor>;

    /// Whether members may hurt each other.
    fn friendly_fire(&self) -> bool;

    /// Whether members see their invisible teammates.
    fn see_friendly_invisibles(&self) -> bool;

    /// Who sees the members' name tags.
    fn name_tag_visibility(&self) -> TeamVisibility;

    /// Whether members push and are pushed.
    fn collision_rule(&self) -> CollisionRule;

    /// Visits the members' holder keys in board order. Return `ControlFlow::Break(())` to stop.
    fn visit_members(&self, visit: &mut dyn FnMut(&str) -> ControlFlow<()>) -> ControlFlow<()>;

    /// Copies the display name, or reports why this text cannot be represented.
    fn copy_display_name(&self) -> Result<Text, TextFormError>;

    /// Copies the prefix prepended to each member's name, or reports a conversion error.
    fn copy_prefix(&self) -> Result<Text, TextFormError>;

    /// Copies the suffix appended to each member's name, or reports a conversion error.
    fn copy_suffix(&self) -> Result<Text, TextFormError>;
}
