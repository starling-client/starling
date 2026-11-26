//! The command tree a server sends (ClientboundCommandsPacket) as a
//! dispatcher the chat parses, colours and completes with
//! (ClientPacketListener.handleCommands).
//!
//! The packet holds each argument's parser and its network properties. A
//! node the client's own vanilla tree has at the same path takes that node's
//! argument type and suggestion provider (the parsers' suggestion lists and
//! the custom providers are the same code on both sides); any other node
//! (another server's or a mod's) gets the type its parser and properties
//! describe.
use super::{ArgType, Dispatcher, Node, NodeKind, RegistryId, Suggests};
use std::collections::{HashSet, VecDeque};
use sunrise_protocol::packets::play::{ARGUMENT_TYPES, ArgumentProperties, Commands};

/// The command id of every executable node of a server's tree: the chat
/// sends the command, the server runs it.
pub const SERVER_COMMAND: &str = "server";

/// The dispatcher for a server's tree. `local` is the client's own tree
/// (vanilla's), whose matching nodes lend their types and providers, not executor bindings.
pub fn from_packet(packet: &Commands, local: &Dispatcher) -> Dispatcher {
    let count = packet.nodes.len();
    // A decoded packet always names a root node (`Commands::read`); a tree built in process
    // without one has nothing to convert.
    if count == 0 {
        return Dispatcher::new();
    }
    let root = usize::try_from(packet.root)
        .ok()
        .filter(|&r| r < count)
        .unwrap_or(0);
    // The packet's root becomes node 0, the others follow in packet order.
    let index = |i: usize| -> usize {
        if i == root {
            0
        } else if i < root {
            i + 1
        } else {
            i
        }
    };
    let valid = |c: &i32| usize::try_from(*c).ok().filter(|&c| c < count);
    // The local node at the same path, found breadth first from the roots
    // along children (redirects are not paths of their own).
    let mut matched: Vec<Option<usize>> = vec![None; count];
    matched[root] = Some(0);
    let mut queue = VecDeque::from([root]);
    let mut seen = HashSet::from([root]);
    while let Some(n) = queue.pop_front() {
        for c in packet.nodes[n].children.iter().filter_map(valid) {
            if !seen.insert(c) {
                continue;
            }
            let node = &packet.nodes[c];
            matched[c] = matched[n].and_then(|l| {
                local.nodes[l].children.iter().copied().find(|&lc| {
                    let ln = &local.nodes[lc];
                    ln.name() == node.name
                        && matches!(
                            (&ln.kind, node.kind),
                            (NodeKind::Literal(_), 1) | (NodeKind::Argument { .. }, 2)
                        )
                })
            });
            queue.push_back(c);
        }
    }
    let mut nodes = vec![None; count];
    for (i, node) in packet.nodes.iter().enumerate() {
        let local_node = matched[i].map(|l| &local.nodes[l]);
        let kind = match node.kind {
            1 => NodeKind::Literal(node.name.clone()),
            2 => {
                let ty = match local_node.map(|l| &l.kind) {
                    Some(NodeKind::Argument { ty, .. }) => ty.clone(),
                    _ => node
                        .parser
                        .as_ref()
                        .map_or(ArgType::Word, |(id, props)| argument_type(*id, props)),
                };
                NodeKind::Argument {
                    name: node.name.clone(),
                    ty,
                }
            }
            _ => NodeKind::Root,
        };
        // The server's providers, as the client knows them
        // (ArgumentNodeStub.build): its own custom ones arrive as `ask_server`.
        let suggests = node.suggestions.as_deref().and_then(provider);
        nodes[index(i)] = Some(Node {
            kind: if i == root { NodeKind::Root } else { kind },
            children: node.children.iter().filter_map(valid).map(index).collect(),
            command: node.executable.then_some(SERVER_COMMAND),
            redirect: node.redirect.as_ref().and_then(valid).map(index),
            // The server sent only what the player may use.
            permission: 0,
            execution_permission: 0,
            suggests,
            restricted: node.restricted,
        });
    }
    let mut d = Dispatcher::new();
    d.nodes = nodes.into_iter().flatten().collect();
    d
}

