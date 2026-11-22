# Server services and player handles

Use `ServerHost` services to run commands, send messages and find connected
players.

## Services

| Accessor | Operations |
|---|---|
| `host.commands()` | `run_command(line)` runs through the console command path with permission level 4. `feedback(text)` answers the command source. |
| `host.messages()` | `broadcast(text)` fires the broadcast hook and writes to the server log. |
| `host.players()` | `server_players()` returns `Vec<PlayerRef>` in join order. `acting_player()` returns the optional player whose state is active in the hook. `player_by_name(name)` finds a connected real player by case-insensitive account name, not nickname. |
| `host.levels()` | `list_levels()` returns loaded server levels in vanilla dimension order, or an empty list before a server world opens. |

The players list and name lookup include the single-player owner, even before
opening the world to LAN. The owner joins the integrated server through memory
and has an ordinary `PlayerRef`. Sending a payload to that ref reaches the client
half through the same registration and negotiation as any other player; no
separate local-player send path is needed.
Fake players are not included in the connected-player list.

Player and level lookup services accept `&dyn ServerHost` and do not mutate the
world. These services are unavailable on `Host`, `ClientHost` and restricted
level hosts. Block and fluid scheduling uses `LevelTicksHost`;
`tick_stats()` belongs to `ServerHost`.

On a client, `host.chat_line(Text::literal("Local notice"))?` adds a line only to
the local chat HUD and history. It sends no packet and does not fire
`client_chat_received`. Handle its `Result<(), mod_api::TextFormError>`: text
exceeding the boundary's size or nesting limits is refused before reaching the HUD.

## Running a command as an entity

`host.commands().run_command` runs as the console. To run a line as a player or another entity instead, at a level
you choose, call `entity.run_command(host, line, PermissionLevel::Gamemasters)`: `@s` is that entity, `~` its
position, and only a player is answered. It returns `false` for a line that fails, a stale or removed entity, and an
entity outside the level in place (a player in another level included, since that level is parked while the hook
runs). To run a command in another level, do it from a moment in that level or from a task scheduled there.

## Identity and location

Read identity through `player.uuid()`, `player.name(host)`,
`player.permission_level(host)` and `player.address(host)`. The address is the
connection IP without a port. `player.client_brand(host)` reads the brand the
client announced. Name, address and brand return `Option<&str>`: borrow them
for comparisons, and use `.map(str::to_owned)` only to keep the text beyond the
host borrow. A stale or unavailable player gives `None`; an acting player without
a known connection gives `Some("")` for address and brand.

`player.announced_channels(host)` returns `Option<Arc<[String]>>` in announcement
order. The snapshot includes `minecraft:register` minus `minecraft:unregister`
from configuration and play. It can outlive the host borrow and does not change
when the client updates its list. Read it with `.iter()` and keep the `Arc` if
you need that exact list later; call the method again to see new announcements.
An acting player without a known connection gets an empty snapshot; a stale or
unavailable player returns `None`.

`player.latency(host)` reads the ping the Tab list shows, in milliseconds: the keep-alive round
trips averaged three parts old to one part new. It is 0 until the first keep-alive answer, for a fake
player and for an acting player without a known connection. It refuses with `PlayerWorldRefusal::Client`
on a client and `PlayerWorldRefusal::Stale` for a stale handle.

The online list includes connected players during nested player turns. Name,
permission and address reads work for connected players whose bodies are displaced
by another player's turn; stale handles are refused. Entity and living handles
provide world-state reads, which require a resolvable body.

In `player_stat_award`, find the recipient with
`host.players().player_by_name(event.name)` or use their retained `PlayerRef`.
`player.stat(host, stat_type, key)` reads that player's count before the award,
even if a nested turn has displaced them. This does not make displaced players
available for statistic writes.

