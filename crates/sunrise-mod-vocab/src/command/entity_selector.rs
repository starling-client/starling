//! Entity selector tokens a mod registers (`Registrar::register_entity_selector`): `@allies`
//! beside vanilla's `@p @a @r @s @e @n`, in every entity and player argument (NeoForge's
//! `EntitySelectorManager` and `IEntitySelectorType`).

use crate::{CommandSource, EntityRef, ServerHost};

/// Why [`Registrar::register_entity_selector`](crate::Registrar::register_entity_selector) rejected a selector token.
// no wire: selector registration errors stay in-process.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EntitySelectorRegistrationError {
    /// The selector token name was empty.
    EmptyName,
    /// The selector token name contained a character outside Brigadier's unquoted-string syntax (`[0-9A-Za-z_.+-]`).
    InvalidToken {
        /// The rejected token name.
        name: String,
    },
    /// The selector token name collided with a vanilla selector (`p`, `a`, `r`, `s`, `e`, or `n`).
    ReservedVanillaToken {
        /// The rejected vanilla token name.
        name: String,
    },
    /// Another registration already claimed this selector token name.
    DuplicateToken {
        /// The already-registered token name.
        name: String,
    },
    /// Startup registration has already closed and frozen the selector table.
    RegistrationClosed {
        /// The token name submitted after registration closed.
        name: String,
    },
}

impl std::fmt::Display for EntitySelectorRegistrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyName => f.write_str("entity selector token must not be empty"),
            Self::InvalidToken { name } => write!(
                f,
                "entity selector token @{name} contains characters outside Brigadier unquoted-string syntax ([0-9A-Za-z_.+-])"
            ),
            Self::ReservedVanillaToken { name } => {
                write!(
                    f,
                    "entity selector token @{name} clashes with vanilla @{name}"
                )
            }
            Self::DuplicateToken { name } => {
                write!(f, "entity selector token @{name} is already registered")
            }
            Self::RegistrationClosed { name } => write!(
                f,
                "entity selector token @{name} cannot be registered after startup registration closes"
            ),
        }
    }
}

impl std::error::Error for EntitySelectorRegistrationError {}

/// How a registered selector token orders its candidates before its limit, as `sort=` does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum SelectorSort {
    /// `sort=arbitrary`: the level's order, as `@e` and `@a`.
    #[default]
    Arbitrary,
    /// `sort=nearest`, as `@p` and `@n`.
    Nearest,
    /// `sort=furthest`.
    Furthest,
    /// `sort=random`, as `@r`.
    Random,
}

/// The base selection of a registered token, before its `[...]` options: what `@p` or `@e`
/// set for themselves. Options a player writes after the token apply on top, as for `@e`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct SelectorBase {
    /// Only players are candidates, as for `@a` (`type=minecraft:player`).
    pub players_only: bool,
    /// At most this many results; `None` for no limit. `Some(1)` lets a single-entity
    /// argument (`entity`, `player`) take the token.
    pub limit: Option<u32>,
    /// How the candidates are ordered before the limit.
    pub sort: SelectorSort,
    /// The translation key of the completion's tooltip.
    pub tooltip: String,
}

impl SelectorBase {
    /// A base selection with its tooltip key.
    pub fn new(
        players_only: bool,
        limit: Option<u32>,
        sort: SelectorSort,
        tooltip: impl Into<String>,
    ) -> Self {
        Self {
            players_only,
            limit,
            sort,
            tooltip: tooltip.into(),
        }
    }
}

/// A selector token a mod registers: its base selection and which candidates it keeps.
///
/// Register it once, from `Hooks::register`, with `Registrar::register_entity_selector`. Its
/// name is the word after `@`. The server parses `@name[...]` in every entity, entities,
/// player and players argument, vanilla commands included; selector permission is checked as
/// for vanilla selectors. Sunrise clients receive token metadata beside the command tree;
/// vanilla clients may still send the command without local token completion.
///
/// The game keeps one value per token and calls it on the server thread: implement it to be
/// shared, without mutable state.
pub trait EntitySelectorType: Send + Sync + 'static {
    /// The base selection, read when building command registries and syncing selector metadata.
    /// A panic disables this selector for the session and reports an error naming the mod.
    fn base(&self) -> SelectorBase;

    /// Whether `entity` stays selected, asked for each candidate after the base selection
    /// and the `[...]` options, before sorting and the limit. `source` is the command's origin.
    /// A panic disables this selector for the session and reports an error naming the mod.
    fn matches(&self, host: &mut dyn ServerHost, source: &CommandSource, entity: EntityRef)
    -> bool;
}
