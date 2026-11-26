# Server services and player handles

Use `ServerHost` services to run commands, send messages and find connected
players.

## Services

| Accessor | Operations |
|---|---|
| `host.commands()` | `run_command(line)` runs through the console command path with permission level 4. `feedback(text, broadcast_to_admins)` answers the command source and, broadcast, tells the other operators and the log (`sendSuccess(text, true)`). |
| `host.messages()` | `broadcast(text)` fires the broadcast hook and writes to the server log. |
| `host.players()` | `server_players()` returns `Vec<PlayerRef>` in join order. `acting_player()` returns the optional player whose state is active in the hook. `player_by_name(name)` finds a connected real player by case-insensitive account name, not nickname. `singleplayer_profile()` is an integrated server's launch profile ([below](#identity-and-location)). |
| `host.levels()` | `visit_levels(&mut visit)` visits loaded server levels, Overworld, Nether and End first, then the active extra level and parked extra levels. Before a world opens it completes without calling the visitor; an unreadable phase returns `ServerReadError::Unavailable`. |

The players list and name lookup include the single-player owner, even before
opening the world to LAN. The owner joins the integrated server through memory
and has an ordinary `PlayerRef`. Sending a payload to that ref reaches the client
half through the same registration and negotiation as any other player; no
separate local-player send path is needed.
Fake players are not included in the connected-player list. To look up an
identity or its permission level without a live handle, use the players
service's [profile reads](server-commands.md#game-profiles-including-offline-players).

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
you choose, call `entity.run_command(host, line, PermissionLevel::Gamemasters, false)`: `@s` is that entity and
`~` its position. Pass `true` as the last argument to suppress the player's command feedback and operator
notifications. Other entities already have a silent source. It returns `false` for a line that fails, a stale or
removed entity, and an entity outside the level in place (a player in another level included, since that level is
parked while the hook runs). To run a command in another level, do it from a moment in that level or from a task
scheduled there.

## Identity and location

Read identity through `player.uuid()`, `player.name(host)`,
`player.permission_level(host)` and `player.address(host)`. The address is the
connection IP without a port. `player.client_brand(host)` reads the brand the
client announced. Name, address and brand return `Option<&str>`: borrow them
for comparisons, and use `.map(str::to_owned)` only to keep the text beyond the
host borrow. A stale or unavailable player gives `None`; an acting player without
a known connection gives `Some("")` for address and brand.

`player.virtual_host(host)` returns `Option<&VirtualHost>` on the server. Its
`host` and `port` are the destination sent in this connection's login handshake,
not the peer IP, after any `player_handshake` listener's rewrite. The host keeps its original case and punctuation up to the
first NUL; any suffix is discarded. A transfer uses the new login handshake.
The value comes from the client, so it is not a verified name or necessarily
what the player originally typed. Clone it only if you need it after the host
borrow ends. The host may be empty. The window's own player on an integrated server
reports its memory connection's handshake: the host `local`, standing for the local address, and port 0. It is `None`
on a client, for stale or unavailable players, and for fake players.

`player.announced_channels(host)` returns `Option<Arc<[String]>>` in announcement
order. The snapshot includes `minecraft:register` minus `minecraft:unregister`
from configuration and play. It can outlive the host borrow and does not change
when the client updates its list. Read it with `.iter()` and keep the `Arc` if
you need that exact list later; call the method again to see new announcements.
An acting player without a known connection gets an empty snapshot; a stale or
unavailable player returns `None`.

On an integrated (single player) server, `host.players().singleplayer_profile()` is the profile the
server was created with (`MinecraftServer.getSingleplayerProfile`): the owner's name, UUID and login
properties, borrowed from the host. It is fixed when the server is created, so it reads the same from
`server_about_to_start` and `server_starting` on, before the owner joins and after they leave.
`player.is_singleplayer_owner(host)` says whether a player is that owner, compared as
`IntegratedServer.isSingleplayerOwner` does: by name, ignoring case, not by UUID. Cheats, opening to
LAN, join order and permission level don't change the answer. Use `player.permission_level(host)` to
check for an operator. A dedicated server has no owner: `None` and `false`, and a stale handle is
`false` too. Neither read grants anything or changes who may log in.

`player.latency(host)` reads the ping the Tab list shows, in milliseconds: the keep-alive round
trips averaged three parts old to one part new. It is 0 until the first keep-alive answer, for a fake
player and for an acting player without a known connection. It refuses with `PlayerWorldRefusal::Client`
on a client and `PlayerWorldRefusal::Stale` for a stale handle.

`player.send_ping(host, id)` queues a play-phase ping to that player's current
connection from a mutable `ServerHost`. The ID is a signed `i32`, and a vanilla
client echoes it without a mod. Success means the packet was queued (or staged
in the current transaction), not delivered or answered. `PlayerPingRefusal`
distinguishes a stale handle, no open play connection, and a send failure.

Listen to `player_pong` to observe `event.player`, `event.name` (the profile
name) and `event.id`. This non-cancellable notice runs on the server thread at
the packet's turn, before later input from that connection is applied. Match
pongs by player and ID if you need to correlate them, but the ID is client
supplied and unauthenticated: unsolicited and duplicate pongs are still
reported. Configuration, login, status, and the separate play latency request
do not fire this hook. Neither API changes keep-alive latency.