`player.level(host)` returns the authoritative `LevelRef`;
`player.dimension(host)` returns its dimension `Identifier`, including registered
custom dimensions. Both accept `&dyn Host`. On a server they identify the player's
current dimension, even when another dimension is ticking. On a client they
identify the local player's level. Unknown, disconnected, replaced or stale handles
return `None`.

## Messages and visibility

Send a message with `player.tell(host, text)`, `player.action_bar(host, text)` or
`player.tab_list(host, header, footer)`. Messaging reaches acting and connected
players, including displaced online records, and refuses stale or disconnected
identities.

| Method | Behavior |
|---|---|
| `set_nickname(host, nickname)` | Changes the name shown in chat and the Tab list; `None` restores the account name. Commands, selectors, logs and events use the real name. Validate the nickname's length, filtering and conflicts yourself. |
| `nickname(host)` | Borrows the nickname `set_nickname` set as `Result<Option<&str>, PlayerWorldRefusal>`: `Ok(None)` when there is none. Refuses with `Client` on a client and `Stale` for a stale or offline player. |
| `set_sleep_ignored(host, ignored)` | Controls exclusion from the sleep count. |
| `disconnect(host, reason)` | Disconnects the player with a `Text` reason, as `/kick` does, also inside their own hook or a payload handler (the connection closes after it). Answers `false` for a stale or unavailable handle, the single-player owner and a player already being disconnected (the first reason stands). A fake player's kick keeps the first reason and completes at the next hub poll through the shared leave path. Invalid reason text returns `TextFormError` before anything changes. |
| `hide(host, permission)` | Hides the player from players below the supplied permission level, starting next tick; `None` restores visibility. Hidden players disappear from the world and Tab list, and their join and leave messages are suppressed for those viewers. |
| `hidden(host)` | Reads the level `hide` set: `Ok(None)` when the player is shown. A player who rejoined hidden reads it too, and a level set before the world is opened to others reads as stored. Refuses with `PlayerWorldRefusal::Client` on a client and `Stale` for a handle of an earlier open of the world. |

Nickname and sleep settings require an online player. These settings survive
reconnections until the server stops or you change them. Visibility is stored by
UUID: you can use a retained handle after disconnection until its world epoch
closes. You cannot target arbitrary offline UUIDs. Retain the handle value, not a
borrowed service.

## Chat lines and respawn

Keeping a public chat line from some players (an ignore list, a chat channel, a party chat): the `chat_recipient`
hook is asked once for each placed receiver, the sender included, before any of them is sent the line, with the
`sender`, `sender_name`, the `message` players see and the `recipient`. Set `hidden` and that one receiver is sent
nothing, no chat index and no pending acknowledgement; the others still get the line signed as it was sent, so its
signature verifies for them. It covers a player's chat after `server_chat` let it through and `/say` and `/me`, signed
or not. For a non-player `/say` source, such as the console or a command block, `sender` is the nil UUID (`0`) and
`sender_name` is the source's display name. Whispers (`/msg`) are not public and are not asked. It is not a second
`server_chat`: that hook is heard once for a typed line and cancels it for everyone, while this one is per receiver
and also covers `/say` and `/me`. With no listener nothing is asked or allocated. A `/say` requested inside
`chat_recipient` fails instead of reentering the hook or sending an unchecked line. The server's log keeps every
broadcast line, hidden or not.

`player.respawn(host)` makes the same request as the client's Respawn button (`PERFORM_RESPAWN`). Call it outside
player hooks, for example from `server_tick`. `Ok(())` means the request was accepted, not that the replacement is
already placed. In a transaction, the server begins the respawn only after the outermost scope commits. It may still
wait for destination chunks. Use `player_respawned` to act on the placed replacement:

```rust,ignore
use mod_api::{Hooks, PlayerRespawnedEvent, ServerHost, Text};

struct WelcomeBack;

#[mod_api::sunrise_mod]
impl Hooks for WelcomeBack {
    fn player_respawned(&self, host: &mut dyn ServerHost, event: &mut PlayerRespawnedEvent<'_>) {
        if event.end_conquered {
            return; // This notice is only for a return after death.
        }
        if let Some(player) = host.player(event.player) {
            player.tell(host, Text::translate("example.respawn.welcome_back", Vec::new()));
        }
    }
}
```

