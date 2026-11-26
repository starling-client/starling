# Piston push reactions

`BlockPart::piston_push_reaction(event)` returns `Option<PushReaction>` for
`event.state`, without a Host. `None` preserves the block properties. Values
are `PushPull`, `Push`, `Popped`, `Immoveable`, and `IgnoreEntity`.
Answers must depend only on state and agree on both sides. They are resolved
when parts are installed at load and reload; simulation and client replay
read one cached answer. Removing the last subscriber restores the properties.

On either side, `host.block_states().piston_push_reaction(state)` (or the same read
through `LevelHost::block_states`) reads that resolved answer without calling the Part.
`Ok(None)` means the block key or ordinal is unknown; `Err(LevelReadError::Unavailable)`
means the backend or required block-state tables are unavailable. This is a state fact, not the
entity reaction (`EntityRef::piston_push_reaction`) or a decision to move a block.

The order is the reaction first, then the shared block-entity rule for any
reaction that would move the block: push-pull, ignore-entity, and push only.
Unbreakable hardness, immovable reactions, and push-only in the wrong direction
refuse before `piston_carries_block_entity` is asked. The carries hook is asked
only for an otherwise-pushable block. For one with a block entity, only
`Some(true)` permits movement; `None` and `Some(false)` refuse it.

The carry machinery takes the entity and attachments out before a
source is overwritten, holds them in the moving piston (including mid-move
saves), and installs them at the destination without inventory drops.
For example, a chest with 27 slots and seven diamonds in slot 4 remains intact
at its source when push only has no carry answer. With carry true, all seven
diamonds remain in slot 4 at the destination, with exactly one block entity.
Push-pull with carry true follows the same rule; push-pull alone is refused.

**Diverges from vanilla: push only never discards a block entity**.
`Popped` destroys the block through the normal drop and removal
paths without asking the carry hook. The chest drops its seven diamonds once,
and no block entity travels. The destruction callback and entity push-reaction
query remain separate APIs.

Position-owned names, lock/loot/source fields, container limits and hopper cooldowns
travel in the carried payload, including saves during movement. Open menus close;
opener counts, animation state and render membership are invalidated rather than
retargeted. The destination becomes a fresh ticker holder. A vanilla entity whose
state exists only in raw saved data or a separate circuit store (for example a
comparator), and any payload borrowed by its ticker, is refused:
there is no live owned payload to transfer safely.

## Destroyed by a push reaction

`BlockPart::on_destroyed_by_push_reaction(level, event)` hears a piston's move destroy the
block (a `Popped` / `Destroy` push reaction, such as a torch's), after the block dropped
its resources and while it is still in its cell. `event.direction` is the way
the blocks move and `event.fluid` is the fluid state the cell holds. The removal still happens: the cell becomes air afterwards.
The level host writes and schedules ticks, but a write to any cell of the move
is lost. The destroyed cells turn to air, and the pushed blocks were read
before, so each moves as it was and the cells they leave and enter end as air
or a moving block, as does the arm. Writes outside the move stay. A move that
`piston_pre` cancels and a client's replay of a move are not heard. A panic is
contained and the move goes on.
