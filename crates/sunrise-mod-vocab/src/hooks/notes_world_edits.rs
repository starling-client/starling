//! Falling-block and bulk block-edit notes, discovered with the rest of the hook catalog.
pub trait Notes {
    #[notes = r#"### Falling blocks

`crates/sunrise-client/src/game/modding/native_falling.rs`: falling-block spawning for server mods through a
`LevelRef`, sharing `Game::spawn_falling_block_entity` with `LevelEvent::FallingBlock`.

- `level.spawn_falling_block(host, state, pos, velocity, falling)` spawns a falling block of `state` at `pos` (its
  bottom centre) with initial `velocity` in blocks a tick (`Entity.setDeltaMovement`, kept exact on live state while
  network packets quantize through `LpVec3` on send). It flies, lands and places itself as sand does, or drops as an
  item where it can't. `Falling` holds `drop_item` (default true) and `hurts` (hurts what it lands on as an anvil
  does, 2 a block fallen and 40 at most; default false). Any valid block state is accepted, including air (which
  vanilla discards on its first tick in `FallingBlockEntity.tick`), with no per-level count cap.
- The block at `pos` stays: take it away first with `level.set_block` to make a block fall out of the world, as an
  explosion's debris or a collapsing tower would.
- The named open level can be parked. The call neither requires nor loads the destination chunk;
  section visibility and ticking govern when the admitted entity is active.
- It runs `entity_constructing` and `entity_join_level` at the call. A transaction in the destination level
  delays the join until root commit and makes the returned handle stale on abort or a veto at commit.
  `Ok(None)` means non-finite coordinates or velocity, exhausted entity ids, or an immediate join veto.
  `Err(LevelWriteError)` means a stale or closed level, a client, or an unknown block state.

### Bulk block edits

`crates/sunrise-client/src/game/modding/native_world_edit.rs`: `LevelRef::edit_blocks(host, edit)` for server mods,
on any open server level (active or parked).

- `level.edit_blocks(host, edit)` makes many blocks at once: `BlockEdit` holds a `palette` of block states and
  a list of typed `StatePredicate` filters (exact blocks or tags), and `runs`, each
  `height` blocks up column (`x`, `z`) from `y`, set to palette entry `block`, only where filter `only` passes the block
  there. With `surface` (`Heightmap`: `WorldSurface`, `MotionBlocking`, `MotionBlockingNoLeaves` or `OceanFloor`),
  `y` counts from that height of the column, read once a column before anything changes. Runs are made in order, each
  seeing the changes of those before it.
- The blocks go straight into the chunks, a section packed once however many of its blocks change, as `fill ... strict`
  does: no neighbour is told, nothing drops, and no block updates or ticks; light is worked out again for the chunks
  changed, and they go to the players whole. A block replaced by another loses its block entity, and a new block gets
  the one `set_block` would give it (a mod block's own, included).
- It answers the number of blocks changed. Runs in chunks that aren't loaded, and the parts of runs outside the level's
  height, are left out. It answers `Err`, changing nothing, on a client, for another dimension, when a palette state
  or filter name cannot be resolved, or when a run names a palette entry or filter past the end. Filter properties are
  checked when the mod makes the predicate; NBT is not representable."#]
    fn world_edits();
}