`player_respawn_position` runs with the removed original before cloning. Use `event.from_end_fight` there and
`event.end_conquered` in `player_respawned` to distinguish the return after the End credits from a death respawn.
Both causes replace the player, so neither calls `entity_travel_to_dimension` or `player_changed_dimension`.
An ordinary End portal exit teleports the same player and runs the travel hooks, not the respawn hooks.
The player keeps ticking in the End while the destination is being resolved. The travel veto sees the final
level, including the world spawn if the saved block is missing, and refusing it leaves the player in the End.
An exit to a point in the End itself sends a position update with the usual pitch clamp; it does not fire
`player_changed_dimension` or the command-only `player_teleported` hook.

To change the saved point copied to the replacement, use `event.player.set_respawn_config(host, Some(config))`
inside `player_respawn_position` and set `*event.copy_original_spawn_position = true`. This works even
on the removed player after death. It does not change the destination of this respawn: edit the event's
`level`, `position`, `yaw` and `pitch` for that.

`player.respawn(host)` returns `Result<(), RespawnRefusal>`, and each refusal changes nothing, checked in
this order: `Client`, `InPlayerHook` (a player's state is swapped in), `Stale` (no player on this server behind the
handle), `NoConnection` (a fake player, or the world's own player in a single-player world, which respawns through its
own screen) and `Alive` (health above 0 and the game not won: the case the button ignores; a player who has won the game and is watching the End poem is respawned even alive, as the button does).

## Player rules and sleep

Reads accept `Host`; writes accept `ServerHost`.

| Method | Behavior |
|---|---|
| `may_use_item_at(host, pos, face, stack)` | Applies `Player.mayUseItemAt`: builders may use the item; adventure and spectator players need matching `can_place_on`. |
| `can_harm(host, other)` | Applies `ServerPlayer.canHarmPlayer`, including the `pvp` rule and teams. |
| `respawn_config(host)` | Reads the respawn point; `Some(None)` means no point is set. |
| `set_respawn_config(host, config)` | Sets the point as `/spawnpoint` does, including the `player_set_spawn` veto. Refuses stale, unavailable or dead players (except the original in `player_respawn_position`), unknown dimensions, non-finite yaw and canceled changes. |
| `text_filtering(host)` | Reads the option announced by the player's client. |
| `sleep_ignored(host)` | Reads the sleep-count exclusion setting. |
| `counts_for_sleep(host)` | Reports whether the player counts toward the night skip. |
| `sleep_timer(host)` / `set_sleep_timer(host, ticks)` | Reads or writes `Player.sleepCounter`, from 0 to 109; 100 means slept long enough. The write refuses larger values, fake players, guests in another level and stale players. |

A second disconnect on a guest already leaving is a no-op and keeps the first reason.

