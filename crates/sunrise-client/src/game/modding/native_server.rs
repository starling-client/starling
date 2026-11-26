//! Server commands and players for native mods (docs/modding/src/how-to/native-mods.md,
//! "Server commands and players"): the commands mods add to the
//! server's command tree (`native_register_commands`), a player's command
//! reaching its bound executor, and the helpers a server mod
//! answers with: the players online, a line to one of them or to all, the
//! command's feedback and the world's folder; and the acting
//! player's plain items: count, take, give. They act on the server's own
//! players (the hub's: a dedicated server's, a LAN host's, single player's
//! server thread's), never on a client's.
#[path = "native_server/argument_kinds.rs"]
mod argument_kinds;
#[cfg(test)]
#[path = "native_server/command_failure_tests.rs"]
mod command_failure_tests;
#[cfg(test)]
#[path = "native_server/command_shape_tests.rs"]
mod command_shape_tests;
#[cfg(test)]
#[path = "native_server/profile_tests.rs"]
mod profile_tests;
#[cfg(test)]
#[path = "native_server/provider_tests.rs"]
mod provider_tests;
#[path = "native_server/registered_commands.rs"]
mod registered_commands;
#[cfg(test)]
#[path = "native_server/selector_tests.rs"]
mod selector_tests;
#[cfg(test)]
#[path = "native_server/sidebar_tests.rs"]
mod sidebar_tests;

use super::api_values::{TryIntoApi, TryIntoEngine};
use super::{Dim, Game, respawn_anchor::RespawnConfig};
pub(super) use argument_kinds::{CommandArguments, command_arguments};
use argument_kinds::{
    UNCENTERED, argument_type, bounded_parser, custom_parser, custom_value, dimension_value,
    nbt_value, parsed_type, parser, positional_value, resource_or_tag_parser, resource_parser,
};
use mod_api::__private::game::*;
pub(super) use registered_commands::{
    ClientCommandChain, NativeClientCommandState, NativeCommandState, PendingClientCommandKind,
    RegisteredClientExecutor,
};
// Their old home, for mixin code that names them here.
use mod_api::LevelRef;
use mod_api::{
    ActionOutcome, ActionRefused, EntityRef, ExperienceOrbRef, ExperienceOrbWriteError, FakeEffect,
    GameMode, GlobalPos, Identifier, LevelReadError, PlayerAction, PlayerActionSetup,
    PlayerDataRefusal, PlayerFood, PlayerRef, PlayerRespawnConfig, ScopedActionResult, Sidebar,
    TextFormError, VirtualHost,
};
use serde_json::{Map, Value, json};
use std::sync::{
    LazyLock, Mutex, PoisonError,
    atomic::{AtomicBool, Ordering},
};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use sunrise_client::{
    command::{Arg, ArgType, Context, Dispatcher},
    items::ItemStack,
    lan::{
        abilities::PlayerAbilities, data::component_nbt, guest::Guest, scoreboard::SidebarContents,
    },
    motion::PlayerMotion,
    text::Text,
};
use sunrise_core::position::BlockPos;
use sunrise_protocol::packets::play::{self, Clientbound as C};

/// The report's name of an operator level (`commands.json`'s
/// `permissions`).
const LEVELS: [&str; 5] = ["", "moderators", "gamemasters", "admins", "owners"];

/// One parsed word of a usage: its key in the tree and its node, without
/// children.
fn usage_nodes(
    usage: &str,
    custom: &CommandArguments,
    client: bool,
) -> Result<Vec<(String, Value, bool)>, String> {
    let mut out = Vec::new();
    let mut optional_seen = false;
    let mut greedy_seen = false;
    for (index, word) in usage.split_whitespace().enumerate() {
        if greedy_seen {
            return Err(format!("`{usage}`: a text argument must be last"));
        }
        let (inner, optional) = if let Some(w) = word.strip_prefix('<') {
            (w.strip_suffix('>'), false)
        } else if let Some(w) = word.strip_prefix('[') {
            (w.strip_suffix(']'), true)
        } else {
            if optional_seen {
                return Err(format!("`{usage}`: `{word}` follows an optional argument"));
            }
            if !argument_kinds::command_word(word, index == 0) {
                return Err(format!("`{usage}`: `{word}` is not a command word"));
            }
            out.push((word.to_owned(), json!({"type": "literal"}), false));
            continue;
        };
        let Some((name, kind)) = inner.and_then(|w| w.split_once(':')) else {
            return Err(format!("`{usage}`: `{word}` is not `<name:kind>`"));
        };
        if client
            && matches!(
                kind,
                "player" | "players" | "entity" | "entities" | "dimension"
            )
        {
            return Err(format!(
                "`{usage}`: `{kind}` is a server-only argument kind and cannot be used in a client command"
            ));
        }
        if optional_seen && !optional {
            return Err(format!("`{usage}`: `{word}` follows an optional argument"));
        }
        if client && kind == "game_profile" {
            return Err(format!(
                "`{usage}`: `game_profile` requires a server command"
            ));
        }
        let resource = kind
            .strip_prefix("resource:")
            .map(|registry| resource_parser(registry).map_err(|e| format!("`{usage}`: {e}")))
            .transpose()?;
        let resource_or_tag = kind
            .strip_prefix("resource_or_tag:")
            .map(|registry| resource_or_tag_parser(registry).map_err(|e| format!("`{usage}`: {e}")))
            .transpose()?;
        let bounded = bounded_parser(kind)
            .transpose()
            .map_err(|e| format!("`{usage}`: {e}"))?;
        let builtin = resource
            .or(resource_or_tag)
            .or(bounded)
            .or_else(|| parser(kind).map(|(parser, properties, _)| (parser, properties)));
        let (parser, properties, greedy, custom) = match builtin {
            Some((parser, properties)) => (parser, properties, kind == "text", None),
            None => {
                let Some(argument) = custom.get(kind) else {
                    return Err(format!("`{usage}`: unknown kind `{kind}`"));
                };
                let declared =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| argument.kind()))
                        .map_err(|_| format!("`{usage}`: argument type `{kind}` panicked"))?;
                let Some((parser, properties)) = custom_parser(declared) else {
                    return Err(format!(
                        "`{usage}`: argument type `{kind}` uses a kind this build cannot read"
                    ));
                };
                let greedy = properties["type"] == "greedy";
                (parser, properties, greedy, Some(kind))
            }
        };
        optional_seen |= optional;
        greedy_seen |= greedy;
        let mut node = json!({"type": "argument", "parser": parser});
        if !properties.is_null() {
            node["properties"] = properties;
        }
        if matches!(kind, "vec3_uncentered" | "vec2_uncentered") {
            node[UNCENTERED] = true.into();
        }
        // A registered type goes out as the Brigadier or vanilla parser its kind names, which an
        // unmodded client can read; it and the free `id`, which has no list of its own, ask the
        // server (the mods' `tab_complete`).
        if custom.is_some() || kind == "id" {
            if let Some(id) = custom {
                node["custom_argument"] = id.into();
            }
            node["suggestions"] = "minecraft:ask_server".into();
        }
        out.push((name.to_owned(), node, optional));
    }
    if out.first().is_none_or(|n| n.1["type"] != "literal") {
        return Err(format!("`{usage}`: a command starts with its name"));
    }
    Ok(out)
}

/// The tree a mod's usages meet at their root word.
#[derive(Clone, Copy)]
enum BaseTree<'a> {
    /// The client's own chat tree: a client command whose root it has (`has` answers) is left
    /// out, so it never hides a server command.
    Reserved(&'a dyn Fn(&str) -> bool),
    /// The server's vanilla tree. A usage whose root vanilla registers merges into that root, as
    /// NeoForge's `RegisterCommandsEvent` registers into vanilla's `CommandDispatcher`, whose
    /// `register` adds the literal with Brigadier's `CommandNode.addChild`: vanilla's usages
    /// stay, the mod's children are added beside them, and vanilla's nodes keep their
    /// permissions. Sunrise's own tools (`/modprofile`, `/pregen`) are not merged into.
    Merged(&'a Dispatcher),
}

/// Native mods' commands as a root in the commands report's format
/// (`lan::commands`, which sends each player what their level allows), and
/// the traversal level of each command a mod owns: the lowest of its usages. A node is runnable
/// where only optional arguments follow it; like Brigadier's `CommandNode.addChild`, each
/// later accepted runnable registration overwrites the node's executor and execution level,
/// a vanilla node's executor too. A root merged into vanilla's keeps vanilla's level, so it has
/// none here. A usage that doesn't parse, meets a vanilla node it cannot share or names a root
/// `base` keeps is left out, with why.
#[cfg(test)]
pub(super) fn command_tree(
    commands: &[mod_api::Command],
    vanilla: &Dispatcher,
    custom: &CommandArguments,
) -> (Value, HashMap<String, u8>, Vec<String>) {
    command_tree_for(commands, BaseTree::Merged(vanilla), custom, false, None)
}

/// Build the client's local tree and its executor slots. Server handle kinds are unavailable
/// to client commands. No root is left out: the chat tree takes each one's missing children
/// (`register_command_nodes`).
pub(super) fn client_command_tree(
    commands: &[mod_api::Command],
    custom: &CommandArguments,
    executors: Option<&mut CommandExecutorSlots>,
) -> (Value, HashMap<String, u8>, Vec<String>) {
    command_tree_for(
        commands,
        BaseTree::Reserved(&|_| false),
        custom,
        true,
        executors,
    )
}

/// Whether a usage merges into a vanilla root of `base`, or why it cannot go into the tree. Each
/// word under a merged root that vanilla has at the same place must be a literal on a literal:
/// Brigadier would keep vanilla's argument type for the mod's argument of the same name, and the
/// mod's executor would read a value of another kind.
fn base_root(
    base: BaseTree<'_>,
    usage: &str,
    nodes: &[(String, Value, bool)],
) -> Result<bool, String> {
    let root = nodes[0].0.as_str();
    let vanilla = match base {
        BaseTree::Reserved(has) if has(root) => {
            return Err(format!("`{usage}`: vanilla has /{root}"));
        }
        BaseTree::Reserved(_) => return Ok(false),
        BaseTree::Merged(vanilla) => vanilla,
    };
    let Some(mut at) = vanilla.find(root) else {
        return Ok(false);
    };
    if sunrise_client::command::vanilla::is_session_tool(root) {
        return Err(format!("`{usage}`: /{root} is the game's own command"));
    }
    for (name, node, _) in &nodes[1..] {
        let Some(child) = vanilla.nodes[at]
            .children
            .iter()
            .copied()
            .find(|&child| vanilla.nodes[child].name() == name)
        else {
            break;
        };
        let literal = matches!(
            vanilla.nodes[child].kind,
            sunrise_client::command::NodeKind::Literal(_)
        );
        if !literal || node["type"] != "literal" {
            return Err(format!(
                "`{usage}`: `{name}` is a node of vanilla's /{root} that a mod argument cannot share"
            ));
        }
        at = child;
    }
    Ok(true)
}

// Executable paths (including argument names) mapped to the input registration slot.
pub(super) type CommandExecutorSlots = HashMap<Vec<String>, usize>;

fn command_tree_for(
    commands: &[mod_api::Command],
    base: BaseTree<'_>,
    custom: &CommandArguments,
    client: bool,
    mut executors: Option<&mut CommandExecutorSlots>,
) -> (Value, HashMap<String, u8>, Vec<String>) {
    let mut root: Map<String, Value> = Map::new();
    let mut levels: HashMap<String, u8> = HashMap::new();
    let mut problems = Vec::new();
    for (slot, command) in commands.iter().enumerate() {
        let nodes = match usage_nodes(&command.usage, custom, client) {
            Ok(n) => n,
            Err(e) => {
                problems.push(e);
                continue;
            }
        };
        let merged = match base_root(base, &command.usage, &nodes) {
            Ok(merged) => merged,
            Err(e) => {
                problems.push(e);
                continue;
            }
        };
        // A rejected usage must not open an earlier executable prefix or lower its level.
        let mut children = &root;
        let mut conflict = None;
        for (name, node, _) in &nodes {
            let Some(entry) = children.get(name) else {
                break;
            };
            // `word` and `text` share a parser; their properties tell them apart, and a custom
            // kind shares the word (or greedy) properties with them.
            if entry["type"] != node["type"]
                || entry["parser"] != node["parser"]
                || entry["properties"] != node["properties"]
                || entry["custom_argument"] != node["custom_argument"]
                || entry[UNCENTERED] != node[UNCENTERED]
            {
                conflict = Some(name);
                break;
            }
            children = entry["children"]
                .as_object()
                .expect("working tree children");
        }
        if let Some(name) = conflict {
            problems.push(format!(
                "`{}`: `{name}` is another kind in an earlier usage",
                command.usage
            ));
            continue;
        }
        let execution_level = u64::from(command.level.min(4));
        let mut children = &mut root;
        for (i, (name, node, _)) in nodes.iter().enumerate() {
            let runnable = nodes.get(i + 1).is_none_or(|n| n.2);
            let entry = children.entry(name.clone()).or_insert_with(|| {
                let mut n = node.clone();
                n["executable"] = false.into();
                n["level"] = 4.into();
                n
            });
            // Like Brigadier's `CommandNode.addChild`, each later accepted runnable
            // registration overwrites the node's executor and execution level.
            if runnable {
                entry["executable"] = true.into();
                entry["execution_level"] = execution_level.into();
                if let Some(executors) = executors.as_deref_mut() {
                    let path = nodes[..=i]
                        .iter()
                        .map(|(name, _, _)| name.clone())
                        .collect();
                    executors.insert(path, slot);
                }
            }
            let level = entry["level"]
                .as_u64()
                .unwrap_or(4)
                .min(command.level.into());
            entry["level"] = level.into();
            if !entry["children"].is_object() {
                entry["children"] = Value::Object(Map::new());
            }
            children = entry["children"].as_object_mut().expect("made above");
        }
        if !merged {
            let level = levels.entry(nodes[0].0.clone()).or_insert(4);
            *level = (*level).min(command.level);
        }
    }
    let tree = if root.is_empty() {
        Value::Null
    } else {
        let mut root = Value::Object(root);
        finish(&mut root);
        json!({"type": "root", "children": root})
    };
    (tree, levels, problems)
}

/// Turns the working `level` into the report's `permissions`, and drops
/// empty child lists.
fn finish(children: &mut Value) {
    let Some(map) = children.as_object_mut() else {
        return;
    };
    for node in map.values_mut() {
        let level = node["level"].as_u64().unwrap_or(0) as usize;
        let object = node.as_object_mut().expect("a node");
        object.remove("level");
        if level > 0 {
            object.insert(
                "permissions".into(),
                json!({"type": "require", "permission": {"level": LEVELS[level.min(4)]}}),
            );
        }
        if object
            .get("children")
            .is_some_and(|c| c.as_object().is_some_and(Map::is_empty))
        {
            object.remove("children");
        } else if let Some(c) = object.get_mut("children") {
            finish(c);
        }
    }
}

/// The result of a command handled by the client's dispatcher, never sent to the server.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::game) enum ClientCommandOutcome {
    Failed,
    Success(i32),
}

/// Add a validated native report for descriptor-only consumers.
pub(super) fn register_dispatcher(
    dispatcher: &mut Dispatcher,
    tree: &Value,
    handler: &'static str,
    arguments: &CommandArguments,
) {
    register_command_nodes(dispatcher, tree, handler, arguments, None);
}

/// Add the report's nodes with the same grammar on both sides. When `executors` is provided,
/// associates each runnable node ID with its registration path in a fresh dispatcher generation.
pub(super) fn register_command_nodes(
    dispatcher: &mut Dispatcher,
    tree: &Value,
    handler: &'static str,
    arguments: &CommandArguments,
    executors: Option<&CommandExecutorSlots>,
) {
    fn add(
        dispatcher: &mut Dispatcher,
        parent: usize,
        children: &Map<String, Value>,
        handler: &'static str,
        arguments: &CommandArguments,
        path: &mut Vec<String>,
        on_executable: &mut impl FnMut(&[String], usize),
    ) {
        for (name, node) in children {
            path.push(name.clone());
            let before = dispatcher.nodes.len();
            let id = if node["type"] == "literal" {
                dispatcher.literal(parent, name)
            } else {
                dispatcher.argument(parent, name, argument_type(node, arguments))
            };
            // `CommandNode.addChild` merges into a node the tree already has, which keeps its
            // own requirement, executor and execution level; a node the mod adds takes the
            // usage's level.
            let added = dispatcher.nodes.len() > before;
            if added {
                let level = node["permissions"]["permission"]["level"]
                    .as_str()
                    .and_then(|level| LEVELS.iter().position(|known| *known == level))
                    .unwrap_or(0) as u8;
                dispatcher.requires(id, level);
            }
            if node["executable"] == true {
                dispatcher.executes(id, handler);
                dispatcher.requires_execution(
                    id,
                    node["execution_level"]
                        .as_u64()
                        .expect("runnable usage level") as u8,
                );
                on_executable(path, id);
            }
            if let Some(children) = node["children"].as_object() {
                add(
                    dispatcher,
                    id,
                    children,
                    handler,
                    arguments,
                    path,
                    on_executable,
                );
            }
            path.pop();
        }
    }
    let mut bindings = HashMap::new();
    if let Some(children) = tree["children"].as_object() {
        add(
            dispatcher,
            0,
            children,
            handler,
            arguments,
            &mut Vec::new(),
            &mut |path: &[String], id| {
                if let Some(executors) = executors {
                    bindings.insert(id, executors[path]);
                }
            },
        );
    }
    if executors.is_some() {
        dispatcher.replace_executor_bindings(bindings);
    }
}

/// The live player body, or a guest record outside a player swap.
pub(super) enum PlayerRead<'a> {
    Acting {
        /// Server authoritative mode, or the local player's resolved `PlayerInfo` mode on a client.
        mode: Option<i32>,
        body: Option<&'a PlayerMotion>,
        /// The player's stored abilities, when it is a server player's turn.
        abilities: Option<PlayerAbilities>,
        /// Where the player last died: the acting guest's record, swapped
        /// into the level for their turn.
        last_death: Option<&'a (String, [i32; 3])>,
    },
    Guest(&'a Guest),
}

/// The resident record a server write will borrow, without borrowing `Game` itself.
pub(super) enum PlayerTurn {
    Acting,
    Parked(u32),
}

impl PlayerTurn {
    pub(super) fn run<'a, R>(
        self,
        game: &mut Game<'a>,
        f: impl FnOnce(&mut Game<'a>) -> R,
    ) -> Option<R> {
        match self {
            Self::Acting => Some(f(game)),
            Self::Parked(conn) => game.as_guest(conn, f),
        }
    }
}

impl PlayerRead<'_> {
    fn last_death(&self) -> Option<&(String, [i32; 3])> {
        match self {
            Self::Acting { last_death, .. } => *last_death,
            Self::Guest(guest) => guest.last_death.as_ref(),
        }
    }

    fn mode(&self) -> Option<i32> {
        match self {
            Self::Acting { mode, .. } => *mode,
            Self::Guest(guest) => Some(guest.game_mode),
        }
    }

    fn creative(&self) -> bool {
        self.mode() == Some(1)
    }

    fn may_build(&self) -> bool {
        match self {
            Self::Acting {
                abilities: Some(a), ..
            } => a.may_build,
            Self::Guest(guest) => guest.abilities.may_build,
            Self::Acting { mode, .. } => mode.is_some_and(|m| m < 2),
        }
    }

    fn body(&self) -> Option<&PlayerMotion> {
        match self {
            Self::Acting { body, .. } => *body,
            Self::Guest(guest) => guest.motion.as_deref(),
        }
    }

    fn food(&self) -> Option<PlayerFood> {
        let food = &self.body()?.health.food;
        let (level, saturation) = match self {
            Self::Acting { .. } => (food.food, food.saturation),
            Self::Guest(guest) => (guest.food, guest.saturation),
        };
        Some(PlayerFood::from(mod_api::__private::built::PlayerFood {
            level,
            saturation,
            exhaustion: food.exhaustion,
        }))
    }

    fn abilities(&self) -> Option<mod_api::PlayerAbilities> {
        let a = match self {
            Self::Acting {
                abilities: Some(a), ..
            } => *a,
            Self::Acting { mode, body, .. } => {
                // No server turn: what the mode gives, with the body's flight.
                let body = body.as_ref()?;
                let mut a = PlayerAbilities {
                    flying: body.flying,
                    flying_speed: body.flying_speed,
                    ..Default::default()
                };
                a.update_for_mode(mode.unwrap_or(0));
                a.flying = body.flying;
                a
            }
            Self::Guest(guest) => guest.abilities,
        };
        Some(mod_api::PlayerAbilities::from(
            mod_api::__private::built::PlayerAbilities {
                invulnerable: a.invulnerable,
                flying: a.flying,
                may_fly: a.may_fly,
                instabuild: a.instabuild,
                may_build: a.may_build,
                fly_speed: a.flying_speed,
                walk_speed: a.walking_speed,
            },
        ))
    }
}

