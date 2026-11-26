//! Command argument types a mod registers (`Registrar::register_command_argument`): parsing and
//! tab-completion for one kind of value in a `Command` usage.
//!
//! A client that does not know the type still parses the command: the wire tree sends the
//! argument as a vanilla parser of the declared shape (a Brigadier string of one word, a word or
//! quoted phrase, or the rest of the line, or `minecraft:resource_location` for an identifier) and
//! asks the server for its completions. The tree never exposes the mod's parser or provider to the
//! client, so a vanilla client reads the tree.

use crate::{PermissionLevel, Suggestion, Text};

/// How much of the command line a custom argument's value takes.
///
/// The server and a client that does not know the type parse the value as this before the mod's
/// own parser sees it, so the type must say it up front, when it is registered.
///
/// The same bounded numbers are kinds of a usage: `<amount:integer(0,1000)>`, `<total:long>`,
/// `<total:long(0,)>` (see [`Command`](crate::Command)).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum CommandArgumentKind {
    /// One word, as Brigadier's `word` string: ASCII letters and digits, and `_ - . +`. The word
    /// ends at the first other character, a space included, so a value containing `:` or `/`
    /// needs [`Id`](Self::Id), [`Greedy`](Self::Greedy) as the usage's last argument, or
    /// [`Quotable`](Self::Quotable) with the value in quotes.
    #[default]
    Word,
    /// The rest of the line, spaces included: the argument must be the usage's last.
    Greedy,
    /// One unquoted word or one quoted phrase, as Brigadier's `StringArgumentType.string()`
    /// (`QUOTABLE_PHRASE`, `StringReader.readString`): a word of [`Word`](Self::Word)'s
    /// characters, or text between `"` or `'` quotes, empty included, in which only the active
    /// quote and `\` may follow a `\`. Other arguments may follow it, so `"#main" @a` reads
    /// `#main` and then a player. [`parse`](CommandArgumentType::parse) receives the decoded text,
    /// without its quotes; [`suggest`](CommandArgumentType::suggest) receives the prefix as typed,
    /// an opening quote included. A bad escape is refused with Brigadier's `parsing.quote.escape`
    /// and an unclosed phrase with `parsing.quote.expected.end`, before the parser runs. It goes
    /// to clients as `brigadier:string` in its quotable mode, so an unmodded client reads the
    /// phrase too.
    Quotable,
    /// A whole number in `i32` with optional bounds, as Brigadier's
    /// `IntegerArgumentType.integer(min, max)` (NeoForge registers the same Brigadier type in
    /// `RegisterCommandsEvent`). It goes to clients as `brigadier:integer` with the bounds
    /// vanilla's `IntegerArgumentInfo` sends (a bound at `i32::MIN` or `i32::MAX` is left out, as
    /// `None` is), so an unmodded client refuses an out-of-range value too. The server refuses
    /// one with Brigadier's `argument.integer.low` / `argument.integer.big` before the mod's
    /// [`parse`](CommandArgumentType::parse) sees the digits it accepted.
    Integer {
        /// The smallest value accepted; `None` for `i32::MIN`.
        min: Option<i32>,
        /// The largest value accepted; `None` for `i32::MAX`.
        max: Option<i32>,
    },
    /// A whole number in `i64` with optional bounds, as Brigadier's
    /// `LongArgumentType.longArg(min, max)`: `brigadier:long` on the wire, refused out of range
    /// with `argument.long.low` / `argument.long.big`. Otherwise as [`Integer`](Self::Integer).
    Long {
        /// The smallest value accepted; `None` for `i64::MIN`.
        min: Option<i64>,
        /// The largest value accepted; `None` for `i64::MAX`.
        max: Option<i64>,
    },
    /// A vanilla identifier (`minecraft:resource_location`), reading `a-z`, `0-9`, `_`, `-`,
    /// `.`, `:` and `/` through the first other character. It can precede other arguments.
    /// The parser receives the original text, not the canonical identifier. `#` tags are not read.
    Id,
}