Vetoing a player's movement: the `player_move` hook hears each validated movement report whose position or rotation differs from the player's last reported location, after the server's own checks and before the report is committed, with the exact `from`, `to` and both rotations. The last reported location is kept on the player's connection, apart from the accepted position: it is the position given to the listener before it runs, and the destination of every server-side teleport (a join, a teleport in the level, a level change, a respawn, the resend of an unconfirmed teleport and the correction after a veto); `from` is that location. Set `cancel` and the report is not applied; the client is sent back to the accepted position and rotation. The report was already seen by the server's own checks before the hook, so what a violation of those would have done still happens. The engine applies no threshold, so a mod that wants to ignore small moves compares the two positions itself (`(to - from).length() < 1.0 / 16.0` skips anything under 1/16 of a block). A held report is ruled on in the same tick; that player's later packets (further reports, the end of the client tick, pongs) wait in order behind it, then go on in that tick, so each later report is heard from the one before. A report is out of time at its 21st tick, counting the tick it was held at as the first: it is dropped with the correction, never committed, even when a ruling arrives after that; a disconnect, teleport or level change drops it cleanly. Not heard: the first tick in a level (the tick after a join or a level change; the second is heard), a player riding something (`vehicle_moved` hears the vehicle), any teleport (its confirmation's own position is validated from the teleport's target and committed without the hook, and so is the veto's own correction; a report that arrives while a teleport is unconfirmed is not heard either) and a passage crossing (its caller reads the position at once, so the report is committed in place). The first report after a teleport that passed validation is heard, with the teleport's destination as its `from`. With or without a listener, accepted movement finishes its game effects before later packets run. Only a listening mod causes a validated report to be held before it commits. A native listener that never returns is out of scope: it blocks the game thread like any native code, and the 20-tick deadline covers only a report held pending across ticks.

## Text conversions

`host.text_from_json(json)` reads one JSON or SNBT component the way `/tellraw`
does and returns `Result<Option<Text>, TextFormError>`: `Ok(None)` when it does
not parse as a single component, `Err` when the text is over the size or
nesting limits. An object needs one content key (`text`, `translate`, `keybind`,
`score`, `selector`, `nbt` with a source, or an inline object); a `type`
binds alone and the other content keys are ignored, a null `type` counts as
absent. `text` must be a string, `extra` a non-empty list (null counts as
absent), `selector` a selector and `nbt` an NBT path that parse, and a style
field that is present and wrong (`color`, `shadow_color`, the format flags,
`click_event`, `hover_event`, `insertion`, `font`) refuses the whole component.
Command, URL and page click actions use `command`, `url` and `page`, respectively;
a legacy `value`-only click for those actions is refused. `open_url` requires a
well-formed HTTP(S) URI; a malformed URI or another scheme refuses the component
(`Ok(None)`).
`show_item` refuses `minecraft:air`; a component removal (`!id`) requires an object value.
A translation's `fallback` and an `nbt` component's `interpret`, `plain` and
`separator` of the wrong kind are dropped instead. An object whose only content
key is `index` is refused, as in vanilla; Sunrise follows vanilla there. JSON
escapes such as `\u0041` are decoded.

`host.text_from_legacy("§cRed")` reads `§` formatting codes as
the font draws them; `&` codes are not read, so replace `&` with `§` first.
Both work on either side.

## Server clock, rules and distances

`level.set_clock_time(host, ticks)` does what `/time set` does in that level's
default clock; `time_skip` fires with cause `mod` and a mod can keep the old
time. `level.set_game_rule(host, "keep_inventory", GameRuleValue::Bool(true))`
does what `/gamerule` does and fires `game_rule_changed` with no actor. Both
return `false` for a stale handle, a client, or a value the rule refuses.

On a server, `host.tick_times()` returns the last 100 tick durations in
nanoseconds (the tick numbered `n` at index `n % 100`; `tick_stats()` has the
average). `host.server_view_distance()` and `host.server_simulation_distance()`
read the distances in chunks; `host.server_set_view_distance(d)` and
`host.server_set_simulation_distance(d)` change them live for 2 to 32 on a
dedicated server and return `false` otherwise. `server.properties` is not
written.

`host.tick_rate_is_frozen()` is `TickRateManager.isFrozen` (`/tick freeze`): `Some(true)` while the server's
ticking is frozen, `None` on a client. A frozen server still runs the ticks `/tick step` asks for, so
`server_tick_start`'s `world_running` can be true while it is frozen; read this to tell a frozen server from a
paused level.

## Finding blocks in a chunk

`chunk.sections_with_block(host, block)` names the sections of a loaded chunk whose block palette may hold any
state of `block`, as section Y coordinates (block Y `>> 4`) from the bottom up, without reading their blocks
(`PalettedContainer.maybeHas`). Read the blocks of the sections it names to find the ones that stand there: a
palette keeps a state no block uses any more, and a section of more distinct states than a palette holds is
named whenever the block exists. A mod that points at the nearest block of a kind asks it on `chunk_load` for
blocks placed before it tracked them. `None` for a stale handle, on a client, or for a chunk that isn't
loaded; it loads nothing.

## World mutation: chunks, blocks shown and placed as a player

For mods that write blocks by other means (an editor's own section writes, a schematic) and then have to make the game and the client agree. Server only. Each method returns `Result` and names every refusal; a refusal changes nothing.

| Method | Behavior |
|---|---|
| `level.relight_chunks(host, &[[cx, cz]])` | Queues the given loaded chunks of the level in place, and the loaded chunks around them, to be lit again whole at the level's next light update, as `edit_blocks` and `level.check_light` queue theirs; nothing is lit during the call. A column whose eight neighbours are not all loaded waits until they are. The light that changed goes to the players who see it. There is no completion event: read the light on a later tick (`level.brightness`), or `chunk.resend` once it is there. Returns how many columns were queued, the ring included (columns not loaded or already waiting are not counted). Refuses with `ChunkRefusal::{Stale, Client, NotInPlace, TooManyChunks { asked, max }}` when the call has more than 256 chunks or would take the level's queue past 4096 columns; `max` is what a call may take now. |
| `chunk.resend(host)` | Queues the loaded chunk to go whole (light, block entities, biomes) to its watchers at the end of the tick, as an `edit_blocks` chunk does. `Ok` with nothing sent when nobody watches. Refuses with `ChunkRefusal::{Stale, Client, NotInPlace, NotLoaded}`. |
| `player.sees(host, target)` | Whether the player's client is shown `target` by the tracker's rule (`ServerPlayer.broadcastToPlayer`): a spectator only to spectators watching through their own camera, a player hidden with `hide` only to players of at least that level. Distance is not part of it. Refuses with `PlayerWorldRefusal::{Client, Stale, NoConnection}` (a fake player is no viewer or target). |
| `player.ack_block_changes(host, sequence)` | Acknowledges the player's block actions up to `sequence` for an action a mod handled on its own channel; the ack goes after the next tick's block changes, taking the maximum pending sequence. A lower value after a higher one has no effect. Any nonnegative value is accepted; a value too high can end pending client predictions early. Refuses with `Client`, `Stale`, `NoConnection` and `NegativeSequence { sequence }`. |
| `player.show_block(host, pos, state)` | One block update to that player alone; the world is unchanged and the real block returns when it or its chunk is next sent, so a mod sends it again while it wants it shown. The override is not remembered and does not itself acknowledge or clear pending client predictions. Refuses with `Client`, `Stale`, `NoConnection`, `UnknownState` and `PositionNotSendable { pos }` (y outside -2048..=2047, x or z outside -33554432..33554432: the packet's position wraps there). |
| `player.place_block(host, level, pos, state, update_neighbour_shapes)` | `level.set_block` with a placement's flags (neighbours told and, unless `update_neighbour_shapes` is false, their shapes updated; the block entity follows that write), then `Block.setPlacedBy` of the block standing there, in the player's turn (a bed gets its head, a door its upper half). It is not vanilla's use-on and not "as player" in the full sense: no survival, obstruction, build-permission or spawn-protection check, no `placed_block` advancement or statistic, generic item-placement sound or game event, or item used. `setPlacedBy` gets no placer but can have block-specific effects. Refuses with `Client`, `Stale`, `Dead`, `StaleLevel`, `OtherLevel` (not the level in place, where the player is), `UnknownState`, `OutOfWorld { pos }`, `NotLoaded { pos }`, `Fenced` (a carrier transfer holds a directly written cell: origin, companion or callback write; neighbour updates follow the level's normal rules), `InTransaction` (the placement would summon a wither while a Host transaction is open), and `Unchanged` (that state already stands there or the write was refused). |

Where Sunrise differs from vanilla: vanilla sends and relights per edit and has no call that lights a set of chunks at once, so `relight_chunks` only queues the work (256 chunks a call, 4096 columns waiting) and never stalls the server thread; `show_block` and `place_block` check the position and the state before anything runs, where vanilla's packet and `setBlock` would wrap or ignore them.

A command usage may also name `<pos:vec3>` (`Vec3Argument.vec3()`): the point typed, delivered as `CommandArgumentValue::Vec3([f64; 3])`. A whole x or z is the block's centre (`1` is `1.5`) and y is as typed; relative and local coordinates resolve against the command's source. Fewer than three numbers, or text, is a parse error and the command does not run.
## Console and player limit

For a mod that mirrors or drives the server's console (a Discord console channel, a remote assistant) or
sets how many players may be on. Server only.

| Call | Behavior |
|---|---|
| `console_line` hook | Observe operator-visible log lines on dedicated and integrated servers: `level` (`INFO`, `WARN` or `ERROR`), `thread`, and `message` without the log prefix. The tap starts on the first tick with an enabled listener; when idle it does not capture lines, allocate a queue or take the queue lock. Lines from every logging thread arrive in order on the server thread at the start of the next tick. The queue keeps the oldest 4096 lines and drops newer ones until it drains. `dropped` gives the count of those newer lines on the batch's first event, and is zero on its other events. Each line keeps up to 8192 bytes of `message` and 128 of `thread` at a character boundary. The listener cannot change the log, and its own log calls are not captured. Paper and NeoForge have no matching event. |
| `host.commands().run_command_with_output(line)` | `run_command` with its feedback kept, on the same run path and source (the console's: one leading slash dropped, permission level 4, from the overworld's spawn): the same `CommandResult` as `entity.run_command_with_output` (see [server commands](server-commands.md)), with `success` (what `run_command` returns) and `feedback`, what the console would print as plain text, at most 256 lines of 4096 bytes each (further lines dropped, a longer one cut at a character boundary). Only the command's own feedback is kept, never lines logged meanwhile. A command that ran and failed is `success: false` with its message in `feedback`. A line run where no server is open to players, or empty once whitespace and the slash are dropped, runs nothing and is `success: false` with no feedback, as `run_command` answers `false` for it; there is no refusal type, as nothing here tells those apart from a failure that `run_command` cannot. The dedicated console's own commands (`stop`, `op`, `whitelist`, `ban`) are not reached and answer as a command that failed. A call made while another one runs captures its own feedback; the outer capture keeps what it had. |
| `host.server_max_players()` / `host.server_set_max_players(n)` | Paper's `Server.getMaxPlayers` and `setMaxPlayers`: the live cap used for logins and shown in the server list. The read is `None` where no server is open to players. The write takes effect on the next login and ping; already-connected players stay, and operators flagged `bypassesPlayerLimit` on a dedicated server still bypass the cap. Zero refuses every login without the bypass. This call does not write `server.properties` (a restart restores its value; the management protocol's `max_players` setting does write it). Refuses without changing anything with `PlayerLimitRefusal::NotOpen` or `TooHigh { maximum }` for a limit above `i32::MAX`, vanilla's `int`. |
## Inventory, ender chest and saved data

For mods that inspect or edit an online player's state (a player-data sync, `/invsee`, `/endersee`). Writes require a server host and refuse stale handles; reads that also have a client view are marked below.

| Method | Behavior |
|---|---|
| `with_inventory(host, read)` | Lends an `InventoryView` by `Inventory.getItem`'s numbers: 0 to 35 main (0 to 8 hotbar), 36 feet, 37 legs, 38 chest, 39 head, 40 offhand, 41 body, 42 saddle. On a client, reads only the local player's copy (server slot packets plus local edits), not authoritative server state. |
| `set_inventory_slot(host, slot, stack)` | `Inventory.setItem`: `None` empties the slot; there is no item check, so a worn slot takes any item. The client is sent the slot with the tick's inventory sync, and an open inventory menu shows the written stack. Returns `Result<(), PlayerDataRefusal>`; each refusal writes nothing: `Stale` (no online player), `Client`, `Dead`, `SlotOutOfRange { slot, last: 42 }` (never a panic) and `Stack(StackWriteError)` (a count or component value the item does not allow, an item the world lacks). |
| `with_ender_chest(host, read)` / `set_ender_chest_slot(host, slot, stack)` | Ender chest slots 0 to 26. The scoped read is server-only and reads the live slots when the chest is open. The setter refuses an out-of-range slot with `SlotOutOfRange { last: 26 }`; an open ender chest menu shows the written stack. |
| `open_ender_chest(host, title)` | Opens the player's own ender chest with no block, as a satchel or a command would: `ChestMenu.threeRows` over `PlayerEnderChestContainer` with no active chest, titled `title`, so it stays valid wherever the player goes. The game moves the stacks for every click, shift-click, drag and the close, exactly as for an ender chest block's menu. It replaces the menu they had open, and `container_opened` may cancel it (`player_container_open` is the notice after it opened and cannot). Called inside a hook of the player's own action (`use_item`, `right_click_item`, `right_click_block`, their command), it opens when that action ends, as a block's menu does. `Ok(true)` inside an action means queued, and still subject to cancellation when the queue runs; outside one, it opened. `Ok(false)` is an immediate cancellation by a mod, `Err(PlayerUnavailable)` for a stale, dead or fake player or on a client, `Err(Title)` for a title that could not cross. |
| `give_stack(host, stack)` | Gives one stack as `/give` does: matching stacks, then empty slots, and what does not fit is thrown from the player's eyes for that player alone, with no pickup delay. Refuses with `Stale`, `Client`, `Dead` or `Stack`, giving nothing. A stack that fits spawns the short-lived pickup visual and plays the pickup sound. These effects occur in the player's level, even when the hook runs in another one. `Ok(None)` means every item was accepted. The ordinary entity-join hook can still cancel the overflow drop. If the player's level is unavailable, returns the overflow as `Ok(Some(rest))`. |
| `persistent_data(host)` | Borrows the player's `NeoForgeData` compound (NeoForge's `Entity.getPersistentData`) for the lifetime of the shared Host borrow. `None` for an absent compound, an unavailable player or a client; `Some` can hold an empty compound. Clone deliberately if you need an owned snapshot. |
| `set_persistent_data_key(host, &key, value)` | Edits one literal `JavaString` key. `Some(&tag)` inserts or replaces it, creating the compound if needed; `None` removes just that key. Removing the last key leaves a present empty compound; removing a missing key does nothing. Returns `Result<(), PlayerPersistentDataError>`. |
| `host.item_from_snbt(snbt)` | Decodes a saved-item compound from SNBT: `{id:"minecraft:stone",count:3,components:{...}}`. A missing `count` defaults to 1; unknown fields such as `Count` are ignored. Returns `None` for text that is not one compound with only whitespace around it, air, unknown items, and patches naming a transient component. A present `count` must be numeric and in 1 to 99 after Java's `Number.intValue` conversion. The text parser also enforces binary NBT's 2 MiB input and 512-level nesting bounds. |