/// Helpers for server mods, kept as stable as the hook points.
// Native mods use these; nothing in Sunrise does.
#[allow(dead_code)]
impl Game<'_> {
    /// Create or retrieve a connectionless player for the level currently in place.
    pub(super) fn fake_player(
        &mut self,
        level: LevelRef,
        uuid: u128,
        name: &str,
    ) -> Option<PlayerRef> {
        if !self.level_in_place(level)
            || !self.lan_guest_swaps.is_empty()
            || !sunrise_client::lan::conn::valid_name(name)
        {
            return None;
        }
        let epoch = self.native_lifecycle.epoch()?;
        if self
            .level
            .player_uuid
            .map(sunrise_client::lan::playerdata::ints_uuid)
            == Some(uuid)
        {
            return None;
        }
        let host = self.lan.as_ref()?;
        if host
            .hub
            .guests
            .iter()
            .any(|g| g.uuid == uuid && g.participates_online())
        {
            return None;
        }
        if let Some(g) = host
            .hub
            .guests
            .iter()
            .find(|g| g.uuid == uuid && g.dimension == level.dimension_id())
        {
            return (g.name == name).then_some(mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(epoch, g.entity_id as u32),
                uuid,
            ));
        }
        let id = self.next_entity.checked_add(1)?;
        let entity_id = i32::try_from(id).ok()?;
        let motion = PlayerMotion::new(
            self.motion,
            sunrise_core::geometry::Vec3::new(0.0, 0.0, 0.0),
        )
        .ok()?;
        let host = self.lan.as_mut()?;
        let conn = host.hub.lan.shared.next_id()?;
        let mut guest = Guest::connectionless(conn, uuid, name.into(), entity_id, [0.0; 3], motion);
        guest.dimension = level.dimension_id().to_owned();
        host.hub.guests.push(guest);
        host.play
            .insert(conn, super::lan_actions::GuestPlay::default());
        self.next_entity = id;
        Some(mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(epoch, id),
            uuid,
        ))
    }

    /// Discard state belonging to an already removed connectionless guest.
    pub(super) fn cleanup_fake_player(&mut self, host: &mut super::lan::LanHost, guest: &Guest) {
        host.play.remove(&guest.conn);
        host.forget_menus(guest.conn);
        host.poses.remove(&guest.conn);
        host.mob_players.retain(|_, conn| *conn != guest.conn);
        self.destroying.remove(&guest.entity_id);
        self.guest_combat.remove(&guest.conn);
    }

    /// Remove a fake without treating it as a disconnect or saving it.
    pub(super) fn player_remove_fake(&mut self, player: PlayerRef) -> bool {
        if self.native_lifecycle.epoch() != Some(player.fields().entity.epoch())
            || !self.lan_guest_swaps.is_empty()
        {
            return false;
        }
        let Some(mut host) = self.lan.take() else {
            return false;
        };
        let index = host.hub.guests.iter().position(|g| {
            g.is_fake()
                && g.uuid == player.fields().uuid
                && g.entity_id as u32 == player.fields().entity.id()
                && host
                    .hub
                    .holder_guest(g.uuid, &g.dimension)
                    .is_some_and(|owner| owner.conn == g.conn)
        });
        let removed = if let Some(index) = index {
            let guest = host.hub.guests.remove(index);
            self.cleanup_fake_player(&mut host, &guest);
            true
        } else {
            false
        };
        self.lan = Some(host);
        removed
    }

    /// A fake player acts as a player does, through the game's own path for the action
    /// (`ServerGamePacketListenerImpl.handleUseItemOn` / `handleUseItem` / `handleInteract`'s
    /// ATTACK, and `ServerPlayerGameMode.destroyBlock`), as its packet would run it: reach,
    /// protection and the border, then the action. Once it runs, an effect a fake can't take
    /// part in (a menu, sleep, a ride, another dimension) is stopped where it would start, and
    /// the answer says which; the action's own side effects (the swing, the hooks before it,
    /// vanilla's result) have still happened then. The fake is found once, and its turn
    /// starts at that place in the list.
    pub(super) fn player_perform_action(
        &mut self,
        player: PlayerRef,
        action: PlayerAction,
    ) -> Result<ActionOutcome, ActionRefused> {
        let (gi, conn) = self.fake_action_actor(player, action)?;
        self.beside_listener_journal(|game| game.perform_fake_action(gi, conn, action))
    }

    /// A machine's one use on the persistent actor, with its private state put back even
    /// when the action refuses. Only the existing guest turn runs gameplay and its hooks.
    pub(super) fn player_perform_action_scoped(
        &mut self,
        player: PlayerRef,
        action: PlayerAction,
        setup: PlayerActionSetup,
    ) -> Result<ScopedActionResult, ActionRefused> {
        let (gi, conn) = self.fake_action_actor(player, action)?;
        self.beside_listener_journal(|game| game.perform_scoped(gi, conn, action, setup))
    }

    fn perform_scoped(
        &mut self,
        gi: usize,
        conn: u32,
        action: PlayerAction,
        setup: PlayerActionSetup,
    ) -> Result<ScopedActionResult, ActionRefused> {
        if !setup.eye.iter().all(|v| v.is_finite())
            || !setup.yaw.is_finite()
            || !setup.pitch.is_finite()
        {
            return Err(ActionRefused::InvalidSetup);
        }
        let full_attack_charge = setup.full_attack_charge;
        let mut state = ScopedPlayerState::new(self.motion, self.items(), setup)?;
        let host = self.lan.as_mut().expect("validated fake's host");
        host.hub.guests.update(gi, |guest| state.swap(guest));
        let play = host.play.insert(conn, Default::default());
        let menus = host.menus.insert(conn, Default::default());
        // This body is created with the setup's equipment and never reaches a body tick.
        // Install its modifiers before the first action or calculation of attack charge.
        let initialized = self.as_guest_at(gi, conn, |game| {
            game.put_equipment_modifiers();
            if full_attack_charge {
                game.charge_attack_fully();
            }
        });
        let outcome = if initialized.is_ok() {
            self.perform_fake_action(gi, conn, action)
        } else {
            Err(ActionRefused::Busy)
        };
        let host = self.lan.as_mut().expect("guest turns retain the host");
        let (held, other, position) = host.hub.guests.update(gi, |guest| {
            let held = guest
                .inventory
                .get_mut(guest.selected)
                .and_then(Option::take)
                .filter(|s| s.count > 0);
            let other: Vec<sunrise_items::items::ItemStack> = guest
                .inventory
                .iter_mut()
                .chain([&mut guest.offhand, &mut guest.carried])
                .chain(guest.armor.iter_mut())
                .filter_map(Option::take)
                .filter(|s| s.count > 0)
                .collect();
            let position = guest.position;
            state.swap(guest);
            (held, other, position)
        });
        host.menus.remove(&conn);
        if let Some(menus) = menus {
            host.menus.insert(conn, menus);
        }
        host.play.remove(&conn);
        if let Some(play) = play {
            host.play.insert(conn, play);
        }
        // A stack the API can't represent (a saved item this build doesn't have, #1364) is never
        // destroyed or replaced: it goes back into the world where the actor stood, with
        // vanilla's default pickup delay so a repeating action doesn't take it again next tick.
        // It joins as any drop does, so a join listener may keep it out; only the stacks that
        // joined are counted.
        let mut unrepresentable = 0;
        let mut represent = |stack: sunrise_items::items::ItemStack| match stack.try_into_api() {
            Ok(stack) => Some(stack),
            Err(_) => {
                // ItemEntity.setDefaultPickUpDelay.
                self.push_item_entity(stack, position, None, 10, None);
                if self.join_item_entity(self.next_entity, position) {
                    unrepresentable += 1;
                }
                None
            }
        };
        let held = held.and_then(&mut represent);
        let other = other.into_iter().filter_map(&mut represent).collect();
        outcome.map(|outcome| ScopedActionResult::new(outcome, held, other, unrepresentable))
    }

    fn fake_action_actor(
        &self,
        player: PlayerRef,
        action: PlayerAction,
    ) -> Result<(usize, u32), ActionRefused> {
        if self.native_lifecycle.epoch() != Some(player.fields().entity.epoch()) {
            return Err(ActionRefused::Stale);
        }
        let found = self.lan.as_ref().and_then(|host| {
            let index = host
                .hub
                .guests
                .identity_index(player.fields().uuid, player.fields().entity.id())?;
            let guest = host.hub.guests.get(index)?;
            guest.placed.then_some((index, guest))
        });
        let (gi, conn) = match found {
            Some((gi, g)) if g.is_fake() => (gi, g.conn),
            Some(_) => return Err(ActionRefused::NotFake),
            // Not in the hub's list as itself: the acting player, or gone.
            None => {
                return Err(match self.player_is_fake(player) {
                    Some(false) => ActionRefused::NotFake,
                    Some(true) => ActionRefused::Busy,
                    None => ActionRefused::Stale,
                });
            }
        };
        if !self.lan_guest_swaps.is_empty()
            || !(self.transactions.is_empty() || self.in_level_tick_boundary_alone())
            || self.fake_action.acting()
        {
            return Err(ActionRefused::Busy);
        }
        if let PlayerAction::InteractOn { location, .. } = action
            && !location.iter().all(|v| v.is_finite())
        {
            return Err(ActionRefused::InvalidSetup);
        }
        if let PlayerAction::Attack { target } | PlayerAction::InteractOn { target, .. } = action
            && (self.native_lifecycle.epoch() != Some(target.epoch())
                || target.id() == player.fields().entity.id())
        {
            return Err(ActionRefused::InvalidTarget);
        }
        Ok((gi, conn))
    }

    /// Whether the only open transaction is the one a server level tick's dispatch opens around
    /// each listener (#5750): a machine's fake player acts from its level tick.
    pub(super) fn in_level_tick_boundary_alone(&self) -> bool {
        LevelTickListeners::marked() && self.transactions.len() == 1
    }

    /// Runs a fake player's action outside the level tick listener's own transaction, as it ran
    /// before that listener boundary existed: the action is no part of the listener's journal,
    /// so the listener's failure doesn't undo the action itself, and the action's hooks open and
    /// commit their own transactions. A rollback of the listener's own earlier writes still
    /// puts back what they changed, over the action's result at the same place. With no
    /// transaction open this is just `act`.
    pub(super) fn beside_listener_journal<T>(&mut self, act: impl FnOnce(&mut Self) -> T) -> T {
        if self.transactions.is_empty() {
            return act(self);
        }
        let frames = std::mem::take(&mut *self.transactions);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _outside = LevelTickListeners::mark(false);
            act(self)
        }));
        // Anything the action left open nests in the listener's transaction.
        let inner = std::mem::replace(&mut *self.transactions, frames);
        self.transactions.extend(inner);
        result.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    }

    fn perform_fake_action(
        &mut self,
        gi: usize,
        conn: u32,
        action: PlayerAction,
    ) -> Result<ActionOutcome, ActionRefused> {
        let acting = Acting::start(&self.fake_action);
        let failed = std::mem::replace(&mut self.use_failed, false);
        let consumed = (|| match action {
            PlayerAction::UseOn { hand, pos, hit } => {
                let hit = super::Hit {
                    pos,
                    face: hit.face,
                    point: hit.point,
                    inside: hit.inside,
                };
                self.guest_use_item_on(conn, gi, hit, hand, None)
            }
            PlayerAction::UseItem { hand } => self.guest_use_item(conn, gi, hand, None),
            PlayerAction::InteractOn {
                hand,
                target,
                location,
            } => {
                let id = i32::try_from(target.id()).map_err(|_| ActionRefused::InvalidTarget)?;
                self.guest_interact(conn, gi, id, hand, location)
            }
            PlayerAction::Attack { target } => {
                let id = i32::try_from(target.id()).map_err(|_| ActionRefused::InvalidTarget)?;
                self.guest_attack(conn, gi, id)
            }
            PlayerAction::BreakBlock { pos } => self.guest_break_block(conn, gi, pos),
        })();
        // FAIL ends the click, but is not a consuming InteractionResult.
        let consumed = consumed.map(|used| used && !self.use_failed);
        self.use_failed = failed;
        let consumed = consumed?;
        Ok(match acting.blocked() {
            Some(effect) => ActionOutcome::EffectBlocked(effect),
            None if consumed => ActionOutcome::Consumed,
            None => ActionOutcome::Passed,
        })
    }

    /// Whether `effect` must not start because the player acting is a fake one: true stops it,
    /// and, inside `perform_action`, records it for the action's answer. The one check every
    /// effect a fake can't take part in makes where it starts.
    pub(super) fn fake_blocks(&self, effect: FakeEffect) -> bool {
        if !self.lan_acting_fake {
            return false;
        }
        self.fake_action.block(effect);
        true
    }

    /// Drop the fake players belonging to a level as it closes.
    pub(super) fn remove_level_fake_players(&mut self, dimension: &str) {
        let Some(mut host) = self.lan.take() else {
            return;
        };
        let mut i = 0;
        while i < host.hub.guests.len() {
            let g = &host.hub.guests[i];
            if g.is_fake() && g.dimension == dimension {
                let guest = host.hub.guests.remove(i);
                self.cleanup_fake_player(&mut host, &guest);
            } else {
                i += 1;
            }
        }
        self.lan = Some(host);
    }

    /// Whether a live player handle names a connectionless actor.
    pub(super) fn player_is_fake(&self, player: PlayerRef) -> Option<bool> {
        match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => Some(self.lan_acting_fake),
            PlayerRead::Guest(guest) => Some(guest.is_fake()),
        }
    }

    /// Resolve a real online player by UUID, never a fake. A fake is reached
    /// through the handle returned by its factory. During a nested player swap,
    /// outer players whose records are displaced return None.
    pub(in crate::game) fn player(&self, uuid: u128) -> Option<PlayerRef> {
        let epoch = self.native_lifecycle.epoch()?;
        if let Some(remote) = self
            .remote
            .as_ref()
            .filter(|r| r.logged_in && r.profile == uuid)
        {
            return Some(mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(epoch, u32::try_from(remote.entity_id).ok()?),
                uuid,
            ));
        }
        let (_, id) = self.player_lookup(uuid)?;
        Some(mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(epoch, id),
            uuid,
        ))
    }

    /// The chunks players started and stopped watching that the hub
    /// recorded (`lan::chunk_watch`), told to the mods in the hub's order,
    /// with no player's state swapped in, and a sent chunk's passages sent
    /// after it. The player handle is built from the record, so an unwatch
    /// after a disconnect still names them. Nothing while the log is empty.
    pub(super) fn run_chunk_watches(&mut self) {
        use sunrise_client::lan::chunk_watch::ChunkWatchKind;
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        if host.hub.chunk_watches.is_empty() {
            return;
        }
        let runs = host.hub.chunk_watches.take();
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return;
        };
        for run in runs {
            // A level the server no longer holds: nothing to name.
            let Some(level) = self.level(&run.dimension) else {
                continue;
            };
            for watch in run.watches {
                let Ok(id) = u32::try_from(watch.entity_id) else {
                    continue;
                };
                let player = mod_api::__private::game::player_ref(
                    mod_api::EntityRef::new(epoch, id),
                    watch.uuid,
                );
                let chunk = mod_api::ChunkRef::new(level, watch.chunk);
                match watch.kind {
                    ChunkWatchKind::Watch => self.native_chunk_watch(player, chunk),
                    ChunkWatchKind::Sent => {
                        self.native_chunk_sent(player, chunk);
                        self.send_chunk_passages(level, watch.uuid, watch.chunk);
                    }
                    ChunkWatchKind::Unwatch => self.native_chunk_unwatch(player, chunk),
                }
            }
        }
    }

    /// The level changes the level in place's tickets made since the last
    /// call (`ChunkTickets::take_changes`), told to the mods in order as
    /// `chunk_ticket_level_updated`, with no player's state swapped in. The
    /// tickets keep them only while a mod listens (`update_chunk_tickets`).
    pub(in crate::game) fn run_chunk_ticket_level_updates(&mut self) {
        use sunrise_client::ticket_levels::UNLOADED;
        let changes = self.tickets.take_changes();
        if changes.is_empty() {
            return;
        }
        let Some(level) = self.level(self.dim.id()) else {
            return;
        };
        let told = |level| match level {
            UNLOADED => mod_api::ChunkTicketLevel::Unloaded,
            level => mod_api::ChunkTicketLevel::Loaded(level),
        };
        for change in changes {
            let chunk = mod_api::ChunkRef::new(level, [change.chunk.0, change.chunk.1]);
            self.native_chunk_ticket_level_updated(chunk, told(change.old), told(change.new));
        }
    }

    fn player_lookup(&self, uuid: u128) -> Option<(PlayerRead<'_>, u32)> {
        if self.lan_acting_uuid == Some(uuid) && !self.lan_acting_fake {
            return Some((
                PlayerRead::Acting {
                    mode: Some(self.mode_id()),
                    body: self.player.as_ref(),
                    abilities: self.acting_abilities,
                    last_death: self.level.last_death.as_ref(),
                },
                self.lan_acting_entity?,
            ));
        }
        if let Some(guest) = self.cloning_guest(uuid).or_else(|| {
            self.lan.as_ref().and_then(|host| {
                host.hub.guests.simulation_iter().find(|guest| {
                    host.hub.online(guest)
                        && guest.uuid == uuid
                        && !self.lan_guest_swaps.contains(&guest.conn)
                })
            })
        }) {
            return Some((PlayerRead::Guest(guest), guest.entity_id as u32));
        }
        if self.lan_acting_uuid.is_none()
            && self.serving.is_some()
            && self
                .level
                .player_uuid
                .map(sunrise_client::lan::playerdata::ints_uuid)
                == Some(uuid)
        {
            let body = self.player.as_ref()?;
            return Some((
                PlayerRead::Acting {
                    mode: Some(self.mode_id()),
                    body: Some(body),
                    abilities: self.acting_abilities,
                    last_death: self.level.last_death.as_ref(),
                },
                body.entity_id as u32,
            ));
        }
        None
    }

    /// The dimension id and block of the player's last death, through the
    /// shared player resolver. None for a stale or unavailable player.
    pub(super) fn native_player_last_death_record(
        &self,
        player: PlayerRef,
    ) -> Option<(String, [i32; 3])> {
        if !self.is_logical_server() {
            // A client's copy: what the login and respawn packets carried.
            return (self.local_player()? == player)
                .then(|| self.level.last_death.clone())
                .flatten();
        }
        self.player_handle_read(player)?.last_death().cloned()
    }

    /// `player_handle_read` for the reads that work on both sides. On a
    /// client the local player (`Minecraft.player`) reads its own copy of
    /// what the server sent (`LocalPlayer`): the entity's resolved `PlayerInfo`
    /// game mode (`AbstractClientPlayer.gameMode`, None before `ADD_PLAYER`),
    /// food, live `Abilities`, hotbar slot and experience. Any other player
    /// reads None there: the client is not sent their state.
    pub(super) fn player_read_both(&self, player: PlayerRef) -> Option<PlayerRead<'_>> {
        if self.is_logical_server() {
            return self.player_handle_read(player);
        }
        (self.local_player()? == player).then(|| PlayerRead::Acting {
            mode: self
                .player_info_game_mode()
                .map(sunrise_client::command::args::GameMode::id),
            body: self.player.as_ref(),
            abilities: self.client_abilities(),
            last_death: self.level.last_death.as_ref(),
        })
    }
    pub(super) fn player_supporting_block(
        &self,
        player: PlayerRef,
    ) -> Result<Option<mod_api::BlockPos>, mod_api::LevelReadError> {
        Ok(self
            .player_read_both(player)
            .and_then(|read| read.body()?.ground.supporting_block))
    }

    pub(super) fn player_on_block(
        &self,
        player: PlayerRef,
    ) -> Result<Option<mod_api::BlockPos>, mod_api::LevelReadError> {
        Ok(self.player_read_both(player).and_then(|read| {
            let body = read.body();
            let position = match read {
                PlayerRead::Acting { .. } => body?.position,
                PlayerRead::Guest(guest) => {
                    let [x, y, z] = guest.position;
                    sunrise_core::geometry::Vec3::new(x, y, z)
                }
            };
            let [x, y, z] = super::native_entity_motion::on_block_position(
                body.and_then(|motion| motion.ground.supporting_block),
                position,
            );
            Some(mod_api::BlockPos::new(x, y, z))
        }))
    }

    /// Reads live `Remote::abilities` with predicted `flying` and `flying_speed` from the body.
    fn client_abilities(&self) -> Option<PlayerAbilities> {
        let mut a = *self.remote_abilities()?;
        if let Some(body) = self.player.as_ref() {
            a.flying = body.flying;
            a.flying_speed = body.flying_speed;
        }
        Some(a)
    }

    pub(super) fn player_handle_read(&self, player: PlayerRef) -> Option<PlayerRead<'_>> {
        (self.native_lifecycle.epoch()? == player.fields().entity.epoch()).then_some(())?;
        if self.lan_acting_fake
            && self.lan_acting_uuid == Some(player.fields().uuid)
            && self.lan_acting_entity == Some(player.fields().entity.id())
        {
            return Some(PlayerRead::Acting {
                mode: Some(self.mode_id()),
                body: self.player.as_ref(),
                abilities: self.acting_abilities,
                last_death: self.level.last_death.as_ref(),
            });
        }
        if let Some((read, id)) = self.player_lookup(player.fields().uuid)
            && id == player.fields().entity.id()
        {
            return Some(read);
        }
        let guests = &self.lan.as_ref()?.hub.guests;
        let guest = guests
            .get(guests.identity_index(player.fields().uuid, player.fields().entity.id())?)?;
        if !guest.placed || !guest.is_fake() || self.lan_guest_swaps.contains(&guest.conn) {
            return None;
        }
        Some(PlayerRead::Guest(guest))
    }

    pub(super) fn player_collision_flags(
        &self,
        player: PlayerRef,
    ) -> Result<Option<mod_api::CollisionFlags>, mod_api::LevelReadError> {
        Ok(self.player_read_both(player).and_then(|read| {
            // A connected player's position reports do not retain collision
            // flags, even when its body is swapped in for a server guest turn.
            let PlayerRead::Acting {
                body: Some(body), ..
            } = read
            else {
                return None;
            };
            if self.lan_acting.is_some() {
                return None;
            }
            let flags = body.collision;
            Some(mod_api::CollisionFlags::new(
                flags.horizontal,
                flags.minor,
                flags.vertical,
                flags.below,
            ))
        }))
    }

    /// `ServerPlayerGameMode.getGameModeForPlayer` on a server; `AbstractClientPlayer.gameMode`
    /// (resolved `PlayerInfo`) on a client. None for stale, offline or unavailable players
    /// (including displaced outer players), and on a client before `ADD_PLAYER` resolves the
    /// entity's `PlayerInfo`. Distinct from `player_abilities`, which reads `LocalPlayer.abilities`.
    pub(super) fn player_game_mode(&self, player: PlayerRef) -> Option<GameMode> {
        self.player_read_both(player)?
            .mode()
            .and_then(GameMode::from_id)
    }

    /// Changes a player's game mode as `ServerPlayer.setGameMode` and
    /// `/gamemode` do, including the cancellable change hook and ability
    /// updates. Returns false for a stale or unavailable player, an active
    /// outer guest, no change, or a canceled change.
    pub(super) fn player_set_game_mode(&mut self, player: PlayerRef, mode: GameMode) -> bool {
        if self.player_handle_read(player).is_none() {
            return false;
        }
        let mut changed = false;
        self.native_as_player_handle(player, |game| {
            changed = game.set_game_mode(mode);
        }) && changed
    }

    /// Stores a server player's `Abilities` and tells their client
    /// (`ServerPlayer.onUpdateAbilities`). They stay as written until the
    /// player's game mode changes (`GameType.updatePlayerAbilities`), and are
    /// saved with the player. As in the `Abilities` setters, both speed values are
    /// stored as supplied. Returns false for a stale, unavailable or displaced outer player.
    pub(super) fn player_set_abilities(
        &mut self,
        player: PlayerRef,
        abilities: mod_api::PlayerAbilities,
    ) -> bool {
        let stored = PlayerAbilities {
            invulnerable: abilities.invulnerable,
            flying: abilities.flying,
            may_fly: abilities.may_fly,
            instabuild: abilities.instabuild,
            may_build: abilities.may_build,
            flying_speed: abilities.fly_speed,
            walking_speed: abilities.walk_speed,
        };
        if self.player_handle_read(player).is_none() {
            return false;
        }
        let mut stored_here = false;
        self.native_as_player_handle(player, |game| {
            let Some(slot) = game.acting_abilities.as_mut() else {
                return;
            };
            *slot = stored;
            if let Some(body) = game.player.as_mut() {
                body.invulnerable = stored.invulnerable;
                body.may_fly = stored.may_fly;
                body.flying = stored.flying;
                body.flying_speed = stored.flying_speed;
            }
            stored_here = true;
        }) && stored_here
    }

    /// `Player.isCreative`; Paper `HumanEntity#getGameMode`.
    pub(super) fn player_is_creative(&self, player: PlayerRef) -> Option<bool> {
        Some(self.player_read_both(player)?.creative())
    }

    /// `Inventory.getSelectedSlot`: the hotbar slot the player holds, 0..9.
    pub(super) fn player_selected_slot(&self, player: PlayerRef) -> Option<usize> {
        Some(match self.player_read_both(player)? {
            PlayerRead::Acting { .. } => self.selected,
            PlayerRead::Guest(guest) => guest.selected,
        })
    }

    /// `Inventory.setSelectedSlot` for a server write: `slot` (0 to 8) becomes the hotbar slot the
    /// player holds and a guest's client is told (`SetHeldSlot`). A new slot ends a main-hand use,
    /// as `handleSetCarriedItem` does, without `player_switch_hotbar_slot_pre`, which is for the
    /// player's own input. False for a slot past 8 or an unavailable player.
    pub(super) fn player_set_selected_slot(&mut self, player: PlayerRef, slot: usize) -> bool {
        if slot >= 9 {
            return false;
        }
        // The guest turn stores the selected slot back but sends nothing. Tell the same
        // connection after the turn, whether the player was parked or already acting.
        let mut conn = None;
        if !self.native_as_resident_player(player, |game| {
            conn = game.lan_acting;
            if game.selected != slot
                && game
                    .using
                    .is_some_and(|(held, _)| held != crate::game::shield::OFFHAND_SLOT)
            {
                game.stop_active_item_use();
            }
            game.selected = slot;
        }) {
            return false;
        }
        if let (Some(conn), Some(host)) = (conn, self.lan.as_ref()) {
            host.hub.send_to(
                conn,
                &C::SetHeldSlot(play::SetHeldSlot { slot: slot as i32 }),
            );
        }
        true
    }

    /// `Player.drop(stack, false, thrown_by_player)`: `stack` thrown from the player's eyes along
    /// their view as an item entity with the drop key's pickup delay, the main hand swinging and
    /// `item_toss` asked first. Nothing is taken from an inventory: the stack is the caller's
    /// (take it out through the item places first). `thrown_by_player` names the player as the
    /// thrower, as the drop key does. None once the item is in the world; the stack back,
    /// untouched, for an empty stack, a stale or dead player, a player in another level than the
    /// one in place and a toss a mod canceled, so a refused drop never loses items.
    pub(super) fn player_drop_stack(
        &mut self,
        player: PlayerRef,
        stack: ItemStack,
        thrown_by_player: bool,
    ) -> Option<ItemStack> {
        if self.entity_ids_exhausted() {
            return Some(stack);
        }
        if stack.count == 0 {
            return Some(stack);
        }
        // `throw_from_eyes` discards a vetoed toss, as `item_toss` always has: keep a copy so
        // the caller gets it back.
        let copy = stack.clone();
        let mut dropped = false;
        let ran = self.native_as_player_handle(player, |game| {
            // A stack thrown from a player in another level would land in this one.
            if game.acting_in_level_in_place() {
                dropped = game.drop_stack_from_eyes(stack, thrown_by_player).is_some();
            }
        });
        (!(ran && dropped)).then_some(copy)
    }

    /// `Player.isSpectator` (`gameMode() == GameType.SPECTATOR` on a client, authoritative
    /// `ServerPlayerGameMode` on a server); Paper `HumanEntity#getGameMode`.
    pub(super) fn player_is_spectator(&self, player: PlayerRef) -> Option<bool> {
        Some(self.player_read_both(player)?.mode() == Some(3))
    }

    /// `Player.mayBuild`; Paper `HumanEntity#getGameMode`.
    pub(super) fn player_may_build(&self, player: PlayerRef) -> Option<bool> {
        Some(self.player_read_both(player)?.may_build())
    }

    /// `FoodData.getFoodLevel/getSaturationLevel/getExhaustionLevel`; Paper
    /// `HumanEntity#getFoodLevel/getSaturation/getExhaustion`. None until the
    /// guest's body exists, as well as for stale or unavailable players.
    pub(super) fn player_food(&self, player: PlayerRef) -> Option<PlayerFood> {
        self.player_read_both(player)?.food()
    }

    /// `Player.getAbilities`; Paper `Player#getAllowFlight/isFlying/getFlySpeed/getWalkSpeed`.
    pub(super) fn player_abilities(&self, player: PlayerRef) -> Option<mod_api::PlayerAbilities> {
        self.player_read_both(player)?.abilities()
    }

    /// The real players on this server, in join order, including an integrated
    /// server's own player even when the world is not open to LAN. Empty on a client.
    pub(super) fn native_server_players(&self) -> Vec<PlayerRef> {
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return Vec::new();
        };
        let Some(host) = &self.lan else {
            return Vec::new();
        };
        // Construct from the online records, not the live-body resolver: outer
        // players remain connected while their bodies are displaced by a turn.
        host.hub
            .online_players()
            .map(|g| {
                mod_api::__private::game::player_ref(
                    mod_api::EntityRef::new(epoch, g.entity_id as u32),
                    g.uuid,
                )
            })
            .collect()
    }

    /// Find a connected player by their account name, case-insensitively,
    /// including an integrated server's own player.
    pub(super) fn native_player_by_name(&self, name: &str) -> Option<PlayerRef> {
        let epoch = self.native_lifecycle.epoch()?;
        let player = self
            .lan
            .as_ref()?
            .hub
            .online_players()
            .find(|player| player.name.eq_ignore_ascii_case(name))?;
        Some(mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(epoch, player.entity_id as u32),
            player.uuid,
        ))
    }

    /// A connected player's record, including a body removed for respawn or displaced by a turn.
    pub(super) fn online_player_record(&self, player: PlayerRef) -> Option<&Guest> {
        self.player_connection_record(player).filter(|g| g.joined())
    }

    /// Connection data is already present while the joining recipient acts, before
    /// `PlayerList.placeNewPlayer` would add the player to its online list.
    fn player_connection_record(&self, player: PlayerRef) -> Option<&Guest> {
        (self.native_lifecycle.epoch()? == player.entity().epoch()).then_some(())?;
        let guests = &self.lan.as_ref()?.hub.guests;
        let guest = guests.get(guests.identity_index(player.uuid(), player.entity().id())?)?;
        (guest.participates_online() && (guest.joined() || self.lan_acting == Some(guest.conn)))
            .then_some(guest)
    }

    pub(super) fn cloning_guest(&self, uuid: u128) -> Option<&Guest> {
        let pending = self
            .player_state
            .cloning_players
            .iter()
            .rev()
            .find(|player| player.uuid == uuid)
            .filter(|player| player.entity.is_some())?;
        self.command_guest(pending.conn)
            .filter(|_| !self.lan_guest_swaps.contains(&pending.conn))
    }

    /// The stable name of a connected or acting player. None for a stale or
    /// unavailable player; displaced online players retain their identity.
    pub(super) fn player_name(&self, player: PlayerRef) -> Option<&str> {
        if self.native_acting_player() == Some(player) {
            return self.lan_acting_name.as_deref();
        }
        Some(&self.online_player_record(player)?.name)
    }

    /// The dimension key of an online player (`ServerPlayer.level().dimension()`):
    /// the dimension the player is placed in.
    pub(super) fn player_dimension_key(&self, player: PlayerRef) -> Option<mod_api::DimensionKey> {
        let epoch = self.native_lifecycle.epoch()?;
        if epoch != player.fields().entity.epoch() {
            return None;
        }
        if !self.is_logical_server() {
            return (self.local_player()? == player).then(|| self.native_level_key());
        }
        if self.native_acting_player() == Some(player) {
            let dim = match self.lan_acting {
                Some(conn) => {
                    let g = self.command_guest(conn)?;
                    Dim::from_id(&g.dimension).key()
                }
                None => self.dim.key(),
            };
            return Some(dim);
        }
        if let Some(guest) = self.online_player_record(player) {
            return Some(Dim::from_id(&guest.dimension).key());
        }
        if self.serving.is_some()
            && self.player(player.uuid()) == Some(player)
            && self
                .level
                .player_uuid
                .map(sunrise_client::lan::playerdata::ints_uuid)
                == Some(player.uuid())
        {
            return Some(self.dim.key());
        }
        None
    }

    /// The level the online player is in (`ServerPlayer.level`): on an integrated
    /// server or dedicated server, the player's current dimension; on a client,
    /// the client's level. None for a stale or unavailable player.
    pub(super) fn player_level(&self, player: PlayerRef) -> Option<LevelRef> {
        let dim = self.player_dimension_key(player)?;
        Some(self.level_ref(player.fields().entity.epoch(), dim))
    }

    /// The dimension the online player is in (`ServerPlayer.level().dimension()`):
    /// `minecraft:overworld`, `minecraft:the_nether`, ... None for a stale
    /// or unavailable player.
    pub(super) fn player_dimension(&self, player: PlayerRef) -> Option<Identifier> {
        let level = self.player_level(player)?;
        Identifier::parse(level.dimension_id()).ok()
    }

    /// The IP address the player connects from, without the port. Displaced connected
    /// players keep their address; an acting player with no known connection
    /// has an empty address. None for a stale or unavailable player.
    pub(super) fn player_address(&self, player: PlayerRef) -> Option<&str> {
        if let Some(guest) = self.player_connection_record(player) {
            return Some(guest.ip_address());
        }
        (self.native_acting_player() == Some(player)).then_some("")
    }

    pub(super) fn player_virtual_host(&self, player: PlayerRef) -> Option<&VirtualHost> {
        if !self.is_logical_server() {
            return None;
        }
        self.online_player_record(player)?.sender.virtual_host()
    }

    /// The brand the player's client announced when it connected (`ServerPlayer.clientBrand`),
    /// empty when it sent none. Displaced connected players keep theirs; an acting player with
    /// no known connection has an empty one. None for a stale or unavailable player.
    pub(super) fn player_client_brand(&self, player: PlayerRef) -> Option<&str> {
        if let Some(guest) = self.player_connection_record(player) {
            return Some(&guest.brand);
        }
        (self.native_acting_player() == Some(player)).then_some("")
    }

    /// A snapshot of the plugin channels the player's client announced (`minecraft:register`
    /// less `minecraft:unregister`), kept by its connection from configuration on.
    pub(super) fn player_announced_channels(&self, player: PlayerRef) -> Option<Arc<[String]>> {
        static EMPTY: LazyLock<Arc<[String]>> = LazyLock::new(Arc::default);
        if let Some(guest) = self.player_connection_record(player) {
            return Some(guest.sender.announced.list());
        }
        (self.native_acting_player() == Some(player)).then(|| Arc::clone(&EMPTY))
    }

    /// Validate identity without requiring a live body: sending to an online
    /// displaced player still works, as it did through the UUID send path.
    fn message_player_uuid(&self, player: PlayerRef) -> Option<u128> {
        (self.native_acting_player() == Some(player) || self.online_player_record(player).is_some())
            .then_some(player.uuid())
    }
    /// A system line in that player's chat, also inside a player's hook for
    /// the acting player; false when they aren't on.
    pub(super) fn player_tell(&mut self, player: PlayerRef, line: Text) -> bool {
        let Some(uuid) = self.message_player_uuid(player) else {
            return false;
        };
        self.native_send(uuid, system_chat(&line))
    }
    /// A line over that player's hotbar (the action bar), also inside a
    /// player's hook for the acting player; false when they aren't on.
    pub(super) fn player_action_bar(&mut self, player: PlayerRef, line: Text) -> bool {
        let Some(uuid) = self.message_player_uuid(player) else {
            return false;
        };
        let packet = C::SystemChat(play::SystemChat {
            content: component_nbt(&line.component()),
            overlay: true,
        });
        self.native_send(uuid, packet)
    }
    /// Sends `packet` to that player, also when they are the acting player
    /// (their packets then go after the hook); false when they aren't on.
    pub(super) fn native_send(&mut self, uuid: u128, packet: C) -> bool {
        if self.lan_acting_uuid == Some(uuid) && !self.lan_acting_fake {
            self.guest_packets.push(packet);
            return true;
        }
        let Some(host) = &self.lan else {
            return false;
        };
        let Some(g) = host.hub.online_players().find(|g| g.uuid == uuid) else {
            return false;
        };
        host.hub.send_to(g.conn, &packet);
        true
    }
    /// Resolve the physical recipient before converting any of the supplied text.
    pub(super) fn player_set_sidebar(
        &mut self,
        player: PlayerRef,
        sidebar: Option<Sidebar>,
    ) -> Result<bool, TextFormError> {
        let Some(conn) = self.online_player_record(player).map(|guest| guest.conn) else {
            return Ok(false);
        };
        let contents = sidebar
            .map(|sidebar| {
                let items = self.res.item_defaults().entries();
                SidebarContents::prepare(sidebar, &|item| items.contains_key(item)).map(Box::new)
            })
            .transpose()?;
        self.after_commit(super::transactions::After::PersonalSidebar(conn, contents));
        Ok(true)
    }
    /// The lines over and under that player's list of players (Tab):
    /// `ClientboundTabListPacket`, an empty text for none. False when they
    /// aren't on.
    pub(super) fn player_tab_list(
        &mut self,
        player: PlayerRef,
        header: &Text,
        footer: &Text,
    ) -> bool {
        let Some(uuid) = self.message_player_uuid(player) else {
            return false;
        };
        let packet = C::TabList(play::TabList {
            header: component_nbt(&header.component()),
            footer: component_nbt(&footer.component()),
        });
        self.native_send(uuid, packet)
    }
    /// The server's tick rate (20 by default) and the
    /// milliseconds its ticks took on average lately, as `/tick query`
    /// reports them. The ticks a second it keeps up is the rate, or
    /// 1000 / milliseconds when that is lower.
    pub(super) fn tick_stats(&self) -> (f32, f64) {
        (
            self.clock.rate(),
            self.clock.average_nanos() as f64 / 1_000_000.0,
        )
    }
    /// `TickRateManager.isFrozen`; None on a client, which follows its server's.
    pub(in crate::game) fn tick_rate_is_frozen(&self) -> Option<bool> {
        self.is_logical_server().then(|| self.clock.frozen())
    }
    /// Whether that player counts for `players_sleeping_percentage`: an
    /// ignored player who isn't in bed is left out of the players the night
    /// needs asleep, as a spectator is (an away-from-keyboard mod). Kept
    /// until a mod sets it back or the server stops, also if they leave and
    /// come back. False when the server list is unavailable, the handle is
    /// stale or the player isn't online.
    pub(super) fn player_set_sleep_ignored(&mut self, player: PlayerRef, ignored: bool) -> bool {
        let Some(uuid) = self.sleep_list_uuid(player) else {
            return false;
        };
        let Some(host) = &mut self.lan else {
            return false;
        };
        if ignored {
            host.sleep_ignored.insert(uuid);
        } else {
            host.sleep_ignored.remove(&uuid);
        }
        true
    }
    /// The UUID of a player the server list holds, whom the sleep count can
    /// leave out: None when the list is unavailable, the handle is stale or
    /// the player isn't online.
    fn sleep_list_uuid(&self, player: PlayerRef) -> Option<u128> {
        let uuid = self.message_player_uuid(player)?;
        self.lan
            .as_ref()?
            .hub
            .online_players()
            .any(|g| g.uuid == uuid)
            .then_some(uuid)
    }
    /// Whether `player_set_sleep_ignored` left the player out of the sleep
    /// count. None as the write's false.
    pub(super) fn player_sleep_ignored(&self, player: PlayerRef) -> Option<bool> {
        let uuid = self.sleep_list_uuid(player)?;
        Some(self.lan.as_ref()?.sleep_ignored.contains(&uuid))
    }
    /// Whether `SleepStatus.update` counts the player: not a spectator, not a
    /// fake, and not an ignored player who is out of bed (`sleep_counts`'s
    /// own rule). None for a stale or unavailable player.
    pub(super) fn player_counts_for_sleep(&self, player: PlayerRef) -> Option<bool> {
        let ignored = |game: &Self| {
            game.lan
                .as_ref()
                .is_some_and(|host| host.sleep_ignored.contains(&player.uuid()))
        };
        Some(match self.player_handle_read(player)? {
            // The player in place: the host's own, or a guest in their turn.
            PlayerRead::Acting { .. } => {
                !self.lan_acting_fake
                    && !self.spectator()
                    && (self.sleeping.is_some() || !ignored(self))
            }
            PlayerRead::Guest(guest) => self.guest_counts_for_sleep(guest),
        })
    }
    /// `Player.getSleepTimer`: the ticks in bed (`sleepCounter`), in the
    /// record of a guest or in place for the acting player. None for a
    /// stale or unavailable player.
    pub(super) fn player_sleep_timer(&self, player: PlayerRef) -> Option<i32> {
        Some(match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => self.sleep_timer,
            PlayerRead::Guest(guest) => guest.sleep_timer,
        })
    }
    /// Writes `Player.sleepCounter` without entering or leaving a bed. `Player.tick` still
    /// handles the sleep and wake thresholds.
    pub(super) fn player_set_sleep_timer(&mut self, player: PlayerRef, ticks: i32) -> bool {
        if !self.is_logical_server() {
            return false;
        }
        match self.player_handle_read(player) {
            Some(PlayerRead::Acting { .. }) => {
                self.sleep_timer = ticks;
                true
            }
            Some(PlayerRead::Guest(guest)) => {
                let conn = guest.conn;
                self.lan
                    .as_mut()
                    .and_then(|host| {
                        host.hub
                            .update_guest(conn, |guest| guest.sleep_timer = ticks)
                    })
                    .is_some()
            }
            None => false,
        }
    }
    /// `ServerPlayer.isTextFilteringEnabled`: what the client's
    /// `ClientInformation` said, false for the player running the server and
    /// for an acting player with no connection. None for a stale or
    /// unavailable player.
    pub(super) fn player_text_filtering(&self, player: PlayerRef) -> Option<bool> {
        if let Some(guest) = self.online_player_record(player) {
            return Some(guest.info.text_filtering);
        }
        (self.native_acting_player() == Some(player)).then_some(false)
    }
    /// `ServerPlayer.getRespawnConfig`: the player's respawn point as the
    /// game keeps it (the guest's record, or the acting player's in place),
    /// `Some(None)` when they have none. None for a stale or unavailable
    /// player, or a dimension key the API can't name.
    pub(super) fn player_respawn_config(
        &self,
        player: PlayerRef,
    ) -> Option<Option<PlayerRespawnConfig>> {
        let saved = match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => self.respawn_config.map(|c| c.saved()),
            PlayerRead::Guest(guest) => guest.respawn_elsewhere.clone(),
        };
        let Some((dimension, [x, y, z], yaw, _pitch, forced)) = saved else {
            return Some(None);
        };
        Some(Some(PlayerRespawnConfig {
            position: GlobalPos::new(Identifier::parse(&dimension).ok()?, BlockPos::new(x, y, z)),
            yaw,
            forced,
        }))
    }
    /// `ServerPlayer.setRespawnPosition`, as `/spawnpoint` sets it (the
    /// same `set_respawn_point`): `config`, or none for None. The removed
    /// original is writable here during the respawn-position callback, even
    /// after death. Otherwise dead players, stale handles, unknown levels,
    /// non-finite yaw and canceled changes are refused.
    pub(super) fn player_set_respawn_config(
        &mut self,
        player: PlayerRef,
        config: Option<PlayerRespawnConfig>,
    ) -> bool {
        let stored = match config {
            Some(config) => {
                let dimension = config.position.dimension().to_string();
                if !config.yaw.is_finite() || !Dim::exists(&dimension) {
                    return false;
                }
                let at = config.position.pos();
                Some(RespawnConfig::wrapped(
                    Dim::from_id(&dimension),
                    [at.x, at.y, at.z],
                    config.yaw,
                    config.forced,
                ))
            }
            None => None,
        };
        // The cloning scope lends the removed original to PlayerRespawnPositionEvent.
        // Allow this write without publishing the player or relaxing other dead-player writes.
        match self.player_handle_read(player) {
            Some(PlayerRead::Acting { .. }) => {
                let mut set = false;
                self.native_as_acting(|game| set = game.set_respawn_point(stored)) && set
            }
            Some(PlayerRead::Guest(guest))
                if !guest.dead
                    || self
                        .cloning_guest(guest.uuid)
                        .is_some_and(|original| original.conn == guest.conn) =>
            {
                let conn = guest.conn;
                self.as_guest(conn, |game| game.set_respawn_point(stored)) == Some(true)
            }
            _ => false,
        }
    }
    /// The ping the Tab list shows (`ServerCommonPacketListenerImpl.latency`, kept by the
    /// connection's keep-alives): 0 for a fake player and for an acting player with no known
    /// connection.
    pub(super) fn player_latency(
        &self,
        player: PlayerRef,
    ) -> Result<i32, mod_api::PlayerWorldRefusal> {
        use mod_api::PlayerWorldRefusal as Refusal;
        if !self.is_logical_server() {
            return Err(Refusal::Client);
        }
        if let Some(guest) = self.online_player_record(player) {
            return Ok(guest.latency.load(Ordering::Relaxed));
        }
        (self.native_acting_player() == Some(player) || self.player_handle_read(player).is_some())
            .then_some(0)
            .ok_or(Refusal::Stale)
    }
    pub(super) fn player_send_ping(
        &mut self,
        player: PlayerRef,
        id: i32,
    ) -> Result<(), mod_api::PlayerPingRefusal> {
        use mod_api::PlayerPingRefusal as Refusal;
        let Some(guest) = self.online_player_record(player) else {
            return Err(
                if self.native_acting_player() == Some(player)
                    || self.player_handle_read(player).is_some()
                {
                    Refusal::NoConnection
                } else {
                    Refusal::Stale
                },
            );
        };
        if !guest.play_connection_open() {
            return Err(Refusal::NoConnection);
        }
        guest
            .sender
            .try_send_play_ping(id)
            .then_some(())
            .ok_or(Refusal::SendFailed)
    }
    /// The permission level `player_hide` set, `None` when shown: kept by UUID, and read as
    /// stored before the world is opened to others (an unpublished single-player world runs a
    /// hub for its owner too), where it applies once it is.
    pub(super) fn player_hidden(
        &self,
        player: PlayerRef,
    ) -> Result<Option<u8>, mod_api::PlayerWorldRefusal> {
        use mod_api::PlayerWorldRefusal as Refusal;
        if !self.is_logical_server() {
            return Err(Refusal::Client);
        }
        let hub = &self.lan.as_ref().ok_or(Refusal::Stale)?.hub;
        if self.native_lifecycle.epoch() != Some(player.entity().epoch()) {
            return Err(Refusal::Stale);
        }
        Ok(hub.hidden.get(&player.uuid()).copied())
    }
    /// The name `player_set_nickname` set, `None` when there is none; the player must be
    /// online. An unpublished single-player world has a server list: `player_set_nickname`
    /// works there, so its nickname reads back.
    pub(super) fn player_nickname(
        &self,
        player: PlayerRef,
    ) -> Result<Option<&str>, mod_api::PlayerWorldRefusal> {
        use mod_api::PlayerWorldRefusal as Refusal;
        if !self.is_logical_server() {
            return Err(Refusal::Client);
        }
        let uuid = self.message_player_uuid(player).ok_or(Refusal::Stale)?;
        let hub = &self.lan.as_ref().ok_or(Refusal::Stale)?.hub;
        if !hub.online_players().any(|g| g.uuid == uuid) {
            return Err(Refusal::Stale);
        }
        Ok(hub.nicknames.get(&uuid).map(String::as_str))
    }
    /// Hides that player (a vanish) from the players below permission
    /// `level`, or shows them again (None): from the next tick those players
    /// lose them from the Tab list and the world (their client stops
    /// tracking the player, as it does a spectator), and don't read their
    /// joining or leaving; players of `level` or more still see them, and
    /// a player made an operator meanwhile sees them again. Kept by UUID, even
    /// after this handle disconnects (the player rejoins hidden), until a mod sets it again
    /// or the server stops. Works inside a player's hook too; false for a stale
    /// world epoch and on a client.
    pub(super) fn player_hide(&mut self, player: PlayerRef, level: Option<u8>) -> bool {
        if self.native_lifecycle.epoch() != Some(player.entity().epoch()) {
            return false;
        }
        let uuid = player.uuid();
        let Some(host) = &mut self.lan else {
            return false;
        };
        match level {
            Some(level) => host.hub.hidden.insert(uuid, level),
            None => host.hub.hidden.remove(&uuid),
        };
        true
    }
    /// Shows `nickname` in place of that player's name in chat (their
    /// messages, `/me`, `/msg`) and in the Tab list; None shows their name
    /// again. Hovering the shown name gives the real one, which stays in the
    /// log, commands, selectors and the server's events. Kept until a mod
    /// sets it again or the server stops (also if they leave and come back).
    /// Plain text: the mod checks it (its length, other players' names). False when the server list is unavailable,
    /// the handle is stale or the player isn't online.
    pub(super) fn player_set_nickname(
        &mut self,
        player: PlayerRef,
        nickname: Option<&str>,
    ) -> bool {
        let Some(uuid) = self.message_player_uuid(player) else {
            return false;
        };
        let Some(host) = &mut self.lan else {
            return false;
        };
        if !host.hub.online_players().any(|g| g.uuid == uuid) {
            return false;
        }
        host.hub.set_nickname(uuid, nickname.map(str::to_owned));
        true
    }
    /// A system line in every player's chat and the server's log.
    pub(super) fn native_broadcast(&self, mut line: Text) {
        let mut cancel = false;
        self.native_broadcast_message(&mut line, &mut cancel);
        if cancel {
            return;
        }
        let plain = line.plain();
        if self.is_dedicated_server() {
            super::dedicated::log("INFO", &plain);
        } else {
            log::info!("{plain}");
        }
        let Some(host) = &self.lan else {
            return;
        };
        let packet = system_chat(&line);
        for g in host.hub.guests.iter().filter(|g| g.placed) {
            host.hub.send_to(g.conn, &packet);
        }
    }
    /// Posts a line to the dedicated server's event stream (`GET /api/v1/events`)
    /// as an `announce` event, which sunrise-server's Discord bridge relays when
    /// `[discord] events` lists `announce`: `name` and `uuid` say who it is
    /// about (empty and 0 for no one), `message` is the line (`§` codes colour it).
    /// Only on a dedicated server; false elsewhere and for an empty message.
    pub(super) fn native_server_event(&self, name: &str, uuid: u128, message: &str) -> bool {
        if !self.is_dedicated_server() || message.trim().is_empty() {
            return false;
        }
        super::dedicated::events::record("announce", name, uuid, json!({ "message": message }));
        true
    }
    /// A command's success feedback: `CommandSourceStack.sendSuccess(line,
    /// broadcast_to_admins)` from the executor's source, the line to its
    /// originating source and, with `broadcast_to_admins`, the other operators
    /// and the log (`send_success`). The executing level is in place: the
    /// entity's name is read there, the origin block or minecart in its own.
    pub(super) fn native_feedback(&mut self, line: Text, broadcast_to_admins: bool) {
        let source = self
            .command_source
            .clone()
            .unwrap_or_else(|| self.player_source());
        self.with_source(source, |game| game.send_success(line, broadcast_to_admins));
    }
    /// `CommandSourceStack.sendFailure` for a mod's executor: the red line to the originating
    /// source, using vanilla's silence and block/cart track-output rules. The command's result
    /// is whatever the executor returns.
    pub(super) fn native_failure(&mut self, line: Text) {
        let source = self
            .command_source
            .clone()
            .unwrap_or_else(|| self.player_source());
        self.with_source(
            super::commands::execute::Source {
                dimension: source.origin_dimension,
                ..source
            },
            |game| game.command_failure(line),
        );
    }
    /// The player whose action or command a hook hears, as their state is
    /// swapped in (command executors, `native_right_click_block`,
    /// `native_use_on`, `native_break_block`...): None outside such a
    /// hook.
    pub(in crate::game) fn native_acting_player(&self) -> Option<PlayerRef> {
        self.player.as_ref()?;
        self.lan_acting_name.as_ref()?;
        self.lan_acting_permission?;
        Some(mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(self.native_lifecycle.epoch()?, self.native_acting_entity()),
            self.lan_acting_uuid?,
        ))
    }
    /// The open world's folder (`world/`, beside its `level.dat`): a server
    /// mod keeps its data in a folder of its own there, so the world's
    /// backups carry it. None when no world is open.
    pub(super) fn world_dir(&self) -> Option<PathBuf> {
        (!self.world_root.as_os_str().is_empty()).then(|| self.world_root.clone())
    }
}

/// Whether an orb at `position` (its box: 0.5 wide and high, standing on
/// it) meets the box `min`..`max`, touching faces excluded (`AABB.intersects`).
fn orb_meets(position: [f64; 3], min: [f64; 3], max: [f64; 3]) -> bool {
    position[0] - 0.25 < max[0]
        && position[0] + 0.25 > min[0]
        && position[1] < max[1]
        && position[1] + 0.5 > min[1]
        && position[2] - 0.25 < max[2]
        && position[2] + 0.25 > min[2]
}

/// The box `min`..`max` with each axis's pair in order, as vanilla's `AABB` takes it;
/// None when a bound is not finite (NaN or infinite).
fn ordered_box(min: [f64; 3], max: [f64; 3]) -> Option<([f64; 3], [f64; 3])> {
    if !min.iter().chain(&max).all(|v| v.is_finite()) {
        return None;
    }
    Some((
        std::array::from_fn(|i| min[i].min(max[i])),
        std::array::from_fn(|i| min[i].max(max[i])),
    ))
}