/// SuggestionProviders by id (`SuggestionProviders.getProvider`).
fn provider(id: &str) -> Option<Suggests> {
    match id {
        "minecraft:ask_server" => Some(Suggests::AskServer),
        "minecraft:summonable_entities" => Some(Suggests::SummonableEntities),
        _ => None,
    }
}

/// The argument type a parser and its properties describe
/// (ArgumentTypeInfo.Template.instantiate).
pub fn argument_type(id: i32, props: &[u8]) -> ArgType {
    let Some(name) = usize::try_from(id).ok().and_then(|i| ARGUMENT_TYPES.get(i)) else {
        return ArgType::Word;
    };
    let p = ArgumentProperties::decode(id, props).unwrap_or(ArgumentProperties::None);
    // The property decoder reads the name in vanilla's STRING_UTF8 frame. An invalid name
    // resolves no registry entries, just like a missing registry property.
    let registry = || {
        let parsed = match &p {
            ArgumentProperties::Registry(name) => RegistryId::parse(name),
            _ => None,
        };
        parsed.unwrap_or_else(|| RegistryId::from_static(""))
    };
    match *name {
        "brigadier:bool" => ArgType::Bool,
        "brigadier:integer" => match p {
            ArgumentProperties::Integer { min, max } => ArgType::Integer {
                min: min.unwrap_or(i32::MIN),
                max: max.unwrap_or(i32::MAX),
            },
            _ => ArgType::Integer {
                min: i32::MIN,
                max: i32::MAX,
            },
        },
        "brigadier:long" => match p {
            ArgumentProperties::Long { min, max } => ArgType::Long {
                min: min.unwrap_or(i64::MIN),
                max: max.unwrap_or(i64::MAX),
            },
            _ => ArgType::Long {
                min: i64::MIN,
                max: i64::MAX,
            },
        },
        "brigadier:float" => match p {
            ArgumentProperties::Float { min, max } => ArgType::Float {
                min: min.unwrap_or(f32::MIN),
                max: max.unwrap_or(f32::MAX),
            },
            _ => ArgType::Float {
                min: f32::MIN,
                max: f32::MAX,
            },
        },
        "brigadier:double" => match p {
            ArgumentProperties::Double { min, max } => ArgType::Double {
                min: min.unwrap_or(f64::MIN),
                max: max.unwrap_or(f64::MAX),
            },
            _ => ArgType::Double {
                min: f64::MIN,
                max: f64::MAX,
            },
        },
        "brigadier:string" => match p {
            ArgumentProperties::String(1) => ArgType::Str,
            ArgumentProperties::String(2) => ArgType::Greedy,
            _ => ArgType::Word,
        },
        "minecraft:entity" => match p {
            ArgumentProperties::Entity {
                single,
                players_only,
            } => ArgType::Entity {
                single,
                players: players_only,
            },
            _ => ArgType::Entity {
                single: false,
                players: false,
            },
        },
        "minecraft:game_profile" => ArgType::GameProfile,
        "minecraft:block_pos" => ArgType::BlockPos,
        "minecraft:column_pos" => ArgType::ColumnPos,
        "minecraft:vec3" => ArgType::Vec3 { center: true },
        "minecraft:vec2" => ArgType::Vec2 { center: true },
        "minecraft:block_state" => ArgType::BlockState,
        "minecraft:block_predicate" => ArgType::BlockPredicate,
        "minecraft:item_stack" => ArgType::Item,
        "minecraft:item_predicate" => ArgType::ItemPredicate,
        "minecraft:team_color" => ArgType::TeamColor,
        "minecraft:hex_color" => ArgType::HexColor,
        "minecraft:component" => ArgType::Component,
        "minecraft:style" => ArgType::Style,
        "minecraft:message" => ArgType::Message,
        "minecraft:nbt_compound_tag" => ArgType::Nbt,
        "minecraft:nbt_tag" => ArgType::NbtTag,
        "minecraft:nbt_path" => ArgType::NbtPath,
        "minecraft:objective" => ArgType::Objective,
        "minecraft:objective_criteria" => ArgType::Criteria,
        "minecraft:operation" => ArgType::Operation,
        "minecraft:particle" => ArgType::Particle,
        "minecraft:angle" => ArgType::Angle,
        "minecraft:rotation" => ArgType::Rotation,
        "minecraft:scoreboard_slot" => ArgType::ScoreboardSlot,
        "minecraft:score_holder" => ArgType::ScoreHolder {
            multiple: matches!(p, ArgumentProperties::ScoreHolder { multiple: true }),
        },
        "minecraft:swizzle" => ArgType::Swizzle,
        "minecraft:team" => ArgType::Team,
        "minecraft:item_slot" => ArgType::Slot,
        "minecraft:item_slots" => ArgType::Slots,
        "minecraft:resource_location" => ArgType::Id("none"),
        "minecraft:function" => ArgType::Function,
        "minecraft:entity_anchor" => ArgType::Anchor,
        "minecraft:int_range" => ArgType::IntRange,
        "minecraft:float_range" => ArgType::FloatRange,
        "minecraft:dimension" => ArgType::Dimension,
        "minecraft:gamemode" => ArgType::GameMode,
        "minecraft:time" => ArgType::Time {
            min: match p {
                ArgumentProperties::Time { min } => min,
                _ => 0,
            },
        },
        "minecraft:resource_or_tag" => ArgType::ResOrTag(registry()),
        "minecraft:resource_or_tag_key" => ArgType::ResOrTagKey(registry()),
        "minecraft:resource" => ArgType::Res(registry()),
        "minecraft:resource_key" => ArgType::ResKey(registry()),
        "minecraft:resource_selector" => ArgType::ResourceSelector(registry()),
        "minecraft:template_mirror" => ArgType::Enum(&["none", "front_back", "left_right"]),
        "minecraft:template_rotation" => {
            ArgType::Enum(&["none", "clockwise_90", "180", "counterclockwise_90"])
        }
        "minecraft:heightmap" => ArgType::Enum(&[
            "world_surface",
            "motion_blocking",
            "motion_blocking_no_leaves",
            "ocean_floor",
        ]),
        "minecraft:swing_animation" => ArgType::Enum(&["none", "whack", "stab"]),
        "minecraft:loot_table" => {
            ArgType::ResourceOrId(RegistryId::from_static("minecraft:loot_table"))
        }
        "minecraft:loot_predicate" => {
            ArgType::ResourceOrId(RegistryId::from_static("minecraft:predicate"))
        }
        "minecraft:loot_modifier" => {
            ArgType::ResourceOrId(RegistryId::from_static("minecraft:item_modifier"))
        }
        "minecraft:context_float_provider" => {
            ArgType::ResourceOrId(RegistryId::from_static("minecraft:context_float_provider"))
        }
        "minecraft:context_int_provider" => {
            ArgType::ResourceOrId(RegistryId::from_static("minecraft:context_int_provider"))
        }
        "minecraft:slot_source" => ArgType::SlotSource,
        "minecraft:dialog" => ArgType::ResourceOrId(RegistryId::from_static("minecraft:dialog")),
        "minecraft:feature" => {
            ArgType::ResourceOrId(RegistryId::from_static("minecraft:worldgen/feature"))
        }
        "minecraft:uuid" => ArgType::Uuid,
        _ => ArgType::Word,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use sunrise_core::vanilla_bytes::Writer;
    use sunrise_protocol::{
        buf::{Codec, Reader},
        packets::play::CommandNode,
    };

    #[test]
    fn registry_properties_are_owned_and_use_the_vanilla_string_frame() {
        let id = ARGUMENT_TYPES
            .iter()
            .position(|&name| name == "minecraft:resource")
            .unwrap() as i32;
        let decode = |name: &str| {
            let mut props = Writer::new();
            props.string(name);
            let ArgType::Res(registry) = argument_type(id, &props.data) else {
                panic!("expected a resource argument");
            };
            registry
        };
        for i in 0..1025 {
            let name = format!("test:registry_{i}");
            assert_eq!(decode(&name).as_str(), name);
        }
        let mut long = format!(
            "test:registry/{}",
            "r".repeat(32767 - "test:registry/".len())
        );
        let first = decode(&long);
        let second = decode(&long);
        assert_eq!(first.as_str(), long);
        assert_eq!(second.as_str(), long);
        assert_ne!(first.as_ptr(), second.as_ptr(), "no shared global interner");
        long.push('r');
        assert!(decode(&long).is_empty());

        for invalid in [
            "",
            "item",
            "test:",
            ":item",
            "test:a:b",
            "te/st:item",
            "Test:item",
            "..:item",
            "test:é",
        ] {
            assert!(decode(invalid).is_empty(), "{invalid:?}");
        }
    }

    fn literal(name: &str, children: Vec<i32>, executable: bool) -> CommandNode {
        CommandNode {
            kind: 1,
            executable,
            restricted: false,
            children,
            redirect: None,
            name: name.into(),
            parser: None,
            suggestions: None,
        }
    }

    fn argument(name: &str, parser: &str, props: Vec<u8>, executable: bool) -> CommandNode {
        let id = ARGUMENT_TYPES.iter().position(|p| *p == parser).unwrap() as i32;
        CommandNode {
            kind: 2,
            executable,
            restricted: false,
            children: Vec::new(),
            redirect: None,
            name: name.into(),
            parser: Some((id, props)),
            suggestions: Some("minecraft:ask_server".into()),
        }
    }

    /// A tree with the root last (as vanilla's numbering need not put it
    /// first), a vanilla command and one the client does not know.
    fn packet() -> Commands {
        let mut range = Writer::new();
        range.u8(3);
        range.i32(1);
        range.i32(64);
        Commands {
            nodes: vec![
                literal("give", vec![2], false),
                literal("warp", vec![3], false),
                argument("targets", "minecraft:entity", vec![2], false),
                argument("count", "brigadier:integer", range.data, true),
                CommandNode {
                    kind: 0,
                    executable: false,
                    restricted: false,
                    children: vec![0, 1],
                    redirect: None,
                    name: String::new(),
                    parser: None,
                    suggestions: None,
                },
            ],
            root: 4,
        }
    }

    /// `stop` runs but is restricted, `say <message>` signs, `list` is plain.
    fn tree_for_checks() -> Dispatcher {
        let mut stop = literal("stop", Vec::new(), true);
        stop.restricted = true;
        let tree = Commands {
            nodes: vec![
                stop,
                literal("say", vec![2], false),
                argument("message", "minecraft:message", Vec::new(), true),
                literal("list", Vec::new(), true),
                CommandNode {
                    kind: 0,
                    executable: false,
                    restricted: false,
                    children: vec![0, 1, 3],
                    redirect: None,
                    name: String::new(),
                    parser: None,
                    suggestions: None,
                },
            ],
            root: 4,
        };
        from_packet(&tree, &Dispatcher::new())
    }

    #[test]
    fn is_valid_needs_a_whole_command_that_runs() {
        let tree = tree_for_checks();
        let reg = crate::Registries::default();
        assert!(tree.parse("list", &reg).is_valid());
        assert!(tree.parse("say hello", &reg).is_valid());
        // Unknown, stopping short of a command, and with something left over.
        assert!(!tree.parse("nosuchcommand", &reg).is_valid());
        assert!(!tree.parse("say", &reg).is_valid());
        assert!(!tree.parse("list extra", &reg).is_valid());
    }

    #[test]
    fn executor_keys_stay_with_authorized_nodes_in_their_dispatcher() {
        use crate::Registries;

        let mut server = Dispatcher::new();
        let warp = server.literal(0, "warp");
        let count = server.argument(warp, "count", ArgType::Integer { min: 1, max: 64 });
        let preview = server.literal(count, "preview");
        server.executes(warp, "native_mod_command");
        server.requires_execution(warp, 2);
        server.executes(count, "native_mod_command");
        server.requires_execution(count, 4);
        server.executes(preview, "preview");
        server.replace_executor_bindings([(warp, 7), (count, 8)].into());

        let mut registries = Registries {
            permission: 2,
            ..Default::default()
        };
        let parsed = server.parse("warp", &registries);
        let key = parsed.executable().unwrap().0.executor.unwrap();
        assert_eq!(key.slot(), 7, "omitting the optional count runs the prefix");
        assert!(server.owns_executor(key));

        let denied = server.parse("warp 7", &registries);
        assert!(denied.executable().is_err());
        assert_eq!(denied.context.executor, None);
        registries.permission = 4;
        let parsed = server.parse("warp 7", &registries);
        assert_eq!(parsed.executable().unwrap().0.executor.unwrap().slot(), 8);
        let parsed = server.parse("warp 7 preview", &registries);
        let (context, command) = parsed.executable().unwrap();
        assert_eq!(command, "preview");
        assert_eq!(context.executor, None, "the previous node's key is cleared");

        let mut display = packet();
        display.nodes[1].executable = true; // The authorized display of the optional prefix.
        let client = from_packet(&display, &server);
        for input in ["warp", "warp 7"] {
            let parsed = client.parse(input, &Registries::default());
            let (context, command) = parsed.executable().unwrap();
            assert_eq!(command, SERVER_COMMAND);
            assert_eq!(context.executor, None, "packets carry display data only");
        }
        assert!(!client.owns_executor(key));

        // A rebuild of the same grammar, with identical IDs, must still invalidate old keys.
        server.replace_executor_bindings([(warp, 7), (count, 8)].into());
        assert!(!server.owns_executor(key));
        let parsed = server.parse("warp", &registries);
        let new_key = parsed.executable().unwrap().0.executor.unwrap();
        assert_eq!(new_key.slot(), 7);
        assert!(server.owns_executor(new_key));
    }

    #[test]
    fn without_restricted_drops_only_restricted_nodes() {
        let tree = tree_for_checks();
        let reg = crate::Registries::default();
        assert!(tree.parse("stop", &reg).is_valid());
        let restricted = tree.without_restricted();
        assert!(!restricted.parse("stop", &reg).is_valid());
        assert!(restricted.parse("list", &reg).is_valid());
        assert!(restricted.parse("say hello", &reg).is_valid());
    }

    #[test]
    fn argument_nodes_round_trip() {
        let p = packet();
        let mut w = Writer::new();
        Commands::write(&p, &mut w);
        let back = Commands::read(&mut Reader::new(&w.data)).unwrap();
        assert_eq!(back, p);
    }

    /// What a client signs (SignableCommand.of over the server's tree): the
    /// message arguments of vanilla's /msg and of a command the client does
    /// not know, and nothing for a command without one.
    #[test]
    fn signable_arguments_of_a_server_tree() {
        let mut shout = argument("text", "minecraft:message", vec![], true);
        shout.suggestions = None;
        let mut targets = argument("targets", "minecraft:entity", vec![0], false);
        targets.children = vec![2];
        let packet = Commands {
            nodes: vec![
                literal("msg", vec![1], false),
                targets,
                argument("message", "minecraft:message", vec![], true),
                literal("shout", vec![4], false),
                shout,
                literal("spawn", vec![], true),
                CommandNode {
                    kind: 0,
                    executable: false,
                    restricted: false,
                    children: vec![0, 3, 5],
                    redirect: None,
                    name: String::new(),
                    parser: None,
                    suggestions: None,
                },
            ],
            root: 6,
        };
        let d = from_packet(&packet, super::super::vanilla::dispatcher());
        let reg = crate::Registries::default();
        let signable = |command: &str| d.parse(command, &reg).signable_arguments(&d);
        assert_eq!(
            signable("msg Steve hi there @a"),
            vec![("message".to_owned(), "hi there @a".to_owned())]
        );
        assert_eq!(
            signable("shout hello"),
            vec![("text".to_owned(), "hello".to_owned())]
        );
        assert!(signable("spawn").is_empty());
        assert!(signable("unknown words").is_empty());

        // A server that hides a signable root (`player_commands_sent`) leaves the client
        // nothing to sign it by, as NeoForge's `ClientPacketListener.sendCommand` does with its
        // own tree: the command is sent unsigned, and the server's secure-chat policy decides.
        let mut hidden = packet;
        hidden.nodes[6].children = vec![3, 5];
        let d = from_packet(&hidden, super::super::vanilla::dispatcher());
        assert!(
            d.parse("msg Steve hi there", &reg)
                .signable_arguments(&d)
                .is_empty()
        );
        assert_eq!(
            d.parse("shout hello", &reg).signable_arguments(&d),
            vec![("text".to_owned(), "hello".to_owned())]
        );
    }

    /// A server's `brigadier:long` node parses as Brigadier's long on the client, not as a
    /// double over the same bounds: a fraction is refused, and so is a value past a bound, with
    /// `argument.long.low` / `big`, as the server will refuse it.
    #[test]
    fn a_servers_long_argument_is_a_long_on_the_client() {
        let mut bounds = Writer::new();
        bounds.u8(3);
        bounds.i64(-5);
        bounds.i64(9_000_000_000);
        let packet = Commands {
            nodes: vec![
                literal("bank", vec![2], false),
                CommandNode {
                    kind: 0,
                    executable: false,
                    restricted: false,
                    children: vec![0],
                    redirect: None,
                    name: String::new(),
                    parser: None,
                    suggestions: None,
                },
                argument("total", "brigadier:long", bounds.data, true),
            ],
            root: 1,
        };
        let d = from_packet(&packet, &Dispatcher::new());
        let reg = crate::Registries::default();
        let parsed = d.parse("bank 9000000000", &reg);
        let (context, _) = parsed.executable().unwrap();
        assert_eq!(context.get("total"), Some(&crate::Arg::Long(9_000_000_000)));
        for (input, key, args) in [
            ("bank 1.5", "parsing.long.invalid", vec!["1.5".to_owned()]),
            (
                "bank 9000000001",
                "argument.long.big",
                vec!["9000000000".to_owned(), "9000000001".to_owned()],
            ),
            (
                "bank -6",
                "argument.long.low",
                vec!["-5".to_owned(), "-6".to_owned()],
            ),
        ] {
            let error = d.parse(input, &reg).executable().unwrap_err();
            let args: Vec<&dyn std::fmt::Display> =
                args.iter().map(|a| a as &dyn std::fmt::Display).collect();
            assert_eq!(error.message, crate::Text::tr(key, &args), "{input}");
        }
    }

    #[test]
    fn server_tree_becomes_a_dispatcher() {
        let local = super::super::vanilla::dispatcher();
        let d = from_packet(&packet(), local);
        assert!(matches!(d.nodes[0].kind, NodeKind::Root));
        let warp = d.find("warp").expect("warp");
        let count = d.nodes[warp].children[0];
        match &d.nodes[count].kind {
            NodeKind::Argument {
                ty: ArgType::Integer { min, max },
                ..
            } => assert_eq!((*min, *max), (1, 64)),
            other => panic!("count is {other:?}"),
        }
        assert_eq!(d.nodes[count].command, Some(SERVER_COMMAND));
        // `give <targets>` is vanilla's: it keeps the client's type.
        let give = d.find("give").expect("give");
        let targets = d.nodes[give].children[0];
        assert!(matches!(
            d.nodes[targets].kind,
            NodeKind::Argument {
                ty: ArgType::Entity {
                    single: false,
                    players: true
                },
                ..
            }
        ));
    }

    #[test]
    fn every_argument_type_decodes() {
        for (id, name) in ARGUMENT_TYPES.iter().enumerate() {
            let mut w = Writer::new();
            match *name {
                "brigadier:integer" | "brigadier:long" | "brigadier:float" | "brigadier:double" => {
                    w.u8(0)
                }
                "brigadier:string" => w.varint(2),
                "minecraft:entity" | "minecraft:score_holder" => w.u8(1),
                "minecraft:time" => w.i32(0),
                n if n.starts_with("minecraft:resource") && n != "minecraft:resource_location" => {
                    w.string("minecraft:item")
                }
                _ => {}
            }
            let mut r = Reader::new(&w.data);
            ArgumentProperties::read(&mut r, id as i32).unwrap();
            assert_eq!(r.remaining(), 0, "{name}");
            let _ = argument_type(id as i32, &w.data);
        }
    }
}
