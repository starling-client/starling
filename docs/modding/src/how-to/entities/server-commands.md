# Typed server commands

In `register_commands`, push each server usage as
`Command::new(usage).executes(|host, source, command| { /* ... */ Ok(1) })`.
Use `Command::op(usage, level)` for a usage that requires operator permission.
The executor receives a borrowed `CommandInvocation`, not a raw line to split
or a command path to route. Read named values through `command.argument(name)`
and `CommandArgumentValue`. An integer is already an `i32` and a `long` an `i64`, inside the
bounds of `integer(min,max)` or `long(min,max)` (`<amount:integer(0,1000)>`, `<total:long(0,)>`:
Brigadier's `IntegerArgumentType.integer(min, max)` and `LongArgumentType.longArg(min, max)`,
sent to clients with their bounds and refused out of range with `argument.integer.low` / `big`
or `argument.long.low` / `big` before the executor runs); a player argument is
a live `PlayerRef`, not selector text.

Only the executor whose registration owns the parsed node runs, with its mod's
identity. If the owner is disabled after registration, the command is refused.
Return `Ok(n)` for success with integer result `n` (zero is also success), or
`Err(message.into())` with a `Text` for failure. Failure text defaults to red and does
not depend on `sendCommandFeedback`. For Java's `source.sendFailure(text); return 0;`, call
`host.commands().failure(text)` and return `Ok(0)`: the same red line, but the command succeeds
with result 0, as in vanilla (`execute store result` reads 0, `execute store success` reads 1). It is not shown for forked commands, silent sources,
or sources that do not accept failures. "Unknown or incomplete command" remains a parse error for input
that matches no usage. Neither failure nor a disabled owner tries another executor.
`execute store result` sees `n` on success; `execute store success` sees one.
On a returned failure, both store zero.
An executor can capture owned or shared state, such as an `Arc`, but it cannot
borrow values from the registration call.

A panic fails the command, is logged with the mod's name and queues an error toast
on a local client. That mod's executor for the registered usage is disabled for the
session, even if the tree is rebuilt. Other usages still run. If the caller already
had a transaction, the panicking invocation's journaled writes are undone. A failed
invocation also aborts any transactions the executor opened and left unfinished.
The game does not implicitly open a transaction for an ordinary command, and
unjournaled effects are not undone. Like a vanilla command throwing an unexpected
exception, a panicking executor does not run the command's `execute store` callbacks.

The server asks for commands when it builds the session command tree: at world open before
functions compile, and again on every `/reload` and data pack change. No player or network hub
needs to exist yet. Register the complete current set on each call. Plain function lines and
substituted macro lines are checked against this session tree; commands no longer registered
are refused when functions next load. Starting the hub reuses the session's accepted command
grammar rather than asking again. Clients receive only display and completion data,
never executors. A parsed invocation belongs to the server dispatcher that parsed it;
if the tree is rebuilt, the server must parse the command again before it can run.

When accepted usages execute at the same node, each later accepted registration
overwrites that node's executor (matching Brigadier's `CommandNode.addChild`, which
replaces `child.command`) together with Sunrise's paired execution permission level
for that node. A usage with optional arguments also executes where those
arguments begin, so `status` and `status [player:player]` overlap at `status`. By
contrast, a public `status detail` allows traversal through `status`, but does not
make an operator-only `status` executable. Use distinct paths to keep distinct
executors.

A usage whose first word is a vanilla command's merges into that command, as NeoForge's
`RegisterCommandsEvent` registers into vanilla's dispatcher (Brigadier's `CommandNode.addChild`):
`Command::new("time season <name:word>")` adds `season` beside vanilla's `time set`, `add` and
`query`, which stay. Vanilla's nodes keep their permission, so the words a mod adds under `/time`
are reached only by who may use `/time`, and a world without cheats refuses them as it refuses
`/time`. A usage that executes at a vanilla node, such as `Command::new("seed")`, replaces
vanilla's executor there, as the later registration does in Brigadier. A mod argument cannot share
the place of a vanilla node of the same name, nor a mod literal a vanilla argument's: such a usage
is left out, with the reason in the log. Sunrise's own `/modprofile` and `/pregen` are never
merged into.

