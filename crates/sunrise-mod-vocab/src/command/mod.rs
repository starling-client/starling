//! Parsed native command input and the source that executes it.

use crate::{DimensionKey, EntityRef, Identifier, ItemStack, LevelRef, PermissionLevel, PlayerRef};
use sunrise_core::nbt::Compound;

/// The origin passed to a registered server command's executor, independent of
/// the acting entity selected by `execute as`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum CommandSource {
    /// A player's command; their state is swapped in for the executor.
    Player {
        /// The player who submitted the command.
        player: PlayerRef,
        /// The execution permission, including function overrides.
        permission: PermissionLevel,
    },
    /// The dedicated server console or RCON.
    Console {
        /// The execution permission.
        permission: PermissionLevel,
    },
    /// A command block, including when `execute` moves its execution position.
    CommandBlock {
        /// The block's position, not the execution position.
        position: [i32; 3],
        /// The block's dimension.
        level: DimensionKey,
        /// The execution permission.
        permission: PermissionLevel,
    },
    /// A command-block minecart.
    CommandBlockMinecart {
        /// The minecart that submitted the command.
        entity: EntityRef,
        /// The execution permission.
        permission: PermissionLevel,
    },
    /// The innermost executing function; feedback still follows its caller.
    Function {
        /// The function's namespaced identifier.
        id: Identifier,
        /// The function's execution permission.
        permission: PermissionLevel,
    },
}

impl CommandSource {
    /// The effective permission checked before parsing and argument resolution.
    pub fn permission(&self) -> PermissionLevel {
        match self {
            Self::Player { permission, .. }
            | Self::Console { permission }
            | Self::CommandBlock { permission, .. }
            | Self::CommandBlockMinecart { permission, .. }
            | Self::Function { permission, .. } => *permission,
        }
    }
}

/// One validated native command argument, resolved against its command source.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum CommandArgumentValue {
    /// A `word`, `string` or greedy `text` argument, exactly as the dispatcher parsed it: a
    /// `string` arrives decoded, without its quotes and escapes (`"a \"b\""` as `a "b"`).
    String(String),
    /// A signed `integer` argument, within the bounds of `integer(min,max)`.
    Integer(i32),
    /// A signed `long` argument, within the bounds of `long(min,max)`: Brigadier's
    /// `LongArgumentType.getLong`.
    Long(i64),
    /// A finite double-precision `number` argument.
    Number(f64),
    /// A `boolean` argument.
    Boolean(bool),
    /// The single online player selected by a `player` argument. Side: server.
    Player(PlayerRef),
    /// The single handled entity selected by an `entity` argument. Side: server.
    /// Unresolvable handles are refused, not delivered.
    Entity(EntityRef),
    /// Handled entities selected by `entities`, in selector order. Side: server.
    ///
    /// Entities without a resolvable handle are omitted; an empty result refuses the
    /// command. Lists have no additional cap beyond the wire frame limits.
    /// Dragon parts are not selector members. Leash knots, item frames,
    /// paintings and tridents are delivered when their residency handle resolves;
    /// temporarily held-out and removed entities are omitted.
    Entities(Vec<EntityRef>),
    /// Online players selected by `players`, in selector order. Side: server.
    /// Nonempty, with no additional cap beyond the wire frame limits.
    Players(Vec<PlayerRef>),
    /// An `item` argument with its components applied and a count of one.
    Item(ItemStack),
    /// A `block_pos` argument resolved from world, relative or local coordinates.
    /// Resolution does not load chunks or require the block to be loaded.
    BlockPos([i32; 3]),
    /// A `vec3` or `vec3_uncentered` argument resolved against the source: `~` and `^` offsets
    /// applied to its position and rotation. `vec3` centres whole `x` and `z` on their block
    /// (`5` is `5.5`); `vec3_uncentered` keeps them as written. Resolution does not load chunks.
    Vec3([f64; 3]),
    /// A `column_pos` argument: a block `x z`, `~` resolved against the source's block and
    /// floored.
    ColumnPos([i32; 2]),
    /// A `rotation` argument as yaw and pitch in degrees, `~` resolved against the source's
    /// rotation, as `entity_rotation` returns them.
    Rotation([f32; 2]),
    /// A `dimension` argument: the level of a dimension the world has loaded. An unknown one is
    /// refused with `argument.dimension.invalid`. Side: server.
    Dimension(LevelRef),
    /// An `nbt` argument: a compound tag written as SNBT.
    Nbt(Compound),
    /// A custom argument type's canonical value: the registered type's id
    /// (`mymod:machine`) and what its parser returned for the typed text.
    Custom {
        /// The id of the registered `CommandArgumentType`.
        kind: String,
        /// The value `CommandArgumentType::parse` returned.
        value: String,
    },
    /// A `resource:<registry>` argument: an id present in that registry (static or data pack).
    Id(Identifier),
    /// A `resource_or_tag:<registry>` argument (`ResourceOrTagKeyArgument.Result`): `tag` true
    /// for `#namespace:path`, a tag key of that registry, false for a resource key. `id` is the
    /// identifier without the `#`. Neither is checked against the registry when parsed, as in
    /// vanilla: look it up when the command runs.
    ResourceOrTag {
        /// Whether the text named a tag (`#minecraft:is_ocean`).
        tag: bool,
        /// The resource or tag identifier, without the `#`.
        id: Identifier,
    },
    /// A `game_profile` argument: UUID and account name for each result, in selector order.
    /// A plain name can also select an offline player whose profile is still in the user cache.
    /// Never asks the profile service. Side: server.
    GameProfiles(Vec<(u128, String)>),
    /// A `vec2` or `vec2_uncentered` argument resolved against the source: `~` offsets applied
    /// to its `x` and `z` position, narrowed to `f32` as vanilla's `Vec2Argument.getVec2` does.
    /// `vec2` centres whole numbers on their block (`5` is `5.5`); `vec2_uncentered` keeps them
    /// as written. Local `^` coordinates are refused. Resolution does not load chunks.
    Vec2([f32; 2]),
}

