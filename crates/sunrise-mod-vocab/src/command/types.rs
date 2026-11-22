//! Commands and their authority.

use crate::{CommandInvocation, CommandSource, ServerHost};
use std::sync::Arc;

// no wire: a native executor runs only in the server process.
/// The server's retained callable for an executable command usage.
pub type CommandExecutor = dyn Fn(&mut dyn ServerHost, &CommandSource, &CommandInvocation) -> Option<i32>
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

/// A data-only command usage. Attach behavior with [`Command::executes`] when
/// registering it on the server.
///
/// Literal words use lowercase letters, digits, `_` and `-`. Only the first word may
/// additionally begin with one `/`: register `/set <x:word>` and type `//set stone` in chat.
/// The command's literals retain `/set`. A slash alone, repeated leading slashes and
/// slashes in later literal words are refused.
///
/// A usage is words followed by arguments `<name:kind>`, with optional arguments
/// last in brackets `[name:kind]`:
/// `pay <player:player> <amount:integer>`, `balance [player:player]`,
/// `eco give <player:player> <amount:integer>`. Kinds: `word`, `text` (the
/// rest of the line, last), `integer`, `number`, `boolean`, `player`, `players`,
/// `entity`, `entities`, `item`, `block_pos`, `vec3` (a point, `Vec3Argument.vec3()`; delivered
/// as `Vec3`), `id` (a free `namespace:path` identifier, delivered as that canonical string),
/// `mod_id` (a loaded package id, delivered as a
/// string), `resource:<registry>` (an id that registry holds, delivered as `Id`:
/// `<kind:resource:minecraft:entity_type>`), or a command argument type a mod registered with
/// `Registrar::register_command_argument`, named by its namespaced id:
/// `machine use <machine:mymod:machine>`. Usages of one command share their
/// words; the server merges them into one tree. A public `foo bar` lets everyone traverse
/// `foo`, but does not by itself make an operator-only `foo` executable. Optional arguments
/// allow a usage to execute at their preceding prefixes too. If accepted usages execute at
/// the same node, each later accepted registration overwrites that node's executor (matching
/// Brigadier's `CommandNode.addChild`) and updates Sunrise's paired execution permission
/// level for that node.
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
    /// Return `Some(n)` for success with result `n` (including zero), or `None` for
    /// failure. A failed executor or disabled owner never gives another mod a turn.
    /// A panic is reported with the mod's name and disables this usage's executor for the
    /// session. It aborts the executor's writes in the caller's open transaction and does
    /// not publish a result to `execute store`. A failed invocation also aborts any
    /// transaction the executor opened and left unfinished. Ordinary commands do not get
    /// an implicit transaction, and neither failure undoes unjournaled effects.
    /// Only a parse from the current server dispatcher can reach this executor.
    /// Capture owned or shared state rather than borrowing the registration event.
    pub fn executes(
        self,
        executor: impl Fn(&mut dyn ServerHost, &CommandSource, &CommandInvocation) -> Option<i32>
        + Send
        + Sync
        + 'static,
    ) -> CommandRegistration {
        CommandRegistration {
            command: self,
            executor: Arc::new(executor),
        }
    }
}

/// Which `Game` observes the phase; a mirror `Game` never fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum Authority {
    /// A single-player game with no server thread; does not fire.
    LocalAuthoritative,
    /// The server thread of a single-player or LAN world.
    IntegratedServer,
    /// A dedicated server.
    DedicatedServer,
    /// A client of someone else's server; does not fire.
    RemoteClient,
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
