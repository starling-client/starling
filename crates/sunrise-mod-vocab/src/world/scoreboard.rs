//! The server scoreboard's objectives, display slots and scores, and the client's view of them.

use crate::text::TextStyle;
use crate::{ClientTeam, EntityRef, PlayerRef, Text, TextFormError};

/// Who a score belongs to (vanilla's `ScoreHolder`). The board keys scores by one string, and
/// every variant resolves to it: a player's profile name, another entity's UUID text, or the
/// name as given.
///
/// [`Player`](Self::Player) and [`Entity`](Self::Entity) of the same player's entity are the
/// same holder, keyed by the player's name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
// exhaustive: the board keys a score by a player, an entity or a name, and nothing else
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ScoreHolder {
    /// An online player.
    Player(PlayerRef),
    /// An entity: a player's entity is that player, any other entity is keyed by its UUID.
    Entity(EntityRef),
    /// Any string, as vanilla's `ScoreHolder.forNameOnly`: offline players, fake holders such
    /// as `#global`, display-source keys. The board accepts every string, including an empty
    /// one and `*`; a name starting with `#` is stored and synced but hidden from the sidebar.
    Name(String),
}

/// `ObjectiveCriteria.RenderType`: how clients draw the list and below-name slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ObjectiveRenderType {
    /// The number.
    Integer,
    /// Hearts, two points to a heart.
    Hearts,
}

/// The 16 team colors (`ChatFormatting`'s colors), as team display slots name them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum TeamColor {
    /// `black`.
    Black,
    /// `dark_blue`.
    DarkBlue,
    /// `dark_green`.
    DarkGreen,
    /// `dark_aqua`.
    DarkAqua,
    /// `dark_red`.
    DarkRed,
    /// `dark_purple`.
    DarkPurple,
    /// `gold`.
    Gold,
    /// `gray`.
    Gray,
    /// `dark_gray`.
    DarkGray,
    /// `blue`.
    Blue,
    /// `green`.
    Green,
    /// `aqua`.
    Aqua,
    /// `red`.
    Red,
    /// `light_purple`.
    LightPurple,
    /// `yellow`.
    Yellow,
    /// `white`.
    White,
}

impl TeamColor {
    /// Every color, in `ChatFormatting`'s order.
    pub const ALL: [Self; 16] = [
        Self::Black,
        Self::DarkBlue,
        Self::DarkGreen,
        Self::DarkAqua,
        Self::DarkRed,
        Self::DarkPurple,
        Self::Gold,
        Self::Gray,
        Self::DarkGray,
        Self::Blue,
        Self::Green,
        Self::Aqua,
        Self::Red,
        Self::LightPurple,
        Self::Yellow,
        Self::White,
    ];
    /// The color's name as vanilla writes it (`dark_blue`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Black => "black",
            Self::DarkBlue => "dark_blue",
            Self::DarkGreen => "dark_green",
            Self::DarkAqua => "dark_aqua",
            Self::DarkRed => "dark_red",
            Self::DarkPurple => "dark_purple",
            Self::Gold => "gold",
            Self::Gray => "gray",
            Self::DarkGray => "dark_gray",
            Self::Blue => "blue",
            Self::Green => "green",
            Self::Aqua => "aqua",
            Self::Red => "red",
            Self::LightPurple => "light_purple",
            Self::Yellow => "yellow",
            Self::White => "white",
        }
    }
    /// The color with that name, or None for one vanilla doesn't have.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|color| color.name() == name)
    }
}

/// `DisplaySlot`: where an objective is shown. A slot holds at most one objective, and one
/// objective may be in several slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DisplaySlot {
    /// The player list (`list`).
    List,
    /// The sidebar (`sidebar`).
    Sidebar,
    /// Under players' names (`below_name`).
    BelowName,
    /// The sidebar for members of a team of this color (`sidebar.team.<color>`).
    TeamSidebar(TeamColor),
}

impl DisplaySlot {
    /// The slot's name as vanilla writes it (`sidebar.team.red`).
    pub fn name(self) -> String {
        match self {
            Self::List => "list".to_owned(),
            Self::Sidebar => "sidebar".to_owned(),
            Self::BelowName => "below_name".to_owned(),
            Self::TeamSidebar(color) => format!("sidebar.team.{}", color.name()),
        }
    }
    /// The slot with that name, or None for one vanilla doesn't have.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "list" => Some(Self::List),
            "sidebar" => Some(Self::Sidebar),
            "below_name" => Some(Self::BelowName),
            _ => TeamColor::from_name(name.strip_prefix("sidebar.team.")?).map(Self::TeamSidebar),
        }
    }
    /// Every slot, in id order: the three, then one sidebar per team color.
    pub fn all() -> impl Iterator<Item = Self> {
        [Self::List, Self::Sidebar, Self::BelowName]
            .into_iter()
            .chain(TeamColor::ALL.into_iter().map(Self::TeamSidebar))
    }
}