The online list includes connected players during nested player turns. Name,
permission, address and virtual-host reads work for connected players whose bodies
are displaced by another player's turn; stale handles are refused. Entity and living handles
provide world-state reads, which require a resolvable body. Read position and
rotation through `player.entity().position(host)` and `player.entity().rotation(host)`;
resolve that entity with `host.living(player.entity())` for `living.health(host)`.
These reads use the live player, rather than a single player snapshot.

In `player_stat_award`, find the recipient with
`host.players().player_by_name(event.name)` or use their retained `PlayerRef`.
`player.stat(host, stat_type, key)` reads that player's count before the award,
even if a nested turn has displaced them. This does not make displaced players
available for statistic writes.

On the server, `player.statistics(host)` takes `&dyn Host` and returns owned save
JSON (`Option<String>`): namespaced statistic types and entries under `stats`,
with the current `DataVersion`. An online player without counters has an empty
map; a stale or unavailable player gives `None`, as does a client host.

On the server, `player.advancement_progress(host, &id)` takes `&dyn Host` and
returns `Option<AdvancementProgress>`, the same owned snapshot carried by
`player_advancement_progress`: criteria with their stored grant dates (or `None`),
requirement groups, satisfied group count, and `done`. An untouched loaded
advancement answers with all criteria remaining and `done == false`; an unknown id,
stale or offline player, or client host answers `None`.

On the server, `player.award_advancement(host, &id, criterion)` and
`player.revoke_advancement(host, &id, criterion)` take `&mut dyn ServerHost` and
`criterion: Option<&str>` (`Some(name)` for one criterion, `None` for all
remaining or obtained criteria) and return `Result<bool, AdvancementError>`.
They route through the same engine path as triggers and `/advancement`, granting
rewards, announcements, toasts, hooks and client updates on completion, and
reporting `UnknownAdvancement`, `UnknownCriterion`, `PlayerUnavailable`,
`NotServer`, or `NoBackend` on refusal. Inside a transaction, validation runs
immediately while the change, its rewards, messages and hooks are deferred to
the root commit and dropped on abort.

`player.set_statistics(host, snapshot)` takes `&mut dyn ServerHost` and borrowed
JSON. It upgrades older saves, reports decode errors and keeps recoverable
registered counters. An absent or empty `stats` map clears the counters. It
returns `false` without changing the player for malformed JSON, a snapshot that
cannot be decoded as a map, or a player unavailable for writes. A successful
restore replaces rather than merges counters: omitted counters read as zero,
and only changed statistic-criterion scores are updated. The next regular
statistics request sends the changed counts, including zero for removals, and
the normal player save persists the new map. Restoring does not fire
`player_stat_award`.

`player.level(host)` returns the authoritative `LevelRef`;
`player.dimension(host)` returns its dimension `Identifier`, including registered
custom dimensions. Both accept `&dyn Host`. On a server they identify the player's
current dimension, even when another dimension is ticking. On a client they
identify the local player's level. Unknown, disconnected, replaced or stale handles
return `None`.

## Known recipes

On the server, `player.has_discovered_recipe(host, id)` checks one recipe.
`player.visit_discovered_recipes(host, &mut visit)` visits the qualified IDs in that
player's resident recipe book, whether they are acting now or waiting between turns.
Each ID is borrowed as `&str` for the call; use `id.to_owned()` inside the visitor
if you need to retain one. Return `ControlFlow::Break(())` to stop or
`ControlFlow::Continue(())` to keep visiting. An empty book completes with
`Ok(Continue(()))`; `EntityReadError::Stale` and `EntityReadError::Unavailable` mean
there is no readable book, and neither calls the visitor.

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
| `post_effects(host)` | Reads the ordered `Vec<Identifier>` of active server-driven post effects (`ServerPlayer.getPostEffects`), shared with `/posteffect` and persisted with playerdata. Refuses with `PlayerWorldRefusal::Client` on a client and `Stale` for a stale or offline player. |
| `add_post_effect(host, effect)` | Appends `effect` if not already active (`ServerPlayer.addPostEffect`), marking the player dirty to send `ClientboundPostEffectsPacket` at the end of the tick, and returns `Ok(true)` (`Ok(false)` when already present). |
| `remove_post_effect(host, effect)` | Removes `effect` if active (`ServerPlayer.removePostEffect`), marking dirty for client sync, and returns `Ok(true)` (`Ok(false)` when absent). |
| `clear_post_effects(host)` | Clears all active post effects (`ServerPlayer.clearPostEffects`), marking dirty for client sync, and returns `Ok(true)` (`Ok(false)` when already empty). |

Nickname and sleep settings require an online player. These settings survive
reconnections until the server stops or you change them. Visibility is stored by
UUID: you can use a retained handle after disconnection until its world epoch
closes. You cannot target arbitrary offline UUIDs. Retain the handle value, not a
borrowed service.

## Chat lines and respawn