/// An argument's registered name and its validated value.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct CommandArgument {
    /// The name from the command's usage, without its kind or brackets.
    pub name: String,
    /// The value produced by parsing and source resolution.
    pub value: CommandArgumentValue,
}

impl CommandArgument {
    /// Pair a registered argument name with its parsed value.
    pub fn new(name: String, value: CommandArgumentValue) -> Self {
        Self { name, value }
    }
}

/// Validated input borrowed by a registered server or client command's executor, never a raw line.
///
/// Missing optional arguments are absent. Selectors are permission-checked and resolved
/// by the current server dispatcher before the executor runs. `entity` and `player` select exactly one;
/// `entities` and `players` select one or more, in selector order. Unhandled entities
/// are omitted, and an empty result is refused. Failed parsing or resolution produces
/// command feedback instead.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
#[wire(via = InvocationRecord)]
pub struct CommandInvocation {
    /// The literal path, including the registered command name and subcommands.
    pub literals: Vec<String>,
    /// Present arguments in parse order; omitted optional arguments have no entry.
    pub arguments: Vec<CommandArgument>,
    /// The level the command executes in, after `execute in`; the console's is the overworld.
    pub dimension: DimensionKey,
    /// The execution position, after `execute at` or `positioned`; the console's is the
    /// world spawn's block corner.
    pub position: [f64; 3],
    /// The entity the command executes as (`CommandSourceStack.getEntity`), after `execute as`
    /// or `on`: the submitting player or command-block minecart, or the entity `as` selected.
    /// None for the console, a command block, or a target that no longer resolves.
    pub entity: Option<EntityRef>,
    /// The execution rotation in degrees as `[yaw, pitch]` (Java's `Vec2` holds pitch first),
    /// after `execute at`, `rotated` or `facing`, like `CommandArgumentValue::Rotation`. The
    /// console's is zero; a command block's yaw is the direction it faces.
    pub rotation: [f32; 2],
    /// `entity` as a player, set with it by `with_player`. `player()` answers it only while
    /// `entity` is still its entity, and a record where the two differ does not decode.
    player: Option<PlayerRef>,
}

