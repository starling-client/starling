# Playing level events

A level event is vanilla's numbered effect at a block: a door's sound, a block's break particles, a
dispenser's smoke. Vanilla has two methods for it, and a `LevelRef` has both. Both are server operations
on the level in place.

- `LevelRef::level_event(host, id, pos, data)` is `Level.levelEvent(null, id, pos, data)`: the players of
  that level within 64 blocks of `pos` get it.
- `LevelRef::global_level_event(host, id, pos, data)` is `ServerLevel.globalLevelEvent`: every player on
  the server gets it, in every level and at any distance.

- `LevelRef::block_break_effect(host, pos, state)` plays the particles and sound of a block of `state`
  breaking at `pos` (`Level.levelEvent(2001, pos, Block.getId(state))`), to the same players as
  `level_event`. Nothing is written. The game resolves the state's number itself, so a mod never
  handles one; prefer it to `level_event` with id 2001.

  The client dispatches the destroyed state's [`add_destroy_effects` Part](../../concepts/parts/block-parts.md#hit-and-destroy-particles)
  before producing debris. A handled callback replaces the debris, not the break sound or any block state.

Both `level_event` and `global_level_event` return `false` without playing anything for an event that carries a block state (2001, 2014, 3008)
when `data` is not one, for a parked level, for a stale handle and on a client.

Inside an open transaction, `level_event`, `block_break_effect`, and `global_level_event` defer publication until
the outermost commit for the owning level and discard on rollback or an aborted nested transaction. Each event replays in the
level where it was requested (checking `global_sound_events` at commit for `global_level_event`) and delivers
packets in call order alongside deferred sounds, particles, and entity events.

## Sending particles

`LevelRef::send_particles` sends a particle to players within 32 blocks in the level. Supply
`ty.options(value)` for a [registered particle type](../client/particles-and-movement-queries.md#particle-providers)
and `None` for its block state. The mod belongs on both sides; custom particles are not sent to
vanilla clients. The server encodes the registered stream value once per packet, and a receiving
client decodes it once for all the spawns. A count of zero still spawns one particle, with
auxiliary values of `speed × spread` on each axis.

## The global events

Vanilla plays three ids through the global form: 1023, the wither's spawn (`WitherBoss`); 1028, the ender
dragon's death (`EnderDragon`); 1038, an End portal opening when the last eye of ender goes into its frame
(`EnderEyeItem.useOn`). The client has a sound for them only in its global handler
(`LevelEventHandler.globalLevelEvent`), so use `global_level_event` for them: as plain level events they
play nothing.

The client plays the sound two blocks from the camera, towards the position its packet carries. Each
player gets a position of their own, as vanilla's `globalLevelEvent` sends it:

| The player | The position they get |
| --- | --- |
| In the event's level, less than 32 blocks from the centre of `pos` | `pos` |
| In the event's level, 32 blocks or more away | the block 32 blocks from them towards the centre of `pos` |
| In another level | the block they stand in |

"The event's level" is the level the `LevelRef` names, the one the event happened in, whatever level
the server is ticking when the packets go out. Distances are from the player's feet. So a player in the
event's level hears it from the event's direction, and a player in another level hears it close by.

With the `global_sound_events` game rule off, `global_level_event` is a plain `level_event`, as in
vanilla: only the players of that level within 64 blocks of `pos` get it, and 1023, 1028 and 1038 play
nothing.

## Emitting game events with context

Use `LevelRef::game_event(host, event, at, context)` to emit a vanilla game event
at an explicit position. For example, `event` can be `"minecraft:block_change"`
and `at` is an `[f64; 3]` position. This is a server operation; vibration listeners
such as sculk sensors and wardens hear the event through the normal game path.

`GameEventContext` mirrors vanilla's `GameEvent.Context`:

- `source_entity: Option<EntityRef>` identifies the responsible entity. Its current
  spectator, sneaking, vibration-dampening and other listener facts are captured
  when the event is emitted.
- `affected_state: Option<BlockState>` identifies the state affected by the action,
  not necessarily the state currently at the position. Wool, for example, dampens
  the vibration when supplied as the affected state.

Use `GameEventContext::default()` when neither field applies. An unknown event, invalid position,
stale or foreign source, invalid state or inactive level returns `false` without
emitting an event. A source must belong to the level in place.

`EntityRef::game_event(host, event)` is the convenience operation for an event at
the entity's position with that entity as source and no affected state. It calls
the same level operation; use the level form when the position or state differs.

## Notify clients and show breaking cracks

### Notify clients

`level.send_block_updated(host, pos)` tells clients to read the current block again.
It does not change that block or mark the chunk unsaved. Repeated notifications of
one position are coalesced until the update log is consumed. The request is recorded immediately,
including inside a transaction; aborting storage does not retract it. A stale,
parked, unloaded or out-of-height write returns `false` and queues nothing.

### Show breaking cracks

`level.set_break_progress(host, breaker, pos, stage)` broadcasts a server crack.
Use a stable negative breaker ID for a machine. Stages `0..=9` draw a crack;
any other stage clears it. Inside a transaction the packet waits for the outermost
commit and aborting sends nothing. This is distinct from
`level.set_client_break_progress(client_host, breaker, pos, stage)`, which draws
only in the local client's view and requires `&mut dyn ClientHost`, not a server
host. Local cracks expire after 400 ticks without an update and clear when the
client changes levels or disconnects.

### Hear breaking cracks on a client

`client_break_progress(dimension, breaker, pos, progress)` tells a client mod each crack its
client level is told of (`ClientLevel.destroyBlockProgress`): the server's crack packets, and
the local player's own dig, whose `breaker` is the player's entity id. `progress` `0..=9` is the
stage; any other value means that breaker's crack went. A server that runs mining itself shows the
player's own crack only through its packets, so this is how a client mod follows how far the block
it mines has cracked:

```rust
{{#include ../../../examples/src/client_misc.rs:client_break_progress}}
```

A dig tells its stage every tick, so keep the listener cheap. A mod's own
`set_client_break_progress` is not told back to it.

### Read tool speed

`level.block_break_speed(host, pos, tool)` reads the tool's destroy speed on that
block, including its `minecraft:tool` component rules and default speed. It does
not include Efficiency, player attributes, effects, fluid/airborne penalties or
block hardness. Bare hands return `1.0`; an unavailable position returns `None`.
This read takes an immutable host and does not modify blocks or tools.

## Hearing and cancelling the server's level events

`server_level_event` is asked for every level event the server sends, the game's and the mods' alike,
before it goes out: the level, the event id, its position and data, and whether it is a global event
(`global_level_event` while the `global_sound_events` rule is on). Return `Flow::Cancel` and nobody
gets it: no packet is sent, and the world's own player neither hears nor sees it. To narrow an event,
for example a wither's spawn heard only nearby, cancel it and play your own sound for the players you
choose with `PlayerRef::play_sound`. Check the id first: the hook is asked for every event.
