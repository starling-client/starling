# Placing a block from code

`level.set_block(host, pos, state, flags)` writes a state without running `Block.setPlacedBy`.
The placement operations below run on the server level in place. To change one property
instead of replacing the whole state, use `level.set_block_property(host, pos, key, value, flags)`.
It preserves the holder and the other properties. It returns `Ok(false)` if there is no
change or the position refuses the write. This property write also works in parked server
levels, except inside an open Host transaction (`LevelWriteError::InTransaction`).

- `level.update_from_neighbour_shapes(host, pos, state)` is `Block.updateFromNeighbourShapes`: it returns
  the state fitted to the blocks around `pos` (a fence gets its connections, a stair its corner). Nothing
  is written; pass the result to `set_block`. Neighbours that are not loaded read as air.
- `level.set_placed_by(host, pos, stack)` runs `Block.setPlacedBy` on the block already at `pos`, with
  no placer. First write the state and load the block-entity data. Vanilla's callback can write
  companion cells and produce block-specific effects; it does not place the initial block or copy
  the stack's data into it. `stack` is read only (`None` supplies an empty stack); it need not fit an inventory
  slot. The carrier-transfer fence check covers the origin and cells the callback writes directly,
  including companions. Neighbour updates follow the level's normal rules. If the placement would
  summon a wither, it returns `LevelWriteError::InTransaction` inside a Host transaction.
- `level.place_block(host, pos, state, update_neighbour_shapes)` writes `state` with a placement's
  flags (neighbours and clients told, and unless `update_neighbour_shapes` is false their shapes
  updated) and then runs `Block.setPlacedBy`, so a bed gets its head, a door its upper half and a
  tall plant its top. It returns `Ok(true)` when the block was written and `Ok(false)` for an
  unchanged state, an unloaded or out-of-height position, or a carrier-transfer refusal. The fence
  check covers direct writes (origin, companions and callback writes); neighbour updates follow the
  level's normal rules. It has the same refusal when it would summon a wither in a Host transaction.

```rust,ignore
let fitted = level.update_from_neighbour_shapes(host, pos, door)?;
if !level.can_survive(host, pos, fitted)? {
    return Ok(false);
}
level.place_block(host, pos, fitted, true)?;
```

`can_survive` evaluates the supplied state even when it is not placed yet. Block parts answer in
dispatch order: the first explicit `Some(true)` or `Some(false)` wins, and `None` defers to the next
part. The block's survival rule runs only when every part defers. Vegetation also asks the soil's
`can_sustain_plant` hook before the plant's `may_place_on` hook. It answers only survival: it does
not run `must_survive`, placement-context, obstruction, or permission checks. A client reads its
loaded copy as a prediction. The destination must be in bounds and loaded. `Ok(false)` means the
read succeeded and the state failed the survival check; an unavailable read returns
`Err(LevelReadError)` instead.

`level.place_block` is playerless: it does not fire `entity_place`, and `setPlacedBy` gets no placer or
item. Use `player.place_block(host, level, pos, state, face, update_neighbour_shapes)` to let placement
listeners veto a chosen state attributed to a real player. The face tells the hook which adjacent block
it was placed against. The hook reads that neighbour after staging, and sees the states and block
entities of every cell written by `setPlacedBy`. A veto restores them. If a listener panics, only
that listener's tentative writes roll back; later eligible listeners still run. The block callback
still gets no placer or item. Neither handle runs an item use. Nothing else of `BlockItem.place` runs:
no survival or obstruction check, item placement sound, placement game event or advancement.
`place_block` and `update_from_neighbour_shapes` refuse with `LevelWriteError`:
game event or advancement. `place_block` and `update_from_neighbour_shapes` refuse with `LevelWriteError`:
`Stale` for a dead handle, `Unavailable` on a client or for a level that is not the one in place, and
`InvalidState` for a state the world lacks.

To place from a stack or saved data, write the state with `set_block`, then load the block entity data
if any, then call `set_placed_by` with the original stack (if any). `BlockItem.place` applies item data
before `Block.setPlacedBy`; the standalone call only runs the last step.

## Initialize a block after placement

Implement `BlockPart::set_placed_by` on the placed block to read the supplied stack and initialize
block-entity data or companion state. The part gets the level, position, placed state, optional
placer and optional read-only stack. It runs after vanilla's placement callback, using the same
`ServerHost` transaction if one is open. Direct placement from code has no placer.

A block-item placement runs the block's part before the separate `ItemPart::on_block_placed_by`
notice, with the stack still unconsumed. `level.set_placed_by` runs the block's part but does not
notify an item part.

## Observe the update flags

The server's `block_changed` hook hears a successful state change before its block side effects
and neighbour updates. `event.flags` gives the flags applied to that write. For example,
`event.flags.contains(BlockUpdateFlags::KNOWN_SHAPE)` tells you that this write skips the
neighbour shape passes, even though the state change itself is still reported. If a transaction
staged changes, the notice describes the write applied at commit. Ordinary changes can coalesce;
a captured placement or a strict edit sharing its cell is replayed in call order.
Rebuild listeners against the version 2 hook.
