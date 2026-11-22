//! The server scoreboard's teams, and the client's view of them.

use crate::{TeamColor, Text, TextFormError};

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

/// A team as this client was sent it. Every team goes to every client, shown or not.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientTeam {
    /// The team's name.
    pub name: String,
    /// The display name.
    pub display_name: Text,
    /// The team's color, or None for none.
    pub color: Option<TeamColor>,
    /// What is put before a member's name.
    pub prefix: Text,
    /// What is put after a member's name.
    pub suffix: Text,
    /// Whether members may hurt each other.
    pub friendly_fire: bool,
    /// Whether members see their invisible teammates.
    pub see_friendly_invisibles: bool,
    /// Who sees the members' name tags.
    pub name_tag_visibility: TeamVisibility,
    /// Whether members push and are pushed.
    pub collision_rule: CollisionRule,
    /// The members' holder keys, in the order they arrived. The server does not send a team's
    /// death message visibility, so a client has none to read.
    pub members: Vec<String>,
}

impl ClientTeam {
    /// A team named `name` as vanilla's `PlayerTeam` starts: its name as the display name, no
    /// color, empty prefix and suffix, friendly fire and friendly invisibles on, name tags and
    /// collision for everyone, and no members. Set the public fields for the rest.
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            display_name: Text::literal(name.clone()),
            name,
            color: None,
            prefix: Text::literal(""),
            suffix: Text::literal(""),
            friendly_fire: true,
            see_friendly_invisibles: true,
            name_tag_visibility: TeamVisibility::Always,
            collision_rule: CollisionRule::Always,
            members: Vec::new(),
        }
    }
}
