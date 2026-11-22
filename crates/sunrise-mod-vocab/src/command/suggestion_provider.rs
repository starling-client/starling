//! A suggestion provider a mod declares on a command argument type
//! (`CommandArgumentType::suggestion_provider`): asked for that argument alone, with the text
//! typed for it, the arguments parsed before it and the sender.
//!
//! The engine mirrors the declaration as the argument node's ask-the-server marker, so a client
//! sends a completion request for the arguments that declare a provider instead of completing
//! them itself: no mod code ever runs on a client. The provider answers candidates and the engine
//! keeps the ones that match the typed text, so every mod completes the same way.

use super::command_argument::{CommandSender, CommandSuggestion};

/// One argument the engine parsed before the one being completed: its name from the command's
/// usage and the text the player wrote for it.
///
/// The text is what the engine accepted for that argument, not the line around it; a provider that
/// needs a value it can use compares the text itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct TypedArgument {
    /// The name between the brackets of the usage, `<name:kind>`.
    pub name: String,
    /// The text the engine parsed for that argument.
    pub text: String,
}

impl TypedArgument {
    /// An argument named `name`, parsed from `text`.
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            text: text.into(),
        }
    }
}

/// What a provider is asked for one argument being completed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct SuggestionRequest {
    /// The text typed for this argument so far, without the words before it.
    pub prefix: String,
    /// The arguments the engine parsed before this one, in parse order.
    pub arguments: Vec<TypedArgument>,
    /// The player asking, or `None` when no player asks: the console, a command block, and a
    /// client completing its own commands before its profile is known.
    pub sender: Option<CommandSender>,
}

impl SuggestionRequest {
    /// The request the engine builds for one argument, from what its dispatcher parsed.
    pub fn new(
        prefix: impl Into<String>,
        arguments: Vec<TypedArgument>,
        sender: Option<CommandSender>,
    ) -> Self {
        Self {
            prefix: prefix.into(),
            arguments,
            sender,
        }
    }
}

/// What a provider answers: candidates, which the engine filters against the typed text.
///
/// At most 1,000 candidates are kept, counted after the engine's own matching and drops, each
/// within the protocol's 32,767 UTF-16 code units (and a tooltip within 65,535 bytes of Java
/// modified UTF-8). A candidate past either bound is dropped and reported in the diagnostics; the
/// others are kept. Non-exhaustive so a later field does not break a mod that builds one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct SuggestionReply {
    /// The candidates for the typed prefix; the engine keeps the matching ones and sorts them
    /// before the chat shows them.
    pub suggestions: Vec<CommandSuggestion>,
}

impl SuggestionReply {
    /// A reply offering `suggestions`.
    pub fn new(suggestions: impl IntoIterator<Item = CommandSuggestion>) -> Self {
        Self {
            suggestions: suggestions.into_iter().collect(),
        }
    }
}

/// A mod's suggestions for one argument of one of its command argument types.
///
/// Declare it from the type: `CommandArgumentType::suggestion_provider` returns it, and the type's
/// arguments then ask it instead of [`CommandArgumentType::suggest`](crate::CommandArgumentType::suggest).
/// The engine calls it on the server, when a player's client asks for that argument: nothing is
/// kept between two requests, and a panic answers no suggestion for that request. A candidate
/// that does not match the typed text, or that is past the protocol's bounds, is dropped alone,
/// and the others are kept.
///
/// The game keeps one value per registered argument type and may ask it from more than one
/// request: implement it to be shared, without mutable state.
pub trait SuggestionProvider: Send + Sync + 'static {
    /// The candidates for `request`: the text typed, the arguments before it and the sender.
    fn suggest(&self, request: &SuggestionRequest) -> SuggestionReply;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PermissionLevel;

    #[test]
    fn a_reply_behind_an_arc_crosses_the_wire_like_any_plain_value() {
        let request = SuggestionRequest {
            prefix: "pre".into(),
            arguments: vec![TypedArgument::new("mode", "use")],
            sender: Some(CommandSender {
                uuid: 7,
                name: "Simon".into(),
                permission: PermissionLevel::Gamemasters,
            }),
        };
        let mut out = crate::wire::Writer::new();
        out.put(&request);
        assert_eq!(
            crate::wire::Reader::decode::<SuggestionRequest>(out.bytes()).unwrap(),
            request
        );

        let reply = SuggestionReply::new([CommandSuggestion::with_tooltip("press", "a press")]);
        let mut out = crate::wire::Writer::new();
        out.put(&reply);
        assert_eq!(
            crate::wire::Reader::decode::<SuggestionReply>(out.bytes()).unwrap(),
            reply
        );
    }
}