Both scoped reads return `Some` of the callback's result even when the inventory is empty,
or `None` without calling it when the player or inventory is unavailable. Inside the callback,
`view.get(n)` reads slot `n`, not the `n`th occupied slot. It returns `None` for an empty,
unreadable or out-of-range slot. `view.iter()` visits readable nonempty slots in ascending
order without renumbering them. Read item, count and damage directly inside the callback:

```rust,ignore
let first_hotbar_count = player.with_inventory(host, |view| view.get(0).map(|stack| stack.count()));
let main_item_count = player.with_inventory(host, |view| {
    view.iter()
        .take_while(|(slot, _)| *slot < 36)
        .map(|(_, stack)| u64::from(stack.count()))
        .sum::<u64>()
});
```

To decode components or retain an owned `ItemStack`, project just the stack you need:

```rust,ignore
match player.with_ender_chest(host, |view| view.get(26).map(|stack| stack.project())) {
    None => { /* the chest is unavailable */ }
    Some(None) => { /* the slot is empty or unreadable */ }
    Some(Some(None)) => { /* the stack cannot be projected */ }
    Some(Some(Some(stack))) => { /* keep the owned stack or read its components */ }
}
```

A projected stack carries its components, so mods can write it back when moving inventories.
`item_from_snbt` does not apply the item's stack or component limits, so its result can still be
refused on write with `Err(Stack(StackWriteError))`.