/// `NumberFormat`: how a score's number is shown. An objective has one, and a score may override
/// it; with neither, the slot's default shows (red in the sidebar, yellow in the player list).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum NumberFormat {
    /// No number (`BlankFormat`).
    Blank,
    /// This text in place of the number (`FixedFormat`).
    Fixed(Text),
    /// The number, drawn in this style (`StyledFormat`). Only the color, shadow color,
    /// decorations and font of the style apply; a click, hover or insertion is dropped.
    Styled(TextStyle),
}

/// Why an objective call changed nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ObjectiveError {
    /// The handle is stale: its objective was removed (whatever was added under its name since),
    /// or it is from an earlier open, or there is no server world.
    Stale,
    /// A score write named a holder that does not resolve: a player or entity that is not on this
    /// server (an offline player's entity, a stale or earlier-open handle). The objective is fine.
    /// [`ScoreHolder::Name`] always resolves. Only
    /// [`set_score`](crate::ObjectiveRef::set_score) and [`add_score`](crate::ObjectiveRef::add_score)
    /// answer it; the other score calls answer `None` or `false` for such a holder.
    UnknownHolder,
    /// An objective of that name exists (`Scoreboard.addObjective` throws).
    NameTaken,
    /// The criteria is not one vanilla has: a fixed name, `teamkill.<color>`,
    /// `killedByTeam.<color>`, or `<statistic type>:<statistic id>`.
    UnknownCriteria,
    /// The objective's criteria is read-only (`health`, `food`, `air`, `armor`, `xp`, `level`):
    /// only the server's own updates write its scores, and a command is refused the same way.
    ReadOnlyCriteria,
    /// A text was too deep to apply.
    Text(TextFormError),
}

impl From<TextFormError> for ObjectiveError {
    fn from(error: TextFormError) -> Self {
        Self::Text(error)
    }
}

/// One holder's score on an objective.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ScoreEntry {
    /// The holder, as [`ScoreHolder::Name`]: the board's key.
    pub holder: ScoreHolder,
    /// The score.
    pub value: i32,
}

impl ScoreEntry {
    /// An entry from its parts.
    pub fn new(holder: ScoreHolder, value: i32) -> Self {
        Self { holder, value }
    }
}

/// An objective as this client was sent it: one shown in a display slot.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientObjective {
    /// The objective's name.
    pub name: String,
    /// The display name.
    pub display_name: Text,
    /// How the list and below-name slots draw it.
    pub render_type: ObjectiveRenderType,
    /// The objective's number format, or None for the slot's default.
    pub number_format: Option<NumberFormat>,
    /// The slots it is shown in, in id order.
    pub slots: Vec<DisplaySlot>,
    /// The scores it was sent, in the order they arrived. A holder starting with `#` is hidden
    /// from the sidebar, but is sent.
    pub scores: Vec<ClientScore>,
}

impl ClientObjective {
    /// An objective from its parts.
    pub fn new(
        name: String,
        display_name: Text,
        render_type: ObjectiveRenderType,
        number_format: Option<NumberFormat>,
        slots: Vec<DisplaySlot>,
        scores: Vec<ClientScore>,
    ) -> Self {
        Self {
            name,
            display_name,
            render_type,
            number_format,
            slots,
            scores,
        }
    }
}

/// One score this client was sent.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientScore {
    /// The holder's key: a player's name, an entity's UUID text, or any other string.
    pub holder: String,
    /// The score.
    pub value: i32,
    /// The name to show in place of the holder's, if the server set one.
    pub display: Option<Text>,
    /// This score's own number format, or None for the objective's.
    pub number_format: Option<NumberFormat>,
}

impl ClientScore {
    /// A score from its parts.
    pub fn new(
        holder: String,
        value: i32,
        display: Option<Text>,
        number_format: Option<NumberFormat>,
    ) -> Self {
        Self {
            holder,
            value,
            display,
            number_format,
        }
    }
}

/// What this client has been sent of the scoreboard (`ClientPacketListener`'s board): the
/// objectives the server shows in a display slot, with their scores, and every team. The server
/// sends no other objective, and no score of one it does not show. Read-only; the client has no
/// handles.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientScoreboard {
    /// The shown objectives, each once, in the board's order.
    pub objectives: Vec<ClientObjective>,
    /// Every team: the server sends all of them, shown or not.
    pub teams: Vec<ClientTeam>,
}

impl ClientScoreboard {
    /// A view from its objectives and teams.
    pub fn new(objectives: Vec<ClientObjective>, teams: Vec<ClientTeam>) -> Self {
        Self { objectives, teams }
    }
    /// The objective shown in `slot`.
    pub fn in_slot(&self, slot: DisplaySlot) -> Option<&ClientObjective> {
        self.objectives.iter().find(|o| o.slots.contains(&slot))
    }
}

impl crate::NoBackend for ObjectiveError {
    /// No game: as for a stale handle.
    fn no_backend() -> Self {
        Self::Stale
    }
}