/// Experience for server mods.
#[allow(dead_code)]
impl Game<'_> {
    /// Resolves `entity` to a live experience orb in the active or any parked server level.
    pub(super) fn native_orb_ref(
        &self,
        entity: EntityRef,
    ) -> Result<&sunrise_client::entities::ExperienceOrb, LevelReadError> {
        if !self.is_logical_server() {
            return Err(LevelReadError::Unavailable);
        }
        if self.native_lifecycle.epoch() != Some(entity.epoch())
            || self.held_out.contains(&entity.id())
        {
            return Err(LevelReadError::Stale);
        }
        let id = entity.id();
        let target = super::commands::Target::Orb(id);
        if let Some(orb) = self.orbs.iter().find(|o| o.id == id) {
            return if self.resident_source_target_visible(self.dim.key(), target) {
                Ok(orb)
            } else {
                Err(LevelReadError::Stale)
            };
        }
        for (dim, parked) in &self.levels.parked {
            if let Some(orb) = parked.orbs.iter().find(|o| o.id == id) {
                return if self.resident_source_target_visible(dim.key(), target) {
                    Ok(orb)
                } else {
                    Err(LevelReadError::Stale)
                };
            }
        }
        Err(LevelReadError::Stale)
    }

    /// Resolves `entity` to a mutable live experience orb in the active or any parked server level.
    fn native_orb_mut(
        &mut self,
        entity: EntityRef,
    ) -> Result<&mut sunrise_client::entities::ExperienceOrb, ExperienceOrbWriteError> {
        if !self.is_logical_server() {
            return Err(ExperienceOrbWriteError::Unavailable);
        }
        if self.native_lifecycle.epoch() != Some(entity.epoch())
            || self.held_out.contains(&entity.id())
        {
            return Err(ExperienceOrbWriteError::Stale);
        }
        let id = entity.id();
        let target = super::commands::Target::Orb(id);
        if self.orbs.iter().any(|o| o.id == id) {
            if !self.resident_source_target_visible(self.dim.key(), target) {
                return Err(ExperienceOrbWriteError::Stale);
            }
            return self
                .orbs
                .iter_mut()
                .find(|o| o.id == id)
                .ok_or(ExperienceOrbWriteError::Stale);
        }
        let Some(dim) = self
            .levels
            .parked
            .iter()
            .find_map(|(&dim, parked)| parked.orbs.iter().any(|o| o.id == id).then_some(dim))
        else {
            return Err(ExperienceOrbWriteError::Stale);
        };
        if !self.resident_source_target_visible(dim.key(), target) {
            return Err(ExperienceOrbWriteError::Stale);
        }
        self.levels
            .parked
            .get_mut(&dim)
            .and_then(|parked| parked.orbs.iter_mut().find(|o| o.id == id))
            .ok_or(ExperienceOrbWriteError::Stale)
    }

    /// Narrows `entity` to an [`ExperienceOrbRef`] across active and parked server levels.
    pub(super) fn experience_orb(
        &self,
        entity: EntityRef,
    ) -> Result<Option<ExperienceOrbRef>, LevelReadError> {
        match self.native_orb_ref(entity) {
            Ok(_) => Ok(Some(ExperienceOrbRef::new(entity))),
            Err(LevelReadError::Stale) => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// Visits experience orbs in `level` (active or parked) whose bounding box meets `min`..`max`
    /// in level order without allocating a list.
    pub(super) fn level_visit_experience_orbs(
        &self,
        level: LevelRef,
        min: [f64; 3],
        max: [f64; 3],
        visit: &mut dyn FnMut(ExperienceOrbRef) -> core::ops::ControlFlow<()>,
    ) -> Result<core::ops::ControlFlow<()>, LevelReadError> {
        self.native_level_world_result(level)?;
        let (min, max) = ordered_box(min, max).ok_or(LevelReadError::Unavailable)?;
        let epoch = level.epoch();
        let dimension = level.dimension();
        let orbs = if dimension == self.dim.key() {
            &self.orbs
        } else {
            &self
                .levels
                .parked_of(level.dimension_id())
                .ok_or(LevelReadError::Stale)?
                .orbs
        };
        for o in orbs.iter() {
            if o.count == 0
                || self.held_out.contains(&o.id)
                || !self
                    .resident_source_target_visible(dimension, super::commands::Target::Orb(o.id))
                || !orb_meets([o.position.x, o.position.y, o.position.z], min, max)
            {
                continue;
            }
            if visit(ExperienceOrbRef::new(EntityRef::new(epoch, o.id))).is_break() {
                return Ok(core::ops::ControlFlow::Break(()));
            }
        }
        Ok(core::ops::ControlFlow::Continue(()))
    }

    /// Reads `ExperienceOrb.getValue()` (`DATA_VALUE`, full `i32`).
    pub(super) fn experience_orb_value(
        &self,
        orb: ExperienceOrbRef,
    ) -> Result<i32, LevelReadError> {
        self.native_orb_ref(orb.entity()).map(|o| o.value)
    }

    /// Reads `ExperienceOrb.count`.
    pub(super) fn experience_orb_count(
        &self,
        orb: ExperienceOrbRef,
    ) -> Result<u32, LevelReadError> {
        self.native_orb_ref(orb.entity()).map(|o| o.count)
    }

    /// Reads `ExperienceOrb.age` (full signed `i32`).
    pub(super) fn experience_orb_age(&self, orb: ExperienceOrbRef) -> Result<i32, LevelReadError> {
        self.native_orb_ref(orb.entity()).map(|o| o.age)
    }

    /// Writes `ExperienceOrb.DATA_VALUE` (full `i32`; save truncates to `(short)`).
    pub(super) fn experience_orb_set_value(
        &mut self,
        orb: ExperienceOrbRef,
        value: i32,
    ) -> Result<(), ExperienceOrbWriteError> {
        self.native_orb_mut(orb.entity())?.value = value;
        Ok(())
    }

    /// Writes `ExperienceOrb.count`, rejecting `0` and values above `i32::MAX` (`ExtraCodecs.POSITIVE_INT`).
    pub(super) fn experience_orb_set_count(
        &mut self,
        orb: ExperienceOrbRef,
        count: u32,
    ) -> Result<(), ExperienceOrbWriteError> {
        let orb = self.native_orb_mut(orb.entity())?;
        if count == 0 || count > crate::orb_limits::MAX_COUNT {
            return Err(ExperienceOrbWriteError::InvalidCount);
        }
        orb.count = count;
        Ok(())
    }

    /// Writes `ExperienceOrb.age` (full signed `i32`; save truncates to `(short)`).
    pub(super) fn experience_orb_set_age(
        &mut self,
        orb: ExperienceOrbRef,
        age: i32,
    ) -> Result<(), ExperienceOrbWriteError> {
        self.native_orb_mut(orb.entity())?.age = age;
        Ok(())
    }
    /// Runs `f` in the turn of a living player, against the level in place.
    pub(super) fn native_as_player_handle(
        &mut self,
        player: PlayerRef,
        f: impl FnOnce(&mut Self),
    ) -> bool {
        self.player_handle_turn(player, true)
            .and_then(|turn| turn.run(self, f))
            .is_some()
    }

    /// Resolve a server setter's receiver. Dead players still have a writable record.
    pub(super) fn native_resident_player_turn(
        &self,
        player: PlayerRef,
    ) -> Result<PlayerTurn, PlayerDataRefusal> {
        if !self.is_logical_server() {
            return Err(PlayerDataRefusal::Client);
        }
        self.player_handle_turn(player, false)
            .ok_or(PlayerDataRefusal::Stale)
    }

    pub(super) fn native_as_resident_player(
        &mut self,
        player: PlayerRef,
        f: impl FnOnce(&mut Self),
    ) -> bool {
        self.native_resident_player_turn(player)
            .ok()
            .and_then(|turn| turn.run(self, f))
            .is_some()
    }

    fn player_handle_turn(&self, player: PlayerRef, require_living: bool) -> Option<PlayerTurn> {
        match self.player_handle_read(player)? {
            PlayerRead::Acting { body, .. } => {
                if require_living {
                    self.native_live_acting_body()?;
                } else {
                    body?;
                }
                Some(PlayerTurn::Acting)
            }
            PlayerRead::Guest(guest) if !require_living || !guest.dead => {
                Some(PlayerTurn::Parked(guest.conn))
            }
            PlayerRead::Guest(_) => None,
        }
    }

    /// Runs `f` with the player `uuid`'s state in place, against the level in place: theirs
    /// already when they are acting (a guest in their turn, or the `Game`'s own player where no
    /// guest holds them), else a guest's through `as_guest`, including within another guest's
    /// turn. What `f` writes to the player's own state holds wherever they are; what would land
    /// in the world checks [`Self::acting_in_level_in_place`]. False when it didn't run: no such
    /// player here, or dead.
    pub(super) fn native_as_player(&mut self, uuid: u128, f: impl FnOnce(&mut Self)) -> bool {
        if self.lan_acting_uuid == Some(uuid) {
            return self.native_as_acting(f);
        }
        // With no guest's turn running, `native_acting_id` is the world's owner: on a server
        // thread that owner is a guest too, whose state is theirs, not the `Game`'s own.
        let conn = self
            .native_player_holder(uuid)
            .filter(|g| !g.dead)
            .map(|g| g.conn);
        match conn {
            Some(conn) => self.as_guest(conn, f).is_some(),
            None if uuid == self.native_acting_id() => self.native_as_acting(f),
            None => false,
        }
    }

    fn native_player_holder(&self, uuid: u128) -> Option<&Guest> {
        self.cloning_guest(uuid)
            .or_else(|| self.lan.as_ref()?.hub.holder_guest(uuid, self.dim.id()))
    }

    /// Resolve a UUID-based item place in the acting state or a parked guest's own record.
    /// Unlike a write, a read never initializes a body or the level owner's UUID.
    pub(super) fn native_item_place_player(&self, uuid: u128) -> Option<PlayerRead<'_>> {
        if self.lan_acting_uuid != Some(uuid) {
            if let Some(guest) = self.native_player_holder(uuid) {
                return (!guest.dead && !self.lan_guest_swaps.contains(&guest.conn))
                    .then_some(PlayerRead::Guest(guest));
            }
            if self.lan_acting_uuid.is_some()
                || self
                    .level
                    .player_uuid
                    .map(sunrise_client::lan::playerdata::ints_uuid)
                    != Some(uuid)
            {
                return None;
            }
        }
        Some(PlayerRead::Acting {
            mode: Some(self.mode_id()),
            body: Some(self.native_live_acting_body()?),
            abilities: self.acting_abilities,
            last_death: self.level.last_death.as_ref(),
        })
    }

    pub(super) fn native_live_acting_body(&self) -> Option<&PlayerMotion> {
        let body = self.player.as_ref()?;
        if body.health.health() <= 0.0 {
            return None;
        }
        Some(body)
    }

    /// `f` for the acting player, whose state is in place: false when they are dead.
    fn native_as_acting(&mut self, f: impl FnOnce(&mut Self)) -> bool {
        if self.native_live_acting_body().is_none() {
            return false;
        }
        f(self);
        true
    }

    /// Whether the acting player is in the level in place: a guest's own level is their hub
    /// record's, also when a hook of another level borrows them into its turn. A raw throw,
    /// equip sound or block break requires that level in place; `/give`'s effects replay in
    /// that level separately.
    pub(in crate::game) fn acting_in_level_in_place(&self) -> bool {
        let Some(conn) = self.lan_acting else {
            return true;
        };
        self.lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .is_none_or(|g| g.dimension == self.dim.id())
    }
}

pub(super) fn system_chat(line: &Text) -> C {
    C::SystemChat(play::SystemChat {
        content: component_nbt(&line.component()),
        overlay: false,
    })
}

/// Where the game calls the server hook points.
impl Game<'_> {
    /// A mod changed the commands it registers (`native_register_commands`):
    /// at the start of the next server tick every mod is asked again, the
    /// commands players may run are rebuilt, and every player on the server is
    /// sent their tree again, after `native_player_commands_sent`. Nothing
    /// happens where no world is open to players, and nothing for a server
    /// whose mods never call it. API @API:refresh_commands@.
    pub(super) fn native_refresh_commands(&mut self) {
        self.native_commands.refresh = true;
    }

    /// Commands.sendCommands: build this recipient's tree once, let the installed
    /// listeners select exclusions, then encode and send that same tree.
    pub(in crate::game) fn send_player_command_tree(&mut self, conn: u32) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some(i) = host.hub.guests.connection_index(conn) else {
            return;
        };
        let guest = &host.hub.guests[i];
        if !guest.participates_online() {
            return;
        }
        let graph = host.hub.command_graph(guest.permission);
        let mut hidden = Vec::new();
        if mod_api::__private::listened(mod_api::HookId::PlayerCommandsSent)
            && let Some(epoch) = self.native_lifecycle.epoch()
        {
            let player = mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(epoch, guest.entity_id as u32),
                guest.uuid,
            );
            let mut dispatch = |game: &Self| {
                let roots = graph.roots();
                let send = Arc::new(());
                let name_at = |index: usize| roots.clone().nth(index).expect("indexed usable root");
                let mut answers = |answer: mod_api::Answered<mod_api::CommandTreeSelection>| {
                    if let Some(indexes) =
                        mod_api::__private::selected_command_roots(&send, answer.answer)
                    {
                        hidden.extend(indexes);
                    } else {
                        log::warn!(
                            "mod {}: player_commands_sent returned an exclusion from a different command tree",
                            answer.mod_id
                        );
                    }
                };
                game.native_player_commands_sent(mod_api::__private::PlayerCommandsSentCall {
                    player,
                    tree: mod_api::__private::command_tree_filter(roots.len(), &name_at, &send),
                    answers: &mut answers,
                });
            };
            if !guest.placed && self.lan_acting != Some(conn) {
                // The joining body exists but is not in the online player list yet.
                if let Err(error) = self.as_guest_at(i, conn, |game| dispatch(game)) {
                    log::error!(
                        "player_commands_sent: cannot lend the joining player for connection {conn}: {error:?}"
                    );
                }
            } else {
                dispatch(self);
            }
        }
        // A joining turn may have moved hub rows while settling callbacks.
        if let Some(host) = self.lan.as_ref()
            && let Some(i) = host.hub.guests.connection_index(conn)
        {
            let packet = host.hub.encode_command_graph(graph, &hidden);
            host.hub.send_commands(i, packet);
        }
    }

    /// Refresh permissions, then send each placed player their own tree.
    pub(super) fn resend_commands_filtered(&mut self) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        host.hub.refresh_permissions();
        self.send_placed_command_trees(true);
    }

    /// `Commands.sendCommands` sends only display data; `PlayerList.sendPlayerPermissionLevel`
    /// also sends the permission event before that data.
    fn send_placed_command_trees(&mut self, send_permission: bool) {
        let Some(count) = self.lan.as_ref().map(|host| host.hub.guests.len()) else {
            return;
        };
        for i in 0..count {
            let Some(host) = self.lan.as_ref() else {
                return;
            };
            let guest = &host.hub.guests[i];
            if guest.placed && guest.participates_online() {
                let conn = guest.conn;
                if send_permission {
                    host.hub.send_permission_level(i);
                }
                self.send_player_command_tree(conn);
            }
        }
    }

    /// Resolve the source once at the command boundary, retaining its output origin.
    pub(super) fn native_command_origin(
        &mut self,
        permission: u8,
    ) -> super::commands::R<mod_api::CommandSource> {
        use super::commands::err;
        use mod_api::CommandSource;
        let permission = mod_api::PermissionLevel::from_level(i32::from(permission));
        let source = self
            .command_source
            .clone()
            .unwrap_or_else(|| self.player_source());
        let origin = if let Some(id) = self.exec.functions.last() {
            CommandSource::Function {
                id: mod_api::Identifier::parse(id).map_err(|_| err("command.failed", &[]))?,
                permission,
            }
        } else if let Some((x, y, z)) = source.block {
            CommandSource::CommandBlock {
                position: [x, y, z],
                level: source.origin_dimension.key(),
                permission,
            }
        } else if let Some(id) = source.cart {
            let entity = self
                .with_source(
                    super::commands::execute::Source {
                        dimension: source.origin_dimension,
                        ..source
                    },
                    |game| game.entity(id),
                )
                .ok_or_else(|| err("argument.entity.notfound.entity", &[]))?;
            CommandSource::CommandBlockMinecart { entity, permission }
        } else if source.origin_player {
            let player = self
                .native_acting_player()
                .ok_or_else(|| err("argument.entity.notfound.player", &[]))?;
            CommandSource::Player { player, permission }
        } else {
            CommandSource::Console { permission }
        };
        Ok(origin)
    }

    /// The shared command-node executor, also reached by functions and `execute`.
    pub(super) fn run_native_command(&mut self, context: Context) -> super::commands::R<i32> {
        use super::commands::err;
        use mod_api::__private::RetainedFailure;
        // Argument resolution uses node IDs from the parse. Never use them against a
        // replacement tree, even if the new tree has the same command at the same ID.
        let executor = context
            .executor
            .filter(|&key| self.ui.dispatcher.0.owns_executor(key))
            .and_then(|key| {
                self.native_commands
                    .registered
                    .as_ref()?
                    .executors
                    .get(key.slot())
            })
            .filter(|executor| executor.callback.enabled())
            .ok_or_else(|| err("command.unknown.command", &[]))?;
        let callback = executor.callback.clone();
        let call = Arc::clone(&executor.call);
        let command = self.resolve_native_command(context)?;
        let origin = self.native_command_origin(self.ui.registries.permission)?;
        // Isolate journaled writes when the caller already has a transaction. An ordinary
        // command is not implicitly transactional, and writes outside the journal stay put.
        let result = if self.has_open_transaction() {
            self.storage_transaction(|game, _| callback.call(|| call(game, &origin, &command)))
        } else {
            callback.call(|| call(self, &origin, &command))
        };
        match result {
            Ok(Ok(value)) => Ok(value),
            // Failure text can reach a packet or a saved command block. An
            // invalid component still reports an ordinary failed command.
            Ok(Err(message)) => Err(sunrise_client_lib::api_text::to_serialized_engine(&message)
                .map_or_else(
                    |_| err("command.failed", &[]),
                    sunrise_client::command::CommandError::new,
                )),
            Err(RetainedFailure::Disabled) => Err(err("command.unknown.command", &[])),
            Err(RetainedFailure::Panicked) => {
                Err(sunrise_client::command::CommandError::unexpected())
            }
        }
    }

    pub(super) fn native_command_player_target(
        &self,
        target: super::commands::Target,
    ) -> Option<PlayerRef> {
        use super::commands::Target;
        match target {
            Target::Player => self.native_acting_player(),
            Target::Guest(conn) if self.lan_acting == Some(conn) => self.native_acting_player(),
            // The selected guest's own identity, a fake player's too (`execute on origin` of
            // its projectile). `player_handle_read` keeps the checks: a real guest must be
            // online, not displaced (or the one being cloned) and still that entity; a fake
            // must be placed and not displaced.
            Target::Guest(conn) => {
                let guest = self.command_guest(conn)?;
                let player = mod_api::__private::game::player_ref(
                    mod_api::EntityRef::new(
                        self.native_lifecycle.epoch()?,
                        u32::try_from(guest.entity_id).ok()?,
                    ),
                    guest.uuid,
                );
                self.player_handle_read(player).map(|_| player)
            }
            _ => None,
        }
    }

    pub(super) fn native_command_entity_target(
        &mut self,
        target: super::commands::Target,
    ) -> Option<mod_api::EntityRef> {
        if let Some(player) = self.native_command_player_target(target) {
            return Some(player.entity());
        }
        let id = super::native_entities::target_id(target)?;
        self.in_target_level(target, |game| game.entity(id))
            .flatten()
    }

    /// Resolve argument kinds shared by server and client native commands.
    fn resolve_client_safe_argument(
        &self,
        ty: Option<&ArgType>,
        value: Arg,
        coords_position: impl FnOnce(&sunrise_client::command::args::Coordinates) -> [f64; 3],
        rotation: [f32; 2],
    ) -> Result<mod_api::CommandArgumentValue, sunrise_client::command::CommandError> {
        use super::commands::err;
        use mod_api::CommandArgumentValue as Value;
        Ok(match value {
            Arg::Str(text) if matches!(ty, Some(ArgType::Nbt)) => nbt_value(&text)?,
            Arg::Str(value) => Value::String(value),
            Arg::Int(value) => Value::Integer(value),
            Arg::Long(value) => Value::Long(value),
            Arg::Double(value) => Value::Number(value),
            Arg::Bool(value) => Value::Boolean(value),
            Arg::Coords(coordinates) => {
                positional_value(ty, &coordinates, coords_position(&coordinates), rotation)
            }
            Arg::Item(input) => Value::Item(
                self.command_item_stack(&input, 1)?
                    .try_into_api()
                    .expect("command item validation resolves registered items"),
            ),
            // `ResourceOrTagKeyArgument.Result`: `#` names a tag, else a resource.
            Arg::Resource(id) if matches!(ty, Some(ArgType::ResOrTagKey(_))) => {
                let (tag, name) = id
                    .strip_prefix('#')
                    .map_or((false, &*id), |name| (true, name));
                Value::ResourceOrTag {
                    tag,
                    id: mod_api::Identifier::parse(name).map_err(|_| err("command.failed", &[]))?,
                }
            }
            Arg::Resource(id) => {
                Value::Id(mod_api::Identifier::parse(&id).map_err(|_| err("command.failed", &[]))?)
            }
            Arg::Custom { kind, value } => custom_value(&kind, value),
            _ => unreachable!("the native tree contains only registered native argument kinds"),
        })
    }

    fn client_command_position(&self) -> [f64; 3] {
        match &self.player {
            Some(p) => [p.position.x, p.position.y, p.position.z],
            None => self.server_position(),
        }
    }

    fn resolve_client_native_command(
        &self,
        context: Context,
    ) -> Result<mod_api::CommandInvocation, sunrise_client::command::CommandError> {
        use mod_api::CommandArgument;
        let position = self.client_command_position();
        let rotation = [self.yaw, self.pitch];
        let mut arguments = Vec::with_capacity(context.args.len());
        for argument in context.args {
            let ty = parsed_type(
                &self.native_client_commands.dispatcher,
                &context.nodes,
                argument.range,
            );
            let value = self.resolve_client_safe_argument(
                ty,
                argument.value,
                |coordinates| coordinates.position(position, rotation),
                rotation,
            )?;
            arguments.push(CommandArgument::new(argument.name, value));
        }
        Ok(mod_api::CommandInvocation::new(
            context.literals,
            arguments,
            self.native_level_key(),
            position,
            rotation,
        ))
    }

    fn client_command_failure(&mut self, message: Text) {
        self.chat_line(
            Text::empty()
                .append(message)
                .with_color(sunrise_client::text::RED),
        );
    }

    fn client_syntax_failure(
        &mut self,
        error: &sunrise_client::command::CommandError,
        command: &str,
    ) {
        self.client_command_failure(error.message.clone());
        if let Some(context) = Self::command_syntax_context(error, command) {
            self.client_command_failure(context);
        }
    }

    /// `ClientCommandHandler.runCommand`: only Brigadier's unknown command and unknown argument
    /// outcomes leave the line to the server. All other failures stay on this client.
    pub(in crate::game) fn run_client_native_command(
        &mut self,
        line: &str,
    ) -> Option<ClientCommandOutcome> {
        self.run_client_native_command_in_chain(line, None)
    }

    pub(super) fn run_client_native_command_in_chain(
        &mut self,
        line: &str,
        ancestors: Option<Arc<ClientCommandChain>>,
    ) -> Option<ClientCommandOutcome> {
        if self.is_logical_server() || self.lan_acting.is_some() {
            return None;
        }
        let parse = self
            .native_client_commands
            .dispatcher
            .parse(line, &self.ui.registries);
        let context = match parse.executable() {
            Ok((context, _)) => context.clone(),
            Err(error) => {
                if matches!(
                    error.key.as_str(),
                    "command.unknown.command" | "command.unknown.argument"
                ) {
                    return None;
                }
                self.client_syntax_failure(&error, line);
                return Some(ClientCommandOutcome::Failed);
            }
        };
        let Some(executor) = context
            .executor
            .filter(|&key| self.native_client_commands.dispatcher.owns_executor(key))
            .and_then(|key| self.native_client_commands.executors.get(key.slot()))
            .and_then(Option::as_ref)
            .cloned()
        else {
            log::error!("client command has no executor in its dispatcher generation: {line}");
            self.client_command_failure(Text::translate("command.failed", &[]));
            return Some(ClientCommandOutcome::Failed);
        };
        if std::iter::successors(ancestors.as_deref(), |link| link.parent.as_deref())
            .any(|link| link.executor == executor.callback)
        {
            log::warn!("recursive client command refused: {line}");
            self.client_command_failure(Text::translate("command.failed", &[]));
            return Some(ClientCommandOutcome::Failed);
        }
        let invocation = match self.resolve_client_native_command(context) {
            Ok(invocation) => invocation,
            Err(error) => {
                self.client_syntax_failure(&error, line);
                return Some(ClientCommandOutcome::Failed);
            }
        };
        self.native_client_commands.active_chain = Some(Arc::new(ClientCommandChain {
            executor: executor.callback.clone(),
            parent: ancestors,
        }));
        let executed = executor.execute(self, &invocation);
        self.native_client_commands.active_chain = None;
        match executed {
            Ok(Some(result)) => Some(ClientCommandOutcome::Success(result)),
            failure => {
                match failure {
                    Err(why) => log::warn!("client command failed: {why:?}"),
                    _ => log::warn!(
                        "client command failed: mod {} returned no result",
                        executor.callback.owner()
                    ),
                }
                self.client_command_failure(Text::translate("command.failed", &[]));
                Some(ClientCommandOutcome::Failed)
            }
        }
    }

    /// Dispatch one event-loop wave. The next submissions retain this wave's executor
    /// ancestry but cannot run until the event loop returns again.
    pub(in crate::game) fn drain_pending_client_commands(&mut self) {
        if self.native_client_commands.active_chain.is_some()
            || self.native_client_commands.chat_dispatching
        {
            return;
        }
        for message in std::mem::take(&mut self.native_client_commands.pending_chats) {
            if self.remote.is_none() {
                break;
            }
            let _ = self.send_chat(&message);
        }
        let pending = std::mem::take(&mut self.native_client_commands.pending_commands);
        for item in pending {
            if self.remote.is_none() {
                break;
            }
            let _ = self.submit_client_command(&item.command, item.kind, Some(item.chain));
        }
    }

    /// `EntityArgument.getEntity` / `getEntities` / `getPlayer` / `getPlayers`:
    /// convert validated arguments through the shared source-aware selector resolver.
    fn resolve_native_command(
        &mut self,
        context: Context,
    ) -> Result<mod_api::CommandInvocation, sunrise_client::command::CommandError> {
        use super::commands::err;
        use mod_api::{CommandArgument, CommandArgumentValue as Value};
        let mut arguments = Vec::with_capacity(context.args.len());
        for argument in context.args {
            let ty = parsed_type(&self.ui.dispatcher.0, &context.nodes, argument.range);
            let value = match argument.value {
                value if matches!(ty, Some(ArgType::GameProfile)) => {
                    Value::GameProfiles(self.mod_game_profiles(&value)?)
                }
                Arg::Entity(selector) => {
                    // The parsed node, not its name, determines cardinality: usages may
                    // repeat an argument name or use different kinds on separate branches.
                    let (single, players) = context
                        .nodes
                        .iter()
                        .find_map(|(node, range)| {
                            if *range != argument.range {
                                return None;
                            }
                            match &self.ui.dispatcher.0.nodes[*node].kind {
                                sunrise_client::command::NodeKind::Argument {
                                    ty: ArgType::Entity { single, players },
                                    ..
                                } => Some((*single, *players)),
                                _ => None,
                            }
                        })
                        .expect("parsed entity argument has its dispatcher node");
                    let targets = self.resolve_entity_argument(&selector, players)?;
                    if single && targets.len() > 1 {
                        return Err(err(
                            if players {
                                "argument.player.toomany"
                            } else {
                                "argument.entity.toomany"
                            },
                            &[],
                        ));
                    }
                    if players {
                        if single {
                            Value::Player(
                                self.native_command_player_target(targets[0])
                                    .ok_or_else(|| err("argument.entity.notfound.player", &[]))?,
                            )
                        } else {
                            let selected = targets
                                .into_iter()
                                .map(|target| {
                                    self.native_command_player_target(target)
                                        .ok_or_else(|| err("argument.entity.notfound.player", &[]))
                                })
                                .collect::<Result<Vec<_>, _>>()?;
                            Value::Players(selected)
                        }
                    } else if single {
                        Value::Entity(
                            self.native_command_entity_target(targets[0])
                                .ok_or_else(|| err("argument.entity.notfound.entity", &[]))?,
                        )
                    } else {
                        let selected: Vec<_> = targets
                            .into_iter()
                            .filter_map(|target| self.native_command_entity_target(target))
                            .collect();
                        if selected.is_empty() {
                            return Err(err("argument.entity.notfound.entity", &[]));
                        }
                        Value::Entities(selected)
                    }
                }
                Arg::Resource(id) if matches!(ty, Some(ArgType::Dimension)) => {
                    dimension_value(&id, |dimension| self.level(dimension))?
                }
                other => self.resolve_client_safe_argument(
                    ty,
                    other,
                    |coordinates| self.resolve_coords(coordinates),
                    self.source_rotation(),
                )?,
            };
            arguments.push(CommandArgument::new(argument.name, value));
        }
        Ok(self.native_invocation(context.literals, arguments))
    }

    /// A guest's command for the mods, inside their swap (`run_guest_command`).
    /// `None` leaves dispatch to the caller; `Some(false)` is a handled failure.
    pub(super) fn perform_native_command(
        &mut self,
        conn: u32,
        permission: u8,
        line: &str,
        quiet: bool,
    ) -> Option<bool> {
        let host = self.lan.as_ref()?;
        if self.native_commands.level(line)? > permission {
            return None;
        }
        host.hub.guest(conn)?;
        let previous = std::mem::replace(&mut self.ui.registries.permission, permission);
        let ok = if quiet {
            self.run_command_quietly(line)
        } else {
            self.run_command(line)
        };
        self.ui.registries.permission = previous;
        Some(ok)
    }
}

impl Game<'_> {
    /// A console line for the mods: true when a mod registered its command
    /// (`native_register_commands`). The console is level 4, so a registered
    /// usage is always allowed; a command no mod takes answers as vanilla's
    /// unknown command.
    pub(super) fn perform_native_console_command(&mut self, line: &str) -> bool {
        if self.lan.is_none() || self.native_commands.level(line).is_none() {
            return false;
        }
        self.as_console_in_spawn_level(|game| {
            let previous = game
                .command_source
                .replace(super::commands::execute::Source {
                    silent: false,
                    ..game.server_source()
                });
            game.run_command(line);
            game.command_source = previous;
        });
        true
    }
}

/// The temporary state of the same persistent guest, not another player or action engine.
/// Swapped back after the ordinary guest action has restored the server's outer state.
struct ScopedPlayerState {
    motion: Option<Box<PlayerMotion>>,
    position: [f64; 3],
    yaw: f32,
    pitch: f32,
    head_yaw: f32,
    on_ground: bool,
    game_mode: i32,
    previous_game_mode: Option<i32>,
    abilities: PlayerAbilities,
    sneaking: bool,
    sprinting: bool,
    fall_flying: bool,
    inventory: Vec<Option<sunrise_items::items::ItemStack>>,
    selected: usize,
    armor: [Option<sunrise_items::items::ItemStack>; 4],
    offhand: Option<sunrise_items::items::ItemStack>,
    carried: Option<sunrise_items::items::ItemStack>,
    equipment: sunrise_items::equipment::Equipment,
    effects: sunrise_client::effects::PlayerEffects,
    health: f32,
    food: i32,
    saturation: f32,
}
impl ScopedPlayerState {
    fn new(
        resources: &sunrise_client::motion::MotionResources,
        items: &sunrise_items::items::Items,
        setup: PlayerActionSetup,
    ) -> Result<Self, ActionRefused> {
        // Standing eye height even for secondary use, just as a freshly positioned fake.
        let position = [
            setup.eye[0],
            setup.eye[1] - f64::from(1.62_f32),
            setup.eye[2],
        ];
        let mut motion = PlayerMotion::new(
            resources,
            sunrise_core::geometry::Vec3::new(position[0], position[1], position[2]),
        )
        .map_err(|_| ActionRefused::InvalidSetup)?;
        motion.shift_key_down = setup.secondary_use;
        let mut inventory = vec![None; 36];
        inventory[0] = setup
            .stack
            .try_into_engine(items)
            .map_err(ActionRefused::InvalidStack)?;
        Ok(Self {
            motion: Some(Box::new(motion)),
            position,
            yaw: setup.yaw,
            pitch: setup.pitch,
            head_yaw: setup.yaw,
            on_ground: false,
            game_mode: 0,
            previous_game_mode: None,
            abilities: PlayerAbilities::default(),
            sneaking: setup.secondary_use,
            sprinting: false,
            fall_flying: false,
            inventory,
            selected: 0,
            armor: Default::default(),
            offhand: None,
            carried: None,
            equipment: Default::default(),
            effects: Default::default(),
            health: 20.0,
            food: 20,
            saturation: 5.0,
        })
    }

    fn swap(&mut self, guest: &mut Guest) {
        std::mem::swap(&mut self.motion, &mut guest.motion);
        std::mem::swap(&mut self.position, &mut guest.position);
        std::mem::swap(&mut self.yaw, &mut guest.yaw);
        std::mem::swap(&mut self.pitch, &mut guest.pitch);
        std::mem::swap(&mut self.head_yaw, &mut guest.head_yaw);
        std::mem::swap(&mut self.on_ground, &mut guest.on_ground);
        std::mem::swap(&mut self.game_mode, &mut guest.game_mode);
        std::mem::swap(&mut self.previous_game_mode, &mut guest.previous_game_mode);
        std::mem::swap(&mut self.abilities, &mut guest.abilities);
        std::mem::swap(&mut self.sneaking, &mut guest.sneaking);
        std::mem::swap(&mut self.sprinting, &mut guest.sprinting);
        std::mem::swap(&mut self.fall_flying, &mut guest.fall_flying);
        std::mem::swap(&mut self.inventory, &mut guest.inventory);
        std::mem::swap(&mut self.selected, &mut guest.selected);
        std::mem::swap(&mut self.armor, &mut guest.armor);
        std::mem::swap(&mut self.offhand, &mut guest.offhand);
        std::mem::swap(&mut self.carried, &mut guest.carried);
        std::mem::swap(&mut self.equipment, &mut guest.equipment);
        std::mem::swap(&mut self.effects, &mut guest.effects);
        std::mem::swap(&mut self.health, &mut guest.health);
        std::mem::swap(&mut self.food, &mut guest.food);
        std::mem::swap(&mut self.saturation, &mut guest.saturation);
    }
}

