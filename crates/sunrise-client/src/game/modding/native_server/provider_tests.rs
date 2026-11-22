//! A mod's command argument type declaring a suggestion provider: the tree's ask-the-server
//! marker, the request the engine builds for it, and what reaches a player's completion.
//!
//! The engine talks to the mod API boundary (`mod_api::SuggestionProvider`), so these tests
//! stand where the game does: they build the report tree, register it into a dispatcher, and
//! complete like `guest_suggestions` does with the guest's registries swapped in.
use super::*;
use std::sync::Mutex;
use sunrise_client::command::{Registries, Suggestion};

/// What one provider call saw: the prefix, the arguments before it, and the sender.
type Seen = Arc<Mutex<Vec<(String, Vec<(String, String)>, Option<(u128, String, u8)>)>>>;

fn seen() -> Seen {
    Default::default()
}

/// A command argument type whose suggestions come from a provider, and whose own `suggest`
/// panics: the engine must not call both for the same argument.
struct Declared {
    candidates: Vec<mod_api::CommandSuggestion>,
    seen: Seen,
    panic_on_answer: bool,
}

impl mod_api::CommandArgumentType for Declared {
    fn kind(&self) -> mod_api::CommandArgumentKind {
        mod_api::CommandArgumentKind::Word
    }

    fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
        Ok(input.to_owned())
    }

    fn suggest(
        &self,
        _: Option<&mod_api::CommandSender>,
        _: &str,
    ) -> Vec<mod_api::CommandSuggestion> {
        panic!("`suggest` is not called for an argument that declares a provider")
    }

    fn suggestion_provider(&self) -> Option<Arc<dyn mod_api::SuggestionProvider>> {
        Some(Arc::new(DeclaredProvider {
            candidates: self.candidates.clone(),
            seen: self.seen.clone(),
            panic_on_answer: self.panic_on_answer,
        }))
    }
}

struct DeclaredProvider {
    candidates: Vec<mod_api::CommandSuggestion>,
    seen: Seen,
    panic_on_answer: bool,
}

impl mod_api::SuggestionProvider for DeclaredProvider {
    fn suggest(&self, request: &mod_api::SuggestionRequest) -> mod_api::SuggestionReply {
        self.seen.lock().unwrap().push((
            request.prefix.clone(),
            request
                .arguments
                .iter()
                .map(|argument| (argument.name.clone(), argument.text.clone()))
                .collect(),
            request.sender.as_ref().map(|sender| {
                (
                    sender.uuid(),
                    sender.name().to_owned(),
                    sender.permission().level(),
                )
            }),
        ));
        if self.panic_on_answer {
            panic!("the mod's provider panicked");
        }
        mod_api::SuggestionReply::new(self.candidates.clone())
    }
}

/// A type with no provider declaration: its own `suggest` answers.
struct Plain;

impl mod_api::CommandArgumentType for Plain {
    fn kind(&self) -> mod_api::CommandArgumentKind {
        mod_api::CommandArgumentKind::Word
    }

    fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
        Ok(input.to_owned())
    }

    fn suggest(
        &self,
        _: Option<&mod_api::CommandSender>,
        prefix: &str,
    ) -> Vec<mod_api::CommandSuggestion> {
        ["survival", "creative"]
            .into_iter()
            .filter(|mode| mode.starts_with(prefix))
            .map(mod_api::CommandSuggestion::new)
            .collect()
    }
}

fn arguments(declared: Declared) -> CommandArguments {
    HashMap::from([
        (
            "test:warp".to_owned(),
            Arc::new(declared) as Arc<dyn mod_api::CommandArgumentType>,
        ),
        (
            "test:mode".to_owned(),
            Arc::new(Plain) as Arc<dyn mod_api::CommandArgumentType>,
        ),
    ])
}

/// The tree and dispatcher of `warp go <world:test:mode> <name:test:warp>`.
fn warp(declared: Declared) -> (Value, Dispatcher) {
    let arguments = arguments(declared);
    let (tree, _, problems) = super::command_tree(
        &[mod_api::Command::new(
            "warp go <world:test:mode> <name:test:warp>",
        )],
        |_| false,
        &arguments,
    );
    assert!(problems.is_empty(), "{problems:?}");
    let mut dispatcher = Dispatcher::new();
    register_dispatcher(&mut dispatcher, &tree, "native_mod_command", &arguments);
    (tree, dispatcher)
}

fn guest() -> Registries {
    Registries {
        permission: 3,
        sender: Some(sunrise_client_core::registries::CommandSender {
            uuid: 0x5eed,
            name: "Simon".into(),
        }),
        ..Registries::default()
    }
}

