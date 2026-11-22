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