thread_local! {
    static LEVEL_TICK_LISTENERS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Marks a server level tick's listeners while they run, with nothing open around the one
/// transaction their dispatch opens for each (#5750): a fake player's action may run inside
/// that one (`player_perform_action`), not inside one the listener opened itself. Dropping it
/// puts back the mark it replaced, also when the listeners unwind.
pub(in crate::game) struct LevelTickListeners(bool);

impl LevelTickListeners {
    /// Sets the mark to `on` until the returned guard drops.
    pub(in crate::game) fn mark(on: bool) -> Self {
        Self(LEVEL_TICK_LISTENERS.with(|mark| mark.replace(on)))
    }
    /// Whether a level tick's listeners are running, marked.
    fn marked() -> bool {
        LEVEL_TICK_LISTENERS.with(std::cell::Cell::get)
    }
}

impl Drop for LevelTickListeners {
    fn drop(&mut self) {
        LEVEL_TICK_LISTENERS.with(|mark| mark.set(self.0));
    }
}

/// A fake player's `perform_action`, or a mod's `PlayerRef::attack` or `MobRef::do_hurt_target`,
/// in progress: set for its length by [`Acting`], whose drop clears it on every exit, an early
/// return or a panic unwinding through a mod's hook, and the effect it stopped, cleared as each
/// action starts.
#[derive(Default)]
pub(super) struct FakeAction {
    acting: AtomicBool,
    /// The action is `PlayerRef::attack`: `Player.attack` alone, without its packet's swing.
    unswung: AtomicBool,
    /// The first effect stopped since the action started.
    blocked: Mutex<Option<FakeEffect>>,
}
impl FakeAction {
    /// Whether a fake player's action, or a mod's attack, is running.
    pub(super) fn acting(&self) -> bool {
        self.acting.load(Ordering::Relaxed)
    }
    /// Whether the action running is a mod's `PlayerRef::attack`, which `swing_arm` leaves
    /// without a swing: vanilla's `Player.attack` has none, the swing is its packet's.
    pub(in crate::game) fn unswung(&self) -> bool {
        self.unswung.load(Ordering::Relaxed)
    }
    fn block(&self, effect: FakeEffect) {
        // The first effect stopped is the one the action answers.
        self.blocked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert(effect);
    }
}
/// Marks a fake player's action, or a mod's attack, as running until it drops.
pub(super) struct Acting(Arc<FakeAction>);
impl Acting {
    pub(super) fn start(action: &Arc<FakeAction>) -> Self {
        *action
            .blocked
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        action.acting.store(true, Ordering::Relaxed);
        Self(Arc::clone(action))
    }
    /// [`Self::start`] for a mod's `PlayerRef::attack`, which runs without a swing.
    pub(super) fn start_unswung(action: &Arc<FakeAction>) -> Self {
        action.unswung.store(true, Ordering::Relaxed);
        Self::start(action)
    }
    /// The effect this action stopped, if any.
    fn blocked(&self) -> Option<FakeEffect> {
        *self
            .0
            .blocked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}
impl Drop for Acting {
    fn drop(&mut self) {
        self.0.acting.store(false, Ordering::Relaxed);
        self.0.unswung.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_client::items::ItemStack;

    /// Box bounds come from mod code: non-finite bounds refuse without calling the visitor,
    /// an inverted box visits what the ordered one does, `Break(())` stops early, and parked
    /// levels remain visitable and readable/writable through `ExperienceOrbRef`.
    #[test]
    #[ignore = "daily"]
    fn experience_orbs_refuse_bounds_that_are_not_finite_and_swap_an_inverted_box() {
        let mut resources = None;
        let mut game = crate::game::game_tests::tests::server_game(&mut resources);
        game.options.dedicated = Some(Vec::new());
        let at = sunrise_core::geometry::Vec3::new(1.0, 70.0, 1.0);
        let mk_orb = |id| sunrise_client::entities::ExperienceOrb {
            size_answer: None,
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [0.0, 0.0],
            join: Default::default(),
            id,
            position: at,
            previous: at,
            velocity: sunrise_core::geometry::Vec3::ZERO,
            value: 3,
            age: 0,
            ground: sunrise_world::ground_support::GroundState::from_ground_flag(true),
            count: 1,
            fire_ticks: 0,
            ticks: 0,
            health: 5,
            air: 300,
            no_gravity: true,
            following: None,
        };
        game.orbs.push(mk_orb(40));
        game.orbs.push(mk_orb(41));
        let overworld = game.level(game.dim.id()).expect("active overworld handle");
        let visit_ids = |game: &Game<'_>, level: LevelRef, min, max| {
            let mut ids = Vec::new();
            let flow = level.visit_experience_orbs(game, min, max, &mut |orb| {
                ids.push(orb.entity().id());
                core::ops::ControlFlow::Continue(())
            })?;
            Ok::<_, LevelReadError>((flow, ids))
        };
        assert_eq!(
            visit_ids(&game, overworld, [0.0, 69.0, 0.0], [2.0, 71.0, 2.0]),
            Ok((core::ops::ControlFlow::Continue(()), vec![40, 41]))
        );
        // Early stop on ControlFlow::Break(()).
        let mut stopped = Vec::new();
        assert_eq!(
            overworld.visit_experience_orbs(
                &game,
                [0.0, 69.0, 0.0],
                [2.0, 71.0, 2.0],
                &mut |orb| {
                    stopped.push(orb.entity().id());
                    core::ops::ControlFlow::Break(())
                }
            ),
            Ok(core::ops::ControlFlow::Break(()))
        );
        assert_eq!(stopped, [40]);

        for (min, max) in [
            ([f64::NAN, 69.0, 0.0], [2.0, 71.0, 2.0]),
            ([0.0, 69.0, 0.0], [2.0, f64::NAN, 2.0]),
            ([f64::NEG_INFINITY, 69.0, 0.0], [2.0, 71.0, 2.0]),
            ([0.0, 69.0, 0.0], [2.0, 71.0, f64::INFINITY]),
            ([f64::NEG_INFINITY; 3], [f64::INFINITY; 3]),
        ] {
            let mut called = false;
            assert_eq!(
                overworld.visit_experience_orbs(&game, min, max, &mut |_| {
                    called = true;
                    core::ops::ControlFlow::Continue(())
                }),
                Err(LevelReadError::Unavailable),
                "{min:?} {max:?}"
            );
            assert!(!called, "refused visit must not invoke callback");
        }
        assert_eq!(
            visit_ids(&game, overworld, [2.0, 69.0, 0.0], [0.0, 71.0, 2.0]),
            Ok((core::ops::ControlFlow::Continue(()), vec![40, 41]))
        );
        assert_eq!(
            visit_ids(&game, overworld, [2.0, 71.0, 2.0], [0.0, 69.0, 0.0]),
            Ok((core::ops::ControlFlow::Continue(()), vec![40, 41]))
        );

        // Park the overworld by switching to the Nether: the overworld handle still visits,
        // narrows, reads and mutates its parked orbs.
        game.enter_command_level(Dim::Nether).unwrap();
        let mut parked_orbs = Vec::new();
        assert_eq!(
            overworld.visit_experience_orbs(
                &game,
                [0.0, 69.0, 0.0],
                [2.0, 71.0, 2.0],
                &mut |orb| {
                    parked_orbs.push(orb);
                    core::ops::ControlFlow::Continue(())
                }
            ),
            Ok(core::ops::ControlFlow::Continue(()))
        );
        assert_eq!(
            parked_orbs
                .iter()
                .map(|orb| orb.entity().id())
                .collect::<Vec<_>>(),
            vec![40, 41]
        );
        let orb = parked_orbs[0];
        assert_eq!(game.experience_orb(orb.entity()), Ok(Some(orb)));
        assert_eq!(orb.entity().position(&game), Some([1.0, 70.0, 1.0]));
        assert_eq!(orb.value(&game), Ok(3));
        assert_eq!(orb.count(&game), Ok(1));
        assert_eq!(orb.age(&game), Ok(0));
        assert_eq!(orb.set_value(&mut game, 50_000), Ok(()));
        assert_eq!(orb.set_count(&mut game, 4), Ok(()));
        assert_eq!(orb.set_age(&mut game, -12), Ok(()));
        assert_eq!(orb.value(&game), Ok(50_000));
        assert_eq!(orb.count(&game), Ok(4));
        assert_eq!(orb.age(&game), Ok(-12));

        // Client side refuses with Unavailable.
        let role = game.options.execution_role;
        game.options.execution_role = super::super::execution_role::ExecutionRole::Client;
        assert_eq!(
            overworld.visit_experience_orbs(&game, [0.0, 69.0, 0.0], [2.0, 71.0, 2.0], &mut |_| {
                core::ops::ControlFlow::Continue(())
            }),
            Err(LevelReadError::Unavailable)
        );
        assert_eq!(
            game.experience_orb(orb.entity()),
            Err(LevelReadError::Unavailable)
        );
        assert_eq!(orb.value(&game), Err(LevelReadError::Unavailable));
        assert_eq!(
            orb.set_value(&mut game, 1),
            Err(ExperienceOrbWriteError::Unavailable)
        );
        game.options.execution_role = role;
    }

    /// A mod's `announce` reaches the dedicated server's event stream and nothing else's.
    /// The integrated server thread keeps the launch configuration (`options.dedicated`) too,
    /// so it cannot be what decides.
    #[test]
    #[ignore = "daily"]
    fn server_events_are_posted_only_on_the_dedicated_role() {
        use crate::game::execution_role::ExecutionRole;
        let mut resources = None;
        let mut game = crate::game::game_tests::tests::server_game(&mut resources);
        game.options.dedicated = Some(Vec::new());
        for (role, posted) in [
            (ExecutionRole::DedicatedServer, true),
            (ExecutionRole::IntegratedServer, false),
            (ExecutionRole::Client, false),
        ] {
            game.options.execution_role = role;
            assert_eq!(
                game.native_server_event("Alex", 7, "hello"),
                posted,
                "{role:?}"
            );
        }
    }

    pub(super) fn with_command_sources(test: impl FnOnce(&mut Game<'_>)) {
        use sunrise_client::{
            chunk::Chunk, chunk::block_entity::BlockEntity, command_block::CommandBlock,
        };
        use sunrise_core::geometry::Vec3;
        super::super::native_guest_turn_tests::with_guests(|game| {
            game.build_registries();
            game.register_native_commands();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let state = game
                .pack
                .assets
                .registry
                .default_state("minecraft:command_block")
                .unwrap();
            game.world.set(1, 64, 1, state).unwrap();
            game.world.block_entities.insert(
                (1, 64, 1),
                BlockEntity::CommandBlock(Box::new(CommandBlock {
                    command: "execute in minecraft:the_nether run origin".into(),
                    condition_met: true,
                    ..Default::default()
                })),
            );
            let mut cart = super::super::minecarts::Minecart::new(
                99,
                super::super::minecarts::CartKind::CommandBlock,
                Vec3::new(1.0, 64.0, 1.0),
            );
            cart.command.command = "origin".into();
            game.carts.push(cart);
            test(game);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn native_command_feedback_respects_track_output_and_silence() {
        use super::super::commands::execute::Source;
        use sunrise_client::chunk::block_entity::BlockEntity;
        with_command_sources(|game| {
            let source = Source {
                block: Some((1, 64, 1)),
                silent: false,
                ..game.server_source()
            };
            for track in [true, false] {
                if let Some(BlockEntity::CommandBlock(block)) =
                    game.world.block_entities.get_mut(&(1, 64, 1))
                {
                    block.track_output = track;
                    block.last_output = None;
                }
                game.with_source(source.clone(), |game| {
                    game.native_feedback(Text::literal("answer"), false)
                });
                let Some(BlockEntity::CommandBlock(block)) =
                    game.world.block_entities.get(&(1, 64, 1))
                else {
                    panic!("block missing")
                };
                assert_eq!(block.last_output.is_some(), track);
            }
            game.chat_capture = Some(Vec::new());
            game.with_source(
                Source {
                    silent: true,
                    block: None,
                    ..source
                },
                |game| game.native_feedback(Text::literal("silent"), false),
            );
            assert!(game.chat_capture.take().unwrap().is_empty());
            assert!(game.command_source.is_none());
            game.as_guest(12, |game| {
                let source = game.server_source();
                game.with_source(source, |game| {
                    assert!(matches!(
                        game.native_command_origin(4),
                        Ok(mod_api::CommandSource::Console { .. })
                    ));
                });
            })
            .unwrap();
            game.exec.functions.push("test:outer".into());
            assert!(game.native_run_command("list"));
            assert_eq!(game.exec.functions, ["test:outer"]);
        });
    }

    type Heard = Vec<(&'static str, u32, u128, &'static str, [i32; 2])>;
    /// Logs each of the watch trio: (hook, player id, UUID, level, chunk).
    struct Watcher(std::sync::Arc<std::sync::Mutex<Heard>>);
    impl Watcher {
        fn log(&self, hook: &'static str, player: PlayerRef, chunk: mod_api::ChunkRef) {
            self.0.lock().unwrap().push((
                hook,
                player.entity().id(),
                player.uuid(),
                chunk.level().dimension_id(),
                chunk.pos(),
            ));
        }
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Watcher {
        fn chunk_watch(&self, _: &mut dyn mod_api::ServerHost, e: &mut mod_api::ChunkWatchEvent) {
            self.log("watch", e.player, e.chunk);
        }
        fn chunk_sent(&self, _: &mut dyn mod_api::ServerHost, e: &mut mod_api::ChunkSentEvent) {
            self.log("sent", e.player, e.chunk);
        }
        fn chunk_unwatch(
            &self,
            _: &mut dyn mod_api::ServerHost,
            e: &mut mod_api::ChunkUnwatchEvent,
        ) {
            self.log("unwatch", e.player, e.chunk);
        }
    }

    /// `ChunkWatchEvent`: the hub's records reach the mods in order, each
    /// with the player (entity id and UUID, even after they left) and the
    /// chunk in its level; a level the server doesn't hold is skipped.
    #[test]
    #[ignore = "daily"]
    fn the_hubs_chunk_watches_reach_the_mods_in_order() {
        use sunrise_client::lan::chunk_watch::{
            ChunkWatch, ChunkWatchKind, ChunkWatchLog, LevelWatches,
        };
        let heard = std::sync::Arc::default();
        let mods = vec![mod_api::ModEntry::new(
            "watch",
            Box::new(Watcher(std::sync::Arc::clone(&heard))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                super::super::native_item_places::tests::open_to_lan(game);
                let here = game.dim.id();
                let watch = |kind, chunk| ChunkWatch {
                    kind,
                    uuid: 127,
                    entity_id: 42,
                    chunk,
                };
                let run = |dimension: &str, watches| LevelWatches {
                    dimension: dimension.into(),
                    watches,
                };
                game.lan.as_mut().unwrap().hub.chunk_watches = ChunkWatchLog::new(vec![
                    run(
                        here,
                        vec![
                            watch(ChunkWatchKind::Watch, [1, 2]),
                            watch(ChunkWatchKind::Sent, [1, 2]),
                        ],
                    ),
                    run(
                        "minecraft:nowhere",
                        vec![watch(ChunkWatchKind::Unwatch, [0, 0])],
                    ),
                    run(here, vec![watch(ChunkWatchKind::Unwatch, [1, 2])]),
                ]);
                game.run_chunk_watches();
                assert!(game.lan.as_ref().unwrap().hub.chunk_watches.is_empty());
                assert_eq!(
                    *heard.lock().unwrap(),
                    [
                        ("watch", 42, 127, here, [1, 2]),
                        ("sent", 42, 127, here, [1, 2]),
                        ("unwatch", 42, 127, here, [1, 2]),
                    ]
                );
            });
        });
    }

    fn player_guest() -> Guest {
        use std::sync::{
            Arc,
            atomic::{AtomicI32, AtomicUsize},
        };
        use sunrise_client::lan::{Joined, PlayerSender};
        use sunrise_protocol::packets::configuration::ClientInformation;
        let (tx, _rx) = std::sync::mpsc::channel();
        Guest::new(
            Joined {
                conn: 1,
                uuid: 123,
                name: "guest".into(),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            1,
            [0.0; 3],
            10,
        )
    }

    #[test]
    fn player_reads_use_guest_mode_and_abilities() {
        let mut guest = player_guest();
        guest.abilities.flying_speed = 0.08;
        let read = PlayerRead::Guest(&guest);
        assert_eq!(read.mode(), Some(0));
        assert_eq!(
            read.food(),
            None,
            "food is unavailable before body creation"
        );
        assert_eq!(
            read.abilities(),
            Some(mod_api::PlayerAbilities::from(
                mod_api::__private::built::PlayerAbilities {
                    invulnerable: false,
                    flying: false,
                    may_fly: false,
                    instabuild: false,
                    may_build: true,
                    fly_speed: 0.08,
                    walk_speed: 0.1,
                }
            ))
        );
        guest.game_mode = 1;
        guest.abilities.update_for_mode(1);
        guest.abilities.flying = true;
        let read = PlayerRead::Guest(&guest);
        assert_eq!(read.mode(), Some(1));
        assert_eq!(
            read.abilities(),
            Some(mod_api::PlayerAbilities::from(
                mod_api::__private::built::PlayerAbilities {
                    invulnerable: true,
                    flying: true,
                    may_fly: true,
                    instabuild: true,
                    may_build: true,
                    fly_speed: 0.08,
                    walk_speed: 0.1,
                }
            ))
        );
        guest.game_mode = 3;
        guest.abilities.update_for_mode(3);
        let abilities = PlayerRead::Guest(&guest).abilities().unwrap();
        assert!(!abilities.may_build && !abilities.instabuild && abilities.may_fly);
    }

    fn tree(commands: &[mod_api::Command]) -> (Value, HashMap<String, u8>, Vec<String>) {
        command_tree(commands, &Dispatcher::new(), &HashMap::new())
    }

    fn stack(item: &str, count: u32) -> Option<ItemStack> {
        Some(ItemStack {
            item: item.into(),
            count,
            damage: 0,
            extra: None,
        })
    }

    #[test]
    fn orbs_are_found_by_their_half_block_box() {
        // A block's cell, as a mod asks for a broken block's drops.
        let (min, max) = ([10.0, 64.0, -3.0], [11.0, 65.0, -2.0]);
        assert!(orb_meets([10.5, 64.2, -2.5], min, max), "inside");
        assert!(
            orb_meets([11.2, 64.0, -2.5], min, max),
            "its box reaches in"
        );
        assert!(
            orb_meets([10.5, 63.6, -2.5], min, max),
            "standing just below"
        );
        assert!(!orb_meets([11.25, 64.0, -2.5], min, max), "a face touching");
        assert!(!orb_meets([10.5, 63.5, -2.5], min, max), "its top touching");
        assert!(!orb_meets([10.5, 65.0, -2.5], min, max), "standing on top");
    }

    #[test]
    fn whole_stacks_are_given_onto_equal_stacks_only() {
        use super::super::inventory_add;
        let mut inv = vec![stack("minecraft:dirt", 64); 36];
        inv[2] = stack("minecraft:iron_sword", 1);
        inv[2].as_mut().unwrap().damage = 5;
        inv[5] = stack("minecraft:arrow", 60);
        inv[7] = None;
        inv[20] = None;
        let mut off = stack("minecraft:arrow", 62);

        // A damaged sword never joins another stack: the first free slot.
        let mut sword = stack("minecraft:iron_sword", 1).unwrap();
        sword.damage = 40;
        assert!(!inventory_add(&mut inv, &mut off, 5, 1, &mut sword));
        assert_eq!(inv[7].as_ref().unwrap().damage, 40, "its damage is kept");
        assert_eq!(inv[2].as_ref().unwrap().damage, 5);

        // Arrows: the selected slot, then the off hand, then a free slot,
        // and what is left over comes back.
        let mut arrows = stack("minecraft:arrow", 80).unwrap();
        assert!(inventory_add(&mut inv, &mut off, 5, 64, &mut arrows));
        assert_eq!(inv[5].as_ref().unwrap().count, 64);
        assert_eq!(off.as_ref().unwrap().count, 64);
        assert_eq!(inv[20].as_ref().unwrap().count, 64);
        assert_eq!(arrows.count, 80 - 4 - 2 - 64);

        // A full inventory with an empty off hand takes nothing.
        let mut off = None;
        let mut apple = stack("minecraft:apple", 3).unwrap();
        assert!(inventory_add(&mut inv, &mut off, 0, 64, &mut apple));
        assert!(off.is_none(), "never into an empty off hand (getFreeSlot)");
        assert_eq!(apple.count, 3);
    }

    /// The game's `Inventory.add` (pickups, `/give`, a bucket's or a bottle's result) takes
    /// its slots from `getSlotWithRemainingSpace` and `getFreeSlot`
    /// (`sunrise_client::menu::{slot_with_remaining_space, free_slot}`): one dirt at a time
    /// lands where they answer and nowhere else, in vanilla's order (the selected slot, the
    /// off hand, the main slots, then the first empty one).
    #[test]
    fn every_added_item_lands_where_the_slot_pickers_answer() {
        use super::super::inventory_add;
        use sunrise_client::menu::{OFFHAND, free_slot, slot_with_remaining_space};
        let mut inv = vec![stack("minecraft:stone", 64); 36];
        for (slot, have) in [
            (0, stack("minecraft:stone", 5)),
            (1, None),
            (2, None),
            (3, stack("minecraft:dirt", 63)),
            (7, stack("minecraft:dirt", 63)),
            (20, None),
        ] {
            inv[slot] = have;
        }
        let mut off = stack("minecraft:dirt", 62);
        let selected = 3;
        let dirt = stack("minecraft:dirt", 1).unwrap();
        let mut landed = Vec::new();
        for _ in 0..6 {
            let answer = slot_with_remaining_space(&inv, off.as_ref(), selected, &dirt, 64)
                .or_else(|| free_slot(&inv))
                .unwrap();
            let (mut inv_after, mut off_after) = (inv.clone(), off.clone());
            let cell = if answer == OFFHAND {
                &mut off_after
            } else {
                &mut inv_after[answer]
            };
            match cell {
                Some(have) => have.count += 1,
                empty => *empty = Some(dirt.clone()),
            }
            let mut one = dirt.clone();
            assert!(!inventory_add(&mut inv, &mut off, selected, 64, &mut one));
            assert_eq!((&inv, &off), (&inv_after, &off_after), "into {answer}");
            landed.push(answer);
        }
        assert_eq!(landed, [3, OFFHAND, OFFHAND, 7, 1, 1]);
    }

    /// A mod's rows that keep hotbar slots 6 to 8 free change the slot the game's
    /// `Inventory.add` takes (replaced on this thread through `rule_replacements::ADD_SLOT`):
    /// a dirt passes the stack in slot 6 with room and the selected slot 7 for slot 0.
    #[test]
    fn a_replaced_slot_choice_reaches_the_games_inventory_add() {
        use super::super::inventory_add;
        use super::super::rule_replacements::{ADD_SLOT, replaced};
        use sunrise_client::menu::OFFHAND;
        fn open(i: usize) -> bool {
            !(6..=8).contains(&i)
        }
        fn locked(
            items: &[Option<ItemStack>],
            offhand: Option<&ItemStack>,
            selected: usize,
            stack: &ItemStack,
            max: u32,
        ) -> Option<usize> {
            let has = |have: Option<&ItemStack>| {
                have.is_some_and(|have| have.same_item_same_components(stack) && have.count < max)
            };
            if open(selected) && has(items[selected].as_ref()) {
                Some(selected)
            } else if has(offhand) {
                Some(OFFHAND)
            } else {
                (0..items.len())
                    .find(|&i| open(i) && has(items[i].as_ref()))
                    .or_else(|| (0..items.len()).find(|&i| open(i) && items[i].is_none()))
            }
        }
        let mut inv = vec![None; 36];
        inv[6] = stack("minecraft:dirt", 10);
        let mut vanilla = inv.clone();
        let mut off = None;
        let mut one = stack("minecraft:dirt", 1).unwrap();
        let left = replaced(&ADD_SLOT, locked, || {
            inventory_add(&mut inv, &mut off, 7, 64, &mut one)
        });
        assert!(!left);
        assert_eq!(
            (&inv[0], &inv[6], &inv[7]),
            (
                &stack("minecraft:dirt", 1),
                &stack("minecraft:dirt", 10),
                &None
            )
        );
        // Without the rows, vanilla's slot: the stack in slot 6 with room.
        let mut one = stack("minecraft:dirt", 1).unwrap();
        assert!(!inventory_add(&mut vanilla, &mut off, 7, 64, &mut one));
        assert_eq!(vanilla[6], stack("minecraft:dirt", 11));
    }

    #[test]
    fn usages_merge_into_one_tree_with_runnable_nodes() {
        let (t, levels, problems) = tree(&[
            mod_api::Command::new("balance [player:player]"),
            mod_api::Command::new("pay <player:player> <amount:integer>"),
            mod_api::Command::op("eco give <player:player> <amount:integer>", 3),
            mod_api::Command::op("eco take <player:player> <amount:integer>", 3),
        ]);
        assert!(problems.is_empty(), "{problems:?}");
        let c = &t["children"];
        assert_eq!(c["balance"]["executable"], true);
        assert_eq!(c["balance"]["children"]["player"]["executable"], true);
        assert_eq!(
            c["balance"]["children"]["player"]["parser"],
            "minecraft:entity"
        );
        assert_eq!(c["pay"]["executable"], false);
        assert_eq!(c["pay"]["children"]["player"]["executable"], false);
        let amount = &c["pay"]["children"]["player"]["children"]["amount"];
        assert_eq!(amount["executable"], true);
        assert_eq!(amount["parser"], "brigadier:integer");
        assert!(amount.get("children").is_none());
        assert_eq!(c["eco"]["permissions"]["permission"]["level"], "admins");
        assert!(c["eco"]["children"]["give"].is_object());
        assert!(c["eco"]["children"]["take"].is_object());
        assert!(c["pay"].get("permissions").is_none());
        assert_eq!(levels["eco"], 3);
        assert_eq!(levels["pay"], 0);
    }

    #[test]
    fn a_usage_with_another_string_kind_is_left_out_in_either_order() {
        for (first, second, line, other) in [
            (
                "foo <value:word>",
                "foo <value:text>",
                "foo hello",
                "foo hello world",
            ),
            (
                "foo <value:text>",
                "foo <value:word>",
                "foo hello world",
                "foo hello",
            ),
        ] {
            let (tree, _, problems) =
                tree(&[mod_api::Command::new(first), mod_api::Command::new(second)]);
            assert_eq!(problems.len(), 1, "{first} then {second}: {problems:?}");
            assert!(problems[0].starts_with(&format!("`{second}`")));
            let mut dispatcher = Dispatcher::new();
            register_dispatcher(
                &mut dispatcher,
                &tree,
                "native_mod_command",
                &HashMap::new(),
            );
            let registries = sunrise_client::command::Registries::default();
            assert!(dispatcher.parse(line, &registries).executable().is_ok());
            // The kept kind decides: `word` stops at the space, `text` takes it all.
            let kept_greedy = first.contains(":text");
            assert_eq!(
                dispatcher.parse(other, &registries).executable().is_ok(),
                kept_greedy,
                "{first} then {second}: {other}"
            );
        }
    }

    const POSITIONED: &str = "kinds <at:vec3> <by:vec3_uncentered> <flat:vec2> \
         <flat_raw:vec2_uncentered> <column:column_pos> <look:rotation> <level:dimension> <tag:nbt>";

    #[test]
    fn position_rotation_dimension_and_nbt_kinds_are_vanillas_parsers() {
        let (tree, _, problems) = tree(&[mod_api::Command::new(POSITIONED)]);
        assert!(problems.is_empty(), "{problems:?}");
        let mut node = &tree["children"]["kinds"];
        for (name, parser) in [
            ("at", "minecraft:vec3"),
            ("by", "minecraft:vec3"),
            ("flat", "minecraft:vec2"),
            ("flat_raw", "minecraft:vec2"),
            ("column", "minecraft:column_pos"),
            ("look", "minecraft:rotation"),
            ("level", "minecraft:dimension"),
            ("tag", "minecraft:nbt_compound_tag"),
        ] {
            node = &node["children"][name];
            assert_eq!(node["parser"], parser, "{name}");
            assert_eq!(
                node["uncentered"] == true,
                matches!(name, "by" | "flat_raw"),
                "{name}"
            );
        }
    }

    #[test]
    fn vec3_and_vec3_uncentered_do_not_share_a_node() {
        let (_, _, problems) = tree(&[
            mod_api::Command::new("go <to:vec3>"),
            mod_api::Command::new("go <to:vec3_uncentered>"),
        ]);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("another kind"), "{problems:?}");
    }

    #[test]
    fn vec2_and_vec2_uncentered_do_not_share_a_node() {
        let (_, _, problems) = tree(&[
            mod_api::Command::new("go <to:vec2>"),
            mod_api::Command::new("go <to:vec2_uncentered>"),
        ]);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("another kind"), "{problems:?}");
    }

    #[test]
    fn positions_and_rotations_resolve_against_the_source() {
        let (tree, _, problems) = tree(&[mod_api::Command::new(POSITIONED)]);
        assert!(problems.is_empty(), "{problems:?}");
        let mut dispatcher = Dispatcher::new();
        register_dispatcher(
            &mut dispatcher,
            &tree,
            "native_mod_command",
            &HashMap::new(),
        );
        let registries = sunrise_client::command::Registries::default();
        let (origin, rotation) = ([10.0, 70.0, -2.0], [45.0, 30.0]);
        let value = |line: &str, name: &str, kind: &str| {
            let parsed = dispatcher.parse(line, &registries);
            let (context, _) = parsed.executable().unwrap();
            let Some(Arg::Coords(coordinates)) = context.get(name) else {
                panic!("{name} is a position")
            };
            let range = context.args.iter().find(|a| a.name == name).unwrap().range;
            let ty = parsed_type(&dispatcher, &context.nodes, range);
            assert_eq!(
                format!("{ty:?}"),
                format!("{:?}", Some(parser(kind).unwrap().2)),
                "{name}: the type comes from the parsed node"
            );
            positional_value(
                ty,
                coordinates,
                coordinates.position(origin, rotation),
                rotation,
            )
        };
        let relative =
            "kinds ~1 ~ ~-1 ~1 ~ ~-1 ~1 ~-2 ~1 ~-2 ~3 ~-4 ~10 ~-5 minecraft:the_nether {}";
        assert_eq!(
            value(relative, "at", "vec3"),
            mod_api::CommandArgumentValue::Vec3([11.0, 70.0, -3.0])
        );
        assert_eq!(
            value(relative, "flat", "vec2"),
            mod_api::CommandArgumentValue::Vec2([11.0, -4.0])
        );
        assert_eq!(
            value(relative, "flat_raw", "vec2_uncentered"),
            mod_api::CommandArgumentValue::Vec2([11.0, -4.0])
        );
        assert_eq!(
            value(relative, "column", "column_pos"),
            mod_api::CommandArgumentValue::ColumnPos([13, -6])
        );
        assert_eq!(
            value(relative, "look", "rotation"),
            mod_api::CommandArgumentValue::Rotation([55.0, 25.0])
        );
        // A whole `x` and `z` is centred by `vec3`/`vec2` and kept by `*_uncentered`.
        let absolute = "kinds 5 64 -7 5 64 -7 5 7 5 7 0 0 0 0 minecraft:overworld {}";
        assert_eq!(
            value(absolute, "at", "vec3"),
            mod_api::CommandArgumentValue::Vec3([5.5, 64.0, -6.5])
        );
        assert_eq!(
            value(absolute, "by", "vec3_uncentered"),
            mod_api::CommandArgumentValue::Vec3([5.0, 64.0, -7.0])
        );
        assert_eq!(
            value(absolute, "flat", "vec2"),
            mod_api::CommandArgumentValue::Vec2([5.5, 7.5])
        );
        assert_eq!(
            value(absolute, "flat_raw", "vec2_uncentered"),
            mod_api::CommandArgumentValue::Vec2([5.0, 7.0])
        );
        // `vec2` refuses local `^` coordinates, matching `Vec2Argument.parse`.
        let local = "kinds 0 0 0 0 0 0 ^1 ^2 0 0 0 0 0 0 minecraft:overworld {}";
        assert!(dispatcher.parse(local, &registries).executable().is_err());
    }

    #[test]
    fn a_dimension_the_world_lacks_is_refused_and_an_nbt_argument_is_a_compound() {
        let refusal = dimension_value("minecraft:nowhere", |_| None).unwrap_err();
        assert_eq!(refusal.key, "argument.dimension.invalid");
        let mut expected = sunrise_core::nbt::Compound::new();
        expected.insert("answer".into(), sunrise_core::nbt::Tag::Byte(42));
        assert_eq!(
            nbt_value("{answer:42b}").unwrap(),
            mod_api::CommandArgumentValue::Nbt(expected)
        );
    }

    #[test]
    fn every_supported_kind_has_a_parser() {
        assert!(
            argument_kinds::KINDS
                .into_iter()
                .all(|kind| parser(kind).is_some())
        );
    }

    /// A test mod's argument type: named machines, one word, completed by prefix.
    struct MachineArgument;

    impl mod_api::CommandArgumentType for MachineArgument {
        fn kind(&self) -> mod_api::CommandArgumentKind {
            mod_api::CommandArgumentKind::Word
        }

        fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
            match input {
                "press" | "furnace" => Ok(input.to_owned()),
                _ => Err(mod_api::CommandArgumentError::new(
                    mod_api::Text::translate(
                        "test.commands.machine.unknown",
                        vec![mod_api::Text::literal(input)],
                    ),
                )),
            }
        }

        fn suggest(&self, request: &mod_api::SuggestionRequest) -> Vec<mod_api::Suggestion> {
            ["press", "furnace"]
                .into_iter()
                .filter(|machine| machine.starts_with(request.prefix.as_str()))
                .map(|machine| request.suggest(machine))
                .collect()
        }
    }

    struct HostileArgument;

    impl mod_api::CommandArgumentType for HostileArgument {
        fn kind(&self) -> mod_api::CommandArgumentKind {
            mod_api::CommandArgumentKind::Word
        }
        fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
            if input == "panic" {
                panic!("parser")
            }
            Ok(input.into())
        }
        fn suggest(&self, request: &mod_api::SuggestionRequest) -> Vec<mod_api::Suggestion> {
            match request.prefix.as_str() {
                "panic" => panic!("completer"),
                "text" => vec![request.suggest("a".repeat(32768))],
                "tip" => vec![request.suggest_with_tooltip("ok", "a".repeat(65536))],
                "unicode" => vec![request.suggest_with_tooltip("ok", "\0".repeat(32768))],
                _ => vec![request.suggest_with_tooltip("a".repeat(32767), "a".repeat(65535))],
            }
        }
    }

    #[test]
    fn hostile_argument_callbacks_are_refused_without_losing_the_dispatcher() {
        let arguments: CommandArguments = HashMap::from([(
            "test:hostile".into(),
            Arc::new(HostileArgument) as Arc<dyn mod_api::CommandArgumentType>,
        )]);
        let (tree, _, problems) = super::command_tree(
            &[mod_api::Command::new("probe <value:test:hostile>")],
            &sunrise_client::command::Dispatcher::new(),
            &arguments,
        );
        assert!(problems.is_empty());
        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &tree, "native_mod_command", &arguments);
        let registries = sunrise_client::command::Registries::default();
        for prefix in ["panic", "text", "tip", "unicode"] {
            let input = format!("probe {prefix}");
            let parsed = dispatcher.parse(&input, &registries);
            assert_eq!(parsed.executable().is_err(), prefix == "panic");
            let (_, suggestions) = dispatcher.suggestions(&parsed, input.len(), &registries);
            assert!(suggestions.is_empty(), "{prefix}");
        }
        let input = "probe valid";
        let parsed = dispatcher.parse(input, &registries);
        let (_, suggestions) = dispatcher.suggestions(&parsed, input.len(), &registries);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].text.len(), 32767);
    }

    #[test]
    fn panicking_argument_kind_is_left_out_of_the_tree() {
        struct PanicKind;
        impl mod_api::CommandArgumentType for PanicKind {
            fn kind(&self) -> mod_api::CommandArgumentKind {
                panic!("kind")
            }
            fn parse(&self, _: &str) -> Result<String, mod_api::CommandArgumentError> {
                unreachable!()
            }
            fn suggest(&self, _: &mod_api::SuggestionRequest) -> Vec<mod_api::Suggestion> {
                unreachable!()
            }
        }
        let arguments: CommandArguments = HashMap::from([(
            "test:panic".into(),
            Arc::new(PanicKind) as Arc<dyn mod_api::CommandArgumentType>,
        )]);
        let (_, _, problems) = super::command_tree(
            &[mod_api::Command::new("probe <value:test:panic>")],
            &sunrise_client::command::Dispatcher::new(),
            &arguments,
        );
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("panicked"));
    }

    struct Machines;

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Machines {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.register_command_argument("machine", MachineArgument);
        }
    }

    #[test]
    fn a_registered_argument_type_parses_and_completes_its_values() {
        mod_platform::__private::part_tests::install_parts_fixture(&[mod_api::ModEntry::new(
            "test",
            Box::new(Machines),
        )])
        .unwrap();
        let arguments = command_arguments();
        assert!(arguments.contains_key("test:machine"));

        let (tree, _, problems) = super::command_tree(
            &[mod_api::Command::new("machine use <machine:test:machine>")],
            &sunrise_client::command::Dispatcher::new(),
            &arguments,
        );
        assert!(problems.is_empty(), "{problems:?}");
        let machine = &tree["children"]["machine"]["children"]["use"]["children"]["machine"];
        assert_eq!(machine["custom_argument"], "test:machine");
        assert_eq!(machine["suggestions"], "minecraft:ask_server");
        assert_eq!(machine["properties"], json!({"type": "word"}));

        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &tree, "native_mod_command", &arguments);
        let registries = sunrise_client::command::Registries::default();
        let parsed = dispatcher.parse("machine use press", &registries);
        let (context, _) = parsed.executable().unwrap();
        assert_eq!(
            context.get("machine"),
            Some(&Arg::Custom {
                kind: "test:machine".into(),
                value: "press".into(),
            })
        );
        assert!(
            dispatcher
                .parse("machine use banana", &registries)
                .executable()
                .is_err(),
            "the mod's parser rejects what it does not know"
        );

        let typed = "machine use pr";
        let parse = dispatcher.parse(typed, &registries);
        let (_, suggestions) = dispatcher.suggestions(&parse, typed.len(), &registries);
        assert_eq!(
            suggestions
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>(),
            ["press"]
        );
    }

    #[test]
    fn the_id_kind_reads_a_free_identifier_with_no_registry() {
        let (tree, _, problems) = tree(&[mod_api::Command::new("map fullrender <world:id>")]);
        assert!(problems.is_empty(), "{problems:?}");
        let world = &tree["children"]["map"]["children"]["fullrender"]["children"]["world"];
        assert_eq!(world["parser"], "minecraft:resource_location");
        assert!(
            world.get("properties").is_none(),
            "ResourceLocationArgument carries no properties"
        );
        // The free `id` has no list of its own: its node asks the server, which answers from the
        // mods' own suggestions (`tab_complete`), so an unmodded client is not left with nothing.
        assert_eq!(world["suggestions"], "minecraft:ask_server");

        let mut dispatcher = Dispatcher::new();
        register_dispatcher(
            &mut dispatcher,
            &tree,
            "native_mod_command",
            &HashMap::new(),
        );
        let registries = sunrise_client::command::Registries::default();
        for (line, id) in [
            ("map fullrender minecraft:overworld", "minecraft:overworld"),
            ("map fullrender mymod:custom", "mymod:custom"),
            ("map fullrender overworld", "minecraft:overworld"),
        ] {
            let parsed = dispatcher.parse(line, &registries);
            let (context, _) = parsed.executable().unwrap_or_else(|e| {
                panic!("{line}: {}", e.message.plain());
            });
            assert_eq!(context.get("world"), Some(&Arg::Str(id.into())), "{line}");
        }
        for refused in [
            "map fullrender Minecraft:overworld",
            "map fullrender minecraft:overworld:again",
            "map fullrender 50%stone,50%dirt",
        ] {
            assert!(
                dispatcher.parse(refused, &registries).executable().is_err(),
                "{refused}"
            );
        }
    }

    /// A test mod's argument type for a WorldEdit-style pattern list: the greedy text holds
    /// several tokens, and the completion is for the one being typed.
    struct PatternArgument;

    impl mod_api::CommandArgumentType for PatternArgument {
        fn kind(&self) -> mod_api::CommandArgumentKind {
            mod_api::CommandArgumentKind::Greedy
        }

        fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
            Ok(input.trim().to_owned())
        }

        fn suggest(&self, request: &mod_api::SuggestionRequest) -> Vec<mod_api::Suggestion> {
            let prefix = request.prefix.as_str();
            let start = prefix.rfind(' ').map_or(0, |space| space + 1);
            ["stone", "dirt"]
                .into_iter()
                .filter(|block| block.starts_with(&prefix[start..]))
                .map(|block| request.suggest_from(start, block))
                .collect()
        }
    }

    #[test]
    fn a_greedy_argument_completes_the_token_being_typed() {
        let arguments: CommandArguments = HashMap::from([(
            "test:pattern".into(),
            Arc::new(PatternArgument) as Arc<dyn mod_api::CommandArgumentType>,
        )]);
        let (tree, _, problems) = super::command_tree(
            &[mod_api::Command::new("replace <pattern:test:pattern>")],
            &sunrise_client::command::Dispatcher::new(),
            &arguments,
        );
        assert!(problems.is_empty(), "{problems:?}");
        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &tree, "native_mod_command", &arguments);
        let registries = sunrise_client::command::Registries::default();

        // The second token: the range covers it, and the text in front of it is kept.
        let typed = "replace stone st";
        let parse = dispatcher.parse(typed, &registries);
        let (range, suggestions) = dispatcher.suggestions(&parse, typed.len(), &registries);
        assert_eq!(
            suggestions
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>(),
            ["stone"]
        );
        assert_eq!(range, (14, typed.len()), "the token, not the whole text");
        assert_eq!(suggestions[0].apply(typed), "replace stone stone");

        // The first token of the text replaces it whole, as before.
        let typed = "replace dir";
        let parse = dispatcher.parse(typed, &registries);
        let (range, suggestions) = dispatcher.suggestions(&parse, typed.len(), &registries);
        assert_eq!(range, (8, typed.len()));
        assert_eq!(suggestions[0].apply(typed), "replace dirt");
    }

    /// A type whose completion mixes one the text can hold with one it cannot.
    struct MixedOffsets;

    impl mod_api::CommandArgumentType for MixedOffsets {
        fn kind(&self) -> mod_api::CommandArgumentKind {
            mod_api::CommandArgumentKind::Greedy
        }

        fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
            Ok(input.to_owned())
        }

        fn suggest(&self, request: &mod_api::SuggestionRequest) -> Vec<mod_api::Suggestion> {
            vec![
                request.suggest_from(0, "stone"),
                request.suggest_from(request.prefix.len() + 1, "dirt"),
            ]
        }
    }

    #[test]
    fn a_suggestion_with_an_offset_past_the_text_is_refused_and_the_others_stay() {
        let arguments: CommandArguments = HashMap::from([(
            "test:mixed".into(),
            Arc::new(MixedOffsets) as Arc<dyn mod_api::CommandArgumentType>,
        )]);
        let (tree, _, problems) = super::command_tree(
            &[mod_api::Command::new("fill <blocks:test:mixed>")],
            &sunrise_client::command::Dispatcher::new(),
            &arguments,
        );
        assert!(problems.is_empty(), "{problems:?}");
        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &tree, "native_mod_command", &arguments);
        let registries = sunrise_client::command::Registries::default();
        let typed = "fill st";
        let parse = dispatcher.parse(typed, &registries);
        let (_, suggestions) = dispatcher.suggestions(&parse, typed.len(), &registries);
        assert_eq!(
            suggestions
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>(),
            ["stone"],
            "the suggestion past the typed text is left out, never applied"
        );
    }

    #[test]
    fn resource_and_mod_id_kinds_accept_only_known_ids() {
        let mut arguments = command_arguments();
        // The test process loads no package: stand in for one the way `command_arguments` does.
        arguments.insert(
            argument_kinds::MOD_ID.into(),
            Arc::new(mod_api::ChoiceArgument::new(["demo"])),
        );
        let (tree, _, problems) = super::command_tree(
            &[
                mod_api::Command::new("spawnof <kind:resource:minecraft:entity_type>"),
                mod_api::Command::new("about <id:mod_id>"),
                mod_api::Command::new("bad <kind:resource:NotOk>"),
                mod_api::Command::new("worse <kind:resource:a:b:c>"),
                mod_api::Command::new("worst <kind:resource:a/b:c>"),
            ],
            &sunrise_client::command::Dispatcher::new(),
            &arguments,
        );
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(
            problems
                .iter()
                .all(|p| p.contains("is not a `namespace:path` resource location")),
            "{problems:?}"
        );
        for refused in ["bad", "worse", "worst"] {
            assert!(tree["children"].get(refused).is_none(), "{refused}");
        }
        let kind = &tree["children"]["spawnof"]["children"]["kind"];
        assert_eq!(kind["parser"], "minecraft:resource_key");
        assert_eq!(
            kind["properties"],
            json!({"registry": "minecraft:entity_type"})
        );
        assert_eq!(
            tree["children"]["about"]["children"]["id"]["custom_argument"],
            argument_kinds::MOD_ID
        );

        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &tree, "native_mod_command", &arguments);
        let registries = sunrise_client::command::Registries {
            entity_types: vec!["minecraft:zombie".into()],
            ..Default::default()
        };
        let parsed = dispatcher.parse("spawnof minecraft:zombie", &registries);
        let (context, _) = parsed.executable().unwrap();
        assert_eq!(
            context.get("kind"),
            Some(&Arg::Resource("minecraft:zombie".into()))
        );
        for refused in ["spawnof minecraft:nope", "about no_such_mod"] {
            assert!(
                dispatcher.parse(refused, &registries).executable().is_err(),
                "{refused}"
            );
        }
        let parsed = dispatcher.parse("about demo", &registries);
        let (context, _) = parsed.executable().unwrap();
        let Some(Arg::Custom { kind, value }) = context.get("id").cloned() else {
            panic!("a mod id parses as the built-in custom kind");
        };
        assert_eq!(
            custom_value(&kind, value),
            mod_api::CommandArgumentValue::String("demo".into()),
            "a loaded mod id reaches the mod as its plain string"
        );
    }

    #[test]
    fn native_usages_parse_with_their_permissions_in_the_server_dispatcher() {
        let (tree, _, problems) = tree(&[
            mod_api::Command::new("warp [name:word]"),
            mod_api::Command::op("warp set <amount:integer>", 2),
            mod_api::Command::new("toggle <enabled:boolean>"),
        ]);
        assert!(problems.is_empty(), "{problems:?}");
        let mut dispatcher = Dispatcher::new();
        register_dispatcher(
            &mut dispatcher,
            &tree,
            "native_mod_command",
            &HashMap::new(),
        );
        let mut registries = sunrise_client::command::Registries::default();
        assert!(dispatcher.parse("warp", &registries).executable().is_ok());
        assert!(
            dispatcher
                .parse("warp home", &registries)
                .executable()
                .is_ok()
        );
        assert!(
            dispatcher
                .parse("warp set 7", &registries)
                .executable()
                .is_err()
        );
        registries.permission = 2;
        assert!(
            dispatcher
                .parse("warp set 7", &registries)
                .executable()
                .is_ok()
        );
        assert!(
            dispatcher
                .parse("warp set nope", &registries)
                .executable()
                .is_err()
        );
        assert!(
            dispatcher
                .parse("warp set", &registries)
                .executable()
                .is_err()
        );
        assert!(
            dispatcher
                .parse("toggle true", &registries)
                .executable()
                .is_ok()
        );
        assert!(
            dispatcher
                .parse("toggle maybe", &registries)
                .executable()
                .is_err()
        );
    }

    fn command_delivery_packets(
        receiver: &std::sync::mpsc::Receiver<sunrise_client::lan::Out>,
    ) -> impl Iterator<Item = C> + '_ {
        use sunrise_client::lan::Out;
        use sunrise_protocol::{buf::Reader, ids::play::clientbound as ids};
        receiver
            .try_iter()
            .flat_map(|out| match out {
                Out::Packet(body) => vec![body],
                Out::Packets(bodies) => bodies,
                Out::Disconnect(_) => panic!("unexpected disconnect during command delivery"),
            })
            .filter_map(|body| {
                let mut reader = Reader::new(&body);
                let id = reader.varint().unwrap();
                match id {
                    ids::ENTITY_EVENT | ids::COMMANDS | ids::PLAYER_INFO_UPDATE => {
                        Some(C::decode(id, reader.rest()).unwrap())
                    }
                    _ => None,
                }
            })
    }

    #[test]
    #[ignore = "daily"]
    fn outgoing_command_filter_reads_each_live_recipient_and_isolates_panics() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use sunrise_client::lan::PlayerSender;

        struct Filter(Arc<Mutex<Vec<u128>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Filter {
            fn player_commands_sent(
                &self,
                host: &dyn mod_api::Host,
                event: &mut mod_api::PlayerCommandsSentEvent<'_>,
            ) -> Option<mod_api::CommandTreeSelection> {
                let uuid = event.player.uuid();
                assert_eq!(
                    event.player.name(host),
                    Some(if uuid == 12 { "guest12" } else { "guest13" })
                );
                if uuid == 12 {
                    assert_eq!(event.player.address(host), Some("127.0.0.1"));
                    assert_eq!(event.player.client_brand(host), Some("test-join-client"));
                    assert_eq!(
                        event.player.announced_channels(host).unwrap().as_ref(),
                        ["test:join"]
                    );
                }
                self.0.lock().unwrap().push(uuid);
                event
                    .tree
                    .hide_where(|name| uuid == 12 && name == "test_hide")
            }
        }

        struct Broken(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Broken {
            fn player_commands_sent(
                &self,
                _: &dyn mod_api::Host,
                _: &mut mod_api::PlayerCommandsSentEvent<'_>,
            ) -> Option<mod_api::CommandTreeSelection> {
                self.0.fetch_add(1, Ordering::Relaxed);
                panic!("test command filter panicked");
            }
        }

        let calls = Arc::new(Mutex::new(Vec::new()));
        let panics = Arc::new(AtomicUsize::new(0));
        mod_platform::testing::with_mods(
            vec![
                mod_api::ModEntry::new("broken_command_filter", Box::new(Broken(panics.clone()))),
                mod_api::ModEntry::new("command_filter", Box::new(Filter(calls.clone()))),
            ]
            .into(),
            || {
                super::super::native_guest_turn_tests::with_guests(|game| {
                    if game.native_lifecycle.epoch().is_none() {
                        game.native_lifecycle.world_open_ready().unwrap();
                    }
                    let host = game.lan.as_mut().unwrap();
                    let receivers = [12, 13].map(|conn| {
                        let (tx, rx) = std::sync::mpsc::channel();
                        host.hub.update_guest(conn, |g| {
                            g.sender = PlayerSender::new(tx, Arc::default());
                            g.placed = conn != 12;
                            if conn == 12 {
                                g.brand = "test-join-client".into();
                                assert!(
                                    g.sender.announced.apply("minecraft:register", b"test:join")
                                );
                            }
                        });
                        rx
                    });
                    assert!(host.hub.set_mod_commands(
                        0,
                        json!({"type": "root", "children": {
                            "test_hide": {"type": "literal", "executable": true},
                            "test_keep": {"type": "literal", "executable": true}
                        }})
                    ));
                    let unchanged = host.hub.commands_packet(0);
                    for (conn, rx) in [12, 13].into_iter().zip(receivers) {
                        if conn == 12 {
                            game.lan_guest_joined(conn, &[], [3.0, 64.0, 5.0], None);
                            assert!(game.lan.as_ref().unwrap().hub.guest(conn).unwrap().placed);
                        } else {
                            game.send_player_command_tree(conn);
                        }
                        let mut packets = command_delivery_packets(&rx);
                        let packet = packets
                            .find_map(|packet| match packet {
                                C::Commands(tree) => Some(tree),
                                C::PlayerInfoUpdate(info)
                                    if conn == 12
                                        && info.entries.iter().any(|row| row.uuid == 12) =>
                                {
                                    panic!(
                                        "the joining player was announced before its command tree"
                                    );
                                }
                                _ => None,
                            })
                            .expect("expected command tree");
                        if conn == 12 {
                            assert!(packets.any(|packet| matches!(packet,
                                C::PlayerInfoUpdate(info) if info.entries.iter().any(|row| row.uuid == 12)
                            )), "the joining player must be announced after its command tree");
                        }
                        if conn == 13 {
                            assert_eq!(
                                packet, unchanged,
                                "abstaining leaves the whole wire tree unchanged"
                            );
                        }
                        let names: Vec<_> = packet.nodes[packet.root as usize]
                            .children
                            .iter()
                            .map(|&id| packet.nodes[id as usize].name.as_str())
                            .filter(|name| name.starts_with("test_"))
                            .collect();
                        assert_eq!(
                            names,
                            if conn == 12 {
                                vec!["test_keep"]
                            } else {
                                vec!["test_hide", "test_keep"]
                            }
                        );
                    }
                    assert_eq!(*calls.lock().unwrap(), [12, 13]);
                    assert_eq!(panics.load(Ordering::Relaxed), 1);
                })
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn self_deop_sends_the_tree_for_the_permission_the_listener_reads() {
        use crate::game::{
            dedicated::Lists, execution_role::ExecutionRole, lan_actions::GuestCommand,
        };
        use sunrise_client::{
            lan::PlayerSender,
            server::user_lists::{BanList, IpBanList, NameAndId, OpEntry, OpList, WhiteList},
        };

        type Reads = Arc<Mutex<Vec<(Option<u8>, bool)>>>;
        struct Observe(Reads);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Observe {
            fn player_commands_sent(
                &self,
                host: &dyn mod_api::Host,
                event: &mut mod_api::PlayerCommandsSentEvent<'_>,
            ) -> Option<mod_api::CommandTreeSelection> {
                if event.player.uuid() == 12 {
                    self.0.lock().unwrap().push((
                        event.player.permission_level(host),
                        event.tree.root_names().any(|name| name == "deop"),
                    ));
                }
                None
            }
        }

        let reads = Reads::default();
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "observe_deop",
                Box::new(Observe(reads.clone())),
            )]
            .into(),
            || {
                super::super::native_guest_turn_tests::with_guests(|game| {
                    game.options.execution_role = ExecutionRole::DedicatedServer;
                    game.options.dedicated = Some(Vec::new());
                    game.build_registries();
                    game.ui.dispatcher.0 =
                        Arc::new(sunrise_client::command::vanilla::build_for(false, true));
                    let files = tempfile::tempdir().unwrap();
                    let root = files.path();
                    let mut ops = OpList::new(root.join("ops.json"));
                    assert!(ops.add(OpEntry {
                        user: NameAndId {
                            id: 12,
                            name: "guest12".into()
                        },
                        level: 4,
                        bypasses_player_limit: false,
                    }));
                    game.server_lists = Some(Arc::new(Mutex::new(Lists {
                        ops,
                        white: WhiteList::new(root.join("whitelist.json")),
                        bans: BanList::new(root.join("banned-players.json")),
                        ip_bans: IpBanList::new(root.join("banned-ips.json")),
                        use_whitelist: false,
                        enforce_whitelist: false,
                        op_level: 4,
                        player_idle_timeout: 0,
                        stop: false,
                        autosave: true,
                    })));
                    let (sent, received) = std::sync::mpsc::channel();
                    let hub = &mut game.lan.as_mut().unwrap().hub;
                    hub.config.integrated = false;
                    hub.ops.insert(12, 4);
                    hub.refresh_permissions();
                    hub.update_guest(12, |g| g.sender = PlayerSender::new(sent, Arc::default()));
                    game.send_player_command_tree(12);
                    // GameProfileArgument only accepts player-only selectors; bare @s can
                    // select any source entity.
                    let (ok, feedback) = game
                        .run_guest_command_told(GuestCommand {
                            conn: 12,
                            command: "deop @s[type=minecraft:player]".into(),
                            permission: 4,
                            integrated: false,
                            quiet: false,
                        })
                        .expect("the command source is online");
                    assert!(ok, "{feedback:?}");
                    assert_eq!(*reads.lock().unwrap(), [(Some(4), true), (Some(0), false)]);
                    let delivered: Vec<_> = command_delivery_packets(&received).collect();
                    let [
                        C::Commands(before),
                        C::EntityEvent(permission),
                        C::Commands(after),
                    ] = delivered.as_slice()
                    else {
                        panic!(
                            "expected the operator tree, permission update and non-operator tree"
                        );
                    };
                    assert_eq!((permission.id, permission.event), (12, 24));
                    for (packet, operator) in [(before, true), (after, false)] {
                        let roots = &packet.nodes[packet.root as usize].children;
                        assert_eq!(
                            roots
                                .iter()
                                .any(|&i| packet.nodes[i as usize].name == "deop"),
                            operator
                        );
                        assert!(roots.iter().any(|&i| packet.nodes[i as usize].name == "me"));
                    }
                })
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn panicking_command_executors_keep_other_usages_and_only_undo_journaled_writes() {
        crate::game::in_own_process(
            "game::native_server::tests::panicking_command_executors_keep_other_usages_and_only_undo_journaled_writes",
            || {
                use std::sync::atomic::AtomicUsize;
                const JOURNALED: [i32; 3] = [2, 70, 2];
                const UNJOURNALED: [i32; 3] = [3, 70, 2];
                const ABANDONED: [i32; 3] = [4, 70, 2];
                const REFUSED: [i32; 3] = [5, 70, 2];
                struct Executors(Arc<AtomicUsize>);
                #[mod_api::__subscriptions]
                impl mod_api::Hooks for Executors {
                    fn register_commands(
                        &self,
                        _: &mut dyn mod_api::ServerHost,
                        event: &mut mod_api::RegisterCommandsEvent<'_>,
                    ) {
                        for (usage, pos, opens_root) in [
                            ("journalboom", JOURNALED, false),
                            ("plainboom", UNJOURNALED, false),
                            ("rootboom", ABANDONED, true),
                            ("ordinaryfailure", REFUSED, true),
                        ] {
                            let refuses = self.0.clone();
                            event.commands.push(mod_api::Command::new(usage).executes(
                                move |host, _, _| {
                                    let mutate = |host: &mut dyn mod_api::ServerHost| {
                                        let level = host.level("minecraft:overworld").unwrap();
                                        let state = host.block_state_id("minecraft:stone").unwrap();
                                        if level.set_block(
                                            host,
                                            pos,
                                            state,
                                            mod_api::BlockUpdateFlags::NONE,
                                        ) != Ok(true)
                                        {
                                            return Err(
                                                mod_api::Text::literal("block not placed").into()
                                            );
                                        }
                                        if usage == "ordinaryfailure" {
                                            refuses.fetch_add(1, Ordering::Relaxed);
                                            return Err(
                                                mod_api::Text::literal("ordinary failure").into()
                                            );
                                        }
                                        panic!("executor regression panic");
                                    };
                                    if opens_root {
                                        let res = mod_api::HostTransaction::transaction(
                                            host,
                                            |host, _| {
                                                mod_api::HostTransaction::transaction(
                                                    host,
                                                    |host, _| mutate(host),
                                                )
                                                .map_err(|e| match e {
                                                    mod_api::TransactionError::Aborted(msg) => msg,
                                                    _ => mod_api::Text::literal("tx failed").into(),
                                                })
                                            },
                                        );
                                        match res {
                                            Ok(()) => Ok(0),
                                            Err(mod_api::TransactionError::Aborted(msg)) => {
                                                Err(msg)
                                            }
                                            Err(_) => {
                                                Err(mod_api::Text::literal("tx failed").into())
                                            }
                                        }
                                    } else {
                                        mutate(host).map(|()| 0)
                                    }
                                },
                            ));
                        }
                        event
                            .commands
                            .push(mod_api::Command::new("worksagain").executes(|_, _, _| Ok(7)));
                    }
                }
                let refuses = Arc::new(AtomicUsize::new(0));
                mod_platform::testing::with_mods(
                    vec![mod_api::ModEntry::new(
                        "executor_owner",
                        Box::new(Executors(refuses.clone())),
                    )]
                    .into(),
                    || {
                        super::super::native_entity_refs::tests::with_game(|game| {
                            game.options.dedicated = Some(Vec::new());
                            game.world.chunks.insert(
                                (0, 0),
                                Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
                            );
                            let air = game.world.block(JOURNALED[0], JOURNALED[1], JOURNALED[2]);
                            let stone = game
                                .pack
                                .assets
                                .registry
                                .default_state("minecraft:stone")
                                .unwrap();
                            game.build_registries();
                            game.build_native_commands(&[]);

                            game.storage_transaction(|game, outer| {
                                let (failure, outcome) = game.recording_command_outcome(|game| {
                                    game.execute_line("journalboom", game.server_source())
                                });
                                assert_eq!(failure.unwrap_err().key, "command.failed");
                                assert_eq!(outcome, None);
                                assert_eq!(
                                    game.world.block(JOURNALED[0], JOURNALED[1], JOURNALED[2]),
                                    air
                                );
                                assert!(mod_api::ServerHost::transaction_matches(
                                    game,
                                    Some(outer)
                                ));
                                Ok::<_, ()>(())
                            })
                            .unwrap();
                            let failure = game
                                .execute_line("plainboom", game.server_source())
                                .unwrap_err();
                            assert!(failure.unexpected);
                            assert_eq!(
                                game.world
                                    .block(UNJOURNALED[0], UNJOURNALED[1], UNJOURNALED[2]),
                                stone
                            );

                            let failure = game
                                .execute_line("rootboom", game.server_source())
                                .unwrap_err();
                            assert!(failure.unexpected);
                            assert!(!game.has_open_transaction());
                            assert_eq!(
                                game.world.block(ABANDONED[0], ABANDONED[1], ABANDONED[2]),
                                air
                            );

                            game.build_native_commands(&[]);
                            for usage in ["journalboom", "rootboom"] {
                                assert_eq!(
                                    game.execute_line(usage, game.server_source())
                                        .unwrap_err()
                                        .key,
                                    "command.unknown.command"
                                );
                            }
                            assert_eq!(
                                game.execute_line("worksagain", game.server_source()),
                                Ok(7)
                            );
                            for _ in 0..2 {
                                assert_eq!(
                                    game.execute_line("ordinaryfailure", game.server_source())
                                        .unwrap_err()
                                        .message
                                        .plain(),
                                    "ordinary failure"
                                );
                                assert!(!game.has_open_transaction());
                                assert_eq!(
                                    game.world.block(REFUSED[0], REFUSED[1], REFUSED[2]),
                                    air
                                );
                            }
                            assert_eq!(refuses.load(Ordering::Relaxed), 2);
                            assert_eq!(
                                mod_api::__private::take_hook_panics(),
                                [mod_api::__private::HookPanicReport {
                                    mod_id: "executor_owner",
                                    hook: "command_executor",
                                }; 3]
                            );
                        })
                    },
                );
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn command_refresh_rebuilds_the_typed_dispatcher_only_on_request() {
        crate::game::in_own_process(
            "game::native_server::tests::command_refresh_rebuilds_the_typed_dispatcher_only_on_request",
            || {
                use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
                struct ChangingCommands {
                    changed: Arc<AtomicBool>,
                    registrations: Arc<AtomicUsize>,
                    calls: Arc<Mutex<Vec<Option<&'static str>>>>,
                    replacement: bool,
                }
                #[mod_api::__subscriptions]
                impl mod_api::Hooks for ChangingCommands {
                    fn register_commands(
                        &self,
                        _: &mut dyn mod_api::ServerHost,
                        event: &mut mod_api::RegisterCommandsEvent<'_>,
                    ) {
                        let changed = self.changed.load(Ordering::Relaxed);
                        if !self.replacement {
                            self.registrations.fetch_add(1, Ordering::Relaxed);
                            let usage = if changed {
                                "after <n:integer>"
                            } else {
                                "before"
                            };
                            event
                                .commands
                                .push(mod_api::Command::new(usage).executes(|_, _, _| Ok(1)));
                        }
                        if changed == self.replacement {
                            let calls = self.calls.clone();
                            let result = if self.replacement { 37 } else { 0 };
                            event
                                .commands
                                .push(mod_api::Command::new("retained").executes(
                                    move |_, _, _| {
                                        calls
                                            .lock()
                                            .unwrap()
                                            .push(mod_api::content::crash::current_native_mod());
                                        Ok(result)
                                    },
                                ));
                        }
                    }
                    fn player_commands_sent(
                        &self,
                        _: &dyn mod_api::Host,
                        event: &mut mod_api::PlayerCommandsSentEvent<'_>,
                    ) -> Option<mod_api::CommandTreeSelection> {
                        event.tree.hide_where(|name| name == "before")
                    }
                }
                let changed = Arc::new(AtomicBool::new(false));
                let registrations = Arc::new(AtomicUsize::new(0));
                let calls = Arc::new(Mutex::new(Vec::new()));
                mod_platform::testing::with_mods(
                    [("refresh", false), ("replacement", true)]
                        .map(|(id, replacement)| {
                            mod_api::ModEntry::new(
                                id,
                                Box::new(ChangingCommands {
                                    changed: changed.clone(),
                                    registrations: registrations.clone(),
                                    calls: calls.clone(),
                                    replacement,
                                }),
                            )
                        })
                        .into(),
                    || {
                        super::super::native_guest_turn_tests::with_guests(|game| {
                            game.build_registries();
                            let (sent, received) = std::sync::mpsc::channel();
                            game.lan.as_mut().unwrap().hub.update_guest(12, |g| {
                                g.permission = 2;
                                g.sender =
                                    sunrise_client::lan::PlayerSender::new(sent, Arc::default());
                            });
                            game.register_native_commands();
                            let delivered: Vec<_> = command_delivery_packets(&received).collect();
                            let [C::Commands(tree)] = delivered.as_slice() else {
                                panic!("installing a changed display report sends only its tree");
                            };
                            let roots = &tree.nodes[tree.root as usize].children;
                            assert!(
                                roots
                                    .iter()
                                    .any(|&i| tree.nodes[i as usize].name == "retained")
                            );
                            assert!(
                                !roots
                                    .iter()
                                    .any(|&i| tree.nodes[i as usize].name == "before")
                            );
                            assert_eq!(
                                game.lan.as_ref().unwrap().hub.guest(12).unwrap().permission,
                                2
                            );
                            assert_eq!(registrations.load(Ordering::Relaxed), 1);
                            let old_parse =
                                game.ui.dispatcher.0.parse("retained", &game.ui.registries);
                            let old_context = old_parse.executable().unwrap().0;
                            let (result, outcome) = game.recording_command_outcome(|game| {
                                game.execute_line("retained", game.server_source())
                            });
                            assert_eq!(result, Ok(0));
                            assert_eq!(outcome, Some((true, 0)));
                            assert_eq!(*calls.lock().unwrap(), [Some("refresh")]);
                            changed.store(true, Ordering::Relaxed);
                            game.register_native_commands();
                            assert_eq!(registrations.load(Ordering::Relaxed), 1);
                            let registered = game.native_commands.registered.as_ref().unwrap();
                            assert!(registered.levels.contains_key("before"));
                            mod_api::Commands::refresh_commands(game);
                            mod_api::Commands::refresh_commands(game);
                            assert_eq!(
                                registrations.load(Ordering::Relaxed),
                                1,
                                "requests are deferred"
                            );
                            game.register_native_commands();
                            assert!(!game.native_commands.refresh);
                            assert_eq!(
                                registrations.load(Ordering::Relaxed),
                                2,
                                "requests coalesce"
                            );
                            assert_eq!(
                                game.run_native_command(old_context.clone())
                                    .unwrap_err()
                                    .key,
                                "command.unknown.command"
                            );
                            assert_eq!(*calls.lock().unwrap(), [Some("refresh")]);
                            assert_eq!(game.execute_line("retained", game.server_source()), Ok(37));
                            assert_eq!(
                                *calls.lock().unwrap(),
                                [Some("refresh"), Some("replacement")]
                            );
                            assert!(mod_api::__private::set_mod_enabled("replacement", false));
                            assert_eq!(
                                game.execute_line("retained", game.server_source())
                                    .unwrap_err()
                                    .key,
                                "command.unknown.command"
                            );
                            assert_eq!(
                                *calls.lock().unwrap(),
                                [Some("refresh"), Some("replacement")]
                            );
                            assert!(mod_api::__private::set_mod_enabled("replacement", true));
                            let commands = game.native_commands.registered.as_ref().unwrap();
                            assert!(!commands.levels.contains_key("before"));
                            assert!(commands.levels.contains_key("after"));
                            let dispatcher = &game.ui.dispatcher.0;
                            assert!(
                                dispatcher
                                    .parse("after 7", &game.ui.registries)
                                    .executable()
                                    .is_ok()
                            );
                            assert!(
                                dispatcher
                                    .parse("after nope", &game.ui.registries)
                                    .executable()
                                    .is_err()
                            );
                            assert!(
                                dispatcher
                                    .parse("before", &game.ui.registries)
                                    .executable()
                                    .is_err()
                            );
                            game.register_native_commands();
                            assert_eq!(registrations.load(Ordering::Relaxed), 2);
                            mod_api::Commands::refresh_commands(game);
                            game.register_native_commands();
                            assert_eq!(registrations.load(Ordering::Relaxed), 3);
                            assert!(game.native_commands.level("after").is_some());
                            changed.store(false, Ordering::Relaxed);
                            game.reload_datapacks(false, &game.level.enabled_features.clone())
                                .unwrap();
                            let (sent, received) = std::sync::mpsc::channel();
                            game.lan
                                .as_mut()
                                .unwrap()
                                .hub
                                .update_guest(12, |guest| {
                                    guest.sender = sunrise_client::lan::PlayerSender::new(
                                        sent,
                                        Default::default(),
                                    );
                                })
                                .unwrap();
                            game.register_native_commands();
                            let packet = received
                                .try_iter()
                                .flat_map(|out| match out {
                                    sunrise_client::lan::Out::Packet(body) => vec![body],
                                    sunrise_client::lan::Out::Packets(bodies) => bodies,
                                    sunrise_client::lan::Out::Disconnect(_) => {
                                        panic!("reload disconnected guest")
                                    }
                                })
                                .find_map(|body| {
                                    let mut reader = sunrise_protocol::buf::Reader::new(&body);
                                    let id = reader.varint().unwrap();
                                    match C::decode(id, &body[reader.position()..]).unwrap() {
                                        C::Commands(packet) => Some(packet),
                                        _ => None,
                                    }
                                })
                                .expect("reload must deliver a command tree");
                            let names: Vec<_> = packet.nodes[packet.root as usize]
                                .children
                                .iter()
                                .map(|&i| packet.nodes[i as usize].name.as_str())
                                .collect();
                            assert!(!names.is_empty());
                            assert!(
                                !names.contains(&"before"),
                                "new reload root must be filtered"
                            );
                            assert!(!names.contains(&"after"), "old root must be retired");
                        })
                    },
                );
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn native_typed_commands_reach_listeners_after_parse_and_source_resolution() {
        crate::game::in_own_process(
            "game::native_server::tests::native_typed_commands_reach_listeners_after_parse_and_source_resolution",
            || {
                use mod_api::{CommandArgumentValue as Value, CommandInvocation};
                struct Commands(Arc<Mutex<Vec<(bool, CommandInvocation)>>>);
                #[mod_api::__subscriptions]
                impl mod_api::Hooks for Commands {
                    fn register_commands(
                        &self,
                        _: &mut dyn mod_api::ServerHost,
                        event: &mut mod_api::RegisterCommandsEvent<'_>,
                    ) {
                        let heard = self.0.clone();
                        let record =
                            move |_: &mut dyn mod_api::ServerHost,
                                  source: &mod_api::CommandSource,
                                  command: &CommandInvocation| {
                                if command.literals == ["typed", "give"] {
                                    let Some(Value::Integer(amount)) = command.argument("amount")
                                    else {
                                        panic!("executor must receive an integer")
                                    };
                                    let Some(Value::Player(player)) = command.argument("who")
                                    else {
                                        panic!("executor must receive a resolved player")
                                    };
                                    assert_ne!(*amount, 0);
                                    assert_ne!(player.uuid(), 0);
                                }
                                let console =
                                    matches!(source, mod_api::CommandSource::Console { .. });
                                heard.lock().unwrap().push((console, command.clone()));
                                Ok(1)
                            };
                        event.commands.extend([
                            mod_api::Command::new("typed give <who:player> <amount:integer> [message:text]"),
                            mod_api::Command::new("typed data <word:word> <number:number> <enabled:boolean> <item:item> <pos:block_pos>"),
                            mod_api::Command::new("typed opt [value:integer]"),
                            mod_api::Command::new("/set <x:word>"),
                            mod_api::Command::new("typed repeat <value:player> <value:integer>"),
                            mod_api::Command::new("typed at <pos:vec3>"),
                            mod_api::Command::op("protected <who:player>", 4),
                        ].map(|command| command.executes(record.clone())));
                    }
                }
                let heard = Arc::new(Mutex::new(Vec::new()));
                mod_platform::testing::with_mods(
                    vec![mod_api::ModEntry::new(
                        "typed_commands",
                        Box::new(Commands(heard.clone())),
                    )]
                    .into(),
                    || {
                        super::super::native_guest_turn_tests::with_guests(|game| {
                            game.build_registries();
                            game.register_native_commands();
                            let other = game.player(13).unwrap();
                            let acting = game.player(12).unwrap();
                            game.as_guest(12, |game| {
                                for line in [
                                    "typed give guest13 nope",
                                    "typed give guest13",
                                    "typed give missing 7",
                                    "typed give @a 7",
                                    "typed data word 1 maybe stone ~ ~ ~",
                                    "typed data word NaN true stone ~ ~ ~",
                                    "typed data word 1 true does_not_exist ~ ~ ~",
                                    "typed opt 1 extra",
                                ] {
                                    assert_eq!(game.perform_native_command(12, 4, line, false), Some(false), "{line}");
                                }
                                let random = game.world.random.state();
                                assert_eq!(game.perform_native_command(12, 0, "protected @r", false), None);
                                assert_eq!(game.world.random.state(), random, "denied selectors must not resolve");
                                assert!(heard.lock().unwrap().is_empty(), "failed input must not reach listeners");
                                for line in [
                                    "typed give guest13 37 hello typed world",
                                    "typed give @s -4",
                                    "typed opt",
                                    "typed opt 9",
                                    "typed data word -1.25 true stone[minecraft:custom_name='\"Rock\"'] ~1 ~2 ~-3",
                                    "typed data word 2 false dirt ^0 ^0 ^0",
                                ] {
                                    assert_eq!(game.perform_native_command(12, 4, line, false), Some(true), "{line}");
                                }
                            }).unwrap();
                            game.spawn = Some(sunrise_core::geometry::Vec3::new(10.5, 70.0, -2.5));
                            game.yaw = 45.0;
                            game.pitch = 30.0;
                            for (line, key) in [
                                ("typed give guest13 nope", "parsing.int.expected"),
                                ("typed give missing 1", "argument.entity.notfound.player"),
                            ] {
                                game.chat_capture = Some(Vec::new());
                                assert!(game.perform_native_console_command(line));
                                let output = game.chat_capture.take().unwrap();
                                assert_eq!(
                                    output[0].plain(),
                                    Text::translate(key, &[]).plain(),
                                    "{line}"
                                );
                            }
                            assert!(game.perform_native_console_command("typed give @s 1"));
                            assert_eq!(
                                heard.lock().unwrap().len(),
                                6,
                                "console has no source player"
                            );
                            assert!(game.perform_native_console_command("typed give guest13 11"));
                            assert!(game.perform_native_console_command(
                                "typed data console 3 true dirt ~1 ~2 ~3"
                            ));
                            assert!(game.perform_native_console_command(
                                "typed data console 3 true dirt ^0.5 ^0.5 ^0.5"
                            ));
                            assert!(game.perform_native_console_command("typed repeat guest13 3"));
                            assert!(game.perform_native_console_command("typed at 1 64.25 ~1"));
                            // A point is three numbers: two, or text, is a parse error and is not heard
                            // (the count of 12 heard lines below holds neither).
                            game.perform_native_console_command("typed at 1 64.25");
                            game.perform_native_console_command("typed at 1 up 3");
                            assert!(game.command_source.is_none(), "console source restored");
                            game.as_guest(12, |game| {
                                assert!(
                                    game.native_run_command("execute as guest13 run typed opt 13")
                                )
                            })
                            .unwrap();
                            let observations = heard.clone();
                            let heard = heard.lock().unwrap();
                            assert_eq!(heard.len(), 12);
                            let first = &heard[0].1;
                            assert_eq!(first.literals, ["typed", "give"]);
                            assert_eq!(first.argument("who"), Some(&Value::Player(other)));
                            assert_eq!(first.argument("amount"), Some(&Value::Integer(37)));
                            assert_eq!(
                                first.argument("message"),
                                Some(&Value::String("hello typed world".into()))
                            );
                            assert_eq!(heard[1].1.argument("who"), Some(&Value::Player(acting)));
                            assert_eq!(heard[1].1.argument("amount"), Some(&Value::Integer(-4)));
                            assert!(heard[1].1.argument("message").is_none());
                            assert!(heard[2].1.arguments.is_empty());
                            assert_eq!(heard[3].1.argument("value"), Some(&Value::Integer(9)));
                            let data = &heard[4].1;
                            assert_eq!(data.argument("word"), Some(&Value::String("word".into())));
                            assert_eq!(data.argument("number"), Some(&Value::Number(-1.25)));
                            assert_eq!(data.argument("enabled"), Some(&Value::Boolean(true)));
                            assert_eq!(data.argument("pos"), Some(&Value::BlockPos([4, 66, 2])));
                            let Some(Value::Item(item)) = data.argument("item") else {
                                panic!("typed item")
                            };
                            assert_eq!(
                                mod_api::__private::item_key_id(item.item()),
                                Some("minecraft:stone")
                            );
                            assert_eq!(item.count(), 1);
                            assert!(item.components().is_some(), "command components retained");
                            assert_eq!(
                                heard[5].1.argument("enabled"),
                                Some(&Value::Boolean(false))
                            );
                            assert_eq!(
                                heard[5].1.argument("pos"),
                                Some(&Value::BlockPos([3, 64, 5]))
                            );
                            assert!(heard[..6].iter().all(|(console, _)| !console));
                            assert!(heard[6..].iter().all(|(console, _)| *console));
                            assert_eq!(heard[6].1.argument("who"), Some(&Value::Player(other)));
                            assert_eq!(heard[6].1.argument("amount"), Some(&Value::Integer(11)));
                            assert_eq!(
                                heard[7].1.argument("pos"),
                                Some(&Value::BlockPos([11, 72, 0]))
                            );
                            assert_eq!(
                                heard[8].1.argument("pos"),
                                Some(&Value::BlockPos([10, 70, -3]))
                            );
                            assert_eq!(heard[9].1.arguments[0].value, Value::Player(other));
                            assert_eq!(heard[9].1.argument("value"), Some(&Value::Integer(3)));
                            // `1 64.25 ~1`: a whole x is a block centre (1.5), y is as typed, and `~1`
                            // is one block from the console's z.
                            assert_eq!(
                                heard[10].1.argument("pos"),
                                Some(&Value::Vec3([1.5, 64.25, -2.0]))
                            );
                            drop(heard);
                            game.lan
                                .as_mut()
                                .unwrap()
                                .hub
                                .update_guest(12, |g| g.permission = 4)
                                .unwrap();
                            let inventory = game
                                .lan
                                .as_ref()
                                .unwrap()
                                .hub
                                .guest(12)
                                .unwrap()
                                .inventory
                                .clone();
                            game.guest_command(12, "/set stone", None);
                            game.guest_command(12, "typed opt 17", None);
                            game.guest_command(12, "/give guest12 minecraft:stone 1", None);
                            game.run_guest_commands();
                            let heard = observations.lock().unwrap();
                            assert_eq!(heard.len(), 14);
                            assert_eq!(heard[12].1.literals, ["/set"]);
                            assert_eq!(
                                heard[12].1.argument("x"),
                                Some(&Value::String("stone".into()))
                            );
                            assert_eq!(heard[13].1.literals, ["typed", "opt"]);
                            assert_eq!(heard[13].1.argument("value"), Some(&Value::Integer(17)));
                            assert_eq!(
                                game.lan.as_ref().unwrap().hub.guest(12).unwrap().inventory,
                                inventory
                            );
                        });
                    },
                );
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn native_commands_keep_function_block_and_cart_sources() {
        crate::game::in_own_process(
            "game::native_server::tests::native_commands_keep_function_block_and_cart_sources",
            || {
                use mod_api::{CommandSource, PermissionLevel as Permission};
                use sunrise_client::{
                    chunk::block_entity::BlockEntity,
                    datapack::{Function, FunctionLine},
                };
                use sunrise_core::position::BlockPos;
                struct Commands(Arc<Mutex<Vec<CommandSource>>>);
                #[mod_api::__subscriptions]
                impl mod_api::Hooks for Commands {
                    fn register_commands(
                        &self,
                        _: &mut dyn mod_api::ServerHost,
                        event: &mut mod_api::RegisterCommandsEvent<'_>,
                    ) {
                        let heard = self.0.clone();
                        let execute =
                            move |host: &mut dyn mod_api::ServerHost,
                                  source: &CommandSource,
                                  _: &mod_api::CommandInvocation| {
                                heard.lock().unwrap().push(source.clone());
                                host.commands()
                                    .feedback(mod_api::Text::literal("source answer"), false)
                                    .unwrap();
                                Ok(1)
                            };
                        event.commands.extend([
                            mod_api::Command::new("origin").executes(execute.clone()),
                            mod_api::Command::op("denied", 4).executes(execute),
                            // Replaced by the later accepted registration; failure must not fall back here.
                            mod_api::Command::new("unhandled").executes(|_, _, _| Ok(99)),
                            mod_api::Command::new("unhandled")
                                .executes(|_, _, _| Err(mod_api::Text::literal("refused").into())),
                        ]);
                    }
                }
                let heard = Arc::new(Mutex::new(Vec::new()));
                mod_platform::testing::with_mods(
                    vec![mod_api::ModEntry::new(
                        "origins",
                        Box::new(Commands(heard.clone())),
                    )]
                    .into(),
                    || {
                        with_command_sources(|game| {
                            game.command_block_tick(BlockPos::new(1, 64, 1));
                            let Some(BlockEntity::CommandBlock(block)) =
                                game.world.block_entities.get(&(1, 64, 1))
                            else {
                                panic!("block missing")
                            };
                            assert_eq!(block.success_count, 1);
                            let source = heard.lock().unwrap()[0].clone();
                            assert!(
                                matches!(source, CommandSource::CommandBlock { position: [1, 64, 1], level, .. } if level == game.dim.key())
                            );
                            assert_eq!(source.permission(), Permission::Gamemasters);
                            assert!(game.perform_cart_command(99));
                            assert_eq!(game.carts[0].command.success_count, 1);
                            assert!(game.carts[0].command.last_output.is_some());
                            assert!(
                                matches!(heard.lock().unwrap().last().unwrap(), CommandSource::CommandBlockMinecart { entity, .. } if entity.id() == 99)
                            );
                            game.chat_capture = Some(Vec::new());
                            let mut function = Function {
                                id: "test:inner".into(),
                                lines: vec![
                                    FunctionLine::Command(
                                        "execute positioned 2 64 3 run origin".into(),
                                    ),
                                    FunctionLine::Command("denied".into()),
                                ],
                                parameters: vec![],
                            };
                            game.ensure_datapacks();
                            game.fns
                                .store
                                .functions
                                .insert(function.id.clone(), function.clone());
                            function.id = "test:outer".into();
                            function.lines =
                                vec![FunctionLine::Command("function test:inner".into())];
                            let before = heard.lock().unwrap().len();
                            game.run_function(&function, None, game.server_source())
                                .unwrap();
                            assert_eq!(heard.lock().unwrap().len(), before + 1);
                            assert!(
                                matches!(heard.lock().unwrap().last().unwrap(), CommandSource::Function { id, permission } if id.to_string() == "test:inner" && *permission == Permission::Gamemasters)
                            );
                            assert!(
                                game.chat_capture.take().unwrap().is_empty(),
                                "functions suppress both success and failure"
                            );
                            assert!(game.exec.functions.is_empty());
                            game.ui.registries.permission = 4;
                            assert!(!game.run_command("unhandled"));
                            assert!(game.run_command(
                                "execute store result storage test:result value int 1 run origin"
                            ));
                            assert!(game.run_command(
                                "execute if data storage test:result {value:1} run origin"
                            ));
                        });
                    },
                );
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn native_unauthorized_commands_never_reach_the_listener() {
        crate::game::in_own_process(
            "game::native_server::tests::native_unauthorized_commands_never_reach_the_listener",
            || {
                struct Commands(Arc<Mutex<Vec<mod_api::CommandInvocation>>>);
                #[mod_api::__subscriptions]
                impl mod_api::Hooks for Commands {
                    fn register_commands(
                        &self,
                        _: &mut dyn mod_api::ServerHost,
                        event: &mut mod_api::RegisterCommandsEvent<'_>,
                    ) {
                        event.commands.extend(
                            [
                                mod_api::Command::op("foo [value:integer]", 4),
                                mod_api::Command::new("foo bar"),
                                mod_api::Command::op("choice <a:integer>", 4),
                                mod_api::Command::new("choice <a:integer> bar"),
                                mod_api::Command::new("choice <b:word>"),
                            ]
                            .map(|command| {
                                let heard = self.0.clone();
                                command.executes(move |_, _, invocation| {
                                    heard.lock().unwrap().push(invocation.clone());
                                    Ok(1)
                                })
                            }),
                        );
                    }
                }
                let heard = Arc::new(Mutex::new(Vec::new()));
                let mods = vec![mod_api::ModEntry::new(
                    "commands",
                    Box::new(Commands(heard.clone())),
                )]
                .into();
                mod_platform::testing::with_mods(mods, || {
                    super::super::native_entity_refs::tests::with_game(|game| {
                        super::super::native_item_places::tests::open_to_lan(game);
                        game.register_native_commands();
                        game.lan.as_mut().unwrap().hub.guests.push(player_guest());
                        for line in ["foo", "foo 7", "foo nope"] {
                            assert_eq!(game.perform_native_command(1, 0, line, false), Some(false));
                        }
                        assert!(
                            heard.lock().unwrap().is_empty(),
                            "denied usages must not call a mod"
                        );
                        assert_eq!(
                            game.perform_native_command(1, 0, "foo bar", false),
                            Some(true)
                        );
                        assert_eq!(game.perform_native_command(1, 4, "foo", false), Some(true));
                        assert_eq!(
                            game.perform_native_command(1, 4, "foo 7", false),
                            Some(true)
                        );
                        for line in ["choice 7", "choice 7 bar", "choice text"] {
                            assert_eq!(game.perform_native_command(1, 0, line, false), Some(true));
                        }
                        let heard = heard.lock().unwrap();
                        assert_eq!(heard.len(), 6);
                        assert_eq!(heard[0].literals, ["foo", "bar"]);
                        assert_eq!(heard[1].literals, ["foo"]);
                        assert!(heard[1].arguments.is_empty());
                        assert_eq!(heard[2].literals, ["foo"]);
                        assert_eq!(
                            heard[2].argument("value"),
                            Some(&mod_api::CommandArgumentValue::Integer(7))
                        );
                        assert_eq!(heard[3].literals, ["choice"]);
                        assert!(heard[3].argument("a").is_none());
                        assert_eq!(
                            heard[3].argument("b"),
                            Some(&mod_api::CommandArgumentValue::String("7".into()))
                        );
                        assert_eq!(heard[4].literals, ["choice", "bar"]);
                        assert_eq!(
                            heard[4].argument("a"),
                            Some(&mod_api::CommandArgumentValue::Integer(7))
                        );
                        assert_eq!(heard[5].literals, ["choice"]);
                        assert_eq!(
                            heard[5].argument("b"),
                            Some(&mod_api::CommandArgumentValue::String("text".into()))
                        );
                    });
                });
            },
        );
    }

    #[test]
    fn native_executable_permissions_do_not_follow_public_descendants() {
        for optional in [false, true] {
            for reverse in [false, true] {
                let admin = if optional {
                    "foo [value:integer]"
                } else {
                    "foo"
                };
                let mut commands = vec![
                    mod_api::Command::op(admin, 4),
                    mod_api::Command::new("foo bar"),
                ];
                if reverse {
                    commands.reverse();
                }
                let (tree, _, problems) = tree(&commands);
                assert!(problems.is_empty(), "{problems:?}");
                let mut dispatcher = Dispatcher::new();
                register_dispatcher(
                    &mut dispatcher,
                    &tree,
                    "native_mod_command",
                    &HashMap::new(),
                );
                for permission in 0..=4 {
                    let registries = sunrise_client::command::Registries {
                        permission,
                        ..Default::default()
                    };
                    for line in ["foo", "foo bar", "foo 7"] {
                        let allowed =
                            line == "foo bar" || permission == 4 && (line == "foo" || optional);
                        assert_eq!(
                            dispatcher.parse(line, &registries).executable().is_ok(),
                            allowed,
                            "{commands:?}, level {permission}, {line}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn native_public_prefixes_do_not_open_admin_children() {
        for reverse in [false, true] {
            let mut commands = vec![
                mod_api::Command::new("foo"),
                mod_api::Command::op("foo bar", 4),
            ];
            if reverse {
                commands.reverse();
            }
            let (tree, _, problems) = tree(&commands);
            assert!(problems.is_empty(), "{problems:?}");
            let mut dispatcher = Dispatcher::new();
            register_dispatcher(
                &mut dispatcher,
                &tree,
                "native_mod_command",
                &HashMap::new(),
            );
            for permission in 0..=4 {
                let registries = sunrise_client::command::Registries {
                    permission,
                    ..Default::default()
                };
                assert!(dispatcher.parse("foo", &registries).executable().is_ok());
                assert_eq!(
                    dispatcher
                        .parse("foo bar", &registries)
                        .executable()
                        .is_ok(),
                    permission == 4
                );
            }
        }
    }

    #[test]
    fn native_shared_executable_prefix_uses_last_accepted_registration() {
        let cases = [
            (
                vec![
                    mod_api::Command::op("foo [value:integer]", 4),
                    mod_api::Command::op("foo", 2),
                    mod_api::Command::op("foo", 2), // Later registration wins on ties.
                    mod_api::Command::new("foo [value:word]"), // Rejected: parser conflict.
                ],
                (2, 2),
                (4, 0),
            ),
            (
                vec![
                    mod_api::Command::op("foo", 2),
                    mod_api::Command::op("foo [value:integer]", 4), // Later runnable overwrites earlier lower level.
                    mod_api::Command::new("foo [value:word]"),      // Rejected: parser conflict.
                ],
                (4, 1),
                (4, 1),
            ),
        ];
        for (commands, (prefix_level, prefix_slot), (argument_level, argument_slot)) in cases {
            let arguments = HashMap::new();
            let mut executors = HashMap::new();
            let (tree, _, problems) = command_tree_for(
                &commands,
                BaseTree::Merged(&Dispatcher::new()),
                &arguments,
                false,
                Some(&mut executors),
            );
            assert_eq!(problems.len(), 1, "{problems:?}");
            let mut dispatcher = Dispatcher::new();
            register_command_nodes(
                &mut dispatcher,
                &tree,
                "native_mod_command",
                &arguments,
                Some(&executors),
            );
            for permission in 0..=4 {
                let registries = sunrise_client::command::Registries {
                    permission,
                    ..Default::default()
                };
                for (input, level, slot) in [
                    ("foo", prefix_level, prefix_slot),
                    ("foo 7", argument_level, argument_slot),
                ] {
                    let parsed = dispatcher.parse(input, &registries);
                    if permission >= level {
                        let (context, _) = parsed.executable().unwrap();
                        assert_eq!(
                            context.executor.unwrap().slot(),
                            slot,
                            "{commands:?}, level {permission}, {input}"
                        );
                    } else {
                        assert!(
                            parsed.executable().is_err(),
                            "{commands:?}, level {permission}, {input}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn native_rejected_usage_cannot_open_an_executable_prefix() {
        let (tree, _, problems) = tree(&[
            mod_api::Command::op("foo <value:integer>", 4),
            mod_api::Command::new("foo [value:word]"),
        ]);
        assert_eq!(problems.len(), 1);
        let mut dispatcher = Dispatcher::new();
        register_dispatcher(
            &mut dispatcher,
            &tree,
            "native_mod_command",
            &HashMap::new(),
        );
        for permission in 0..=4 {
            let registries = sunrise_client::command::Registries {
                permission,
                ..Default::default()
            };
            assert!(dispatcher.parse("foo", &registries).executable().is_err());
            assert!(
                dispatcher
                    .parse("foo hello", &registries)
                    .executable()
                    .is_err()
            );
            assert_eq!(
                dispatcher.parse("foo 7", &registries).executable().is_ok(),
                permission == 4
            );
        }
    }

    #[test]
    fn a_command_takes_its_lowest_level() {
        let (t, levels, _) = tree(&[
            mod_api::Command::new("warp [name:word]"),
            mod_api::Command::op("warp set <name:word>", 2),
        ]);
        assert_eq!(levels["warp"], 0);
        assert!(t["children"]["warp"].get("permissions").is_none());
        let set = &t["children"]["warp"]["children"]["set"];
        assert_eq!(set["permissions"]["permission"]["level"], "gamemasters");
    }

    #[test]
    fn bad_usages_are_left_out() {
        let (t, levels, problems) = tree(&[
            mod_api::Command::new("a [x:integer] <y:integer>"),
            mod_api::Command::new("b <x:text> <y:word>"),
            mod_api::Command::new("c <x:colour>"),
            mod_api::Command::new("<x:integer>"),
            mod_api::Command::new("Bad"),
            mod_api::Command::new("d <x:integer>"),
            mod_api::Command::new("d <x:word>"),
        ]);
        assert_eq!(problems.len(), 6, "{problems:?}");
        assert_eq!(levels.keys().collect::<Vec<_>>(), ["d"]);
        assert_eq!(
            t["children"]["d"]["children"]["x"]["parser"],
            "brigadier:integer"
        );
        let (none, levels, _) = tree(&[]);
        assert!(none.is_null() && levels.is_empty());
    }

    /// `integer(min,max)` and `long(min,max)` go out as vanilla's `IntegerArgumentInfo` and
    /// `LongArgumentInfo` write them (a bound at the type's extreme is left out), and the server
    /// refuses a value out of range with Brigadier's message before any executor runs.
    #[test]
    fn bounded_numbers_keep_their_bounds_in_the_tree_and_the_dispatcher() {
        let (t, _, problems) = tree(&[
            mod_api::Command::new("pay <amount:integer(0,1000)>"),
            mod_api::Command::new("bank <total:long(-5,9000000000)>"),
            mod_api::Command::new("floor <level:integer(0,)>"),
            mod_api::Command::new("whole <n:integer(-2147483648,2147483647)>"),
            mod_api::Command::new("plain <n:long>"),
            mod_api::Command::new("bad <n:integer(5,1)>"),
            mod_api::Command::new("bad <n:integer(0,2147483648)>"),
            mod_api::Command::new("bad <n:long(1)>"),
            mod_api::Command::new("bad <n:integer(a,)>"),
        ]);
        assert_eq!(problems.len(), 4, "{problems:?}");
        assert!(
            problems.iter().all(|p| p.starts_with("`bad ")),
            "{problems:?}"
        );
        let c = &t["children"];
        let node = |root: &str, name: &str| c[root]["children"][name].clone();
        assert_eq!(node("pay", "amount")["parser"], "brigadier:integer");
        assert_eq!(
            node("pay", "amount")["properties"],
            json!({"min": 0, "max": 1000})
        );
        assert_eq!(node("bank", "total")["parser"], "brigadier:long");
        assert_eq!(
            node("bank", "total")["properties"],
            json!({"min": -5, "max": 9_000_000_000_i64})
        );
        assert_eq!(node("floor", "level")["properties"], json!({"min": 0}));
        assert!(node("whole", "n").get("properties").is_none());
        assert_eq!(node("plain", "n")["parser"], "brigadier:long");
        assert!(node("plain", "n").get("properties").is_none());

        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &t, "native_mod_command", &HashMap::new());
        let registries = sunrise_client::command::Registries::default();
        let value = |line: &str, name: &str| {
            dispatcher
                .parse(line, &registries)
                .executable()
                .unwrap()
                .0
                .get(name)
                .cloned()
        };
        assert_eq!(value("pay 1000", "amount"), Some(Arg::Int(1000)));
        assert_eq!(
            value("bank 9000000000", "total"),
            Some(Arg::Long(9_000_000_000))
        );
        assert_eq!(
            value("plain -9223372036854775808", "n"),
            Some(Arg::Long(i64::MIN))
        );
        for (line, expected) in [
            (
                "pay 1001",
                Text::tr("argument.integer.big", &[&1000, &1001]),
            ),
            ("pay -1", Text::tr("argument.integer.low", &[&0, &-1])),
            ("floor -1", Text::tr("argument.integer.low", &[&0, &-1])),
            (
                "bank 9000000001",
                Text::tr(
                    "argument.long.big",
                    &[&9_000_000_000_i64, &9_000_000_001_i64],
                ),
            ),
            ("bank -6", Text::tr("argument.long.low", &[&-5, &-6])),
            ("bank 1.5", Text::tr("parsing.long.invalid", &[&"1.5"])),
        ] {
            let error = dispatcher
                .parse(line, &registries)
                .executable()
                .unwrap_err();
            assert_eq!(error.message, expected, "{line}");
        }
    }

    /// A mod argument type that declares `CommandArgumentKind::Integer` goes out as vanilla's
    /// bounded `brigadier:integer`, so a client refuses what the server refuses; the server checks
    /// the bounds before the mod's parser.
    #[test]
    fn a_registered_integer_kind_goes_out_bounded() {
        struct Stack;
        impl mod_api::CommandArgumentType for Stack {
            fn kind(&self) -> mod_api::CommandArgumentKind {
                mod_api::CommandArgumentKind::Integer {
                    min: Some(1),
                    max: Some(64),
                }
            }
            fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
                Ok(format!("{input} items"))
            }
            fn suggest(&self, _: &mod_api::SuggestionRequest) -> Vec<mod_api::Suggestion> {
                Vec::new()
            }
        }
        let arguments: CommandArguments = HashMap::from([(
            "test:stack".to_owned(),
            Arc::new(Stack) as Arc<dyn mod_api::CommandArgumentType>,
        )]);
        let (t, _, problems) = command_tree(
            &[mod_api::Command::new("stack <count:test:stack>")],
            &Dispatcher::new(),
            &arguments,
        );
        assert!(problems.is_empty(), "{problems:?}");
        let count = &t["children"]["stack"]["children"]["count"];
        assert_eq!(count["parser"], "brigadier:integer");
        assert_eq!(count["properties"], json!({"min": 1, "max": 64}));
        assert_eq!(count["custom_argument"], "test:stack");
        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &t, "native_mod_command", &arguments);
        let registries = sunrise_client::command::Registries::default();
        let parsed = dispatcher.parse("stack 64", &registries);
        assert_eq!(
            parsed.executable().unwrap().0.get("count"),
            Some(&Arg::Custom {
                kind: "test:stack".into(),
                value: "64 items".into(),
            })
        );
        let error = dispatcher
            .parse("stack 65", &registries)
            .executable()
            .unwrap_err();
        assert_eq!(error.message, Text::tr("argument.integer.big", &[&64, &65]));
    }

    /// `<name:string>` and a type declaring `CommandArgumentKind::Quotable` read Brigadier's
    /// `StringArgumentType.string()`: a word or a quoted phrase, decoded, with an argument after
    /// it. Both go to clients as `brigadier:string` in its quotable mode; only the mod's type asks
    /// the server for completions, as `word` and a `Word` type do.
    #[test]
    fn a_string_usage_and_a_quotable_type_read_a_quoted_phrase_before_the_next_argument() {
        struct QuestObject;
        impl mod_api::CommandArgumentType for QuestObject {
            fn kind(&self) -> mod_api::CommandArgumentKind {
                mod_api::CommandArgumentKind::Quotable
            }
            fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
                Ok(input.to_lowercase())
            }
            fn suggest(&self, _: &mod_api::SuggestionRequest) -> Vec<mod_api::Suggestion> {
                Vec::new()
            }
        }
        let arguments: CommandArguments = HashMap::from([(
            "test:quest_object".to_owned(),
            Arc::new(QuestObject) as Arc<dyn mod_api::CommandArgumentType>,
        )]);
        let (t, _, problems) = command_tree(
            &[
                mod_api::Command::new("quests open <object:test:quest_object> <count:integer>"),
                mod_api::Command::new("quests rename <title:string> <count:integer>"),
            ],
            &Dispatcher::new(),
            &arguments,
        );
        assert!(problems.is_empty(), "{problems:?}");
        let quests = &t["children"]["quests"]["children"];
        let object = &quests["open"]["children"]["object"];
        assert_eq!(object["parser"], "brigadier:string");
        assert_eq!(object["properties"], json!({"type": "phrase"}));
        assert_eq!(object["custom_argument"], "test:quest_object");
        assert_eq!(object["suggestions"], "minecraft:ask_server");
        let title = &quests["rename"]["children"]["title"];
        assert_eq!(title["parser"], "brigadier:string");
        assert_eq!(title["properties"], json!({"type": "phrase"}));
        assert_eq!(
            title["suggestions"],
            Value::Null,
            "vanilla's string suggests nothing"
        );

        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &t, "native_mod_command", &arguments);
        let registries = sunrise_client::command::Registries::default();
        for (line, value) in [
            (r##"quests open "#Chapter_One" 3"##, "#chapter_one"),
            ("quests open 1A2B3C4D5E6F7A8B 3", "1a2b3c4d5e6f7a8b"),
            (r#"quests open "" 3"#, ""),
        ] {
            let parsed = dispatcher.parse(line, &registries);
            let (context, _) = parsed
                .executable()
                .unwrap_or_else(|e| panic!("{line}: {e:?}"));
            assert_eq!(
                context.get("object"),
                Some(&Arg::Custom {
                    kind: "test:quest_object".into(),
                    value: value.into(),
                }),
                "{line}"
            );
            assert_eq!(context.get("count"), Some(&Arg::Int(3)), "{line}");
        }
        for (line, value) in [
            (
                r#"quests rename "Getting \"Started\"" 2"#,
                r#"Getting "Started""#,
            ),
            (r"quests rename 'back\\slash' 2", r"back\slash"),
            ("quests rename '' 2", ""),
            ("quests rename Plain 2", "Plain"),
        ] {
            let parsed = dispatcher.parse(line, &registries);
            let (context, _) = parsed
                .executable()
                .unwrap_or_else(|e| panic!("{line}: {e:?}"));
            assert_eq!(
                context.get("title"),
                Some(&Arg::Str(value.into())),
                "{line}"
            );
            assert_eq!(context.get("count"), Some(&Arg::Int(2)), "{line}");
        }
        for line in [r#"quests rename "Getting 2"#, r#"quests open "Getting 2"#] {
            let error = dispatcher
                .parse(line, &registries)
                .executable()
                .unwrap_err();
            assert_eq!(
                error.message,
                Text::tr("parsing.quote.expected.end", &[]),
                "{line}"
            );
            assert_eq!(error.cursor, line.len(), "{line}");
        }
    }

    /// NeoForge's `RegisterCommandsEvent` registers into vanilla's dispatcher, whose literal
    /// merges by Brigadier's `CommandNode.addChild`: a mod usage under `/time` sits beside
    /// vanilla's, which stay; vanilla's node keeps its permission; an executor at a vanilla node
    /// replaces vanilla's, and a later one replaces that. A mod argument at a vanilla node's
    /// place, a mod argument on a vanilla literal and Sunrise's own tools are refused.
    #[test]
    fn usages_under_a_vanilla_root_merge_beside_its_usages() {
        let vanilla = sunrise_client::command::vanilla::session_dispatcher(
            &sunrise_client::gamerules::RuleRegistry::vanilla(),
            &[],
        );
        let commands = [
            mod_api::Command::new("time season <name:word>"),
            mod_api::Command::new("seed"),
            mod_api::Command::op("seed", 3),
            mod_api::Command::new("time set <time:integer>"),
            mod_api::Command::new("time <set:word>"),
            mod_api::Command::new("modprofile extra"),
            mod_api::Command::new("raidplus go"),
        ];
        let arguments = HashMap::new();
        let mut executors = HashMap::new();
        let (t, levels, problems) = command_tree_for(
            &commands,
            BaseTree::Merged(&vanilla),
            &arguments,
            false,
            Some(&mut executors),
        );
        assert_eq!(problems.len(), 3, "{problems:?}");
        for (problem, usage) in problems.iter().zip([
            "time set <time:integer>",
            "time <set:word>",
            "modprofile extra",
        ]) {
            assert!(problem.starts_with(&format!("`{usage}`")), "{problem}");
        }
        // A merged root keeps vanilla's level: only the mod's own root routes as a mod command.
        assert_eq!(levels.keys().collect::<Vec<_>>(), ["raidplus"]);
        let mut dispatcher = vanilla.clone();
        register_command_nodes(
            &mut dispatcher,
            &t,
            "native_mod_command",
            &arguments,
            Some(&executors),
        );
        let at = |permission| sunrise_client::command::Registries {
            permission,
            ..Default::default()
        };
        let run = |line: &str, permission| {
            dispatcher
                .parse(line, &at(permission))
                .executable()
                .map(|(context, command)| (command, context.executor.map(|key| key.slot())))
        };
        let vanilla_time = vanilla
            .parse("time set day", &at(2))
            .executable()
            .unwrap()
            .1;
        assert_eq!(run("time set day", 2), Ok((vanilla_time, None)));
        assert_eq!(
            run("time season spring", 2),
            Ok(("native_mod_command", Some(0)))
        );
        assert!(
            run("time season spring", 0).is_err(),
            "vanilla's /time keeps its permission"
        );
        assert!(run("seed", 2).is_err(), "the later usage's level runs it");
        assert_eq!(run("seed", 3), Ok(("native_mod_command", Some(2))));
        assert_eq!(run("raidplus go", 0), Ok(("native_mod_command", Some(6))));
    }

    /// WorldEdit-style `//set`: the usage `/set <x:word>` (typed `//set` with the chat slash) is a
    /// root literal `/set` with its argument, and the server dispatches it as typed.
    #[test]
    fn a_first_word_may_start_with_one_slash() {
        let (t, levels, problems) = tree(&[mod_api::Command::new("/set <x:word>")]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(levels.keys().collect::<Vec<_>>(), ["/set"]);
        assert_eq!(t["children"]["/set"]["type"], "literal");
        assert_eq!(t["children"]["/set"]["children"]["x"]["type"], "argument");
        for refused in ["//x", "/", "/ <x:word>", "a /b", "a /"] {
            let (_, levels, problems) = tree(&[mod_api::Command::new(refused)]);
            assert!(levels.is_empty(), "{refused}");
            assert!(
                problems.len() == 1 && problems[0].contains("is not a command word"),
                "{refused}: {problems:?}"
            );
        }
        let arguments = command_arguments();
        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &t, "native_mod_command", &arguments);
        let registries = sunrise_client::command::Registries::default();
        let parsed = dispatcher.parse("/set stone", &registries);
        let (context, _) = parsed.executable().expect("`//set stone` runs");
        assert_eq!(context.literals, ["/set"]);
        assert_eq!(context.get("x"), Some(&Arg::Str("stone".into())));
        assert!(
            dispatcher
                .parse("set stone", &registries)
                .executable()
                .is_err()
        );
    }

    #[test]
    fn client_command_tree_refuses_server_only_handle_kinds_and_binds_slots() {
        let arguments = command_arguments();
        let server_only = [
            mod_api::Command::new("c_player <target:player>"),
            mod_api::Command::new("c_players <targets:players>"),
            mod_api::Command::new("c_entity <target:entity>"),
            mod_api::Command::new("c_entities <targets:entities>"),
            mod_api::Command::new("c_dim <dim:dimension>"),
        ];
        let (refused_tree, _, refused_problems) =
            client_command_tree(&server_only, &arguments, None);
        assert_eq!(
            refused_problems.len(),
            server_only.len(),
            "{refused_problems:?}"
        );
        for root in ["c_player", "c_players", "c_entity", "c_entities", "c_dim"] {
            assert!(
                refused_tree["children"].get(root).is_none(),
                "{root} should be refused on the client"
            );
        }

        let client_safe = [
            mod_api::Command::new("c_word <w:word>"),
            mod_api::Command::new("c_text <t:text>"),
            mod_api::Command::new("c_int <n:integer>"),
            mod_api::Command::new("c_num <x:number>"),
            mod_api::Command::new("c_bool <b:boolean>"),
            mod_api::Command::new("c_item <i:item>"),
            mod_api::Command::new("c_bpos <p:block_pos>"),
            mod_api::Command::new("c_vec <v:vec3>"),
            mod_api::Command::new("c_vecu <v:vec3_uncentered>"),
            mod_api::Command::new("c_vec2 <v:vec2>"),
            mod_api::Command::new("c_vec2u <v:vec2_uncentered>"),
            mod_api::Command::new("c_col <c:column_pos>"),
            mod_api::Command::new("c_rot <r:rotation>"),
            mod_api::Command::new("c_nbt <n:nbt>"),
            mod_api::Command::new("c_id <id:id>"),
            mod_api::Command::new("c_mod <m:mod_id>"),
            mod_api::Command::new("c_res <r:resource:minecraft:item>"),
        ];
        let mut slots = CommandExecutorSlots::new();
        let (tree, _, problems) = client_command_tree(&client_safe, &arguments, Some(&mut slots));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(slots.len(), client_safe.len());

        let mut dispatcher = Dispatcher::new();
        register_command_nodes(
            &mut dispatcher,
            &tree,
            sunrise_client::command::MOD_COMMAND,
            &arguments,
            Some(&slots),
        );
        let reg = sunrise_client::command::Registries::default();
        let parsed = dispatcher.parse("c_int 42", &reg);
        let (ctx, _) = parsed.executable().unwrap();
        let key = ctx
            .executor
            .expect("client command node binds an executor key");
        assert_eq!(key.slot(), 2);
        assert!(dispatcher.owns_executor(key));

        // Rebuilding the client dispatcher invalidates keys from the previous generation.
        let mut rebuilt = Dispatcher::new();
        register_command_nodes(
            &mut rebuilt,
            &tree,
            sunrise_client::command::MOD_COMMAND,
            &arguments,
            Some(&slots),
        );
        assert!(!rebuilt.owns_executor(key));
    }

    #[test]
    #[ignore = "daily"]
    fn client_commands_execute_bound_typed_invocations_contain_panics_and_reject_stale_generations()
    {
        use mod_api::prelude::NetworkClientHost;
        use std::sync::atomic::AtomicUsize;

        struct DropBomb(Arc<AtomicBool>);
        impl Drop for DropBomb {
            fn drop(&mut self) {
                if !self.0.swap(true, Ordering::Relaxed) {
                    panic!("client command capture destructor fixture");
                }
            }
        }

        struct ClientCommandsMod {
            heard: Arc<Mutex<Vec<mod_api::CommandInvocation>>>,
            sibling: Arc<Mutex<Vec<mod_api::CommandInvocation>>>,
            recursed: Arc<AtomicUsize>,
            ping_pong: Arc<Mutex<Vec<&'static str>>>,
            dropped: Arc<AtomicBool>,
        }

        #[mod_api::__subscriptions]
        impl mod_api::Hooks for ClientCommandsMod {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.register_command_argument("machine", MachineArgument);
            }

            fn register_client_commands(
                &self,
                _: &mut dyn mod_api::ClientHost,
                event: &mut mod_api::RegisterClientCommandsEvent<'_>,
            ) {
                let heard = self.heard.clone();
                event.commands.push(
                    mod_api::Command::new(
                        "cmach <machine:client_cmd_fixture:machine> <pos:block_pos> [count:integer]",
                    )
                    .executes_client(move |_, invocation| {
                        heard.lock().unwrap().push(invocation.clone());
                        Some(1)
                    }),
                );
                event.commands.push(
                    mod_api::Command::new("cpanic")
                        .executes_client(|_, _| panic!("client command panic fixture")),
                );
                let sibling = self.sibling.clone();
                event.commands.push(
                    mod_api::Command::new("csibling <word:word>").executes_client(
                        move |_, invocation| {
                            sibling.lock().unwrap().push(invocation.clone());
                            Some(1)
                        },
                    ),
                );
                let recursed = self.recursed.clone();
                event
                    .commands
                    .push(
                        mod_api::Command::new("crecurse").executes_client(move |host, _| {
                            recursed.fetch_add(1, Ordering::Relaxed);
                            assert!(host.send_chat("/crecurse").is_ok());
                            Some(1)
                        }),
                    );
                let ping = self.ping_pong.clone();
                let pong = self.ping_pong.clone();
                event.commands.extend([
                    mod_api::Command::new("cping").executes_client(move |host, _| {
                        ping.lock().unwrap().push("ping");
                        for _ in 0..2 {
                            assert!(host.send_chat("/cpong").is_ok());
                        }
                        Some(1)
                    }),
                    mod_api::Command::new("cpong").executes_client(move |host, _| {
                        pong.lock().unwrap().push("pong");
                        for _ in 0..2 {
                            assert!(host.send_chat("/cping").is_ok());
                        }
                        assert!(host.send_chat("/csibling nested").is_ok());
                        Some(1)
                    }),
                ]);
                let bomb = DropBomb(self.dropped.clone());
                event.commands.extend([
                    mod_api::Command::new("capture").executes_client(move |_, _| {
                        let _capture = &bomb;
                        Some(1)
                    }),
                    mod_api::Command::new("shared local <value:integer>")
                        .executes_client(|_, _| Some(42)),
                    mod_api::Command::new("shared refused").executes_client(|_, _| None),
                ]);
                event
                    .commands
                    .push(mod_api::Command::op("cop", 2).executes_client(|_, _| Some(1)));
                let tagged = self.heard.clone();
                event.commands.push(
                    mod_api::Command::new(
                        "cbiome <biome:resource_or_tag:minecraft:worldgen/biome>",
                    )
                    .executes_client(move |_, invocation| {
                        tagged.lock().unwrap().push(invocation.clone());
                        Some(1)
                    }),
                );
                event.commands.push(
                    mod_api::Command::new("cserver <target:player>")
                        .executes_client(|_, _| Some(1)),
                );
            }
        }

        let heard = Arc::new(Mutex::new(Vec::new()));
        let sibling = Arc::new(Mutex::new(Vec::new()));
        let recursed = Arc::new(AtomicUsize::new(0));
        let ping_pong = Arc::new(Mutex::new(Vec::new()));
        let dropped = Arc::new(AtomicBool::new(false));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "client_cmd_fixture",
            Box::new(ClientCommandsMod {
                heard: heard.clone(),
                sibling: sibling.clone(),
                recursed: recursed.clone(),
                ping_pong: ping_pong.clone(),
                dropped: dropped.clone(),
            }),
        )]
        .into();

        let _ = mod_api::testing::rebuild_parts(|| -> Result<(), ()> {
            crate::game::api_values::install_parts(&mods).unwrap();
            mod_platform::testing::with_mods(mods, || {
                super::super::native_entity_refs::tests::with_client_game(|game| {
                    let _ = mod_api::__private::take_hook_panics();
                    let chat_tree = game.chat_tree_for(None);
                    assert!(chat_tree.find("cmach").is_some());
                    assert!(chat_tree.find("cpanic").is_some());
                    assert!(chat_tree.find("csibling").is_some());
                    // An operator-level client usage is in the tree behind its level (#5654).
                    let cop = chat_tree.find("cop").expect("an operator client command");
                    assert_eq!(chat_tree.nodes[cop].permission, 2);
                    assert!(chat_tree.find("cserver").is_none());
                    let tp = chat_tree.find("tp").expect("tp redirect preserved");
                    let tp_target = chat_tree.nodes[tp].redirect.expect("tp redirect preserved");
                    assert_eq!(chat_tree.nodes[tp_target].name(), "teleport");
                    assert_eq!(chat_tree.nodes[tp].permission, 2);

                    // Unregistered or rejected roots return false so they proceed to the server.
                    assert_eq!(game.run_client_native_command("unknown_root foo"), None);
                    // Below its level the local player can't run it; at the level it runs.
                    game.ui.registries.permission = 0;
                    assert_eq!(game.run_client_native_command("cop"), None);
                    game.ui.registries.permission = 2;
                    assert_eq!(
                        game.run_client_native_command("cop"),
                        Some(ClientCommandOutcome::Success(1))
                    );
                    game.ui.registries.permission = 0;
                    assert_eq!(game.run_client_native_command("cserver @a"), None);
                    // A resource-or-tag argument tells a `#tag` from an id (#5654).
                    for (text, tag, id) in [
                        ("cbiome #minecraft:is_ocean", true, "minecraft:is_ocean"),
                        ("cbiome minecraft:plains", false, "minecraft:plains"),
                    ] {
                        assert_eq!(
                            game.run_client_native_command(text),
                            Some(ClientCommandOutcome::Success(1)),
                            "{text}"
                        );
                        let invocation = heard.lock().unwrap().pop().unwrap();
                        assert_eq!(
                            invocation.argument("biome"),
                            Some(&mod_api::CommandArgumentValue::ResourceOrTag {
                                tag,
                                id: mod_api::Identifier::parse(id).unwrap(),
                            }),
                            "{text}"
                        );
                    }

                    // Typed client invocation parses custom arguments, coordinates and optionals.
                    assert_eq!(
                        game.run_client_native_command("cmach press 1 2 3 7"),
                        Some(ClientCommandOutcome::Success(1))
                    );
                    {
                        let heard = heard.lock().unwrap();
                        assert_eq!(heard.len(), 1);
                        assert_eq!(heard[0].literals, ["cmach"]);
                        assert_eq!(
                            heard[0].argument("machine"),
                            Some(&mod_api::CommandArgumentValue::Custom {
                                kind: "client_cmd_fixture:machine".into(),
                                value: "press".into(),
                            })
                        );
                        assert_eq!(
                            heard[0].argument("pos"),
                            Some(&mod_api::CommandArgumentValue::BlockPos([1, 2, 3]))
                        );
                        assert_eq!(
                            heard[0].argument("count"),
                            Some(&mod_api::CommandArgumentValue::Integer(7))
                        );
                    }

                    // A syntax error on a registered client root reports locally and never calls the executor.
                    let before_err = game.ui.chat.recent_lines(50).len();
                    assert_eq!(
                        game.run_client_native_command("cmach nope 1 2 3"),
                        Some(ClientCommandOutcome::Failed)
                    );
                    assert_eq!(heard.lock().unwrap().len(), 1);
                    assert!(game.ui.chat.recent_lines(50).len() > before_err);

                    // A panicking client executor disables only itself, emits a notice, and leaves siblings working.
                    let before_panic = game.ui.chat.recent_lines(50).len();
                    assert_eq!(
                        game.run_client_native_command("cpanic"),
                        Some(ClientCommandOutcome::Failed)
                    );
                    assert!(game.ui.chat.recent_lines(50).len() > before_panic);
                    let notices: Vec<_> = mod_api::__private::take_hook_panics()
                        .into_iter()
                        .filter(|notice| notice.mod_id == "client_cmd_fixture")
                        .collect();
                    assert_eq!(notices.len(), 1);
                    assert_eq!(notices[0].hook, "client_command.execute");

                    // Disabled executor is not invoked again, and sibling commands still run.
                    assert_eq!(
                        game.run_client_native_command("cpanic"),
                        Some(ClientCommandOutcome::Failed)
                    );
                    assert!(
                        mod_api::__private::take_hook_panics()
                            .into_iter()
                            .all(|notice| notice.mod_id != "client_cmd_fixture")
                    );
                    assert_eq!(
                        game.run_client_native_command("csibling ok"),
                        Some(ClientCommandOutcome::Success(1))
                    );
                    {
                        let sibling = sibling.lock().unwrap();
                        assert_eq!(sibling.len(), 1);
                        assert_eq!(
                            sibling[0].argument("word"),
                            Some(&mod_api::CommandArgumentValue::String("ok".into()))
                        );
                    }

                    // Rebuilding the client tree invalidates parsed executor keys from the previous generation.
                    let old_key = game
                        .native_client_commands
                        .dispatcher
                        .parse("cmach press 1 2 3", &game.ui.registries)
                        .executable()
                        .unwrap()
                        .0
                        .executor
                        .expect("client command binds an executor key");
                    assert!(
                        game.native_client_commands
                            .dispatcher
                            .owns_executor(old_key)
                    );
                    let mut server = Dispatcher::new();
                    let shared = server.literal(0, "shared");
                    let remote = server.literal(shared, "server");
                    let target = server.literal(0, "target");
                    let shared_alias = server.literal(0, "shared_alias");
                    server.requires(shared, 2);
                    server.requires(remote, 3);
                    server.requires_execution(remote, 3);
                    server.executes(remote, "remote");
                    server.redirect(remote, target);
                    server.redirect(shared_alias, shared);
                    // A real connection rebuild asks the client hook again, then a server-tree
                    // refresh merges the same local usage without claiming the shared root.
                    let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                    game.connect_local(client, "CommandsTest").unwrap();
                    game.remote.as_mut().unwrap().joined = true;
                    assert!(dropped.load(Ordering::Relaxed));
                    let notices: Vec<_> = mod_api::__private::take_hook_panics()
                        .into_iter()
                        .filter(|notice| notice.mod_id == "client_cmd_fixture")
                        .collect();
                    assert_eq!(notices.len(), 1);
                    assert_eq!(notices[0].hook, "client_command.drop");
                    let display = game.chat_tree_for(Some(&server));
                    let children = &display.nodes[shared].children;
                    assert!(
                        children
                            .iter()
                            .any(|&id| display.nodes[id].name() == "local")
                    );
                    assert!(children.contains(&remote));
                    assert_eq!(display.nodes[shared].permission, 2);
                    assert_eq!(display.nodes[remote].execution_permission, 3);
                    let remote_target = display.nodes[remote]
                        .redirect
                        .expect("server redirect preserved");
                    assert_eq!(display.nodes[remote_target].name(), "target");
                    let op_reg = sunrise_client::command::Registries {
                        permission: 3,
                        ..Default::default()
                    };
                    assert_eq!(
                        display
                            .parse("shared_alias server", &op_reg)
                            .executable()
                            .unwrap()
                            .1,
                        "remote"
                    );
                    assert!(
                        display
                            .parse("shared_alias local 7", &op_reg)
                            .executable()
                            .is_err()
                    );
                    let alias_suggestions: Vec<String> = display
                        .client_suggestions(&display.parse("shared_alias ", &op_reg), 13, &op_reg)
                        .1
                        .into_iter()
                        .map(|s| s.text)
                        .collect();
                    assert_eq!(alias_suggestions, ["server"]);

                    assert_eq!(game.run_client_native_command("shared server"), None);
                    assert_eq!(game.run_client_native_command("shared"), None);
                    assert_eq!(
                        game.run_client_native_command("shared local nope"),
                        Some(ClientCommandOutcome::Failed)
                    );
                    assert_eq!(
                        game.send_signed_chat_input("/shared local 7"),
                        Some(ClientCommandOutcome::Success(42))
                    );
                    assert_eq!(
                        game.send_signed_chat_input("/shared refused"),
                        Some(ClientCommandOutcome::Failed)
                    );
                    assert_eq!(
                        game.run_client_native_command("cpanic"),
                        Some(ClientCommandOutcome::Failed)
                    );
                    assert!(
                        mod_api::__private::take_hook_panics()
                            .into_iter()
                            .all(|notice| notice.mod_id != "client_cmd_fixture")
                    );
                    assert_eq!(
                        game.send_signed_chat_input("/csibling signed"),
                        Some(ClientCommandOutcome::Success(1))
                    );
                    game.send_unattended_command("/csibling unattended");
                    // Ordinary top-level Host submission intercepts locally without waiting for a frame.
                    assert!(NetworkClientHost::send_chat(game, "/csibling host_top").is_ok());
                    {
                        let sibling = sibling.lock().unwrap();
                        assert_eq!(sibling.len(), 4);
                        assert_eq!(
                            sibling[2].argument("word"),
                            Some(&mod_api::CommandArgumentValue::String("unattended".into()))
                        );
                        assert_eq!(
                            sibling[3].argument("word"),
                            Some(&mod_api::CommandArgumentValue::String("host_top".into()))
                        );
                    }
                    assert!(game.command_prompt_message().is_none());

                    // Submitting the same executor defers the attempt, then refuses it locally.
                    assert!(NetworkClientHost::send_chat(game, "/crecurse").is_ok());
                    assert_eq!(recursed.load(Ordering::Relaxed), 1);
                    let chain = game.native_client_commands.pending_commands[0]
                        .chain
                        .clone();
                    game.native_frame_end().unwrap();
                    assert_eq!(recursed.load(Ordering::Relaxed), 1);
                    assert!(game.native_client_commands.pending_commands.is_empty());
                    for (line, kind) in [
                        ("crecurse", PendingClientCommandKind::Signed),
                        ("/crecurse", PendingClientCommandKind::Unattended),
                    ] {
                        assert_eq!(
                            game.submit_client_command(line, kind, Some(chain.clone())),
                            Ok(Some(ClientCommandOutcome::Failed))
                        );
                    }
                    assert!(game.command_prompt_message().is_none());

                    // Both branches may run the same child, but neither may re-enter an
                    // ancestor. Rebuilding the tree between frames doesn't reset the ancestry.
                    assert!(NetworkClientHost::send_chat(game, "/cping").is_ok());
                    assert_eq!(*ping_pong.lock().unwrap(), ["ping"]);
                    game.native_frame_end().unwrap();
                    assert_eq!(*ping_pong.lock().unwrap(), ["ping", "pong", "pong"]);
                    assert_eq!(sibling.lock().unwrap().len(), 4);
                    game.chat_tree_for(Some(&server));
                    let before_refusals = game.ui.chat.recent_lines(50).len();
                    game.native_frame_end().unwrap();
                    assert_eq!(*ping_pong.lock().unwrap(), ["ping", "pong", "pong"]);
                    assert_eq!(game.ui.chat.recent_lines(50).len(), before_refusals + 4);
                    assert!(game.native_client_commands.pending_commands.is_empty());
                    {
                        let sibling = sibling.lock().unwrap();
                        let nested = Some(mod_api::CommandArgumentValue::String("nested".into()));
                        assert_eq!(sibling.len(), 6);
                        for invocation in &sibling[4..] {
                            assert_eq!(invocation.argument("word"), nested.as_ref());
                        }
                    }
                    // A fresh player submission has no ancestors and still intercepts locally.
                    assert_eq!(
                        game.send_signed_chat_input("/cping"),
                        Some(ClientCommandOutcome::Success(1))
                    );
                    assert_eq!(*ping_pong.lock().unwrap(), ["ping", "pong", "pong", "ping"]);

                    assert!(
                        !game
                            .native_client_commands
                            .dispatcher
                            .owns_executor(old_key)
                    );
                });
            });
            Ok(())
        });
    }
}

#[cfg(test)]
pub(super) mod fake_action_tests {
    use super::super::Game;
    use super::{Acting, ActionOutcome, ActionRefused, FakeEffect, PlayerAction};
    use mod_api::{BlockHit, Hand, PlayerRef};
    use sunrise_client::items::ItemStack;

    /// A server with a real guest (uuid 1) and a fake (uuid 2) standing at (0.5, 64, 0.5) on
    /// a stone floor, holding `item`, in game mode `mode`: the fake's handle and both conns.
    pub(in crate::game) fn fake(
        game: &mut Game<'_>,
        item: Option<&str>,
        mode: i32,
    ) -> (PlayerRef, u32, u32) {
        super::super::bossbars::audience_tests::add_real_and_fake(game);
        game.options.dedicated = Some(Vec::new());
        for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
            let mut chunk = sunrise_client::chunk::Chunk::empty(x, z);
            chunk.lit = true;
            game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
        }
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        for x in -3..=3 {
            for z in -3..=3 {
                game.world.set(x, 63, z, stone).unwrap();
            }
        }
        let level = game.level(game.dim.id()).unwrap();
        let player = game.fake_player(level, 2, "fake").unwrap();
        let host = game.lan.as_mut().unwrap();
        let conn = |uuid| {
            host.hub
                .guests
                .iter()
                .find(|g| g.uuid == uuid)
                .unwrap()
                .conn
        };
        let (fake, real) = (conn(2), conn(1));
        host.hub.guests.update_all(|g| {
            g.position = [0.5, 64.0, 0.5];
            g.game_mode = mode;
            g.abilities.update_for_mode(mode);
            g.inventory[g.selected] = item.map(|i| ItemStack::new(i, 1));
        });
        (player, fake, real)
    }

    fn place(game: &mut Game<'_>, pos: [i32; 3], state: &str) {
        // `name[key=value,...]`, as a command writes a state.
        let (name, props) = state.split_once('[').unwrap_or((state, "]"));
        let properties = props
            .trim_end_matches(']')
            .split(',')
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        let state = sunrise_world::chunk::BlockState {
            name: name.into(),
            properties,
        };
        let id = game.pack.assets.registry.resolve(&state).unwrap();
        game.world.set(pos[0], pos[1], pos[2], id).unwrap();
    }

    /// A click on the top of the block at `pos`.
    fn on_top(pos: [i32; 3]) -> PlayerAction {
        let [x, y, z] = pos.map(f64::from);
        PlayerAction::UseOn {
            hand: Hand::Main,
            pos,
            hit: BlockHit::new([x + 0.5, y + 1.0, z + 0.5], [0, 1, 0], false),
        }
    }

    use super::super::{
        boats::Boat, minecarts::CART_KEY_Y, minecarts::CartKind, minecarts::Minecart,
        respawn_anchor::RespawnConfig, stations::Kind, stations::Station,
    };

    fn held(game: &Game<'_>, uuid: u128) -> Option<ItemStack> {
        let host = game.lan.as_ref().unwrap();
        let g = host.hub.guests.iter().find(|g| g.uuid == uuid).unwrap();
        g.inventory[g.selected].clone()
    }

    /// A wandering trader at `at`, entity id `id`, and its handle.
    fn trader(game: &mut Game<'_>, id: u32, at: [f64; 3]) -> mod_api::EntityRef {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:wandering_trader")
            .unwrap()
            .clone();
        let [x, y, z] = at;
        game.mobs.push(sunrise_client::mobs::Mob::new(
            id,
            kind,
            sunrise_core::geometry::Vec3::new(x, y, z),
            0.0,
        ));
        mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), id)
    }

    /// A fake player's use on a trader frees it again: the session the use
    /// opened closes through the merchant's own close path, so the trader is
    /// not left trading and the next use opens it as the first did.
    #[test]
    #[ignore = "daily"]
    fn a_trader_a_fake_player_used_trades_with_the_next_one() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:stick"), 0);
            let id = 6000;
            let target = trader(game, id, [2.5, 64.0, 0.5]);
            let interact = PlayerAction::InteractOn {
                hand: Hand::Main,
                target,
                location: [0.0, 0.5, 0.0],
            };
            assert_eq!(
                game.player_perform_action(player, interact),
                Ok(ActionOutcome::Consumed)
            );
            let mob = game.mobs.iter().find(|m| m.id == id).unwrap();
            // With no offers the menu never opens, and this test would say nothing.
            let offers = mob.extra.trade.as_ref().map_or(0, |t| t.offers.len());
            assert!(offers > 0, "a wandering trader has offers");
            assert!(!mob.extra.flag2, "the trader is not left trading");
            assert!(game.station.is_none(), "the session is not left open");
            assert_eq!(
                game.player_perform_action(player, interact),
                Ok(ActionOutcome::Consumed),
                "the next use opens it again",
            );
        });
    }

    /// A chest minecart, a hopper minecart and a chest boat: the items stay
    /// on the vehicle, and the menu's stand-in is out of the level, where a
    /// transfer would edit it instead of the vehicle.
    #[test]
    #[ignore = "daily"]
    fn a_vehicles_chest_a_fake_player_used_keeps_its_items_and_no_stand_in() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, fake_conn, _) = fake(game, None, 0);
            let at = sunrise_core::geometry::Vec3::new(2.5, 64.0, 0.5);
            let diamonds = Some(ItemStack::new("minecraft:diamond", 5));
            let mut vehicles = Vec::new();
            for (id, kind) in [(6100, CartKind::Chest), (6101, CartKind::Hopper)] {
                let mut cart = Minecart::new(id, kind, at);
                cart.items[3] = diamonds.clone();
                vehicles.push((id, false));
                game.carts.push(cart);
            }
            let mut boat = Boat::new(6102, "oak".into(), at, 0.0).with_chest();
            boat.items[3] = diamonds.clone();
            // ChestBoat.interact opens the chest for a sneaking player.
            vehicles.push((6102, true));
            game.boats.push(boat);

            for (id, sneaking) in vehicles {
                let target = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), id);
                let interact = PlayerAction::InteractOn {
                    hand: Hand::Main,
                    target,
                    location: [0.0, 0.5, 0.0],
                };
                // Twice: the second use is the next one finding it as it was.
                for _ in 0..2 {
                    game.lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .update_guest(fake_conn, |guest| guest.sneaking = sneaking)
                        .unwrap();
                    assert_eq!(
                        game.player_perform_action(player, interact),
                        Ok(ActionOutcome::Consumed)
                    );
                    let key = (i32::MAX - 15, CART_KEY_Y, id as i32);
                    assert!(
                        game.world.block_entities.get(&key).is_none(),
                        "vehicle {id} left a stand-in",
                    );
                    assert!(game.container.is_none());
                    let items = game
                        .carts
                        .iter()
                        .find(|c| c.id == id)
                        .map(|c| &c.items)
                        .or_else(|| game.boats.iter().find(|b| b.id == id).map(|b| &b.items))
                        .unwrap();
                    assert_eq!(items[3], diamonds, "vehicle {id} lost its items");
                }
            }
        });
    }

    /// A real guest keeps its vehicle menu over a cart a fake player then
    /// uses: the refused open closes only a stand-in nobody is looking at,
    /// and the staging does not reset what the viewer's menu is editing.
    #[test]
    #[ignore = "daily"]
    fn a_viewer_keeps_the_vehicle_menu_a_fake_player_used() {
        use super::super::minecarts::{CART_KEY_Y, CartKind, Minecart};
        use sunrise_client::containers::BlockEntity;
        use sunrise_client::lan::menu::Slots;
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, fake_conn, real_conn) = fake(game, None, 0);
            let at = sunrise_core::geometry::Vec3::new(2.5, 64.0, 0.5);
            let mut cart = Minecart::new(6110, CartKind::Chest, at);
            cart.items[3] = Some(ItemStack::new("minecraft:diamond", 5));
            game.carts.push(cart);
            let gi = game
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .position(|g| g.conn == real_conn)
                .unwrap();
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(real_conn, |guest| guest.position = [0.5, 64.0, 0.5])
                .unwrap();
            assert_eq!(
                game.guest_interact(real_conn, gi, 6110, Hand::Main, [2.5, 64.5, 0.5]),
                Ok(true),
                "the guest's use opens the vehicle's menu",
            );
            let key = (i32::MAX - 15, CART_KEY_Y, 6110);
            let viewer = |game: &Game<'_>| {
                game.lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(real_conn)
                    .unwrap()
                    .menu
                    .as_ref()
                    .is_some_and(|m| m.slots == Slots::Block(key))
            };
            assert!(viewer(game), "the guest's use opened the vehicle's menu");
            assert!(game.world.block_entities.contains_key(&key));
            // The viewer's live edit: its menu's clicks write through to the
            // stand-in (`source.write` after every click).
            let emerald = Some(ItemStack::new("minecraft:emerald", 2));
            if let Some(sunrise_client::containers::BlockEntity::Chest(items)) =
                game.world.block_entities.get_mut(&key)
            {
                items[4] = emerald.clone();
            }
            let target = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), 6110);
            assert_eq!(
                game.player_perform_action(
                    player,
                    PlayerAction::InteractOn {
                        hand: Hand::Main,
                        target,
                        location: [0.0, 0.5, 0.0],
                    },
                ),
                Ok(ActionOutcome::Consumed)
            );
            assert!(viewer(game), "the fake's use kept the viewer's menu");
            assert!(
                game.world.block_entities.contains_key(&key),
                "the fake's use kept the stand-in the menu backs",
            );
            let BlockEntity::Chest(items) = &game.world.block_entities[&key] else {
                unreachable!()
            };
            assert_eq!(items[3], Some(ItemStack::new("minecraft:diamond", 5)));
            assert_eq!(items[4], emerald, "the staging kept the viewer's edit");
            game.guest_close_all(real_conn);
            assert!(!game.world.block_entities.contains_key(&key));
            let items = &game.carts.iter().find(|c| c.id == 6110).unwrap().items;
            assert_eq!(
                items[3],
                Some(ItemStack::new("minecraft:diamond", 5)),
                "the viewer's close put the items back",
            );
            assert_eq!(items[4], emerald, "the viewer's edit reached the vehicle");
            assert!(
                game.lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(fake_conn)
                    .unwrap()
                    .menu
                    .is_none()
            );
        });
    }

    /// A fake player's click on a shulker box or an ender chest is refused
    /// at the open: no open count climbs, no stand-in chest enters the level
    /// and no lid opens.
    #[test]
    #[ignore = "daily"]
    fn a_shulker_box_and_an_ender_chest_a_fake_player_used_are_left_shut() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, ..) = fake(game, None, 0);
            let shulker = [1, 64, 0];
            place(game, shulker, "minecraft:shulker_box");
            let count =
                |game: &Game<'_>| game.be_anims.shulkers.get(&shulker).map(|s| s.open_count);
            assert_eq!(
                game.player_perform_action(player, on_top(shulker)),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Menu))
            );
            assert!(
                count(game).is_none_or(|c| c == 0),
                "the use is refused at the open, nothing opened"
            );
            assert!(game.container.is_none());
            assert_eq!(
                game.player_perform_action(player, on_top(shulker)),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Menu)),
                "the next use is refused the same way",
            );
            assert!(
                count(game).is_none_or(|c| c == 0),
                "the count doesn't climb"
            );

            let diamonds = Some(ItemStack::new("minecraft:diamond", 5));
            game.ender_items[0] = diamonds.clone();
            let ender = [2, 64, 0];
            place(game, ender, "minecraft:ender_chest");
            let key = (ender[0], ender[1], ender[2]);
            // A raw block state has no stand-in. Establish the closed state
            // whose preservation this refused action is meant to check.
            game.open_ender_chest(key);
            game.close_ender_chest();
            game.container = None;
            assert_eq!(
                game.player_perform_action(player, on_top(ender)),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Menu))
            );
            assert!(game.ender_open.is_none());
            match game.world.block_entities.get(&key) {
                Some(sunrise_client::chunk::block_entity::BlockEntity::Chest(items)) => assert!(
                    items.iter().all(Option::is_none),
                    "the player's items stayed"
                ),
                other => panic!("the ender chest lost its block entity: {other:?}"),
            }
            assert_eq!(
                game.ender_items[0], diamonds,
                "the server's own ender items changed"
            );
            assert!(!game.lid_openers.contains_key(&key), "its lid stayed open");
        });
    }

    /// Each field holds a value the use can't have written, so a swap that
    /// forgets one fails whether the use changed it or not.
    #[test]
    #[ignore = "daily"]
    fn the_servers_own_player_state_comes_back_after_a_fake_use() {
        use mod_api::PlayerActionSetup;
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, ..) = fake(game, None, 0);
            let dim = game.dim;
            let config = RespawnConfig {
                dim,
                pos: [1, 2, 3],
                yaw: 0.0,
                pitch: 0.0,
                forced: false,
            };
            let spawn = sunrise_core::geometry::Vec3::new(1.0, 2.0, 3.0);
            game.station = Some(Station::new_kind(Kind::Anvil, [4, 5, 6]));
            game.craft_table = Some([7, 8, 9]);
            game.experience.level = 7;
            game.respawn_config = Some(config);
            game.spawn = Some(spawn);
            // An ender inventory left open goes back while the fake player
            // acts, and its stand-in returns afterwards, as a guest's turn does.
            let stand_in = (12, 64, 8);
            game.open_ender_chest(stand_in);
            game.container = None;

            place(game, [1, 64, 0], "minecraft:crafting_table");
            assert_eq!(
                game.player_perform_action(player, on_top([1, 64, 0])),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Menu))
            );
            assert_eq!(
                game.station.as_ref().map(|s| (s.kind, s.pos)),
                Some((Kind::Anvil, [4, 5, 6])),
            );
            assert_eq!(game.craft_table, Some([7, 8, 9]));
            assert_eq!(game.experience.level, 7);
            assert_eq!(game.respawn_config, Some(config));
            assert_eq!(game.spawn, Some(spawn));
            assert_eq!(game.ender_open, Some(stand_in));
            assert!(game.world.block_entities.get(&stand_in).is_some());

            let setup = PlayerActionSetup::new(
                Some(crate::game::api_values::test_snapshot(&ItemStack::new(
                    "minecraft:fishing_rod",
                    1,
                ))),
                [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                0.0,
                0.0,
                false,
            );
            let rod = player
                .perform_action_scoped(game, PlayerAction::UseItem { hand: Hand::Main }, setup)
                .unwrap();
            assert_eq!(rod.outcome, ActionOutcome::Consumed, "the rod casts");
            assert!(game.hook.is_none(), "no hook is left for nobody to reel in");
        });
    }

    /// A machine acts through its fake player from its level tick (#5750): the transaction the
    /// level tick's dispatch opens around the listener doesn't make the action `Busy`, while
    /// one the listener opens itself still does. Pre uses the scoped action Create's deployer
    /// uses; post the plain one.
    #[test]
    #[ignore = "daily"]
    fn a_level_tick_listener_acts_through_its_fake_player() {
        use super::super::native_entity_refs::tests::with_game;
        use mod_api::{
            Hooks, HostSide, HostTransaction, LevelTickPostEvent, LevelTickPreEvent, ModEntry,
            PlayerActionSetup,
        };
        use std::sync::{Arc, Mutex};

        type Answer = Result<ActionOutcome, ActionRefused>;
        #[derive(Default)]
        struct Seen {
            player: Option<PlayerRef>,
            /// The scoped action from `level_tick_pre`, with the log it holds.
            setup: Option<PlayerActionSetup>,
            /// Whether the dispatch's transaction was open, then each answer: the action
            /// straight from the listener, then the one inside the listener's own transaction.
            answers: Vec<(&'static str, bool, Answer, Answer)>,
        }
        struct Deployer(Arc<Mutex<Seen>>);

        #[mod_api::__subscriptions]
        impl Hooks for Deployer {
            fn level_tick_pre(&self, host: HostSide, _: &mut LevelTickPreEvent) {
                let HostSide::Server(host) = host else { return };
                let mut seen = self.0.lock().unwrap();
                let (Some(player), Some(setup)) = (seen.player, seen.setup.clone()) else {
                    return;
                };
                let boundary = !host.transaction_matches(None);
                let direct = player
                    .perform_action_scoped(host, on_top([0, 63, 1]), setup.clone())
                    .map(|result| result.outcome);
                let nested = HostTransaction::transaction(host, |host, _| {
                    Ok::<_, ()>(
                        player
                            .perform_action_scoped(host, on_top([0, 63, -1]), setup)
                            .map(|result| result.outcome),
                    )
                })
                .unwrap();
                seen.answers.push(("pre", boundary, direct, nested));
            }
            fn level_tick_post(&self, host: HostSide, _: &mut LevelTickPostEvent) {
                let HostSide::Server(host) = host else { return };
                let mut seen = self.0.lock().unwrap();
                let Some(player) = seen.player else { return };
                let boundary = !host.transaction_matches(None);
                let direct = player.perform_action(host, on_top([1, 63, 0]));
                let nested = HostTransaction::transaction(host, |host, _| {
                    Ok::<_, ()>(player.perform_action(host, on_top([-1, 63, 0])))
                })
                .unwrap();
                seen.answers.push(("post", boundary, direct, nested));
            }
        }

        let seen = Arc::new(Mutex::new(Seen::default()));
        let mods = vec![ModEntry::new("deployer", Box::new(Deployer(seen.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let (player, ..) = fake(game, Some("minecraft:oak_log"), 0);
                seen.lock().unwrap().player = Some(player);
                seen.lock().unwrap().setup = Some(PlayerActionSetup::new(
                    Some(crate::game::api_values::test_snapshot(&ItemStack::new(
                        "minecraft:oak_log",
                        1,
                    ))),
                    [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                    0.0,
                    60.0,
                    false,
                ));
                let level = game.level(game.dim.id()).unwrap();
                game.native_level_tick_pre(level);
                game.native_level_tick_post(level);
                let used = |stage| {
                    (
                        stage,
                        true,
                        Ok(ActionOutcome::Consumed),
                        Err(ActionRefused::Busy),
                    )
                };
                assert_eq!(seen.lock().unwrap().answers, [used("pre"), used("post")]);
                let log = game.level_block_property(level, [0, 64, 1], "axis");
                assert_eq!(
                    log,
                    Some("y"),
                    "the pre listener's scoped use placed its log"
                );
                let log = game.level_block_property(level, [1, 64, 0], "axis");
                assert_eq!(log, Some("y"), "the post listener's use placed the log");
                assert_eq!(held(game, 2), None);
                assert!(!game.has_open_transaction());
            });
        });
    }

    /// A mod's `block_interaction_range` (#5824) decides a block's reach: a use 10 blocks away is
    /// out of reach for vanilla, and goes through once a mod answers for that block, as a
    /// remote-access block lets a player act on the blocks around it. A menu's still-valid
    /// reach asks the same.
    #[test]
    #[ignore = "daily"]
    fn a_mods_interaction_range_lets_a_player_act_on_a_far_block() {
        use super::super::native_entity_refs::tests::with_game;
        use mod_api::{BlockInteractionRangeEvent, Hooks, ModEntry};

        const FAR: [i32; 3] = [10, 63, 0];
        struct RemoteReach;
        #[mod_api::__subscriptions]
        impl Hooks for RemoteReach {
            fn block_interaction_range(
                &self,
                _: &dyn mod_api::LoopContext,
                event: &mut BlockInteractionRangeEvent,
            ) {
                let [x, y, z] = FAR;
                if event.pos == mod_api::BlockPos::new(x, y, z) {
                    event.within = true;
                }
            }
        }

        // The client library reaches the hook through the game's installed callbacks.
        crate::entry::install_client_hooks();
        with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:oak_log"), 0);
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(FAR[0], FAR[1], FAR[2], stone).unwrap();
            let far = |game: &Game<'_>, pos: [i32; 3]| {
                let host = game.lan.as_ref().unwrap();
                let g = host.hub.guests.iter().find(|g| g.uuid == 2).unwrap();
                g.within_interaction_range(&game.loop_context, pos, 4.0)
            };
            assert_eq!(
                game.player_perform_action(player, on_top(FAR)),
                Err(ActionRefused::OutOfReach)
            );
            assert!(!far(game, FAR));
            let mods = vec![ModEntry::new("remote_reach", Box::new(RemoteReach))].into();
            mod_platform::testing::with_mods(mods, || {
                assert!(far(game, FAR));
                assert!(
                    !far(game, [10, 63, 3]),
                    "only the block the mod answers for"
                );
                assert_eq!(
                    game.player_perform_action(player, on_top(FAR)),
                    Ok(ActionOutcome::Consumed)
                );
            });
            let level = game.level(game.dim.id()).unwrap();
            let log = game.level_block_property(level, [10, 64, 0], "axis");
            assert_eq!(log, Some("y"), "the far use placed the log");
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_fake_uses_items_through_the_players_own_path() {
        use super::super::native_entity_refs::tests::with_game;
        // A log on stone: placed, one used up.
        with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:oak_log"), 0);
            assert_eq!(
                game.player_perform_action(player, on_top([1, 63, 0])),
                Ok(ActionOutcome::Consumed)
            );
            let level = game.level(game.dim.id()).unwrap();
            let log = game.level_block_property(level, [1, 64, 0], "axis");
            assert_eq!(log, Some("y"), "placed on a top face, standing");
            assert_eq!(held(game, 2), None);
        });
        // A bucket clicked on a block passes, as vanilla's does (BucketItem has no useOn), and
        // empties when used, by its ray from the eyes (looking straight down here).
        with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:water_bucket"), 0);
            game.lan.as_mut().unwrap().hub.guests.update_all(|g| {
                g.pitch = 90.0;
            });
            assert_eq!(
                game.player_perform_action(player, on_top([1, 63, 0])),
                Ok(ActionOutcome::Passed)
            );
            let water = PlayerAction::UseItem { hand: Hand::Main };
            assert_eq!(
                game.player_perform_action(player, water),
                Ok(ActionOutcome::Consumed)
            );
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(
                game.level_block(level, [0, 64, 0])
                    .and_then(mod_api::__private::block_key_id),
                Some("minecraft:water")
            );
            assert_eq!(
                held(game, 2).map(|s| s.item.to_string()).as_deref(),
                Some("minecraft:bucket")
            );
        });
        // A pearl thrown into the air: one used up.
        with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:ender_pearl"), 0);
            let pearl = PlayerAction::UseItem { hand: Hand::Main };
            assert_eq!(
                game.player_perform_action(player, pearl),
                Ok(ActionOutcome::Consumed)
            );
            assert_eq!(held(game, 2), None);
        });
    }

    /// A zombie standing at `at`, entity id `id`, and its handle.
    fn zombie(game: &mut Game<'_>, id: u32, at: [f64; 3]) -> mod_api::EntityRef {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let [x, y, z] = at;
        game.mobs.push(sunrise_client::mobs::Mob::new(
            id,
            kind,
            sunrise_core::geometry::Vec3::new(x, y, z),
            0.0,
        ));
        mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), id)
    }

    fn health(game: &Game<'_>, id: u32) -> f32 {
        game.mobs.iter().find(|m| m.id == id).unwrap().health
    }

    #[test]
    #[ignore = "daily"]
    fn scoped_fake_use_returns_items_and_restores_persistent_state() {
        use mod_api::PlayerActionSetup;
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, conn, _) = fake(game, Some("minecraft:diamond"), 1);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(conn, |guest| {
                    guest.yaw = 31.0;
                    guest.pitch = 12.0;
                    guest.selected = 3;
                    guest.inventory[3] = Some(ItemStack::new("minecraft:emerald", 4));
                    guest.offhand = Some(ItemStack::new("minecraft:apple", 2));
                    guest.sneaking = true;
                    guest.previous_game_mode = Some(3);
                    guest.motion.as_mut().unwrap().shift_key_down = true;
                })
                .unwrap();
            game.lan
                .as_mut()
                .unwrap()
                .play
                .get_mut(&conn)
                .unwrap()
                .cooldowns
                .add("minecraft:bucket".into(), 100);
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            game.mobs.push(sunrise_client::mobs::Mob::new(
                5000,
                kind,
                sunrise_core::geometry::Vec3::new(2.0, 64.0, 0.5),
                0.0,
            ));
            let target = game.entity(5000).unwrap();
            let setup = PlayerActionSetup::new(
                Some(crate::game::api_values::test_snapshot(&ItemStack::new(
                    "minecraft:bucket",
                    2,
                ))),
                [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                170.0,
                -15.0,
                false,
            );
            let action = PlayerAction::InteractOn {
                hand: Hand::Main,
                target,
                location: [0.0, 0.5, 0.0],
            };
            let result = player.perform_action_scoped(game, action, setup).unwrap();
            assert_eq!(result.outcome, ActionOutcome::Consumed);
            assert_eq!(
                result.held,
                Some(crate::game::api_values::test_snapshot(&ItemStack::new(
                    "minecraft:bucket",
                    1
                )))
            );
            assert_eq!(
                result.other,
                vec![crate::game::api_values::test_snapshot(&ItemStack::new(
                    "minecraft:milk_bucket",
                    1
                ))]
            );
            let guest = game.lan.as_ref().unwrap().hub.guest(conn).unwrap();
            assert_eq!(guest.position, [0.5, 64.0, 0.5]);
            assert_eq!(
                (guest.yaw, guest.pitch, guest.selected, guest.game_mode),
                (31.0, 12.0, 3, 1)
            );
            assert_eq!(guest.previous_game_mode, Some(3));
            assert_eq!(guest.offhand, Some(ItemStack::new("minecraft:apple", 2)));
            assert_eq!(
                guest.inventory[3],
                Some(ItemStack::new("minecraft:emerald", 4))
            );
            assert!(guest.sneaking && guest.motion.as_ref().unwrap().shift_key_down);
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(game.fake_player(level, 2, "fake"), Some(player));
            // The original cooldown is still in force, not the temporary action's timer.
            assert!(
                game.lan.as_ref().unwrap().play[&conn]
                    .cooldowns
                    .is_on_cooldown("minecraft:bucket")
            );
            assert_eq!(game.lan.as_ref().unwrap().hub.guests.len(), 2);
        });
    }

    /// A scoped attack with `full_attack_charge` deals the sword's full damage, one without it
    /// only the empty charge's share, and the fake's own ticker comes back after each.
    #[test]
    #[ignore = "daily"]
    fn scoped_fake_attack_with_full_charge_deals_the_weapons_full_damage() {
        use super::super::native_entity_refs::tests::{run_fresh_cases, with_game};
        use mod_api::PlayerActionSetup;
        if run_fresh_cases(
            "game::native_server::fake_action_tests::scoped_fake_attack_with_full_charge_deals_the_weapons_full_damage",
            "SUNRISE_SCOPED_ATTACK_CASE",
            &["attack"],
        ) {
            return;
        }
        with_game(|game| {
            let (player, conn, _) = fake(game, None, 0);
            game.as_guest(conn, |g| g.attack_ticks = 3).unwrap();
            let charged = zombie(game, 5000, [2.0, 64.0, 0.5]);
            // Out of a sweep's reach of the first.
            let uncharged = zombie(game, 5001, [-1.5, 64.0, 0.5]);
            let mut attack = |target, full_attack_charge| {
                let mut setup = PlayerActionSetup::new(
                    Some(crate::game::api_values::test_snapshot(&ItemStack::new(
                        "minecraft:diamond_sword",
                        1,
                    ))),
                    [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                    0.0,
                    0.0,
                    false,
                );
                setup.full_attack_charge = full_attack_charge;
                let result = player
                    .perform_action_scoped(game, PlayerAction::Attack { target }, setup)
                    .unwrap();
                assert_eq!(result.outcome, ActionOutcome::Consumed);
                assert_eq!(
                    game.as_guest(conn, |g| g.attack_ticks).unwrap(),
                    3,
                    "the fake's own ticker comes back"
                );
            };
            attack(charged, true);
            attack(uncharged, false);
            // Diamond sword: 7 damage and 4 + (-2.4f32) attack speed. Player.attack samples
            // the empty ticker at 0.5, then CombatRules applies the zombie's 2 armour.
            let delay = (1.0 / (4.0 + f64::from(-2.4f32)) * 20.0) as f32;
            let charge = 0.5f32 / delay;
            let after_hit = |damage: f32| {
                let armor = (2.0 - damage / 2.0).max(2.0 * 0.2);
                20.0 - damage * (1.0 - armor / 25.0)
            };
            assert_eq!(health(game, 5000), after_hit(7.0));
            assert_eq!(
                health(game, 5001),
                after_hit(7.0 * (0.2 + charge * charge * 0.8))
            );
        });
    }

    /// An armor stand holding a saved item this build doesn't have hands it to a scoped action:
    /// the stack can't cross, so it goes back into the world where the fake stood, unchanged,
    /// with the default pickup delay, and the result counts it instead of panicking.
    #[test]
    #[ignore = "daily"]
    fn scoped_fake_use_returns_an_unrepresentable_stack_to_the_world() {
        use mod_api::PlayerActionSetup;
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, _, _) = fake(game, None, 0);
            let mut stand = sunrise_client::entities::ArmorStand::new(
                5001,
                sunrise_core::geometry::Vec3::new(2.0, 64.0, 0.5),
                0.0,
            );
            stand.equipment[0] = Some(ItemStack::new("test:missing", 1));
            game.armor_stands.push(stand);
            let target = game.entity(5001).unwrap();
            let setup = PlayerActionSetup::new(
                None,
                [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                90.0,
                0.0,
                false,
            );
            let action = PlayerAction::InteractOn {
                hand: Hand::Main,
                target,
                location: [0.0, 1.5, 0.0],
            };
            let dropped = game.dropped.len();
            let result = player.perform_action_scoped(game, action, setup).unwrap();
            assert_eq!(result.outcome, ActionOutcome::Consumed);
            assert_eq!((result.held, result.other.len()), (None, 0));
            assert_eq!(result.unrepresentable, 1);
            assert_eq!(game.armor_stands.last().unwrap().equipment[0], None);
            assert_eq!(game.dropped.len(), dropped + 1);
            let item = game.dropped.last().unwrap();
            assert_eq!(item.stack, ItemStack::new("test:missing", 1));
            assert_eq!(item.pickup_delay, 10);
            assert_eq!((item.position.x, item.position.z), (0.5, 0.5));
        });
    }

    /// A join listener that keeps item entities out discards the returned stack as it would any
    /// drop: no item entity is left, and the result does not count it.
    #[test]
    #[ignore = "daily"]
    fn a_vetoed_return_is_not_counted() {
        use mod_api::{EntityJoinLevelEvent, Flow, Hooks, PlayerActionSetup, ServerHost};
        struct KeepItemsOut;
        #[mod_api::__subscriptions]
        impl Hooks for KeepItemsOut {
            fn entity_join_level(
                &self,
                _: &mut dyn ServerHost,
                event: &mut EntityJoinLevelEvent<'_>,
            ) -> Flow {
                if event.kind == "minecraft:item" {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
        }
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "keep-items-out",
            Box::new(KeepItemsOut),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let (player, _, _) = fake(game, None, 0);
                let mut stand = sunrise_client::entities::ArmorStand::new(
                    5001,
                    sunrise_core::geometry::Vec3::new(2.0, 64.0, 0.5),
                    0.0,
                );
                stand.equipment[0] = Some(ItemStack::new("test:missing", 1));
                game.armor_stands.push(stand);
                let target = game.entity(5001).unwrap();
                let setup = PlayerActionSetup::new(
                    None,
                    [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                    90.0,
                    0.0,
                    false,
                );
                let action = PlayerAction::InteractOn {
                    hand: Hand::Main,
                    target,
                    location: [0.0, 1.5, 0.0],
                };
                let dropped = game.dropped.len();
                let result = player.perform_action_scoped(game, action, setup).unwrap();
                assert_eq!(result.outcome, ActionOutcome::Consumed);
                assert_eq!((result.held, result.other.len()), (None, 0));
                assert_eq!(result.unrepresentable, 0, "the vetoed stack is not counted");
                assert_eq!(game.armor_stands.last().unwrap().equipment[0], None);
                assert_eq!(game.dropped.len(), dropped, "no item entity is left");
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn scoped_fake_use_restores_the_persistent_menu_owner() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, conn, _) = fake(game, Some("minecraft:diamond"), 1);
            let mut menu = Box::new(sunrise_client::menu::Menu::new(
                sunrise_client::menu::Kind::Inventory,
            ));
            menu.carried = Some(ItemStack::new("minecraft:stick", 3));
            let owner = (&*menu) as *const sunrise_client::menu::Menu;
            *game
                .lan
                .as_mut()
                .unwrap()
                .menus
                .entry(conn)
                .or_default()
                .crafting
                .inventory = Some(menu);
            place(game, [1, 64, 0], "minecraft:chest");
            let setup = mod_api::PlayerActionSetup::new(
                Some(crate::game::api_values::test_snapshot(&ItemStack::new(
                    "minecraft:oak_log",
                    1,
                ))),
                [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                0.0,
                90.0,
                false,
            );
            let result = game
                .player_perform_action_scoped(player, on_top([1, 64, 0]), setup)
                .unwrap();
            assert_eq!(
                result.outcome,
                ActionOutcome::EffectBlocked(FakeEffect::Menu)
            );
            let restored = game.lan.as_ref().unwrap().menus[&conn]
                .crafting
                .inventory
                .as_ref()
                .unwrap();
            assert_eq!((&**restored) as *const sunrise_client::menu::Menu, owner);
            assert_eq!(restored.carried, Some(ItemStack::new("minecraft:stick", 3)));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn scoped_fake_use_uses_viewpoint_secondary_use_and_restores_on_refusal() {
        use mod_api::PlayerActionSetup;
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, conn, _) = fake(game, Some("minecraft:diamond"), 1);
            let setup = |item, sneaking| {
                PlayerActionSetup::new(
                    Some(crate::game::api_values::test_snapshot(&ItemStack::new(
                        item, 1,
                    ))),
                    [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                    0.0,
                    90.0,
                    sneaking,
                )
            };
            // Air use rays from the supplied eyes/look, not the persistent fake's view.
            let mut viewpoint = setup("minecraft:water_bucket", false);
            viewpoint.eye = [2.5, 64.0 + f64::from(1.62_f32), 2.5];
            viewpoint.yaw = 117.0;
            let result = game
                .player_perform_action_scoped(
                    player,
                    PlayerAction::UseItem { hand: Hand::Main },
                    viewpoint,
                )
                .unwrap();
            assert_eq!(result.outcome, ActionOutcome::Consumed);
            assert_eq!(
                result.held,
                Some(crate::game::api_values::test_snapshot(&ItemStack::new(
                    "minecraft:bucket",
                    1
                )))
            );
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(
                game.level_block(level, [2, 64, 2])
                    .and_then(mod_api::__private::block_key_id),
                Some("minecraft:water")
            );
            place(game, [1, 64, 0], "minecraft:chest");
            let result = game
                .player_perform_action_scoped(
                    player,
                    on_top([1, 64, 0]),
                    setup("minecraft:oak_log", false),
                )
                .unwrap();
            assert_eq!(
                result.outcome,
                ActionOutcome::EffectBlocked(FakeEffect::Menu)
            );
            let result = game
                .player_perform_action_scoped(
                    player,
                    on_top([1, 64, 0]),
                    setup("minecraft:oak_log", true),
                )
                .unwrap();
            assert_eq!(result.outcome, ActionOutcome::Consumed);
            assert_eq!(
                result.held, None,
                "survival placement consumes the temporary stack"
            );
            assert_eq!(
                game.level_block(level, [1, 65, 0])
                    .and_then(mod_api::__private::block_key_id),
                Some("minecraft:oak_log")
            );
            assert_eq!(
                game.player_perform_action_scoped(
                    player,
                    on_top([40, 63, 0]),
                    setup("minecraft:oak_log", false)
                ),
                Err(ActionRefused::OutOfReach)
            );
            let mut invalid = setup("minecraft:oak_log", false);
            invalid.eye[0] = f64::NAN;
            assert_eq!(
                game.player_perform_action_scoped(player, on_top([1, 63, 0]), invalid),
                Err(ActionRefused::InvalidSetup)
            );
            let real = game.player(1).unwrap();
            assert_eq!(
                game.player_perform_action_scoped(
                    real,
                    on_top([1, 63, 0]),
                    setup("minecraft:oak_log", false)
                ),
                Err(ActionRefused::NotFake)
            );
            let acting = Acting::start(&game.fake_action);
            assert_eq!(
                game.player_perform_action_scoped(
                    player,
                    on_top([1, 63, 0]),
                    setup("minecraft:oak_log", false)
                ),
                Err(ActionRefused::Busy)
            );
            drop(acting);
            assert_eq!(held(game, 2), Some(ItemStack::new("minecraft:diamond", 1)));
            let guest = game.lan.as_ref().unwrap().hub.guest(conn).unwrap();
            assert_eq!(guest.game_mode, 1);
            assert_eq!(guest.position, [0.5, 64.0, 0.5]);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn fake_entity_interaction_checks_targets_and_does_not_use_items_in_air() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:ender_pearl"), 0);
            let near = zombie(game, 5000, [2.0, 64.0, 0.5]);
            let far = zombie(game, 5001, [20.0, 64.0, 0.5]);
            let interact = |target| PlayerAction::InteractOn {
                hand: Hand::Main,
                target,
                location: [0.0; 3],
            };
            assert_eq!(
                game.player_perform_action(player, interact(near)),
                Ok(ActionOutcome::Passed)
            );
            assert_eq!(
                held(game, 2),
                Some(ItemStack::new("minecraft:ender_pearl", 1))
            );
            assert!(game.creatures.projectiles.is_empty());
            assert_eq!(
                game.player_perform_action(player, interact(far)),
                Err(ActionRefused::OutOfReach)
            );
            assert_eq!(
                game.player_perform_action(player, interact(player.entity())),
                Err(ActionRefused::InvalidTarget)
            );
            let stale = mod_api::EntityRef::new(mod_api::WorldEpoch::new(0), near.id());
            assert_eq!(
                game.player_perform_action(player, interact(stale)),
                Err(ActionRefused::InvalidTarget)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn scoped_fake_use_hooks_see_the_setup_and_fail_is_not_consumed() {
        use mod_api::{Flow, Hooks, HostSide, PlayerActionSetup, RightClickBlockEvent};
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        #[derive(Default)]
        struct Deny(Arc<AtomicBool>);
        #[mod_api::sunrise_mod]
        impl Hooks for Deny {
            fn right_click_block(&self, host: HostSide, event: &mut RightClickBlockEvent) -> Flow {
                let HostSide::Server(host) = host else {
                    panic!("server action");
                };
                assert_eq!(event.click.player, 2);
                let entity = host.entity(event.click.player_entity_id as u32).unwrap();
                assert_eq!(entity.rotation(host), Some([123.0, 17.0]));
                assert_eq!(
                    entity.eye_position(host),
                    Some([0.5, 64.0 + f64::from(1.62_f32), 0.5])
                );
                assert_eq!(entity.is_shift_key_down(host), Some(true));
                self.0.store(true, Ordering::Relaxed);
                *event.cancellation_result = mod_api::InteractionResult::Fail;
                Flow::Cancel
            }
        }
        let heard = Arc::new(AtomicBool::new(false));
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "deny",
                Box::new(Deny(heard.clone())),
            )]
            .into(),
            || {
                super::super::native_entity_refs::tests::with_game(|game| {
                    let (player, ..) = fake(game, Some("minecraft:diamond"), 0);
                    game.use_failed = true;
                    let result = player
                        .perform_action_scoped(
                            game,
                            on_top([1, 63, 0]),
                            PlayerActionSetup::new(
                                Some(crate::game::api_values::test_snapshot(&ItemStack::new(
                                    "minecraft:oak_log",
                                    1,
                                ))),
                                [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                                123.0,
                                17.0,
                                true,
                            ),
                        )
                        .unwrap();
                    assert!(
                        heard.load(Ordering::Relaxed),
                        "normal Host-bearing hook ran"
                    );
                    assert_eq!(result.outcome, ActionOutcome::Passed);
                    assert_eq!(
                        result.held,
                        Some(crate::game::api_values::test_snapshot(&ItemStack::new(
                            "minecraft:oak_log",
                            1
                        )))
                    );
                    assert!(game.use_failed, "outer per-action flag restored");
                    let level = game.level(game.dim.id()).unwrap();
                    assert_eq!(
                        game.level_block(level, [1, 64, 0])
                            .and_then(mod_api::__private::block_key_id),
                        Some("minecraft:air")
                    );
                    assert_eq!(held(game, 2), Some(ItemStack::new("minecraft:diamond", 1)));
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn mob_target_setters_refuse_persistent_and_acting_fakes() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, conn, _) = fake(game, None, 0);
            let entity = zombie(game, 5000, [2.0, 64.0, 0.5]);
            let mob = game.mob(entity).unwrap();
            let target = game.living(player.entity()).unwrap();
            assert!(!mob.set_target(game, Some(target)));
            assert_eq!(mob.target(game), None);
            assert_eq!(
                game.as_guest(conn, |g| mob.set_target(g, Some(target))),
                Some(false)
            );
            assert_eq!(mob.target(game), None);
            assert!(!game.lan.as_ref().unwrap().mob_players.contains_key(&5000));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_fake_attacks_through_the_players_own_path() {
        use super::super::native_entity_refs::tests::with_game;
        with_game(|game| {
            let (player, fake_conn, _) = fake(game, None, 0);
            game.lan.as_mut().unwrap().hub.guests.update_all(|g| {
                g.yaw = -90.0;
            });
            // A fist does 1 (the attack damage attribute's base), scaled by the attack strength
            // of a player that hasn't waited (`getAttackStrengthScale(0.5)`: 0.5 of a 5-tick
            // delay, 0.1, so 0.2 + 0.1² × 0.8 = 0.208), then by a zombie's 2 armour
            // (`CombatRules.getDamageAfterAbsorb`: 1 - (2 - 0.208 / 2) / 25).
            let near = zombie(game, 5000, [2.0, 64.0, 0.5]);
            let full = health(game, 5000);
            assert_eq!(
                game.player_perform_action(player, PlayerAction::Attack { target: near }),
                Ok(ActionOutcome::Consumed)
            );
            let dealt = 0.208f32 * (1.0 - (2.0 - 0.208 / 2.0) / 25.0);
            assert!((full - health(game, 5000) - dealt).abs() < 1e-4);
            // No mob takes the fake as the player to answer.
            let bound = game.lan.as_ref().unwrap().mob_players.get(&5000).copied();
            assert_ne!(bound, Some(fake_conn));
            assert_eq!(bound, None);
            // Ten blocks off is past the attack range; the fake itself isn't a target, nor is
            // an id no entity has.
            let far = zombie(game, 5001, [10.5, 64.0, 0.5]);
            assert_eq!(
                game.player_perform_action(player, PlayerAction::Attack { target: far }),
                Err(ActionRefused::OutOfReach)
            );
            assert_eq!(health(game, 5001), full);
            assert_eq!(
                game.player_perform_action(
                    player,
                    PlayerAction::Attack {
                        target: player.entity()
                    }
                ),
                Err(ActionRefused::InvalidTarget)
            );
            let nobody = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), 6000);
            assert_eq!(
                game.player_perform_action(player, PlayerAction::Attack { target: nobody }),
                Err(ActionRefused::InvalidTarget)
            );
        });
    }

    /// A listener on `attack_entity` records the target's health where the hook
    /// runs; with `cancel` set it vetoes the blow.
    struct AttackWatcher {
        seen: std::sync::Arc<std::sync::Mutex<Vec<f32>>>,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for AttackWatcher {
        fn attack_entity(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::AttackEntityEvent<'_>,
        ) -> mod_api::Flow {
            let health = u32::try_from(event.entity)
                .ok()
                .and_then(|id| host.entity(id))
                .and_then(|entity| host.living(entity))
                .and_then(|living| living.health(host))
                .expect("the watcher's target is a live zombie");
            self.seen.lock().unwrap().push(health);
            if self.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return mod_api::Flow::Cancel;
            }
            mod_api::Flow::Continue
        }
    }

    /// `attack_entity` runs at `Player.attack`'s start, before the blow and the
    /// swing's attack-strength reset (vanilla's `getAttackStrengthScale(0.5)`
    /// is read there too, and `onAttack` resets the ticker after it). The hook
    /// sees the target untouched, the blow uses the charge that was there when
    /// the hook ran, and a veto leaves both the target and the charge alone.
    /// The charge itself is read through the damage it deals: the read a mod
    /// gets in the hook is `PlayerRef::attack_strength_scale` (part of the same
    /// issue, landed separately).
    #[test]
    #[ignore = "daily"]
    fn attack_entity_runs_before_the_blow_resets_the_attack_charge() {
        use super::super::native_entity_refs::tests::with_game;
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        };

        let seen = Arc::new(Mutex::new(Vec::new()));
        let cancel = Arc::new(AtomicBool::new(false));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "attack-watcher",
            Box::new(AttackWatcher {
                seen: seen.clone(),
                cancel: cancel.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            // Half a fist's cooldown (delay 5 ticks): the charge is (2 + 0.5) / 5 = 0.5.
            with_game(|game| {
                let (player, fake_conn, _) = fake(game, None, 0);
                // A placed fake participates in actions, not online-player tracking.
                // The packet attack gate must still reach the hook and damage below.
                let host = game.lan.as_ref().unwrap();
                let guest = host.hub.guest(fake_conn).unwrap();
                assert!(guest.is_fake());
                assert!(guest.placed);
                assert_eq!(guest.dimension, host.hub.config.dimension);
                assert!(!guest.participates_online());
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update_all(|g| g.yaw = -90.0);
                let near = zombie(game, 5000, [2.0, 64.0, 0.5]);
                let full = health(game, 5000);
                game.as_guest(fake_conn, |g| g.attack_ticks = 2).unwrap();
                assert_eq!(
                    game.player_perform_action(player, PlayerAction::Attack { target: near }),
                    Ok(ActionOutcome::Consumed)
                );
                assert_eq!(
                    *seen.lock().unwrap(),
                    vec![full],
                    "the hook runs before the blow lands"
                );
                // The hit then uses the charge that was there for the hook: 0.5 scales a fist's
                // 1 to 0.4, and a zombie's 2 armour takes `getDamageAfterAbsorb`'s share.
                let dealt = 0.4f32 * (1.0 - (2.0 - 0.4 / 2.0) / 25.0);
                assert!(
                    (full - health(game, 5000) - dealt).abs() < 1e-4,
                    "the blow uses the pre-reset charge"
                );
                assert_eq!(
                    game.as_guest(fake_conn, |g| g.attack_ticks).unwrap(),
                    0,
                    "the swing resets the charge after the hit"
                );
            });
            // A veto at the hook leaves the target unhurt and the charge where it was: neither
            // the hit nor the reset has run yet.
            with_game(|game| {
                let (player, fake_conn, _) = fake(game, None, 0);
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update_all(|g| g.yaw = -90.0);
                let near = zombie(game, 5001, [2.0, 64.0, 0.5]);
                let full = health(game, 5001);
                seen.lock().unwrap().clear();
                cancel.store(true, Ordering::Relaxed);
                game.as_guest(fake_conn, |g| g.attack_ticks = 2).unwrap();
                assert_eq!(
                    game.player_perform_action(player, PlayerAction::Attack { target: near }),
                    Ok(ActionOutcome::Passed)
                );
                assert_eq!(*seen.lock().unwrap(), vec![full]);
                assert_eq!(health(game, 5001), full, "a veto stops the blow");
                assert_eq!(
                    game.as_guest(fake_conn, |g| g.attack_ticks).unwrap(),
                    2,
                    "and the swing that would reset the charge never starts"
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn attack_strength_rejects_non_finite_partial_without_restricting_finite_ticks() {
        use super::super::native_entity_refs::tests::with_game;
        with_game(|game| {
            let (player, conn, _) = fake(game, None, 0);
            game.as_guest(conn, |g| g.attack_ticks = 2).unwrap();
            // The base attack speed, 4: a full charge takes 5 ticks.
            assert_eq!(
                game.player_current_item_attack_strength_delay(player),
                Some(5.0)
            );
            for partial in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                assert_eq!(game.player_attack_strength_scale(player, partial), None);
                game.as_guest(conn, |g| {
                    assert_eq!(g.attack_strength_scale_at(partial), None);
                    assert_eq!(g.player_attack_strength_scale(player, partial), None);
                })
                .unwrap();
            }
            for (partial, expected) in [(-1.0, 0.2), (0.5, 0.5), (2.0, 0.8)] {
                assert_eq!(
                    game.player_attack_strength_scale(player, partial),
                    Some(expected)
                );
                game.as_guest(conn, |g| {
                    assert_eq!(g.attack_strength_scale_at(partial), Some(expected));
                    assert_eq!(
                        g.player_attack_strength_scale(player, partial),
                        Some(expected)
                    );
                })
                .unwrap();
            }
        });
    }

    /// A listener on `attack_entity` reads the attacking player's attack
    /// strength where the hook runs.
    struct ChargeWatcher(std::sync::Arc<std::sync::Mutex<Vec<Option<f32>>>>);

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ChargeWatcher {
        fn attack_entity(
            &self,
            host: &mut dyn mod_api::ServerHost,
            _event: &mut mod_api::AttackEntityEvent<'_>,
        ) -> mod_api::Flow {
            let player = host.players().acting_player().expect("an attacking player");
            let scale = player.attack_strength_scale(host, 0.5);
            self.0.lock().unwrap().push(scale);
            mod_api::Flow::Continue
        }
    }

    /// `PlayerRef::attack_strength_scale` inside `attack_entity` reads the
    /// charge the blow uses: `Player.attack` reads `getAttackStrengthScale`
    /// itself at its start, and the hook runs before that read and before the
    /// swing resets the ticker. After the blow the same read sees 0. A
    /// resting player reads its saved ticker and body too.
    #[test]
    #[ignore = "daily"]
    fn attack_strength_scale_reads_the_charge_at_the_hook_and_not_after_the_swing() {
        use super::super::native_entity_refs::tests::with_game;
        use std::sync::{Arc, Mutex};

        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "charge-watcher",
            Box::new(ChargeWatcher(seen.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let (player, fake_conn, _) = fake(game, None, 0);
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update_all(|g| g.yaw = -90.0);
                let near = zombie(game, 5000, [2.0, 64.0, 0.5]);
                let full = health(game, 5000);
                // A fist's cooldown is 5 ticks: a full ticker reads 1.0, and half of one
                // ((2 + 0.5) / 5) reads 0.5.
                game.as_guest(fake_conn, |g| g.attack_ticks = 5).unwrap();
                assert_eq!(
                    player.attack_strength_scale(game, 0.5),
                    Some(1.0),
                    "a full ticker is a fully charged blow"
                );
                game.as_guest(fake_conn, |g| g.attack_ticks = 2).unwrap();
                assert_eq!(
                    player.attack_strength_scale(game, 0.5),
                    Some(0.5),
                    "a resting fake reads its saved ticker and body"
                );
                assert_eq!(
                    game.player_perform_action(player, PlayerAction::Attack { target: near }),
                    Ok(ActionOutcome::Consumed)
                );
                assert_eq!(*seen.lock().unwrap(), vec![Some(0.5)]);
                // The blow uses the charge the hook read: 0.5 scales a fist's 1 to 0.4, and a
                // zombie's 2 armour takes `getDamageAfterAbsorb`'s share.
                let dealt = 0.4f32 * (1.0 - (2.0 - 0.4 / 2.0) / 25.0);
                assert!((full - health(game, 5000) - dealt).abs() < 1e-4);
                assert_eq!(
                    player.attack_strength_scale(game, 0.5),
                    Some(0.1),
                    "the swing reset the ticker to 0 after the blow: (0 + 0.5) / 5"
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_fake_breaks_blocks_at_once_by_the_game_modes_rules() {
        use super::super::native_entity_refs::tests::with_game;
        let level_block = |game: &mut Game<'_>, pos| {
            let level = game.level(game.dim.id()).unwrap();
            game.level_block(level, pos)
        };
        // Survival, bare-handed: stone breaks at once, with no digging time.
        with_game(|game| {
            let (player, ..) = fake(game, None, 0);
            let pos = [1, 63, 0];
            assert_eq!(
                game.player_perform_action(player, PlayerAction::BreakBlock { pos }),
                Ok(ActionOutcome::Consumed)
            );
            assert_eq!(
                level_block(game, pos).and_then(mod_api::__private::block_key_id),
                Some("minecraft:air")
            );
            // Air breaks nothing; bedrock, which no digging breaks (destroy time -1), stays;
            // ten blocks off is out of reach.
            assert_eq!(
                game.player_perform_action(player, PlayerAction::BreakBlock { pos }),
                Ok(ActionOutcome::Passed)
            );
            place(game, [-1, 63, 0], "minecraft:bedrock");
            assert_eq!(
                game.player_perform_action(player, PlayerAction::BreakBlock { pos: [-1, 63, 0] }),
                Ok(ActionOutcome::Passed)
            );
            assert_eq!(
                level_block(game, [-1, 63, 0]).and_then(mod_api::__private::block_key_id),
                Some("minecraft:bedrock")
            );
            place(game, [3, 64, 3], "minecraft:stone");
            game.lan.as_mut().unwrap().hub.guests.update_all(|g| {
                g.position = [-6.5, 64.0, -6.5];
            });
            assert_eq!(
                game.player_perform_action(player, PlayerAction::BreakBlock { pos: [3, 64, 3] }),
                Err(ActionRefused::OutOfReach)
            );
        });
        // Creative breaks what survival can't dig.
        with_game(|game| {
            let (player, ..) = fake(game, None, 1);
            place(game, [1, 63, 0], "minecraft:bedrock");
            assert_eq!(
                game.player_perform_action(player, PlayerAction::BreakBlock { pos: [1, 63, 0] }),
                Ok(ActionOutcome::Consumed)
            );
            assert_eq!(
                level_block(game, [1, 63, 0]).and_then(mod_api::__private::block_key_id),
                Some("minecraft:air")
            );
        });
        // Adventure mode can't break a block (blockActionRestricted), and a sword in creative
        // breaks nothing (canDestroyBlock).
        for (mode, item) in [(2, None), (1, Some("minecraft:iron_sword"))] {
            with_game(|game| {
                let (player, ..) = fake(game, item, mode);
                let pos = [1, 63, 0];
                assert_eq!(
                    game.player_perform_action(player, PlayerAction::BreakBlock { pos }),
                    Ok(ActionOutcome::Passed),
                    "game mode {mode}, holding {item:?}"
                );
                assert_eq!(
                    level_block(game, pos).and_then(mod_api::__private::block_key_id),
                    Some("minecraft:stone")
                );
            });
        }
    }

    /// `as_guest_at` starts a turn only for the guest it names: another `conn` at that place,
    /// a place past the list's end, and a guest already in a turn are refused, not a panic.
    #[test]
    #[ignore = "daily"]
    fn a_turn_at_a_place_is_refused_for_another_guest() {
        use super::super::lan_actions::TurnRefused;
        super::super::native_entity_refs::tests::with_game(|game| {
            let (_, fake_conn, real_conn) = fake(game, None, 0);
            let gi = game
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .position(|g| g.conn == fake_conn)
                .unwrap();
            assert_eq!(
                game.as_guest_at(gi, real_conn, |_| ()),
                Err(TurnRefused::Gone)
            );
            assert_eq!(
                game.as_guest_at(999, fake_conn, |_| ()),
                Err(TurnRefused::Gone)
            );
            let nested = game.as_guest_at(gi, fake_conn, |g| g.as_guest_at(gi, fake_conn, |_| ()));
            assert_eq!(nested, Ok(Err(TurnRefused::Nested)));
            assert_eq!(game.as_guest_at(gi, fake_conn, |_| 7), Ok(7));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_fakes_effects_are_stopped_where_they_start() {
        use super::super::native_entity_refs::tests::with_game;
        // A chest's menu (Game::open_menu).
        with_game(|game| {
            let (player, ..) = fake(game, None, 0);
            place(game, [1, 64, 0], "minecraft:chest");
            assert_eq!(
                game.player_perform_action(player, on_top([1, 64, 0])),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Menu))
            );
        });
        // A bed (Game::try_sleep).
        with_game(|game| {
            let (player, ..) = fake(game, None, 0);
            place(
                game,
                [1, 64, 0],
                "minecraft:red_bed[facing=north,part=foot]",
            );
            place(
                game,
                [1, 64, -1],
                "minecraft:red_bed[facing=north,part=head]",
            );
            assert_eq!(
                game.player_perform_action(player, on_top([1, 64, 0])),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Sleep))
            );
        });
        // A spectator's chest, the menu that bypasses Game::open_menu (spectator_open_menu).
        with_game(|game| {
            let (player, ..) = fake(game, None, 3);
            place(game, [1, 64, 0], "minecraft:chest");
            assert_eq!(
                game.player_perform_action(player, on_top([1, 64, 0])),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Menu))
            );
        });
        // A sign placed opens no editor (open_sign_editor), but the sign is placed.
        with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:oak_sign"), 0);
            assert_eq!(
                game.player_perform_action(player, on_top([1, 63, 0])),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Menu))
            );
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(
                game.level_block(level, [1, 64, 0])
                    .and_then(mod_api::__private::block_key_id),
                Some("minecraft:oak_sign")
            );
        });
        // Boarding (native_may_board, every boarding path's gate): refused for the fake only.
        with_game(|game| {
            let (_, fake_conn, real_conn) = fake(game, None, 0);
            assert_eq!(
                game.as_guest(fake_conn, |g| g.native_may_board(99)),
                Some(false)
            );
            assert_eq!(
                game.as_guest(real_conn, |g| g.native_may_board(99)),
                Some(true)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_fakes_action_is_refused_before_anything_runs() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:water_bucket"), 0);
            // Out of reach: nothing used, the bucket still full.
            let far = PlayerAction::UseOn {
                hand: Hand::Main,
                pos: [40, 63, 0],
                hit: BlockHit::new([40.5, 64.0, 0.5], [0, 1, 0], false),
            };
            assert_eq!(
                game.player_perform_action(player, far),
                Err(ActionRefused::OutOfReach)
            );
            assert_eq!(
                held(game, 2).map(|s| s.item.to_string()).as_deref(),
                Some("minecraft:water_bucket")
            );
            // A real player's handle.
            let real = game.player(1).unwrap();
            assert_eq!(
                game.player_perform_action(real, on_top([1, 63, 0])),
                Err(ActionRefused::NotFake)
            );
            // Inside another action, as a hook running during it would ask.
            let acting = Acting::start(&game.fake_action);
            assert_eq!(
                game.player_perform_action(player, on_top([1, 63, 0])),
                Err(ActionRefused::Busy)
            );
            drop(acting);
            assert!(!game.fake_action.acting());
        });
    }

    /// 1, 64 and 256 fakes each clicking the stone floor with an empty hand every tick, the full
    /// admission and use path (vanilla passes). Run with `-- --ignored --nocapture`, in release.
    #[test]
    #[ignore = "timing"]
    fn fakes_acting_each_tick_cost() {
        #[derive(Clone, Copy, Debug)]
        enum Kind {
            UseOn,
            Attack,
            BreakBlock,
        }
        for kind in [Kind::UseOn, Kind::Attack, Kind::BreakBlock] {
            for count in [1u128, 64, 256] {
                super::super::native_entity_refs::tests::with_game(|game| {
                    let (first, ..) = fake(game, None, 0);
                    let level = game.level(game.dim.id()).unwrap();
                    let mut players = vec![first];
                    for i in 1..count {
                        players.push(
                            game.fake_player(level, 100 + i, &format!("fake{i}"))
                                .unwrap(),
                        );
                    }
                    game.lan.as_mut().unwrap().hub.guests.update_all(|g| {
                        g.position = [0.5, 64.0, 0.5];
                        g.yaw = -90.0;
                    });
                    let target = zombie(game, 5000, [2.0, 64.0, 0.5]);
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    let (action, expected) = match kind {
                        Kind::UseOn => (on_top([1, 63, 0]), ActionOutcome::Passed),
                        Kind::Attack => (PlayerAction::Attack { target }, ActionOutcome::Consumed),
                        Kind::BreakBlock => (
                            PlayerAction::BreakBlock { pos: [1, 63, 0] },
                            ActionOutcome::Consumed,
                        ),
                    };
                    let ticks = 200;
                    let start = std::time::Instant::now();
                    for _ in 0..ticks {
                        // The zombie never dies, and each break has a stone to break.
                        game.mobs[0].health = 20.0;
                        for &player in &players {
                            if let Kind::BreakBlock = kind {
                                game.world.set(1, 63, 0, stone).unwrap();
                            }
                            let outcome = game.player_perform_action(player, action);
                            assert_eq!(outcome, Ok(expected));
                        }
                    }
                    let elapsed = start.elapsed();
                    let per_tick = elapsed / ticks;
                    let per_action = elapsed / (ticks * players.len() as u32);
                    eprintln!(
                        "fake_action kind={kind:?} players={count} ns_per_action={}",
                        per_action.as_nanos()
                    );
                    println!(
                        "{kind:?}, {count} fakes: {per_tick:?} a tick, {per_action:?} an action"
                    );
                });
            }
        }
    }

    #[test]
    fn the_acting_mark_clears_on_a_panic_and_each_action_starts_unblocked() {
        let action = std::sync::Arc::new(super::FakeAction::default());
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _acting = Acting::start(&action);
            panic!("a mod's hook panics");
        }));
        assert!(unwound.is_err());
        assert!(!action.acting());
        {
            let acting = Acting::start(&action);
            action.block(FakeEffect::Sleep);
            action.block(FakeEffect::Menu);
            // The first effect stopped is the answer.
            assert_eq!(acting.blocked(), Some(FakeEffect::Sleep));
        }
        assert_eq!(Acting::start(&action).blocked(), None);
    }

    #[test]
    #[ignore = "daily"]
    fn a_fakes_action_leaves_the_per_action_state_as_it_was() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:oak_log"), 0);
            let before = super::super::Hit {
                pos: [7, 7, 7],
                face: [1, 0, 0],
                point: [7.0, 7.5, 7.5],
                inside: true,
            };
            game.forced_hit = Some(before);
            assert_eq!(
                game.player_perform_action(player, on_top([1, 63, 0])),
                Ok(ActionOutcome::Consumed)
            );
            let after = game.forced_hit.expect("the forced hit is put back");
            assert_eq!(
                (after.pos, after.face, after.point, after.inside),
                (before.pos, before.face, before.point, before.inside)
            );
            assert!(game.guest_use.is_none());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_real_player_is_unaffected_at_every_effect_site() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (_, _, real_conn) = fake(game, None, 0);
            for effect in [
                FakeEffect::Menu,
                FakeEffect::Sleep,
                FakeEffect::Ride,
                FakeEffect::ChangeDimension,
            ] {
                assert_eq!(
                    game.as_guest(real_conn, |g| g.fake_blocks(effect)),
                    Some(false)
                );
            }
            // A real guest's click on a chest is the vanilla click, nothing stopped.
            place(game, [1, 64, 0], "minecraft:chest");
            let gi = game
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .position(|g| g.conn == real_conn)
                .unwrap();
            let hit = super::super::Hit {
                pos: [1, 64, 0],
                face: [0, 1, 0],
                point: [1.5, 65.0, 0.5],
                inside: false,
            };
            assert_eq!(
                game.guest_use_item_on(real_conn, gi, hit, Hand::Main, None),
                Ok(true)
            );
            assert_eq!(
                game.as_guest(real_conn, |g| g.native_may_board(99)),
                Some(true)
            );
        });
    }
}