fn texts(suggestions: &[Suggestion]) -> Vec<&str> {
    suggestions.iter().map(|s| s.text.as_str()).collect()
}

#[test]
fn a_declared_provider_is_asked_only_for_its_argument_with_the_sender_and_the_arguments_before_it()
{
    let seen = seen();
    let (tree, dispatcher) = warp(Declared {
        candidates: vec![
            mod_api::CommandSuggestion::with_tooltip("press", "the press"),
            mod_api::CommandSuggestion::new("furnace"),
        ],
        seen: seen.clone(),
        panic_on_answer: false,
    });

    // The engine mirrors the declaration as the node's ask-the-server marker, so a client asks
    // the server for this argument instead of running the mod's code.
    let node = &tree["children"]["warp"]["children"]["go"]["children"]["world"]["children"]["name"];
    assert_eq!(node["custom_argument"], "test:warp");
    assert_eq!(node["suggestions"], "minecraft:ask_server");
    assert_eq!(
        tree["children"]["warp"]["children"]["go"]["children"]["world"]["custom_argument"],
        "test:mode"
    );

    let typed = "warp go survival pr";
    let registries = guest();
    let (_, suggestions) = dispatcher.suggestions(
        &dispatcher.parse(typed, &registries),
        typed.len(),
        &registries,
    );
    // The engine keeps the candidates that match the typed text, in the chat's own order.
    assert_eq!(texts(&suggestions), ["press"]);
    assert_eq!(suggestions[0].tooltip.as_deref(), Some("the press"));
    // The provider saw the prefix of its argument alone, the arguments parsed before it, and the
    // sender's uuid, name and command level.
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        [(
            "pr".to_owned(),
            vec![("world".to_owned(), "survival".to_owned())],
            Some((0x5eed, "Simon".to_owned(), 3))
        )]
    );
}

/// The same builder builds the server's tree and the client's own: a client command that names an
/// argument type declaring a provider is refused by name, and no mod code runs for it, while an
/// ordinary argument type still builds and completes.
#[test]
fn a_client_command_refuses_an_argument_type_that_declares_a_provider_by_name() {
    let seen = seen();
    let arguments = arguments(Declared {
        candidates: vec![mod_api::CommandSuggestion::new("press")],
        seen: seen.clone(),
        panic_on_answer: false,
    });
    let commands = [
        mod_api::Command::new("warp go <world:test:mode> <name:test:warp>"),
        mod_api::Command::new("warp use <world:test:mode>"),
    ];

    // The client's own tree: the usage that would run the provider is left out, named.
    let (tree, _, problems) = super::client_command_tree(&commands, |_| false, &arguments);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("test:warp"), "{}", problems[0]);
    assert!(
        problems[0].contains("suggestion provider"),
        "{}",
        problems[0]
    );
    assert!(
        tree["children"]["warp"]["children"].get("go").is_none(),
        "the provider-backed usage is left out"
    );

    // The ordinary argument type keeps building, and completes by its own `suggest`.
    let mut chat = sunrise_client::command::vanilla::game_dispatcher().clone();
    register_dispatcher(&mut chat, &tree, "native_mod_command", &arguments);
    let registries = Registries::default();
    let typed = "warp use cr";
    let (_, suggestions) =
        chat.client_suggestions(&chat.parse(typed, &registries), typed.len(), &registries);
    assert_eq!(texts(&suggestions), ["creative"]);
    assert!(
        seen.lock().unwrap().is_empty(),
        "no provider ran for a client command"
    );

    // The server's tree keeps the same usage.
    let (tree, _, problems) = super::command_tree(&commands, |_| false, &arguments);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        tree["children"]["warp"]["children"]["go"]["children"]["world"]["children"]["name"]["custom_argument"],
        "test:warp"
    );
}

#[test]
fn a_panicking_provider_answers_no_suggestion_and_the_rest_of_the_line_still_completes() {
    let seen = seen();
    let (_, dispatcher) = warp(Declared {
        candidates: Vec::new(),
        seen,
        panic_on_answer: true,
    });
    let registries = guest();
    let typed = "warp go survival pr";
    let (_, suggestions) = dispatcher.suggestions(
        &dispatcher.parse(typed, &registries),
        typed.len(),
        &registries,
    );
    assert!(suggestions.is_empty(), "{suggestions:?}");

    // One argument earlier, the type that declares nothing still completes by its own `suggest`.
    let typed = "warp go cr";
    let (_, suggestions) = dispatcher.suggestions(
        &dispatcher.parse(typed, &registries),
        typed.len(),
        &registries,
    );
    assert_eq!(texts(&suggestions), ["creative"]);
}