Persistent data is for ports; new mods should use attachments. It is saved with the player, never sent to clients, and raises no notification. On death respawn and the first exit from the End, only the `PlayerPersisted` key is carried over, whatever its tag type; dimension changes, later End exits and teleports keep all keys.

A key edit refuses without writing on `Stale` or `Client` (dead online players accept edits, matching NeoForge's `Entity.getPersistentData()` and `ServerPlayer.restoreFrom()`), and returns `TooDeep` if the new value would nest a list or compound at file depth 512 or more (including mixed-list wrappers). `InvalidNbt` means the entry has an `End` value, a key or string that does not fit the save's modified UTF-8 field, or a collection too long for the save's count field.

Player inventory writes use vanilla numbers: 0 to 35 main (0 to 8 hotbar),
36 to 39 armour feet to head, 40 offhand, 41 body, 42 saddle.
`ItemPlace::PlayerSlot` uses 100 to 103 for armour and -106 for offhand.
The inventory menu has no slot for body or saddle. All four player data writes
(`set_inventory_slot`, `set_ender_chest_slot`, `give_stack`, `set_persistent_data_key`)
are undone by an open transaction abort; equipment effects and overflow drops
wait for the outermost commit.

## The client's open menu

`Host::player_container_menu(player)` reads on both sides. On a client it returns only the
local player's supported open menu, or the inventory menu when none is open; every other
player returns `None`. `MenuRef::slot_count`, `slot` and `carried` read the client's view:
server slot updates plus local prediction, not server authority. A menu ref becomes stale
when that menu closes or another opens. The server retains its supported-menu restrictions.

## Local player reads on a client

These entity reads also accept a client host. They return `None` for a stale handle or
an entity whose body the client doesn't keep; another player the client tracks is such a
kept body:

| Read | Client value |
| --- | --- |
| `air_supply` | Last server-synced air; the client never predicts air loss or refill. |
| `max_air_supply` | Maximum for the kept entity's kind (300 for the local player). |
| `on_ground` | The local player's own predicted flag; for another kept entity, its client copy's flag. |
| `is_in_water` | The local player's own fluid state, or a kept entity's last client update. |
| `is_swimming` | The local player's swimming flag, not the pose (crawling can share that pose). `None` for other players and other entity kinds. |

Client reads are views, not server authority. Inventory access here is read-only;
inventory and movement writes still require a server host.

The local player's `abilities` read takes `flying` from current client prediction;
the other flags and speeds stay as supplied by the last abilities packet.

## A panic in a player's turn

A hook runs inside the turn of the player whose action fired it. When a hook panics, the panic still ends the
server, but the crash save writes nothing for that player: their `players/data/<uuid>.dat`, stats and
advancements stay as last saved, so an item taken but not yet placed is never duplicated or lost in the player's
files. The world save does not roll back what the turn already changed in the world, and a vehicle that player rides
alone stays in the world. The log names the turn with a warning. A panic your code caught does not count: the record ends with the server tick.
