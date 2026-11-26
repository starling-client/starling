# Is this the right tool for a block

Queries answer whether a tool harvests a block, and how fast a tick removes a supplied state. Use them to show that the block under the crosshair drops loot with the held item, or to preview a dig.

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

## How much of a tick removes

`player.destroy_progress(host, level, pos, state)` is vanilla's
`BlockBehaviour.getDestroyProgress(state, player, level, pos)` for a supplied state. It uses the same path as
the game's own dig: the player's selected hotbar slot, the state's hardness and tool rules, efficiency,
effects, block-break speed, submerged mining and the on-ground penalty, then the `can_harvest_block`,
`player_harvest_check` and `player_break_speed` listeners. It returns the share one tick removes (`0.0` for
an unbreakable state), so a value of `1.0` or more breaks the block at once. Nothing requires `pos` to hold
`state`. On a server it answers for a connected or fake player; on a client, for its local player.

A block Part's `destroy_progress` answer for `state` replaces all of that, as a block overriding
`getDestroyProgress` does in vanilla; the query returns the Part's answer. From inside such an answer the
query gives the game's math for the state and asks no Part, which is how a block mines as another:

```rust,ignore
impl BlockPart for MinesAsPedestal {
    fn destroy_progress(&self, host: HostSide<'_>, event: &mut DestroyProgressEvent) -> Option<f32> {
        // The query takes a server host.
        let HostSide::Server(host) = host else {
            return None;
        };
        let level = event.player.level(host)?;
        let below = [event.pos[0], event.pos[1] - 1, event.pos[2]];
        let pedestal = level.block_state(host, below)?;
        event.player.destroy_progress(host, level, below, pedestal)
    }
}
```

`PlayerRef::destroy_progress` takes a server host, so on a client this Part answers `None` and the
client predicts the block's own progress; the server's dig decides when it breaks. A Part whose
answer needs no query (a fixed share, or one worked out from the state) answers the same on both
sides, and the client predicts it.

It runs those three hooks, so it answers `None` from inside any of them, even for a different state. It also
answers `None` for a stale handle, an unknown state, a dead server player and a level other than the one in
place.
