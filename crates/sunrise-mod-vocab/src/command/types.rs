//! Commands and their authority.

use crate::{ClientHost, CommandInvocation, CommandSource, ServerHost, Text};
use std::sync::Arc;

// no wire: a native executor runs only in the server process.
/// The server's retained callable for an executable command usage. See [`Command::executes`].
pub type CommandExecutor = dyn Fn(&mut dyn ServerHost, &CommandSource, &CommandInvocation) -> Result<i32, Box<Text>>
    + Send
    + Sync
    + 'static;

// no wire: a registration binds server code to a descriptor; only the descriptor is data.
// exhaustive: only Command::executes constructs the opaque descriptor/executor pair.
/// A server command usage together with its executor. Create it with [`Command::executes`].
pub struct CommandRegistration {
    command: Command,
    executor: Arc<CommandExecutor>,
}

/// Separate a registration's display descriptor from its process-local executor.
pub fn split_command_registration(
    registration: CommandRegistration,
) -> (Command, Arc<CommandExecutor>) {
    (registration.command, registration.executor)
}

// no wire: a native client executor runs only in the client process.
/// The client's retained callable for an executable client command usage.
pub type ClientCommandExecutor =
    dyn Fn(&mut dyn ClientHost, &CommandInvocation) -> Option<i32> + Send + Sync + 'static;

// no wire: a registration binds client code to a descriptor; only the descriptor is data.
// exhaustive: only Command::executes_client constructs the opaque descriptor/executor pair.
/// A client command usage together with its executor. Create it with [`Command::executes_client`].
pub struct ClientCommandRegistration {
    command: Command,
    executor: Arc<ClientCommandExecutor>,
}

/// Separate a client registration's descriptor from its process-local executor.
pub fn split_client_command_registration(
    registration: ClientCommandRegistration,
) -> (Command, Arc<ClientCommandExecutor>) {
    (registration.command, registration.executor)
}

/// A data-only command usage. Attach behavior with [`Command::executes`] when
/// registering it on the server, or [`Command::executes_client`] when registering
/// it on the client.
///
/// Literal words use lowercase letters, digits, `_` and `-`. Only the first word may
/// additionally begin with one `/`: register `/set <x:word>` and type `//set stone` in chat.
/// The command's literals retain `/set`. A slash alone, repeated leading slashes and
/// slashes in later literal words are refused.
///
/// A usage is words followed by arguments `<name:kind>`, with optional arguments
/// last in brackets `[name:kind]`:
/// `pay <player:player> <amount:integer>`, `balance [player:player]`,
/// `eco give <player:player> <amount:integer>`. Kinds: `word`, `string` (one word or one
/// `"quoted"` / `'quoted'` phrase, Brigadier's `StringArgumentType.string()`; delivered decoded as
/// `String`, and other arguments may follow it), `text` (the rest of the line, last), `integer`, `integer(min,max)` (Brigadier's
/// `IntegerArgumentType.integer(min, max)`; either bound may be left empty, `integer(0,)`, and a
/// value out of range is refused with `argument.integer.low` / `big` before the executor runs),
/// `long` and `long(min,max)` (`LongArgumentType.longArg(min, max)`, delivered as `Long`, refused
/// with `argument.long.low` / `big`), `number`, `boolean`, `player`, `players`,
/// `entity`, `entities`, `item`, `block_pos`, `vec3` (a point, `Vec3Argument.vec3()`; delivered
/// as `Vec3`), `id` (a free `namespace:path` identifier, delivered as that canonical string),
/// `mod_id` (a loaded package id, delivered as a string), `resource:<registry>` (an id that
/// registry holds, delivered as `Id`: `<kind:resource:minecraft:entity_type>`),
/// `resource_or_tag:<registry>` (vanilla's `ResourceOrTagKeyArgument`: an id or a `#tag` of
/// that registry, completed by the chat box and delivered as `ResourceOrTag`, unchecked
/// against the registry: `<biome:resource_or_tag:minecraft:worldgen/biome>`), or a command
/// argument type a mod registered with `Registrar::register_command_argument`, named by its namespaced id:
/// `machine use <machine:mymod:machine>`. Usages of one command share their
/// words; the server merges them into one tree. A public `foo bar` lets everyone traverse
/// `foo`, but does not by itself make an operator-only `foo` executable. Optional arguments
/// allow a usage to execute at their preceding prefixes too. If accepted usages execute at
/// the same node, each later accepted registration overwrites that node's executor (matching
/// Brigadier's `CommandNode.addChild`) and updates Sunrise's paired execution permission
/// level for that node.
///
/// A usage whose first word is a vanilla command's merges into it, as NeoForge's
/// `RegisterCommandsEvent` registers into vanilla's dispatcher (`CommandDispatcher.register`,
/// Brigadier's `CommandNode.addChild`): `time season <name:word>` adds `season` beside vanilla's
/// `time set`, `add` and `query`, which stay. Vanilla's nodes keep their permission, so the
/// mod's words under `/time` are reached only by who may use `/time`. A usage that executes at a
/// vanilla node replaces vanilla's executor there, as the later registration does in
/// Brigadier. A mod argument cannot share the place of a vanilla node of the same name, and a
/// mod literal cannot share a vanilla argument's: the usage is left out, with the reason in the
/// log. Sunrise's own `/modprofile` and `/pregen` are never merged into. The dedicated server's
/// console runs a few vanilla commands itself before the dispatcher (`kick`, `say` and the
/// others in its console handling), so a mod word merged under one of those is reached by players
/// and functions but not from that console.
///
/// A `resource:<registry>` or `resource_or_tag:<registry>` usage requires a `namespace:path`
/// registry name; an invalid name refuses the usage. There is no separate registration budget for registry names.
///
/// Side: server. Selector arguments use vanilla's source position, permissions,
/// cardinality and selection order; `player`/`entity` require one result and
/// `players`/`entities` require a nonempty result. Unresolvable entity handles are
/// omitted; lists have no extra cap beyond the wire frame limits.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct Command {
    /// The usage.
    pub usage: String,
    /// The operator level a player needs to run this usage (0: everyone).
    /// Shared prefixes stay visible when needed to reach a lower-level usage.
    pub level: u8,
}