/// `CommandInvocation`'s wire record: its fields in order. Decoding refuses a player whose
/// entity is not the invocation's.
#[derive(sunrise_mod_api_macros::Wire)]
struct InvocationRecord {
    literals: Vec<String>,
    arguments: Vec<CommandArgument>,
    dimension: DimensionKey,
    position: [f64; 3],
    entity: Option<EntityRef>,
    rotation: [f32; 2],
    player: Option<PlayerRef>,
}

impl From<&CommandInvocation> for InvocationRecord {
    fn from(invocation: &CommandInvocation) -> Self {
        Self {
            literals: invocation.literals.clone(),
            arguments: invocation.arguments.clone(),
            dimension: invocation.dimension,
            position: invocation.position,
            entity: invocation.entity,
            rotation: invocation.rotation,
            player: invocation.player(),
        }
    }
}

impl TryFrom<InvocationRecord> for CommandInvocation {
    type Error = &'static str;
    fn try_from(record: InvocationRecord) -> Result<Self, Self::Error> {
        if record
            .player
            .is_some_and(|player| record.entity != Some(player.entity()))
        {
            return Err("the player is not the invocation's entity");
        }
        Ok(Self {
            literals: record.literals,
            arguments: record.arguments,
            dimension: record.dimension,
            position: record.position,
            entity: record.entity,
            rotation: record.rotation,
            player: record.player,
        })
    }
}

impl CommandInvocation {
    /// Build an invocation from its parsed literal path, resolved arguments and execution
    /// level, position and `[yaw, pitch]` rotation, executing as no entity.
    pub fn new(
        literals: Vec<String>,
        arguments: Vec<CommandArgument>,
        dimension: DimensionKey,
        position: [f64; 3],
        rotation: [f32; 2],
    ) -> Self {
        Self {
            literals,
            arguments,
            dimension,
            position,
            entity: None,
            rotation,
            player: None,
        }
    }

    /// The same invocation executing as `entity`, which is not a player.
    pub fn with_entity(self, entity: EntityRef) -> Self {
        Self {
            entity: Some(entity),
            player: None,
            ..self
        }
    }

    /// The same invocation executing as `player`.
    pub fn with_player(self, player: PlayerRef) -> Self {
        Self {
            entity: Some(player.entity()),
            player: Some(player),
            ..self
        }
    }

    /// The entity the command executes as when it is a player (`CommandSourceStack.getPlayer`):
    /// the player `with_player` set, while `entity` is still theirs. A command that needs one
    /// maps `None` to its own failure, such as `permissions.requires.player`.
    pub fn player(&self) -> Option<PlayerRef> {
        self.player
            .filter(|player| self.entity == Some(player.entity()))
    }

    /// The last argument registered under `name`, or `None` when it is absent.
    pub fn argument(&self, name: &str) -> Option<&CommandArgumentValue> {
        self.arguments
            .iter()
            .rev()
            .find(|argument| argument.name == name)
            .map(|argument| &argument.value)
    }
}

/// One structured message delivered to the native console's command source.
// no wire: these messages travel through an in-process command output sink.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CommandFeedback {
    /// Feedback sent by `CommandSourceStack.sendSuccess`.
    Success(crate::Text),
    /// Feedback sent by `CommandSourceStack.sendFailure`, including its error style.
    Failure(crate::Text),
    /// A direct `CommandSourceStack.sendSystemMessage`, not gated as success feedback.
    System(crate::Text),
}

/// A command's outcome and the messages received by its source.
///
/// The default is an unsuccessful run with no feedback, including when a handle
/// cannot resolve or its host has no backend.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct CommandResult {
    /// Whether the command parsed and ran successfully, as `EntityRef::run_command` reports.
    pub success: bool,
    /// Success and failure messages received by the source, in order, as plain text.
    /// Player chat delivery is preserved. Silent sources have no feedback lines.
    pub feedback: Vec<String>,
}

impl CommandResult {
    /// Build a result from the execution outcome and the source's feedback lines.
    pub fn new(success: bool, feedback: Vec<String>) -> Self {
        Self { success, feedback }
    }
}

pub(crate) mod command_argument;
pub(crate) mod entity_selector;
pub(crate) mod suggestion;
pub(crate) mod tree_filter;
pub(crate) mod types;

#[cfg(test)]
mod invocation_tests;