/// Why a custom argument's parser refused the text a player wrote.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum CommandArgumentError {
    /// The message shown to the player.
    Message(Box<Text>),
}

impl CommandArgumentError {
    /// A refusal with the message the player sees.
    pub fn new(message: Text) -> Self {
        Self::Message(Box::new(message))
    }

    /// The message the player sees.
    pub fn message(&self) -> &Text {
        let CommandArgumentError::Message(message) = self;
        message
    }
}

impl From<(crate::Identifier, String)> for CommandArgumentError {
    fn from((_, message): (crate::Identifier, String)) -> Self {
        Self::new(Text::literal(message))
    }
}

impl From<CommandArgumentError> for Text {
    fn from(error: CommandArgumentError) -> Self {
        let CommandArgumentError::Message(message) = error;
        *message
    }
}

/// One argument the engine parsed before the one being completed: its name from the command's
/// usage and the text the player wrote for it.
///
/// The text is what the engine accepted for that argument, not the line around it; a completer
/// that needs a value it can use compares the text itself.
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

/// What [`CommandArgumentType::suggest`] is asked for one argument being completed.
///
/// Returned [`Suggestion::range`] values are byte offsets `(start, end)` within [`prefix`](Self::prefix)
/// (default `(0, self.prefix.len())`), preserving arbitrary sub-token cursor ranges, tooltips, and
/// numeric sort values ([`Suggestion::int_value`]).
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

    /// Build a completion replacing the whole typed prefix (`(0, self.prefix.len())`).
    pub fn suggest(&self, text: impl Into<String>) -> Suggestion {
        Suggestion::new((0, self.prefix.len()), text)
    }

    /// Build a completion replacing the whole typed prefix, with an explanation tooltip.
    pub fn suggest_with_tooltip(
        &self,
        text: impl Into<String>,
        tooltip: impl Into<String>,
    ) -> Suggestion {
        self.suggest(text).with_tooltip(tooltip)
    }

    /// Build a completion replacing the typed prefix from byte offset `start` to its end
    /// (`(start, self.prefix.len())`).
    pub fn suggest_from(&self, start: usize, text: impl Into<String>) -> Suggestion {
        Suggestion::new((start, self.prefix.len()), text)
    }
}

/// The player a completion is for: the one typing, with the level they run commands at.
///
/// [`SuggestionRequest::sender`] carries it, so a completer that completes from a mod's own
/// registry answers with what that sender may use and nothing else. The game keeps the answer as
/// it comes: it does not filter a completer's suggestions by permission, and a completer that
/// ignores the sender offers everything to everyone.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
// exhaustive: the game builds it (`mod_api::__private::game::command_sender`), mods only read it
pub struct CommandSender {
    pub(crate) uuid: u128,
    pub(crate) name: String,
    pub(crate) permission: PermissionLevel,
}

impl CommandSender {
    /// The player's UUID.
    pub fn uuid(&self) -> u128 {
        self.uuid
    }

    /// The player's account name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The level they run commands at, the same one a usage's `level` is compared with.
    pub fn permission(&self) -> PermissionLevel {
        self.permission
    }
}

impl Default for CommandSender {
    fn default() -> Self {
        Self {
            uuid: 0,
            name: String::new(),
            permission: PermissionLevel::All,
        }
    }
}