#[cfg(test)]
mod player_rule_tests {
    use super::super::native_guest_turn_tests::with_guests;
    use super::*;

    /// The guests of `with_guests` are rows 0 and 1; both stand in the level in place.
    fn in_this_level(game: &mut Game<'_>) {
        let level = game.dim.id().to_owned();
        game.lan.as_mut().unwrap().hub.guests.update_all(|g| {
            g.dimension = level.clone();
        });
    }

    fn at(dimension: &str, [x, y, z]: [i32; 3]) -> GlobalPos {
        GlobalPos::new(
            Identifier::parse(dimension).unwrap(),
            BlockPos::new(x, y, z),
        )
    }

    fn respawn(dimension: &str, pos: [i32; 3], yaw: f32, forced: bool) -> PlayerRespawnConfig {
        PlayerRespawnConfig {
            position: at(dimension, pos),
            yaw,
            forced,
        }
    }

    /// `/spawnpoint`'s write and `getRespawnConfig`'s read, on each player's own record.
    #[test]
    #[ignore = "daily"]
    fn a_respawn_point_is_read_and_written_in_the_players_own_record() {
        with_guests(|game| {
            let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
            assert_eq!(a.respawn_config(&*game), Some(None), "none until set");

            // The yaw wraps as `RespawnData.of` does: 270 degrees is -90.
            let nether = "minecraft:the_nether";
            assert!(a.set_respawn_config(game, Some(respawn(nether, [1, 2, 3], 270.0, true))));
            assert_eq!(
                a.respawn_config(&*game),
                Some(Some(respawn(nether, [1, 2, 3], -90.0, true)))
            );
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[0].respawn_elsewhere,
                Some((nether.into(), [1, 2, 3], -90.0, 0.0, true)),
                "the record a respawn reads"
            );
            assert_eq!(
                b.respawn_config(&*game),
                Some(None),
                "the other is untouched"
            );

            // The acting player reads the point swapped into place.
            let seen = game.as_guest(12, |game| a.respawn_config(&*game)).unwrap();
            assert_eq!(seen, Some(Some(respawn(nether, [1, 2, 3], -90.0, true))));

            // Refused writes change nothing: a dimension the game does not have, a yaw that is not a number.
            let nowhere = respawn("test:nowhere", [9, 9, 9], 0.0, false);
            assert!(!a.set_respawn_config(game, Some(nowhere)));
            let not_a_number = respawn(nether, [9, 9, 9], f32::NAN, false);
            assert!(!a.set_respawn_config(game, Some(not_a_number)));
            assert_eq!(
                a.respawn_config(&*game),
                Some(Some(respawn(nether, [1, 2, 3], -90.0, true)))
            );

            // None forgets it.
            assert!(a.set_respawn_config(game, None));
            assert_eq!(a.respawn_config(&*game), Some(None));
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[0].respawn_elsewhere,
                None
            );
        });
    }

    /// The write goes through `spawn_point_change`, so a mod's `player_set_spawn` veto refuses it, as it does `/spawnpoint`.
    #[test]
    #[ignore = "daily"]
    fn a_mods_veto_refuses_the_respawn_write() {
        struct Veto;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Veto {
            fn player_set_spawn(
                &self,
                _: &mut dyn mod_api::ServerHost,
                _: &mut mod_api::PlayerSetSpawnEvent,
            ) -> mod_api::Flow {
                mod_api::Flow::Cancel
            }
        }
        let mods = vec![mod_api::ModEntry::new("veto", Box::new(Veto))].into();
        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                let a = game.player(12).unwrap();
                let overworld = "minecraft:overworld";
                let config = respawn(overworld, [1, 2, 3], 0.0, false);
                assert!(!a.set_respawn_config(game, Some(config)));
                assert_eq!(a.respawn_config(&*game), Some(None));
            });
        });
    }

    /// The client's text-filtering option, as its `ClientInformation` said.
    #[test]
    #[ignore = "daily"]
    fn text_filtering_is_what_the_clients_information_said() {
        with_guests(|game| {
            let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.info.text_filtering = true;
            });
            assert_eq!(a.text_filtering(&*game), Some(true));
            assert_eq!(b.text_filtering(&*game), Some(false));
            // A later `ClientInformation` replaces it.
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.info.text_filtering = false;
            });
            assert_eq!(a.text_filtering(&*game), Some(false));
        });
    }

    /// The sleep reads and the counter write, with the night skip's own counts as the oracle.
    #[test]
    #[ignore = "daily"]
    fn sleep_eligibility_and_the_timer_follow_the_players_state() {
        with_guests(|game| {
            in_this_level(game);
            let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
            let (active, sleeping, deep) = game.sleep_counts();
            assert_eq!(a.sleep_timer(&*game), Some(0));
            assert_eq!(a.counts_for_sleep(&*game), Some(true));
            assert_eq!(a.sleep_ignored(&*game), Some(false));

            // A player in bed at 100 ticks is one of the deep sleepers the night waits for.
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.bed = Some([5, 64, 5]);
            });
            assert!(a.set_sleep_timer(game, 100));
            assert_eq!(a.sleep_timer(&*game), Some(100));
            assert_eq!(game.lan.as_ref().unwrap().hub.guests[0].sleep_timer, 100);
            assert_eq!(game.sleep_counts(), (active, sleeping + 1, deep + 1));
            assert!(a.set_sleep_timer(game, -1));
            assert_eq!(a.sleep_timer(&*game), Some(-1));
            assert_eq!(game.sleep_counts(), (active, sleeping + 1, deep));
            assert!(a.set_sleep_timer(game, 110));
            assert_eq!(a.sleep_timer(&*game), Some(110));

            // Ignoring leaves an out-of-bed player out of the count, and an in-bed one in.
            assert!(b.set_sleep_ignored(game, true));
            assert_eq!(b.sleep_ignored(&*game), Some(true));
            assert_eq!(b.counts_for_sleep(&*game), Some(false));
            assert_eq!(game.sleep_counts().0, active - 1);
            assert!(a.set_sleep_ignored(game, true));
            assert_eq!(a.counts_for_sleep(&*game), Some(true), "in bed");
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.bed = None);
            assert_eq!(a.counts_for_sleep(&*game), Some(false), "out of bed");
            assert!(a.set_sleep_ignored(game, false));
            assert!(b.set_sleep_ignored(game, false));
            assert_eq!(a.counts_for_sleep(&*game), Some(true));

            // A spectator is never counted.
            game.lan.as_mut().unwrap().hub.guests.update(1, |g| {
                g.game_mode = 3;
            });
            assert_eq!(b.counts_for_sleep(&*game), Some(false));

            // The acting player's state is the one in place.
            let seen = game
                .as_guest(12, |game| {
                    (a.sleep_timer(&*game), a.counts_for_sleep(&*game))
                })
                .unwrap();
            assert_eq!(seen, (Some(110), Some(true)));
        });
    }
}