Literal words use lowercase letters, digits, `_` and `-`. Only the first word may
additionally begin with one `/`: register `/set <x:word>` and type `//set stone` in chat.
The command's literals retain `/set`. A slash alone, repeated leading slashes and
slashes in later literal words are refused.

The server parses and checks permissions before resolving arguments or calling the
executor. Omitted optional arguments are absent, including an omitted optional
player: they do not silently select the source player. `player` selects exactly one
online player, `players` one or more online players, `entity` exactly one entity,
and `entities` one or more entities. Missing or too-large selections produce
vanilla feedback without calling the executor. Word, `string` and greedy text arguments
are strings, numbers are finite doubles, booleans are typed, and item arguments
are component-bearing stacks of one item. `mod_id` takes a loaded package id and
delivers it as a string (NeoForge's `ModIdArgument`). `resource:<registry>`, such as
`<kind:resource:minecraft:entity_type>`, takes an id that registry holds, static or
from a data pack, and delivers `CommandArgumentValue::Id`; any other id is refused
with vanilla's `argument.resource.not_found` before the executor runs. Clients see it
as vanilla's `minecraft:resource_key`, so unmodded clients complete it too. The registry
must be a valid `namespace:path` resource location. An invalid registry name refuses the
usage with a reason in the log. Registry names have no separate registration budget; the
vanilla command-tree string frame applies on the wire. Block positions are resolved and
floored from the source's world, relative or local coordinates without loading chunks.

`resource_or_tag:<registry>`, such as `<biome:resource_or_tag:minecraft:worldgen/biome>`, takes an id
or a `#tag` of that registry (vanilla's `ResourceOrTagKeyArgument`, `minecraft:resource_or_tag_key` on
the wire, so the chat box completes both). It delivers `CommandArgumentValue::ResourceOrTag { tag, id }`:
`tag` is true for `#namespace:path`, and `id` comes without the `#`. As in vanilla, neither is checked
against the registry when parsed; look it up when the command runs. It works in client commands too.

`id` is a free identifier: vanilla's `minecraft:resource_location` argument, lowercase letters,
digits and `_ - . : /`, canonicalised to `namespace:path` with `minecraft` when the namespace is
left out, and checked against no registry. `Command::new("map fullrender <world:id>")` takes
`minecraft:overworld`, `mymod:custom` and `overworld` (delivered as `minecraft:overworld`), and
refuses an upper-case token or a second colon. The value arrives as a plain string. It offers no
list of its own, so its node declares `minecraft:ask_server`: the client, modded or not, asks the
server for the completions, as it does for a mod argument type. The identifiers are yours to
supply in `tab_complete`, which receives the command, the replaced byte range and the suggestions
the server computed, and may add to or filter them, matching its own command path and the
argument's position.

Positions, rotations, dimensions and compound tags have their own kinds, each sent to clients
as vanilla's parser so `~`, `^` and the dimension list complete as in a vanilla command:

| Kind | Vanilla parser | `CommandArgumentValue` |
|---|---|---|
| `vec3` | `minecraft:vec3`, whole `x` and `z` centred on the block | `Vec3([f64; 3])` |
| `vec3_uncentered` | `minecraft:vec3`, coordinates as written | `Vec3([f64; 3])` |
| `vec2` | `minecraft:vec2`, whole `x` and `z` centred on the block | `Vec2([f32; 2])` |
| `vec2_uncentered` | `minecraft:vec2`, coordinates as written | `Vec2([f32; 2])` |
| `column_pos` | `minecraft:column_pos` | `ColumnPos([i32; 2])`, floored |
| `rotation` | `minecraft:rotation` | `Rotation([f32; 2])`, yaw then pitch |
| `dimension` | `minecraft:dimension` | `Dimension(LevelRef)` |
| `nbt` | `minecraft:nbt_compound_tag` | `Nbt(Compound)` |

`~` and `^` resolve against the command source's position and rotation, as `block_pos` does
(`vec2` and `vec2_uncentered` take `~` relative coordinates and refuse `^`). A
`dimension` the world has not loaded is refused with `argument.dimension.invalid` before the
executor runs. Neither `vec3`/`vec3_uncentered` nor `vec2`/`vec2_uncentered` can share a
position in one command's usages, as they look the same to a client.
A player's source is their position and rotation. The console has no source
entity and uses the world spawn's block corner, in the level the spawn is in, with zero rotation, so `@s`
cannot resolve there. Every executor receives the same kind of typed input.

The executor's `source` is a non-exhaustive `CommandSource`: `Player { player, .. }`,
`Console { .. }`, `CommandBlock { position, level, .. }`,
`CommandBlockMinecart { entity, .. }` or `Function { id, .. }`.
`source.permission()` returns the effective `PermissionLevel`: functions use
function permission; blocks and carts use game-master permission. `execute as`,
`at` and `in` change argument resolution, not the origin. `command.dimension` and
`command.position` are where the command executes after `execute in`, `at` or
`positioned`; the console executes in the Overworld at its spawn corner.

Two reads work in a command and elsewhere on the server. `player.profile(host)` returns
the player's `GameProfile<'_>`: UUID, borrowed name and login properties such as the signed
`textures` skin. Iterate `profile.properties()` in login order; each row lends its `name`,
`value` and `signature: Option<&str>`. For example, find the skin with
`profile.properties().find(|p| p.name == "textures")`. An offline-mode player has an empty
iterator; a stale player has no profile. Copy individual strings only to keep them after the
host borrow. A [custom test host](../../concepts/testing.md) can lend a profile from
`testing::GameProfileFixture`.

`host.advancement(&id)` returns a loaded
`AdvancementInfo`: its parent, display (title, description, icon stack with its components, frame and
flags), criterion names and requirements, from the set the last `/reload` built.

## Success feedback and the operators

`host.commands().feedback(text, broadcast_to_admins)` is vanilla's
`CommandSourceStack.sendSuccess(() -> text, broadcast_to_admins)`: Java's
`source.sendSuccess(() -> msg, true)` is `feedback(msg, true)`. Pass `false` for a query or a
list, as vanilla does. A silent source (an `execute` fork, a function, a mod's
`suppress_output` run) gets nothing and tells no one.

| Line | Who gets it | When |
| --- | --- | --- |
| `text` | the originating source: a player, the console or RCON, a command block or minecart | a player, and a block or minecart that tracks output, while `sendCommandFeedback` is on; the console and RCON always. Never the `execute as` entity. |
| `[name: text]` | every operator but the source player (`PlayerList.isOp`: a level-0 operator too, never a non-operator) | `broadcast_to_admins`, the origin informs operators, and `sendCommandFeedback` is on |
| `[name: text]` | the server's log | `broadcast_to_admins`, the origin informs operators, and `logAdminCommands` is on; never when the console is the source |

The `[name: text]` line is `chat.type.admin`, gray and italic, with the display name of the
`execute as` entity, else the command block's or minecart's name, `Server` or `Rcon`. A player
always informs the operators; a command block or minecart does while it tracks output and
`commandBlockOutput` is on; the console and RCON follow `broadcast-console-to-ops` and
`broadcast-rcon-to-ops`. The two game rules are independent: with `sendCommandFeedback` off,
the log still records an operator's command. Sending to no one is still `Ok(())`, and the
executor's return value alone is the command's result.

```rust,ignore
host.commands().feedback(Text::translate("mymod.team.disbanded", vec![team_name]), true)?;
```

### The entity a command executes as

`command.entity` is the `EntityRef` the command executes as (`CommandSourceStack.getEntity`),
and `command.player()` is that entity when it is a player (`getPlayer`). `command.rotation`
is the execution rotation in degrees as `[yaw, pitch]`, the order of a `rotation` argument;
Java's `Vec2` holds pitch first. `execute as` and `on` change only the entity; `at` sets the
level, position and rotation; `rotated` and `facing` set the rotation. Each fork of an `execute`
has its own.

| Source | `entity` | `rotation` |
|---|---|---|
| A player | the player | theirs |
| The console | None | zero |
| A command block | None | the direction it faces (`Direction.toYRot`), pitch zero |
| A command-block minecart | the minecart | its yaw, pitch zero |
| A function | its caller's | its caller's |

An entity that no longer resolves when the executor runs, such as one killed earlier in the
same function, is None, never another player. `source` stays the origin, and
`host.players().acting_player()` the submitting player. A command that needs a player maps a
`None` from `command.player()` to its own failure:

```rust,ignore
Command::new("party leave").executes(|host, _source, command| {
    let Some(player) = command.player() else {
        return Err(Text::translate("permissions.requires.player", Vec::new()).into());
    };
    leave_party(host, player);
    Ok(1)
})
```

## Run a command as a player

`player.entity().run_command_with_output(host, line, level, suppress_output)` runs a command as that
player, from their position and rotation, with `@s` selecting them. Pass
`PermissionLevel::from_level(i32::from(player.permission_level(host)?))` to use
the player's own permission level. The supplied level is taken as given, so a mod
can also choose a different level explicitly. A leading slash is allowed.

The returned `CommandResult` has `success` (the same bool as `run_command`) and
`feedback`: the success and failure lines received by the source, in order, as
plain text. Pass `false` for normal player chat and operator notifications, or
`true` to suppress both and return an empty `feedback` list. Like vanilla's
`withSuppressedOutput`, this silences command feedback, not the command's effects:
public `/say` and `/me` announcements are delivered as chat, not included in `feedback`. Use
`success` to check whether the command ran and `chat_recipient` to observe the public delivery attempts.
Feedback silenced by the source or game rules is absent. Other entities already use a silent source.
A stale, removed or unavailable entity returns `success: false` and no lines.
Teleports to another level still happen at the next tick's start.
`run_command(host, line, level, suppress_output)` takes the same option and
returns only whether the command succeeded.

## Entity and player selectors

Side: server. Bind an executor to `Command::new("heal <targets:players>")` to receive
`CommandArgumentValue::Players(Vec<PlayerRef>)`: `/heal @a` calls it once
with all selected online players, not once per player. The other selector kinds
deliver `Player(PlayerRef)`, `Entity(EntityRef)` and `Entities(Vec<EntityRef>)`.
For example, `<target:entity>` accepts
`@e[type=minecraft:cow,limit=1,sort=nearest]`. The shared vanilla selector parser
and resolver use the command source's position and selector permission, preserving
selection order (including `sort=nearest`, `sort=furthest` and `sort=random`).
The client's command tree carries the matching single/multiple and players-only
flags and offers the normal player-name and selector completions.

Empty selections report `argument.entity.notfound.player` or
`argument.entity.notfound.entity`; multiple-result selectors passed to a single
argument report `argument.player.toomany` or `argument.entity.toomany`.
A source without selector permission receives
`argument.entity.selector.not_allowed` before resolution. Plain online player
names do not require selector permission.

In a multi-entity selection, an entity that cannot resolve to an `EntityRef` is
left out. If none remain, or a single target cannot resolve, the command reports `argument.entity.notfound.entity`. There is no
entity-type exclusion: leash knots, item frames, paintings and tridents
are delivered when resolvable. Dragon parts
are not selector members in vanilla and are never delivered by these arguments.
Temporarily held-out or removed entities cannot be delivered. An unbounded
selector can reach other loaded dimensions; handle lookup uses the entity's own
dimension. There is no extra count cap. A very large `@e` is the caller's cost.

## Game profiles, including offline players

Use `Command::new("inspect <targets:game_profile>")` for commands that can reach
offline players. The executor receives `CommandArgumentValue::GameProfiles`:
a list of `(uuid, name)` pairs, not player handles. A plain name matches an
online player or an unexpired user-cache entry, case-insensitively. Selectors
select online players in vanilla order and use the same permission rules as
other selectors. The client receives vanilla's `minecraft:game_profile` parser
for player-name and selector completion. Unknown names report
`argument.player.unknown`; empty selectors report `argument.entity.notfound.player`.

On the server, `host.players().profile_by_name(name)` and
`host.players().profile_by_uuid(uuid)` return the same borrowed `GameProfile<'_>`
as `player.profile(host)`. They check online players first, then the unexpired
user cache. Cached profiles have no login properties. Neither read contacts
Mojang or invents an offline identity. For a UUID, use `profile_by_uuid`:
vanilla's command argument takes a name or a selector, not a UUID literal.

`host.players().profile_permission_level(uuid, name)` works in `player_login`,
before a `PlayerRef` exists, and for offline profiles. Dedicated servers read
the op list by UUID. Integrated servers also account for the owner's name and
world and guest command access. `PermissionLevel::All` is level 0.

## Custom argument types

Register a value kind once from `Hooks::register`, then use its namespaced id as an
argument's kind:

```rust,ignore
use mod_api::{
    Command, CommandArgumentError, CommandArgumentKind, CommandArgumentType, CommandArgumentValue,
    Hooks, RegisterCommandsEvent, Registrar, RegistrationContext, ServerHost, Suggestion,
    SuggestionRequest, Text,
};

struct Machine;

impl CommandArgumentType for Machine {
    fn kind(&self) -> CommandArgumentKind {
        CommandArgumentKind::Word
    }

    fn parse(&self, input: &str) -> Result<String, CommandArgumentError> {
        match input {
            "press" | "furnace" => Ok(input.to_owned()),
            _ => Err(CommandArgumentError::new(Text::translate(
                "example.commands.machine.unknown",
                vec![Text::literal(input)],
            ))),
        }
    }

    fn suggest(&self, request: &SuggestionRequest) -> Vec<Suggestion> {
        ["press", "furnace"]
            .into_iter()
            .filter(|machine| machine.starts_with(request.prefix.as_str()))
            .map(|machine| request.suggest(machine))
            .collect()
    }
}

#[mod_api::sunrise_mod]
impl Hooks for MyMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.register_command_argument("machine", Machine);
    }

    fn register_commands(&self, _: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        event.commands.push(Command::new("machine use <machine:mymod:machine>").executes(
            |host, _, command| {
                let Some(CommandArgumentValue::Custom { value, .. }) = command.argument("machine") else {
                    return Err(Text::translate("example.commands.machine.missing", vec![]).into());
                };
                host.commands()
                    .feedback(Text::literal(value.as_str()), false)
                    .map_err(|_| Text::translate("example.commands.machine.feedback_failed", vec![]))?;
                Ok(1)
            },
        ));
    }
}
```

The server parses the argument with `parse` before calling the executor; a
rejected value shows the returned `Text` and the executor does not run. A valid value arrives
as `CommandArgumentValue::Custom { kind, value }`, where `kind` is `mymod:machine`
and `value` is the canonical text `parse` returned. `suggest` is asked for the
completions while the player types, with the sender and the arguments before it: a type that completes names
from a mod's own registry returns only the entries that sender may use, keyed by its
UUID and compared with `sender.permission()`. The game keeps the answer as it comes
and does not filter it, so a type that ignores the sender offers everything to
everyone. `request.sender` is `None` when no player asks: the console, and a client
completing its own client commands before its own profile is known.

For a fixed set of words, register `ChoiceArgument` (NeoForge's `EnumArgument`)
instead of writing a type: `r.register_command_argument("mode",
ChoiceArgument::new(["low", "high"]))`. It refuses other words, listing the valid
ones, and completes the choices that start with the typed text.

### What `suggest` sees and how its answer is kept

`suggest` takes a `SuggestionRequest`: `prefix` (the text typed for this argument), `arguments`
(every argument parsed before it, each a `TypedArgument` with its name and the text the engine
parsed) and `sender` (UUID, name and command level). It answers `Vec<Suggestion>`, the same type
`tab_complete` uses, on server and client commands alike. Every request is answered from
scratch; nothing is kept between two requests. A panic answers no suggestions and is logged
against the argument type.

```rust,ignore
fn suggest(&self, request: &SuggestionRequest) -> Vec<Suggestion> {
    let level = request.sender.as_ref().map_or(0, |sender| sender.permission().level());
    let world = request
        .arguments
        .iter()
        .find(|argument| argument.name == "world")
        .map_or("", |argument| argument.text.as_str());
    warps_of(level, world)
        .into_iter()
        .filter(|warp| warp.starts_with(request.prefix.as_str()))
        .map(|warp| request.suggest_with_tooltip(warp, "a warp"))
        .collect()
}
```

The type proposes and the engine orders: the merged list is sorted like Brigadier's. A reply
keeps at most 1,000 candidates (`ServerSuggestionProvider.limitSuggestionCount`) and each text
at the protocol's 32,767 UTF-16 code units (a tooltip at 65,535 bytes of Java modified UTF-8).
The cap counts the candidates the reply keeps, so a list of dropped entries before a good one
does not hide it. A candidate past a bound, with a range outside the prefix or splitting a
character, equal to the text it replaces, or repeating an earlier one, is dropped and named in
the diagnostics; the others are kept, and the player's chat is never disconnected over it.
`tab_complete` still sees and may change the server's completions. A client that does not know
the type asks the server (the node carries `minecraft:ask_server`).

Choose a `CommandArgumentKind` for how much of the input the type reads:

- `Word` takes one word of ASCII letters, digits and `_ - . +`; the word ends at the first
  other character, a space included.
- `Id` reads a vanilla identifier using `a-z 0-9 _ - . : /` and stops at the first other
  character. For example, `<modifier:mymod:modifier> <level:integer>` can read
  `mymod:haste 2`. The type's `parse` receives the original spelling: it decides what
  value to return and whether the identifier belongs to its own list. `#` is not part
  of an identifier; this kind doesn't read tags.
- `Greedy` takes the rest of the line and can only be the last argument.
- `Quotable` takes one word or one quoted phrase (below).

`suggest` receives the text as typed so far: one word, the identifier, or the whole tail of the
line up to the cursor. Players whose client does not know the type parse and complete the
command: the server sends `Id` as vanilla's `minecraft:resource_location` and the other kinds as
Brigadier strings of that shape, all with `minecraft:ask_server`, and their chat asks the server
for the suggestions.

### Quoted phrases

The `string` kind and `CommandArgumentKind::Quotable` read one value the way Brigadier's
`StringArgumentType.string()` (`QUOTABLE_PHRASE`) does: one unquoted word of `Word`'s
characters, or one phrase in `"double"` or `'single'` quotes, empty included. Inside a phrase
a `\` escapes only the quote that opened it and `\` itself. Unlike `text`, other arguments may
follow it:

```rust,ignore
// `/quests open "#main" @a`, `/quests open 1A2B3C4D @a` and `/quests open "Getting Started" @a`
Command::new("quests open <object:string> <targets:players>")
```

| Typed | Delivered |
|---|---|
| `main` | `main` |
| `"Getting Started"` | `Getting Started` |
| `'say "hi"'` | `say "hi"` |
| `"a \"b\" \\ c"` | `a "b" \ c` |
| `""` | the empty string |

The value arrives decoded, without its quotes, as `CommandArgumentValue::String` for `string`
and in `parse`'s `input` for a `Quotable` type. A phrase with no closing quote is refused with
vanilla's `parsing.quote.expected.end` ("Unclosed quoted string"), and a `\` before any other
character with `parsing.quote.escape` ("Invalid escape sequence '\x' in quoted string"), at the
character where the reader stopped, before the executor or the type's `parse` runs. There is no
length limit beyond the chat line's. Clients receive the argument as `brigadier:string` in its
quotable mode, so a vanilla client reads the phrase as the server does.

A `Quotable` type's `suggest`, or its provider's `SuggestionRequest::prefix`, gets the text as
typed, quote included and often unclosed: `"Getting St`. Brigadier gives a suggestion provider
the same. Its suggestions replace that whole prefix, so a type returns text the player could
have typed: a candidate that is not one word is quoted, and its quotes and `\` escaped, as
NeoForge's `EntityCommand` quotes an entity id that needs it. For a declared provider, the
engine's match compares phrase content, without the quotes and their escapes, on both sides. So
`Get`, `'Get` and `"Get` all keep `"Getting Started"`, which is inserted as written:

```rust,ignore
/// Whether `text` can be typed without quotes: one word of `StringReader.isAllowedInUnquotedString`'s
/// characters. An empty value is not one: it is typed as `""`.
fn is_word(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || "_-.+".contains(c))
}

fn suggest(&self, _: Option<&CommandSender>, prefix: &str) -> Vec<CommandSuggestion> {
    let typed = prefix.trim_start_matches(['"', '\'']);
    titles()
        .filter(|title| title.starts_with(typed))
        .map(|title| match is_word(title) {
            true => CommandSuggestion::new(title),
            false => CommandSuggestion::new(format!("\"{}\"", title.replace('\\', "\\\\").replace('"', "\\\""))),
        })
        .collect()
}
```

A bare name uses the registering mod's namespace; an explicit valid `namespace:path` keeps
its namespace. An empty or malformed name, a built-in command argument type ID, or an id
registered twice refuses the mods at startup, naming the mod. A client mod registers client
command argument types the same way; they are available to `register_client_commands`.

### Completing one token of a greedy text

A `Greedy` argument holds free text, and the tokens in it are yours to split. A completion
replaces all of that text by default, so the suggestion names the token it completes with
`request.suggest_from(start, text)`, a byte offset into the prefix (or `Suggestion::with_range`):

```rust,ignore
// `//replace 50%stone,50%dirt st` completes only the block being typed.
fn suggest(&self, request: &SuggestionRequest) -> Vec<Suggestion> {
    let prefix = request.prefix.as_str();
    let start = prefix.rfind(' ').map_or(0, |space| space + 1);
    ["stone", "dirt"]
        .into_iter()
        .filter(|block| block.starts_with(&prefix[start..]))
        .map(|block| request.suggest_from(start, block))
        .collect()
}
```

The offset reaches the player's client as the suggestion's range, so vanilla clients replace
the same token in place and keep the text in front of it. A range past the prefix, or inside
a character, refuses that suggestion: the reason is logged with the type's id, the other
suggestions stay, and nothing panics.

A greedy `text` argument is the last one: no word may follow it, and a usage that puts one
after it is left out, with the reason in the log. Such a usage is written with the free text
last and the mod splitting what it needs from that text: `//undo <text>` for
`//undo 1 <player>`, `/music <text>` for `/music volume 20 30`, `/corecarol <text>` for
`/corecarol skip now`. Vanilla has no reachable node after a greedy argument either, so a
trailing word would never be matched there.

`player_command` provides player-only pre-execution cancellation (NeoForge's `CommandEvent`), including vanilla commands.

When the usages a mod registers change at runtime, call
`host.commands().refresh_commands()` on the server. At the start of the next
server tick, every mod is asked to register again. The server rebuilds the typed
command dispatcher and sends each player's tree after `player_commands_sent`
filters it, as vanilla does after `/reload`. Repeated requests before that tick
coalesce; without a world open to players, nothing is rebuilt.

## Filter one player's command tree

Use `Hooks::player_commands_sent` with a read-only `Host` and
`PlayerCommandsSentEvent<'_>`. The event's `player` is the live `PlayerRef`, and
`event.tree.root_names()` borrows the outgoing roots in send order, after the
server has checked the player's permissions. For example, to hide one root
from a particular recipient, return:

```rust,ignore
if event.player.name(host) == Some("Alex") {
    event.tree.hide_where(|root| root == "my_debug_command")
} else {
    None
}
```

Each send asks again: return the selection from the callback that created it.
Successful selections combine; another listener cannot restore a root. Redirects
into a removed branch disappear with that branch. On join, the recipient's
address, brand and announced channels are already available, but the recipient
is not yet in the online list. A permission update exposes the new permission;
during cross-dimension travel, the recipient is still at the source.
This filters client highlighting and suggestions, not permission to execute a
typed command. Keep execution checks in the dispatcher.

## Who sees a usage

A usage's `level` gates its whole path, and the same test answers running, the tree
and the completions: a player below the level neither receives that literal in their
command tree nor is offered it, and typing the path is refused. A root with one
public usage keeps only the children the player may run, so `/warp` stays visible
while `/warp admin` is absent for a non-operator. An operator level change sends the
trees again (`/op`, `/deop`) and the completions follow from then on; a completer
whose answers depend on its own policy calls `host.commands().refresh_commands()`
after it changes them.

## Custom selector tokens

Side: server. Register a token such as `@allies` once from `Hooks::register` with
`r.register_entity_selector("allies", Allies)`, where `Allies` implements
`EntitySelectorType` (NeoForge's `EntitySelectorManager.register` and
`IEntitySelectorType`). `base` returns its `SelectorBase`: players only, a limit, a
`SelectorSort` and the translation key of its completion tooltip. `matches(host, source,
entity)` is asked for each candidate after the base selection and the `[...]` options,
before sorting and the limit. Return `false` to drop the candidate. A panic in `base` or
`matches` disables that selector for the rest of the session and reports an error toast
naming the mod, while sibling selectors keep working.

Every entity, entities, player and players argument then accepts `@allies[...]`, in
vanilla commands and mod commands alike, with the same options and selector permission
as `@e`. The parser reads the whole Brigadier unquoted word (`[0-9A-Za-z_.+-]`) after `@`
before vanilla's letter, so `@allies` is never `@a`, and an unregistered word still fails
with `argument.entity.selector.unknown`. `register_entity_selector` returns
`Result<(), EntitySelectorRegistrationError>` for an empty name, a character outside
Brigadier's unquoted-string alphabet (`[0-9A-Za-z_.+-]`), one of vanilla's `p a r s e n`,
a duplicate token name, or a call after startup registration closes, without poisoning
unrelated registrations. Handle the result in `Hooks::register`, for example with
`if let Err(error) = r.register_entity_selector("allies", Allies) { eprintln!("{error}"); }`.
`base()` is called only when the logical server builds its command registry; the same
snapshot goes to clients.

The server sends token metadata alongside each command tree to connected Sunrise clients,
including the single-player window. Their command parser and completion use the server's
names, tooltip keys, base limits and sort order, even without the server's mod installed
locally. Network framing follows vanilla's `STRING_UTF8` (32,767 UTF-16 code units) and
`ClientboundCustomPayloadPacket` (1 MiB) bounds: an entry whose name or tooltip exceeds
32,767 UTF-16 code units or would push the payload past 1 MiB is logged and skipped while
earlier valid tokens are still sent. The payload's revision is 1; an unsupported revision
disables token metadata for that connection with one log line. This replaces the previous
server's inventory at connection startup. Vanilla clients do not receive this
Sunrise-specific metadata and can still send commands containing the token without local
completion.

## Client commands

In `Hooks::register_client_commands`, attach a client executor to a usage:

```rust,ignore
let counter = self.counter.clone();
event.commands.push(mod_api::Command::new("count").executes_client(move |_, _| {
    let mut count = counter.lock().unwrap();
    *count += 1;
    Some(*count)
}));
```

The executor receives a `ClientHost` and a typed `CommandInvocation`. Return
`Some(result)` on success or `None` to report failure locally. A usage's level is checked
against the local player's permission level, the one the server's entity events 24 to 28 set
(NeoForge's `ClientCommandHandler` source): below it, the usage is neither completed nor run, and
the line goes to the server as an unknown command would. Use client-safe argument kinds; player, entity and dimension handles and custom
argument types complete through the same `suggest`. The chat display
combines local and server branches, but dispatch is separate: only an unknown local
command or argument goes to the server.

Slash commands submitted by an executor wait until the next client event-loop
pass. A nested command cannot re-enter an executor already in its submission
chain; that refusal stays local, even if the dispatcher was refreshed. Unrelated
nested commands still run normally.