impl Command {
    /// A usage every player may run.
    pub fn new(usage: &str) -> Self {
        Self {
            usage: usage.to_owned(),
            level: 0,
        }
    }
    /// The same, for operators of `level` and above.
    pub fn op(usage: &str, level: u8) -> Self {
        Self {
            usage: usage.to_owned(),
            level: level.min(4),
        }
    }

    /// Bind this usage to a server executor for `register_commands`.
    /// The executor runs as the registering mod, after permission checks and argument
    /// resolution. The source identifies the origin and permission, independent of
    /// `execute as`; feedback respects silence, track-output and the acting player.
    /// Return `Ok(n)` for success with result `n` (including zero), or
    /// `Err(message.into())` with a [`Text`] for failure.
    /// Failure text defaults to red and does not depend on `sendCommandFeedback`.
    /// It is not shown for forked commands, silent sources, or sources that do not accept failures.
    /// A failure stores zero for both `execute store success` and `execute store result`. Neither a failed
    /// executor nor a disabled owner gives another mod a turn.
    /// A panic is reported with the mod's name and disables this usage's executor for the
    /// session. It aborts the executor's writes in the caller's open transaction and does
    /// not publish a result to `execute store`. A failed invocation also aborts any
    /// transaction the executor opened and left unfinished. Ordinary commands do not get
    /// an implicit transaction, and neither failure undoes unjournaled effects.
    /// Only a parse from the current server dispatcher can reach this executor.
    /// Capture owned or shared state rather than borrowing the registration event.
    pub fn executes(
        self,
        executor: impl Fn(
            &mut dyn ServerHost,
            &CommandSource,
            &CommandInvocation,
        ) -> Result<i32, Box<Text>>
        + Send
        + Sync
        + 'static,
    ) -> CommandRegistration {
        CommandRegistration {
            command: self,
            executor: Arc::new(executor),
        }
    }

    /// Bind this usage to a client executor for `register_client_commands`.
    /// The executor runs as the registering mod on the client thread, after parsing and
    /// argument resolution. Return `Some(n)` for success with result `n` (including zero),
    /// or `None` for failure. A failed executor or disabled owner never forwards the
    /// command to the server or gives another mod a turn. Only a parse from the current
    /// client dispatcher generation can reach this executor. Slash commands submitted
    /// during execution run later; an attempt to re-enter an executor in the same
    /// submission chain fails locally, even after the dispatcher refreshes. Capture
    /// owned or shared state rather than borrowing the registration event. A panic
    /// disables the usage from that mod for the session, including across refreshes.
    pub fn executes_client(
        self,
        executor: impl Fn(&mut dyn ClientHost, &CommandInvocation) -> Option<i32>
        + Send
        + Sync
        + 'static,
    ) -> ClientCommandRegistration {
        ClientCommandRegistration {
            command: self,
            executor: Arc::new(executor),
        }
    }
}

// no wire: the server role is only used in in-process lifecycle callbacks.
/// The server role that fired a world lifecycle or server tick phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Authority {
    /// The server thread of a single-player or LAN world.
    IntegratedServer,
    /// A dedicated server.
    DedicatedServer,
}

/// `PermissionLevel`: how much a player may do, as `op-permission-level` and
/// the operator list name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum PermissionLevel {
    /// 0: every player.
    All,
    /// 1: moderators.
    Moderators,
    /// 2: game masters.
    Gamemasters,
    /// 3: admins.
    Admins,
    /// 4: owners.
    Owners,
}

impl PermissionLevel {
    /// The level a list entry's number names; out of range keeps to 0 or 4.
    pub fn from_level(level: i32) -> Self {
        match level {
            i32::MIN..=0 => Self::All,
            1 => Self::Moderators,
            2 => Self::Gamemasters,
            3 => Self::Admins,
            _ => Self::Owners,
        }
    }
    /// The level as the list stores it, 0 to 4.
    pub fn level(self) -> u8 {
        self as u8
    }
}
