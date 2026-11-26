//! The server scoreboard's objectives, display slots and scores, and the client's view of them.

use crate::text::TextStyle;
use crate::{ClientTeam, EntityRef, PlayerRef, Text, TextFormError};
use core::ops::ControlFlow;

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
        self.names().0
    }

    /// The color name and its corresponding display slot name.
    fn names(self) -> (&'static str, &'static str) {
        macro_rules! names {
            ($name:literal) => {
                ($name, concat!("sidebar.team.", $name))
            };
        }
        match self {
            Self::Black => names!("black"),
            Self::DarkBlue => names!("dark_blue"),
            Self::DarkGreen => names!("dark_green"),
            Self::DarkAqua => names!("dark_aqua"),
            Self::DarkRed => names!("dark_red"),
            Self::DarkPurple => names!("dark_purple"),
            Self::Gold => names!("gold"),
            Self::Gray => names!("gray"),
            Self::DarkGray => names!("dark_gray"),
            Self::Blue => names!("blue"),
            Self::Green => names!("green"),
            Self::Aqua => names!("aqua"),
            Self::Red => names!("red"),
            Self::LightPurple => names!("light_purple"),
            Self::Yellow => names!("yellow"),
            Self::White => names!("white"),
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
    /// The slot's static name as vanilla writes it (`sidebar.team.red`).
    pub fn name(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Sidebar => "sidebar",
            Self::BelowName => "below_name",
            Self::TeamSidebar(color) => color.names().1,
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

/// A sidebar shown to one player by [`PlayerRef::set_sidebar`], not an objective on the world board.
// no wire: in-process argument to the player handle; only the game sends scoreboard packets.
// exhaustive: mods construct this record with a literal.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sidebar {
    /// The text above the lines.
    pub title: Text,
    /// Lines in display order, top first. Only the first 15 are read, as in vanilla's sidebar HUD.
    pub lines: Vec<SidebarLine>,
}

/// One line of a [`Sidebar`].
// no wire: in-process argument to the player handle, through Sidebar.
// exhaustive: mods construct this record with a literal.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SidebarLine {
    /// The text on the left.
    pub text: Text,
    /// Text in place of the numeric score on the right. `None` leaves that space blank.
    pub value: Option<Text>,
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

/// Why an objective's score or display-slot visit could not read the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: this refusal is returned only by in-process objective visitors
pub enum ObjectiveReadError {
    /// The objective was removed or the handle is from an earlier world open.
    Stale,
    /// The server scoreboard is not readable from this side, phase or backend.
    Unavailable,
}

impl crate::NoBackend for ObjectiveReadError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// One holder's score, lent for the duration of a visitor callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: the holder key is borrowed from the resident scoreboard
pub struct ScoreEntry<'a> {
    /// The board's holder key, including offline and hidden names.
    pub holder: &'a str,
    /// The score.
    pub value: i32,
}

impl<'a> ScoreEntry<'a> {
    /// An entry borrowing the given holder key.
    pub fn new(holder: &'a str, value: i32) -> Self {
        Self { holder, value }
    }
}

/// An objective on the client scoreboard, lent for one visitor call.
// no wire: in-process scoped scoreboard view.
pub trait ClientObjective {
    /// The objective's key.
    fn name(&self) -> &str;

    /// How the list and below-name slots draw it.
    fn render_type(&self) -> ObjectiveRenderType;

    /// Visits every slot showing this objective, in slot ID order. Return `ControlFlow::Break(())`
    /// to stop; an objective just received from the server may have no slot yet.
    fn visit_slots(&self, visit: &mut dyn FnMut(DisplaySlot) -> ControlFlow<()>)
    -> ControlFlow<()>;

    /// Visits the resident scores in board order, including `#` holders hidden by the sidebar.
    /// Return `ControlFlow::Break(())` to stop. Each row is lent for its own visit.
    fn visit_scores(
        &self,
        visit: &mut dyn FnMut(&dyn ClientScore) -> ControlFlow<()>,
    ) -> ControlFlow<()>;

    /// Copies the display name, or reports why this text cannot be represented.
    fn copy_display_name(&self) -> Result<Text, TextFormError>;

    /// Copies the objective's number format, or returns `Ok(None)` for the slot's default.
    /// A conversion error applies to this copy only.
    fn copy_number_format(&self) -> Result<Option<NumberFormat>, TextFormError>;
}

/// One score the client was sent, lent for one visitor call.
// no wire: in-process scoped scoreboard view.
pub trait ClientScore {
    /// The holder's key: a player's name, an entity's UUID text, or any other string.
    fn holder(&self) -> &str;

    /// The score's value.
    fn value(&self) -> i32;

    /// Copies the display name for this holder, or returns `Ok(None)` if not set.
    /// A conversion error applies to this copy only.
    fn copy_display(&self) -> Result<Option<Text>, TextFormError>;

    /// Copies the score's own number format, or returns `Ok(None)` to use the objective's.
    /// A conversion error applies to this copy only.
    fn copy_number_format(&self) -> Result<Option<NumberFormat>, TextFormError>;
}

/// The live board behind vanilla's `ClientLevel.getScoreboard`: everything received from the
/// server so far, with no server handles or writes. Copy only what you need to retain.
// no wire: in-process scoped scoreboard view.
pub trait ClientScoreboard {
    /// Visits every received objective once, in board order, even before its first slot is set.
    /// Return `ControlFlow::Break(())` to stop. The view lasts for one callback.
    fn visit_objectives(
        &self,
        visit: &mut dyn FnMut(&dyn ClientObjective) -> ControlFlow<()>,
    ) -> ControlFlow<()>;

    /// Visits every received team, in board order. Return `ControlFlow::Break(())` to stop.
    /// The view lasts for one callback.
    fn visit_teams(
        &self,
        visit: &mut dyn FnMut(&dyn ClientTeam) -> ControlFlow<()>,
    ) -> ControlFlow<()>;

    /// Calls `visit` once with the objective in `slot`, or returns `None` without calling it
    /// if the slot is empty.
    fn with_slot(
        &self,
        slot: DisplaySlot,
        visit: &mut dyn FnMut(&dyn ClientObjective),
    ) -> Option<()>;
}

impl crate::NoBackend for ObjectiveError {
    /// No game: as for a stale handle.
    fn no_backend() -> Self {
        Self::Stale
    }
}
