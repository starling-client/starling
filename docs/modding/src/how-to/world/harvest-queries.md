# Is this the right tool for a block

Two read-only queries answer whether a tool harvests a block on both sides. Use them to show that the block under the crosshair drops loot with the held item.

## The stack alone

`host.block_states().is_correct_tool_for_drops(&stack, state)` is vanilla's
`ItemStack.isCorrectToolForDrops(state)`. It reads the stack's `minecraft:tool` component: the first rule
that sets `correct_for_drops` and names the state's block decides, and a stack with no component or no such
rule is not the correct tool. A rule that sets only a speed is skipped. It ignores whether the block needs a
tool at all, the player, the position and every hook, so dirt answers `false` for a pickaxe. It returns `None`
for an unknown state.

## What the game uses for a player

`player.has_correct_tool_for_drops(host, level, pos, state)` takes `&mut dyn Host` and is the answer the game
itself uses for a dig's progress, for whether a break drops loot, and for a door's break:

1. vanilla's rule: `true` when the block does not need a tool, otherwise whether the player's selected
   hotbar slot is the correct tool (never the offhand);
2. then the block's `can_harvest_block` parts;
3. then the global `player_harvest_check` listeners, each hearing what the ones before left.

Because it **runs those hooks**, it takes a mutable host. Asked from inside a `can_harvest_block` or
`player_harvest_check` listener, it answers `None` to prevent recursion, even for a different state.
Outside those hooks, nothing requires `pos` to hold `state`: pass the pair you are asking about.

Sunrise's sites share this one answer (the part, then `player_harvest_check`), so this one query covers
NeoForge's `Player.hasCorrectToolForDrops(state, level, pos)` and `IBlockStateExtension.canHarvestBlock`.
It differs from NeoForge's break progress, which skips the block override.

On a server you can ask about a connected or [fake player](../entities/fake-players.md); on a client,
only the local player. A fake player uses its own selected slot even outside one of its actions.
The query returns `None` for a stale handle, a dead server player, a player the client has no state for,
an unknown state, a `level` other than the one in place, and a call from inside the harvest hooks.

```rust,ignore
fn harvestable_under_crosshair(host: &mut dyn Host, player: PlayerRef, pos: [i32; 3], state: BlockState) -> bool {
    let level = player.level(host).unwrap();
    player.has_correct_tool_for_drops(host, level, pos, state).unwrap_or(false)
}
```