Use `chat_recipient` for per-player filtering of public chat, such as an ignore list. It is asked once per placed
network player whose chat settings accept player chat, including an eligible sender. Return `Flow::Cancel` to omit
that copy; it doesn't advance the recipient's chat index or add an acknowledgement. The other recipients still
get the original signed body. The server logs and checks trust first, then asks immediately before each eligible
delivery; a disconnected player is not asked. The hook covers player chat accepted by `server_chat` and public
commands `/say` and `/me`, after selector resolution and decoration, not private `/msg` deliveries. For a non-player source such as
the console or a command block, `sender` is zero and `sender_name` is the source's display name.

A public message sent by a command inside `chat_recipient` goes through the same hook again. Use the event's
sender and text to distinguish it from the line that prompted your command. A listener that panics is reported
and disabled for the session without canceling other listeners' decisions or unrelated deliveries.

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
| `sleep_timer(host)` / `set_sleep_timer(host, ticks)` | Reads or writes the signed `Player.sleepCounter`. The write does not move the player or change bed occupancy; the normal tick handles the sleep and wake thresholds. |

A second disconnect on a guest already leaving is a no-op and keeps the first reason.

Vetoing a player's movement: the `player_move` hook hears each validated movement report whose position or rotation differs from the player's last reported location, after the server's own checks and before the report is committed, with the exact `from`, `to` and both rotations. The last reported location is kept on the player's connection, apart from the accepted position: it is the position given to the listener before it runs, and the destination of every server-side teleport (a join, a teleport in the level, a level change, a respawn, the resend of an unconfirmed teleport and the correction after a veto); `from` is that location. Set `cancel` and the report is not applied; the client is sent back to the accepted position and rotation. The report was already seen by the server's own checks before the hook, so what a violation of those would have done still happens. The engine applies no threshold, so a mod that wants to ignore small moves compares the two positions itself (`(to - from).length() < 1.0 / 16.0` skips anything under 1/16 of a block). A held report is ruled on in the same tick; that player's later packets (further reports, the end of the client tick, pongs) wait in order behind it, then go on in that tick, so each later report is heard from the one before. A report is out of time at its 21st tick, counting the tick it was held at as the first: it is dropped with the correction, never committed, even when a ruling arrives after that; a disconnect, teleport or level change drops it cleanly. Not heard: the first tick in a level (the tick after a join or a level change; the second is heard), a player riding something (`vehicle_moved` hears the vehicle), any teleport (its confirmation's own position is validated from the teleport's target and committed without the hook, and so is the veto's own correction; a report that arrives while a teleport is unconfirmed is not heard either) and a passage crossing (its caller reads the position at once, so the report is committed in place). The first report after a teleport that passed validation is heard, with the teleport's destination as its `from`. With or without a listener, accepted movement finishes its game effects before later packets run. Only a listening mod causes a validated report to be held before it commits. A native listener that never returns is out of scope: it blocks the game thread like any native code, and the 20-tick deadline covers only a report held pending across ticks.

## Experience and abilities

`player.give_experience_points(host, points)` takes a signed `i32`; use a negative value to remove
points. The signed amount after `player_xp_change` also changes the player's score. A level-up
sound from a point or level grant plays in the receiving player's level. The write never repairs
an item: Mending stays at orb pickup. `set_experience_points` sets the progress bar as
`ServerPlayer.setExperiencePoints` does. `set_total_experience` writes the signed total without
changing progress, level or score. `set_abilities` stores both speeds exactly as supplied,
independently of the game-mode flags.

## Server sound writes

Pass a namespaced event key to `PlayerRef::play_sound`, `EntityRef::play_sound`,
`EntityRef::play_bound_sound` or `LevelRef::play_sound`. The server sends a registered event by
its ID or an inline event by name, regardless of namespace. Success means the sound was accepted
for delivery; a client can only play an event defined in its sound resources. Volume and pitch
are sent as supplied and processed by the recipient's sound engine.

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

`host.text_to_nbt(&text)` encodes the component as vanilla's `ComponentSerialization.CODEC`
with `NbtOps`, without resolving translations or adding styles. It returns `Result<Tag, TextFormError>`.
`host.text_from_nbt(&tag)` reads that form, returning `Result<Option<Text>, TextFormError>`.
An unstyled literal without siblings encodes as an NBT string, and strings are read literally,
including `§` codes. NBT flags are true for nonzero numeric tags, and custom click payloads
keep their tag types. Both methods enforce vanilla's 512-container NBT depth limit.

`host.text_from_legacy("§cRed")` instead reads `§` formatting codes as
the font draws them; `&` codes are not read, so replace `&` with `§` first.
These conversions work on either side.

## Server clock, rules and distances

`level.set_clock_time(host, ticks)` does what `/time set` does in that level's
default clock; `time_skip` fires with cause `mod` and a mod can keep the old
time. `level.set_game_rule(host, "keep_inventory", GameRuleValue::Bool(true))`
does what `/gamerule` does and fires `game_rule_changed` with no actor. Both
return `false` for a stale handle, a client, or a value the rule refuses.

On a server, `host.visit_tick_times(&mut visit)` visits the tick durations in
nanoseconds in vanilla ring-slot order (tick `n` at index `n % 100`; `tick_stats()`
has the average). As with `visit_levels`, return `ControlFlow::Break(())` to stop
or `Continue(())` to keep visiting. An unreadable phase returns
`Err(ServerReadError::Unavailable)`. `host.server_view_distance()` and `host.server_simulation_distance()`
read the live distances in chunks, or return `None` without a logical server
player list. `host.server_set_view_distance(d)` and
`host.server_set_simulation_distance(d)` update the distances sent to players
and used for chunk tickets on either server, even in a world not open to LAN.
The setters take 2 through 32 chunks and return `false` and log a refusal for
an out-of-range distance or an unavailable player list.

The setters change neither `server.properties` nor the owner's options. In an
integrated world, the owner's render distance takes over at the end of an unpaused
server tick, including the tick in which a listener wrote the distance. The owner's
simulation distance only takes over when that option changes.

`host.set_tick_rate(tps)` uses the integrated or dedicated server's clock setter, not
`/tick rate`'s parser. Finite rates below 1 become 1; the command's upper limit does
not apply. A non-finite rate or a call off the logical server returns `false` and logs
the refusal. Even an equal write fires `server_tick_rate_changed` before the server
adjusts the autosave countdown and sends the final rate and freeze state to clients.
A listener can retune the clock through the same setter. The hook also runs when a
server sprint ends, not when a client receives the server's rate.

`host.server_is_published()` is `MinecraftServer.isPublished`: always true on a
dedicated server, and on a single-player world's server only while the world is
open to LAN, so it turns false again when the LAN closes (`/unpublish`). A rule
meant for multiplayer only reads it when it applies, not once at start: no hook
fires when a world is opened to LAN.

`host.tick_rate_is_frozen()` is `TickRateManager.isFrozen` (`/tick freeze`): `Some(true)` while the server's
ticking is frozen, `None` on a client. A frozen server still runs the ticks `/tick step` asks for, so
`server_tick`'s `ServerTickPhase::ServerTickStart { world_running }` can be true while it is frozen; read this to tell a frozen server from a
paused level.

## Weather

Use the setter on any resident server level to request a storm for the server:

```rust,ignore
use mod_api::{LevelRef, ServerHost};

fn request_storm(host: &mut dyn ServerHost, origin: LevelRef) -> bool {
    origin.set_weather(host, 0, 600, true, true)
}
```

The handle names the origin, even if that level is parked. The clear timer and the
shared rain/thunder timer are signed tick values, not `/weather` command arguments.
[`weather_change`](../../concepts/hooks-and-the-host.md#weather-parameters-and-strengths)
can veto all the timers and flags together. The call returns `false` on a final veto
or invalid level; a veto does not undo other writes a listener already made.

To change the interpolated strengths immediately instead, call
`level.set_weather_levels(host, 0.5, 0.25)` on the resident level the handle names,
including a parked level. Both strengths clamp to 0–1, including infinities. NaN
or an unavailable level is refused. This setter changes neither other levels'
strengths nor the shared parameters, and does not call `weather_change`.
`level.weather(host)` reads the named level's own strengths (thunder scaled by rain)
at partial tick 1. These strengths stay with the level across swaps and idle ticks.
`level.is_raining(host)` and `level.is_thundering(host)` compare those strengths, not
the shared flags, against the double thresholds `0.2` and `0.9` respectively.
`0.2_f32` counts as rain in a level that can have weather.

## World borders

`level.world_border(host)` reads `[min_x, min_z, max_x, max_z]` for the resident
server level the handle names, including when another level is in place. It
returns `None` for a stale or closed level. These methods write through the
same setters as `/worldborder`, without the command's range and no-change checks:

- `level.set_world_border_center(host, [x, z])` sets a finite center. The
  geometry clips the edges, not the center itself.
- `level.set_world_border_size(host, size, ticks)` sets a finite size. A size
  is refused if the clamped and outward-rounded collision bounds would be
  inverted. Nonpositive ticks set a fixed size; positive ticks request a timed
  change from the current size.
- `level.set_world_border_damage(host, per_block, safe_zone)` accepts signed,
  finite damage settings. `level.set_world_border_warning(host, blocks, ticks)`
  accepts signed warning settings.

For example, request a border of 256 blocks over 200 ticks in the level a
`LevelRef` names:

```rust,ignore
use mod_api::{LevelRef, ServerHost};

fn resize_border(host: &mut dyn ServerHost, level: LevelRef) -> bool {
    level.set_world_border_size(host, 256.0, 200)
}
```

The setters return `false` and log refusals (for example, an invalid size or
stale handle). They do not open or switch levels. Successful center, size and
warning writes notify that level's watchers, even when a direct write repeats
a value. Damage settings stay on the server.

The `world_border_center_changed` and `world_border_bounds_changed` listeners
can read the changed border with `event.level.world_border(host)`. The bounds
listener also gets `event.lerp_ticks`: `None` for a fixed write, or
`Some(requested_ticks)` for a timed write. A timed write to the current size
still fires that start notice but creates no moving extent.
`world_border_bounds_change_finish` names the level only after an actual
moving extent reaches its target at the start of the level's normal tick, even
if it is idle and parked. The server's tick functions run before the level's
border advances. Loads and client packet application do not trigger these notifications.

### Calling a setter from a listener

For the weather, border and tick-rate hooks above, a nested setter can run.
Its dispatch skips callbacks for that hook already running on the thread, not
other eligible listeners. If a mod's callback panics, Sunrise logs the mod and
hook, shows an error toast and disables just that callback for the rest of the
session. Vetoes and hook refusals are logged each time and do not disable the
callback. These hooks do not roll back writes a listener already made.

## Account profiles

`host.profiles().resolve_name(name)` and `resolve_id(uuid)` look an account's profile up
through the server's own resolver, the one `/fetchprofile` and dynamic player heads use, also on
an offline-mode server. The first call starts the lookup and answers
`ProfileResolution::Pending`; call again with the same name or UUID to poll it until it answers
`Done`, with the `AccountProfile` (UUID, the account's own spelling of its name, and the signed
properties such as `textures`) or `None`. As in vanilla, a failed request also answers `None`, and
that answer is kept like any other: the same name or UUID answers `Done(None)` until the entry
expires, ten minutes after its last lookup.

## Finding blocks in a chunk

`chunk.sections_with_block(host, block)` names the sections of a loaded chunk whose block palette may hold any
state of `block`, as section Y coordinates (block Y `>> 4`) from the bottom up, without reading their blocks
(`PalettedContainer.maybeHas`). Read the blocks of the sections it names to find the ones that stand there: a
palette keeps a state no block uses any more, and a section of more distinct states than a palette holds is
named whenever the block exists. A mod that points at the nearest block of a kind asks it on `chunk_load` for
blocks placed before it tracked them. `None` for a stale handle, on a client, or for a chunk that isn't
loaded; it loads nothing.

## Loading a chunk

`level.load_chunk(host, [x, z])` is vanilla's `Level.getChunk(x, z)` (`ServerChunkCache.getChunk(x, z, FULL, true)`):
it loads the chunk from the save, or generates it, to FULL, in any open server level, and returns `Ok(())` once it
is there with its initial light done, so the level's block and light reads find it. To light it, the 8 chunks around
it are loaded too (vanilla only generates them part of the way); they hold no ticket and unload with the next sweep. **It blocks the server tick until the chunk is loaded or
generated**, as vanilla does, with no timeout; a chunk already full answers at once. Generating a new chunk takes
long enough to be felt, so load ahead of time rather than many chunks in one tick. Like vanilla, the call leaves an
`UNKNOWN` ticket on the chunk that keeps it loaded (not ticking) through the next ticket purge and expires at the one
after, and the first purge can come in the same tick, depending on when the call runs; the chunk then unloads like
any other. Loading a chunk of a parked level doesn't put the level in place: `set_block` and the other writes that
need it in place still answer `LevelWriteError::Unavailable` there. To keep a chunk, force it (`chunk.set_forced`)
or hold a ticket controller's
ticket. The `chunk_load` listeners of each chunk the call brings in run during the call, while that chunk is loaded
and before the call returns, as NeoForge's `ChunkEvent.Load` does inside `getChunk`. `Err(ChunkLoadError::Unloaded)`
for a stale handle, a client, a level the server has not opened, a chunk past `ChunkPos.isValid`
(`ChunkPyramid.MAX_CHUNK_COORDINATE_VALUE`, where vanilla's block reads and writes stop), and while the level's
generation is paused or stopped; `Err(ChunkLoadError::Failed)` when a mod's chunk generator panicked for the chunk
or one its generation needs, which the call reports instead of waiting forever.

For bulk or background loads that must not block the server tick, use `level.request_chunk(host, [x, z])` to queue a
non-blocking level-33 (`FULL`, non-ticking) load at priority below player views and `/forceload`, drain finished
[`ChunkSnapshot`] results on subsequent ticks with `level.drain_chunk_requests(host)`, or release a queued ticket
early with `request.cancel(host)`.

## World mutation: chunks, blocks shown and placed as a player

For mods that write blocks by other means (an editor's own section writes, a schematic) and then have to make the game and the client agree. Server only. Each method returns `Result` and names every refusal; a refusal changes nothing.

| Method | Behavior |
|---|---|
| `level.relight_chunks(host, &[[cx, cz]])` | Queues the given loaded chunks of any open server level (active or parked), and the loaded chunks around them, to be lit again whole on that level's light engine, as `edit_blocks` and `level.check_light` queue theirs; nothing is lit during the call. The light engine schedules and batches queued columns across light updates (matching vanilla `ThreadedLevelLightEngine`), re-queuing invalidates any earlier in-flight light job for that column, and a column whose eight neighbours are not all loaded waits until they are. Light changes go to the players watching that level as usual. There is no completion event: read the light on a later tick (`level.brightness`), or call `chunk.resend` immediately to send the chunk once its light work has been applied. Returns how many columns were newly queued, the ring included (columns not loaded or already waiting are not counted). Refuses with `ChunkRefusal::{Stale, Client}`. |
| `chunk.resend(host)` | Queues the loaded chunk of any open server level (active or parked) to go whole (light, block entities, biomes) to players who hold it, including in a viewed level. The broadcast waits for the column's light work pending when `chunk.resend` was called, including any jobs needed to finish edits already queued (matching vanilla `ChunkMap.waitForLightBeforeSending` and `PlayerChunkSender`). `Ok` with nothing queued when no player holds the chunk at the time of the call. Refuses with `ChunkRefusal::{Stale, Client, NotLoaded}`. |
| `player.sees(host, target)` | Whether the player's client is shown `target` by the tracker's rule (`ServerPlayer.broadcastToPlayer`): a spectator only to spectators watching through their own camera, a player hidden with `hide` only to players of at least that level. Distance is not part of it. Refuses with `PlayerWorldRefusal::{Client, Stale, NoConnection}` (a fake player is no viewer or target). |
| `player.ack_block_changes(host, sequence)` | Acknowledges the player's block actions up to `sequence` for an action a mod handled on its own channel; the ack goes after the next tick's block changes, taking the maximum pending sequence. A lower value after a higher one has no effect. Any nonnegative value is accepted; a value too high can end pending client predictions early. Refuses with `Client`, `Stale`, `NoConnection` and `NegativeSequence { sequence }`. |
| `player.show_block(host, pos, state)` | One block update to that player alone; the world is unchanged and the real block returns when it or its chunk is next sent, so a mod sends it again while it wants it shown. The override is not remembered and does not itself acknowledge or clear pending client predictions. Refuses with `Client`, `Stale`, `NoConnection`, `UnknownState` and `PositionNotSendable { pos }` (y outside -2048..=2047, x or z outside -33554432..33554432: the packet's position wraps there). |
| `player.place_block(host, level, pos, state, face, update_neighbour_shapes)` | `level.set_block` with a placement's flags (neighbours told and, unless `update_neighbour_shapes` is false, their shapes updated; the block entity follows that write), then `Block.setPlacedBy` (a bed gets its head, a door its upper half). `entity_place` sees the tentative states and block entities, the full `setPlacedBy` footprint, the supplied player and the staged state opposite `face` before commit. It is not vanilla's use-on and not "as player" in the full sense: no survival, obstruction, build-permission or spawn-protection check, no `placed_block` advancement or statistic, generic item-placement sound or game event, or item used. `setPlacedBy` gets no placer but can have block-specific effects. Refuses with `Client`, `Stale`, `Dead`, `StaleLevel`, `OtherLevel` (not the level in place, where the player is), `UnknownState`, `OutOfWorld { pos }`, `NotLoaded { pos }`, `Fenced` (a carrier transfer holds a directly written cell: origin, companion or callback write; neighbour updates follow the level's normal rules), `InTransaction` (the placement would summon a wither while a Host transaction is open), `Unchanged` (that state already stands there or the write was refused), `Canceled` (a placement veto) and `PlacementChanged` (the hook invalidated the placement). |

Where Sunrise differs from vanilla: vanilla sends and relights per edit and has no call that lights a set of chunks at once, so `relight_chunks` queues the work on the owning level's light engine (which batches worker jobs across light updates like `ThreadedLevelLightEngine`) and never stalls the server thread; `show_block` and `place_block` check the position and the state before anything runs, where vanilla's packet and `setBlock` would wrap or ignore them.

A command usage may also name `<pos:vec3>` (`Vec3Argument.vec3()`): the point typed, delivered as `CommandArgumentValue::Vec3([f64; 3])`. A whole x or z is the block's centre (`1` is `1.5`) and y is as typed; relative and local coordinates resolve against the command's source. Fewer than three numbers, or text, is a parse error and the command does not run.
## Console and player limit

For a mod that mirrors or drives the server's console (a Discord console channel, a remote assistant) or
sets how many players may be on. Server only.

| Call | Behavior |
|---|---|
| `console_line` hook | Observe operator-visible log lines on dedicated and integrated servers: `level` (`INFO`, `WARN` or `ERROR`), `thread`, and `message` without the log prefix. The tap starts on the first tick with an enabled listener; when idle it does not capture lines, allocate a queue or take the queue lock. Lines from every logging thread arrive in order on the server thread at the start of the next tick. The queue keeps the oldest 4096 lines and drops newer ones until it drains. `dropped` gives the count of those newer lines on the batch's first event, and is zero on its other events. Each line keeps up to 8192 bytes of `message` and 128 of `thread` at a character boundary. The listener cannot change the log, and its own log calls are not captured. Paper and NeoForge have no matching event. |
| `host.commands().run_command_with_output(line)` | Runs as the console from the world spawn level and returns `CommandResult { success, feedback, .. }`, a synchronous snapshot of every source message emitted during dispatch as plain text in order, without truncation. Delayed messages emitted on later ticks are not waited for. This is the same result type as [running a command as an entity](server-commands.md#run-a-command-as-a-player). |
| `host.commands().run_command_with_feedback(line, send)` | Runs as the same console from the world spawn level and sends structured `CommandFeedback` to an `std::sync::mpsc` channel as messages arrive, including delayed messages from listeners bound to that command source (such as `/test` summaries). Success, failure, and direct system messages are distinct. Use the receiver to stream or keep output, or limit what your interface displays. |
| `host.server_max_players()` / `host.server_set_max_players(n)` | Paper's `Server.getMaxPlayers` and `setMaxPlayers`: the live cap used for logins and shown in the server list. The read is `None` where no server is open to players. The write takes effect on the next login and ping; already-connected players stay, and operators flagged `bypassesPlayerLimit` on a dedicated server still bypass the cap. Zero refuses every login without the bypass. This call does not write `server.properties` (a restart restores its value; the management protocol's `max_players` setting does write it). Refuses without changing anything with `PlayerLimitRefusal::NotOpen` or `TooHigh { maximum }` for a limit above `i32::MAX`, vanilla's `int`. |

Both output methods drop one leading slash and run at permission level 4 from the world spawn
level (`MinecraftServer.createCommandSourceStack`), restoring the active level afterwards.
Blank input or no open server yields `false` with no messages. Dedicated-console-only commands
such as `stop` and `op` are unknown to this dispatcher. Output goes to the source that issued
that command, not to an outer command's capture. Public `/say` and `/me` announcements are chat,
not source feedback; use `chat_recipient` to observe per-player delivery attempts. Other chat to
recipients, operator broadcasts, and logs are not source messages. Outgoing confirmations addressed
to the non-player console source (such as `/msg`) arrive as `CommandFeedback::System`. The console accepts success
feedback even when `send_command_feedback` is off for players. A failed run can already have
sent success messages.

```rust,ignore
use mod_api::CommandFeedback;

let (send, receive) = std::sync::mpsc::channel();
let success = host.commands().run_command_with_feedback(line, send);
for message in receive.try_iter() {
    match message {
        Ok(CommandFeedback::Success(text)) => show_success(text),
        Ok(CommandFeedback::Failure(text)) => show_error(text),
        Ok(CommandFeedback::System(text)) => show_system_message(text),
        Err(error) => report_text_error(error),
        _ => {} // CommandFeedback is non-exhaustive.
    }
}
```

The command runs synchronously on the server thread; a receiver on another thread can read while
it runs and continue reading delayed messages until the channel disconnects. The channel closes
once the command returns and any delayed listener attached to that command source (such as a
`/test` runner's summary displayer or `/fetchprofile`) finishes and drops its source. On the
server thread, poll with `try_iter()` or `try_recv()` rather than blocking iteration so delayed
commands can still tick. Closing the receiver stops output delivery, not execution. The receiver
gets a `TextFormError` in place of any component that cannot be represented as mod API `Text`.

## Inventory, ender chest and saved data

For mods that inspect or edit an online player's state (a player-data sync, `/invsee`, `/endersee`). Writes require a server host and refuse stale handles; reads that also have a client view are marked below.

| Method | Behavior |
|---|---|
| `with_inventory(host, read)` | Lends an `InventoryView` by `Inventory.getItem`'s numbers: 0 to 35 main (0 to 8 hotbar), 36 feet, 37 legs, 38 chest, 39 head, 40 offhand, 41 body, 42 saddle. On a client, reads only the local player's copy (server slot packets plus local edits), not authoritative server state. |
| `set_inventory_slot(host, slot, stack)` | `Inventory.setItem`: `None` empties the slot; there is no item check, so a worn slot takes any item. The client is sent the slot with the tick's inventory sync, and an open inventory menu shows the written stack. Returns `Result<(), PlayerDataRefusal>`; each refusal writes nothing: `Stale` (the player is unavailable), `Client`, `SlotOutOfRange { slot, last: 42 }` (never a panic) and `Stack(StackWriteError)` (a count or component value the item does not allow, an item the world lacks). |
| `with_ender_chest(host, read)` / `set_ender_chest_slot(host, slot, stack)` | Ender chest slots 0 to 26. The scoped read is server-only and reads the live slots when the chest is open. The setter refuses an out-of-range slot with `SlotOutOfRange { last: 26 }`; an open ender chest menu shows the written stack. |
| `open_ender_chest(host, title)` | Opens the player's own ender chest with no block, as a satchel or a command would: `ChestMenu.threeRows` over `PlayerEnderChestContainer` with no active chest, titled `title`, so it stays valid wherever the player goes. The game moves the stacks for every click, shift-click, drag and the close, exactly as for an ender chest block's menu. It replaces the menu they had open, and `container_opened` may cancel it (`player_container_open` is the notice after it opened and cannot). Called inside a hook of the player's own action (`use_item`, `right_click_item`, `right_click_block`, their command), it opens when that action ends, as a block's menu does. An explicit transaction also queues the open until the outermost commit and discards it on abort. `Ok(true)` for a queued open is still subject to cancellation when the queue runs; otherwise it opened. `Ok(false)` is an immediate cancellation or a refusal during a speculative quick move or merchant action, `Err(PlayerUnavailable)` for a stale, dead or fake player or on a client, `Err(Title)` for a title that could not cross. |
| `give_stack(host, stack)` | Gives one stack as `/give` does: matching stacks, then empty slots, and what does not fit is thrown from the player's eyes for that player alone, with no pickup delay. Refuses with `Stale`, `Client`, `Dead` or `Stack`, giving nothing. A stack that fits spawns the short-lived pickup visual and plays the pickup sound. These effects occur in the player's level, even when the hook runs in another one. `Ok(None)` means every item was accepted. The ordinary entity-join hook can still cancel the overflow drop. If the player's level is unavailable, returns the overflow as `Ok(Some(rest))`. |
| `persistent_data(host)` | Borrows the player's `NeoForgeData` compound (NeoForge's `Entity.getPersistentData`) for the lifetime of the shared Host borrow. `None` for an absent compound, an unavailable player or a client; `Some` can hold an empty compound. Clone deliberately if you need an owned snapshot. |
| `set_persistent_data_key(host, &key, value)` | Edits one literal `JavaString` key. `Some(&tag)` inserts or replaces it, creating the compound if needed; `None` removes just that key. Removing the last key leaves a present empty compound; removing a missing key does nothing. Returns `Result<(), PlayerPersistentDataError>`. |
| `host.item_from_snbt(snbt)` | Reads one complete SNBT compound as a saved `ItemStack.CODEC` item: `{id:"minecraft:stone",count:3,components:{...}}`. `count` defaults to 1 and must be in 1 to 99. Trailing whitespace is allowed. The SNBT parser's structural rules apply, not the binary NBT byte quota. On bad syntax, an invalid item or component patch, or an unknown item, returns `None` and logs the failure reason. |
| `host.item_to_json(stack)` / `host.item_from_json(json)` | Encodes and decodes an `Option<ItemStack>` as JSON with `ItemStack.OPTIONAL_CODEC` (`ItemStack.CODEC` wrapped in `ExtraCodecs.optionalEmptyMap`) over `JsonOps`: `None` maps to `{}`, non-empty stacks to `{"id","count","components":{...}}` with `"!id":{}` for removals, and typed JSON values per component codec. Errors return `ItemJsonError` naming the failing field. |

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

The last-death location, inventory and ender chest slots, selected slot, hand items,
and item cooldowns can be set on a resident player even at zero health. A stale
handle or a record suspended by a nested player turn is still unavailable. On
death respawn, the last-death location and ender chest carry over. Inventory,
hand items and selection carry over only with `keep_inventory` or for a spectator.
Actions such as opening the ender chest menu still require a living player.

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
when that menu closes or another opens. `MenuRef::kind` returns `Some(Some(id))` for a supported
registry-backed menu, `Some(None)` for the player's inventory menu or a mount's inventory on the client,
and `None` for a stale, foreign or unavailable handle. The server retains its supported-menu restrictions.

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
| `is_in_lava` | The local player's own fluid state, or a kept entity's last client update. |
| `is_sprinting` | The local player's predicted sprinting flag, or a kept entity's synced flag. |
| `pose` | The local player's predicted pose, or the `DATA_POSE` the server last sent for a kept entity. |
| `is_crouching` | `LocalPlayer.isCrouching` for the local player: its own crouching flag, set before its input each tick, so it can differ from its pose for a tick. A kept entity's crouching pose otherwise. |
| `LivingRef::no_jump_delay` | `LivingEntity.noJumpDelay` of the local player's prediction: 10 after a ground jump, counting down. `None` for other entities. |
| `main_arm` | `LivingEntity.getMainArm` for every living entity the client tracks, as the server last sent it: another player's or a mannequin's main hand, a mob's left-handed flag; the local player's own Main Hand option. `None` for an entity that isn't living. |
| `previous_position`, `previous_rotation` | `Entity.xo`, `yo`, `zo` and `yRotO`, `xRotO`: where the local player's or a ticked copy's tick began, the start of the step a frame eases along (`position_at(host, partial)` is `Entity.getPosition(float)`). `previous_rotation` answers where `rotation` does. |
| `is_alive` | `Entity.isAlive` of the client's copy: a living entity's synced health above 0. `None` for an entity whose body the client doesn't keep. |

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

## Custom statistics

Declare custom statistic identities in `Hooks::register`, using `Registrar`:

```rust,ignore
fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
    registrar.register_custom_stat("steps").unwrap();
}
```

For a mod named `rewrite`, this declares `rewrite:steps` in
`minecraft:custom_stat`. An explicit `rewrite:steps`, another manifest-owned
namespace, or a namespace delegated to the mod through `served_by` (see
[Declare content in code](../content-mods.md#declare-content-in-code)) is also accepted; `minecraft`, foreign namespaces, malformed IDs and
duplicates are refused. Ignoring the returned `Result<(), DeclarationError>` does not
bypass installation failure. A mod's explicit registration refusal rolls back
its declarations, and the installed inventory freezes before worlds open.

These declarations populate the integrated command/statistic inventory and
append after vanilla's IDs in LAN and client built-in registry reads, preserving
vanilla IDs. Servers declaring custom statistics require Sunrise clients with
the same ordered statistic inventory; configuration rejects mismatches before
play. Register the target before rewriting an award to it; an arbitrary
identifier alone does not register a statistic.

`/scoreboard objectives add steps minecraft.custom:rewrite.steps` creates an
objective for this registered counter.