// Dedicated network settings stay engine-private.
#[allow(dead_code)]
impl Game<'_> {
    /// Whether the dedicated server runs in online mode (`online-mode` of server.properties):
    /// its players sign in with their accounts. None off a dedicated server and before it listens.
    /// API @API:server_settings@.
    pub(super) fn native_online_mode(&self) -> Option<bool> {
        Some(online_mode_of(&self.dedicated_lan()?.shared.config))
    }
    /// The port the dedicated server listens on: `server-port` of server.properties, or the
    /// `--port` it was started with. None off a dedicated server and before it listens.
    /// API @API:server_settings@.
    pub(super) fn native_server_port(&self) -> Option<u16> {
        Some(self.dedicated_lan()?.port)
    }
    /// The listening socket of a dedicated server; None on any other side and before it listens.
    fn dedicated_lan(&self) -> Option<&sunrise_client::lan::Lan> {
        self.options.dedicated.as_ref()?;
        Some(&self.lan.as_ref()?.hub.lan)
    }
}

/// The online mode a dedicated server's network config carries: it has an
/// `OnlineMode` key (and so signs its logins) only in online mode.
fn online_mode_of(config: &sunrise_client::lan::NetConfig) -> bool {
    config.online.is_some()
}

#[cfg(test)]
mod network_settings_tests {
    use super::*;
    #[test]
    #[ignore = "daily"]
    fn server_settings_follow_the_network_config() {
        use sunrise_client::lan::{NetConfig, OnlineMode};
        let mut config = NetConfig::loopback(25565, String::new(), 20);
        // online-mode=false: no key.
        assert!(!online_mode_of(&config));
        // online-mode=true: the server's key.
        config.online = Some(std::sync::Arc::new(OnlineMode::generate(false).unwrap()));
        assert!(online_mode_of(&config));
    }
}