/// A command argument type a mod registers: how one kind of argument is validated and completed.
///
/// Register it once, from `Hooks::register`, with `Registrar::register_command_argument`. Its
/// namespaced id (`mymod:machine`) is the kind a `Command` usage names after the argument
/// name: `Command::new("machine use <machine:mymod:machine>")`. Register that server usage
/// with [`crate::Command::executes`]. The game validates the argument with [`parse`](Self::parse)
/// before calling its executor, and asks
/// [`suggest`](Self::suggest) for the chat box's completions. Players' clients that do not know
/// the type complete it from the server's answer.
///
/// The game keeps one value per registered id and asks it from the server thread and, for a
/// client command, from the window's thread: implement it to be shared, without mutable state.
pub trait CommandArgumentType: Send + Sync + 'static {
    /// The part of the line the argument takes. The same on every client.
    fn kind(&self) -> CommandArgumentKind;

    /// Validate the text a client wrote and return the value the command executor receives.
    ///
    /// `input` is one word (see [`CommandArgumentKind::Word`] for its characters), the
    /// identifier as typed for [`CommandArgumentKind::Id`], the rest of the line for
    /// [`CommandArgumentKind::Greedy`], or the decoded word or phrase for
    /// [`CommandArgumentKind::Quotable`] (`"a \"b\""` arrives as `a "b"`). Return the canonical
    /// text to deliver (an id, a lower-cased name, `"3"` for `"03"`), or the message shown to the
    /// player when the input is not valid.
    fn parse(&self, input: &str) -> Result<String, CommandArgumentError>;

    /// The completions for `request` at this argument, on both server and client commands.
    ///
    /// `request.prefix` is the text the argument holds so far: one word's worth for
    /// [`Word`](CommandArgumentKind::Word) or [`Id`](CommandArgumentKind::Id), the whole tail of the
    /// line up to the cursor for [`Greedy`](CommandArgumentKind::Greedy), and the raw typed text for
    /// [`Quotable`](CommandArgumentKind::Quotable), often mid-quote (`"Some Ti`), as Brigadier
    /// hands a provider. A quotable type returns insertable syntax: a candidate that is not one
    /// word is quoted (`"Some Title"`), as NeoForge's `EntityCommand` quotes its ids.
    /// `request.arguments` lists every argument parsed before this one in parse order, and
    /// `request.sender` is the player asking (or `None` when no player asks: the console, a
    /// command block, or a client completing its own commands before its profile is known).
    ///
    /// Returned [`Suggestion::range`] values are byte offsets `(start, end)` within
    /// `request.prefix` (use [`SuggestionRequest::suggest`] for `(0, request.prefix.len())`,
    /// [`SuggestionRequest::suggest_from`] for a tail replacement, or
    /// [`Suggestion::with_range`] for an arbitrary sub-token span). Tooltips
    /// ([`Suggestion::with_tooltip`]) and numeric sort values ([`Suggestion::with_int_value`])
    /// are preserved through Brigadier's merge and sort. A suggestion whose range is out of
    /// bounds or splits a UTF-8 character is skipped individually with a warning without
    /// discarding valid siblings.
    ///
    /// Vanilla's packet framing limits (32,767 UTF-16 code units for text, 65,535 bytes of Java
    /// modified UTF-8 for a literal NBT tooltip, and vanilla's post-sort 1,000-entry limit) apply
    /// per entry at the `ClientboundCommandSuggestionsPacket` boundary for server completions;
    /// local client completions are not artificially dropped by server packet bounds. A panicking
    /// callback answers no suggestions and is reported against the argument type.
    fn suggest(&self, request: &SuggestionRequest) -> Vec<Suggestion>;
}

/// A ready argument type that takes one of a fixed set of words: NeoForge's `EnumArgument`.
///
/// Register it like any type: `r.register_command_argument("mode", ChoiceArgument::new(["low",
/// "high"]))`, then name `<mode:mymod:mode>` in a usage. A word outside the set is refused with
/// the valid ones listed; completion offers the choices that start with the typed text, in the
/// order given. The value arrives as `CommandArgumentValue::Custom` with the chosen word.
// no wire: a registered argument type stays in its mod; only its parse and completion results cross.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ChoiceArgument {
    choices: Vec<String>,
}

impl ChoiceArgument {
    /// The words a player may write, in completion order. Each must be a Brigadier `word`.
    pub fn new(choices: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            choices: choices.into_iter().map(Into::into).collect(),
        }
    }
}

impl CommandArgumentType for ChoiceArgument {
    fn kind(&self) -> CommandArgumentKind {
        CommandArgumentKind::Word
    }

    fn parse(&self, input: &str) -> Result<String, CommandArgumentError> {
        if self.choices.iter().any(|choice| choice == input) {
            return Ok(input.to_owned());
        }
        // NeoForge's key; a client without it shows the fallback.
        Err(CommandArgumentError::new(Text::new(
            crate::text::TextContent::Translatable {
                key: "commands.neoforge.arguments.enum.invalid".into(),
                fallback: Some("Enum constant must be one of %s, found %s".into()),
                arguments: vec![Text::literal(self.choices.join(", ")), Text::literal(input)],
            },
        )))
    }

    fn suggest(&self, request: &SuggestionRequest) -> Vec<Suggestion> {
        self.choices
            .iter()
            .filter(|choice| choice.starts_with(request.prefix.as_str()))
            .map(|choice| request.suggest(choice))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_choice_refuses_other_words_and_completes_by_prefix() {
        let mode = ChoiceArgument::new(["low", "high"]);
        assert_eq!(mode.parse("high"), Ok("high".into()));
        let refusal = mode.parse("med").unwrap_err();
        let crate::text::TextContent::Translatable { arguments, .. } = &refusal.message().content
        else {
            panic!("a translatable refusal");
        };
        assert_eq!(
            arguments,
            &vec![Text::literal("low, high"), Text::literal("med")]
        );
        let request = SuggestionRequest::new("h", Vec::new(), None);
        assert_eq!(
            mode.suggest(&request),
            vec![Suggestion::new((0, 1), "high")]
        );
    }

    #[test]
    fn an_id_value_crosses_the_wire() {
        use super::super::CommandArgumentValue;
        let value = CommandArgumentValue::Id(crate::Identifier::parse("minecraft:zombie").unwrap());
        let mut out = crate::wire::Writer::new();
        out.put(&value);
        assert_eq!(
            crate::wire::Reader::decode::<CommandArgumentValue>(out.bytes()).unwrap(),
            value
        );
    }

    #[test]
    fn a_vec2_value_crosses_the_wire() {
        use super::super::CommandArgumentValue;
        let value = CommandArgumentValue::Vec2([5.5, -2.25]);
        let mut out = crate::wire::Writer::new();
        out.put(&value);
        assert_eq!(
            crate::wire::Reader::decode::<CommandArgumentValue>(out.bytes()).unwrap(),
            value
        );
    }

    #[test]
    fn suggestion_request_and_builders_preserve_ranges_tooltips_and_int_values() {
        let request = SuggestionRequest::new(
            "stone st",
            vec![TypedArgument::new("mode", "use")],
            Some(CommandSender {
                uuid: 7,
                name: "Simon".into(),
                permission: PermissionLevel::Gamemasters,
            }),
        );
        let mut out = crate::wire::Writer::new();
        out.put(&request);
        assert_eq!(
            crate::wire::Reader::decode::<SuggestionRequest>(out.bytes()).unwrap(),
            request
        );

        let full = request.suggest_with_tooltip("stone", "a block");
        assert_eq!(full.range, (0, 8));
        assert_eq!(full.text, "stone");
        assert_eq!(full.tooltip.as_deref(), Some("a block"));
        assert_eq!(full.int_value, None);

        let tail = request.suggest_from(6, "stone").with_int_value(42);
        assert_eq!(tail.range, (6, 8));
        assert_eq!(tail.int_value, Some(42));

        let sub = request.suggest("dirt").with_range((0, 5));
        assert_eq!(sub.range, (0, 5));

        for suggestion in [full, tail, sub] {
            let mut out = crate::wire::Writer::new();
            out.put(&suggestion);
            assert_eq!(
                crate::wire::Reader::decode::<Suggestion>(out.bytes()).unwrap(),
                suggestion
            );
        }
    }
}
