# Breaking blocks as a player

`PlayerRef::destroy_block(host, pos, into_inventory)` breaks a block as that player breaks it,
through vanilla's `ServerPlayerGameMode.destroyBlock` and NeoForge's `CommonHooks.fireBlockBreak`,
as FTB Ultimine does for each block of a vein. On a live block, `break_block` posts pre-canceled
when game mode, the main-hand item's restrictions, or game-master permissions refuse the break;
`#[listener(receive_cancelled)]` listeners can observe those attempts, while game-mode and
game-master restrictions still prevent removal even if the cancel is cleared. This is not a dig
packet: there is no check for spawn protection, reach or mining progress. For example, a survival
player can directly destroy bedrock.

`player_destroyed_block` hears a successful removal. Loot is rolled for the main-hand item and
its enchantments; `block_drops` sees the player as `breaker`; and item wear, statistics and
exhaustion follow the player's break path. With `into_inventory`, the block's remaining item
drops go to the player's inventory after `block_drops`, and overflow drops from them.

The result is `true` only if this call removed the block. It is `false` for a player who is dead
or not present in the hook's level, an absent block, a refusal or veto, or a block a listener
already removed or broke in a nested call at that position in that level (even if a non-air block
or fluid such as fewer turtle eggs or water remains).

## Asking first

A mod that changes blocks itself, such as a protection-aware builder or a block swapper, can ask
whether the player's break or placement would be stopped before it makes the change, as NeoForge's
`CommonHooks.fireBlockBreak` and `EventHooks.onBlockPlace` are asked:

- `PlayerRef::break_canceled(host, pos)` is true when the player's checks refuse the break or a
  `break_block` listener cancels it, without breaking anything. The listeners hear it as a break.
  As in vanilla, a held debug stick takes it as a click and selects its next property.
- `PlayerRef::place_canceled(host, pos, face, state)` is true when an `entity_place` listener
  cancels this player placing `state` at `pos`, where the block goes, having clicked the `face`
  of the block `pos` lies against, without placing anything. It is heard as a single placement
  against the clicked block. The game's own rule is `PlayerRef::may_use_item_at`.

Both are None for a player who isn't here or is in another level than the one in place, and for
an unloaded block. They share the nesting limit below with `destroy_block`.

## A vein miner

A vein miner keeps the drops of its own breaks with `block_drops`: it takes the item list and the
experience during the breaks, without canceling, so the other effects of each break still happen.
Then it merges the stacks and sets them down at the block the player mined, with
`LevelRef::pop_resource` and `LevelRef::award_experience`. Nothing spawns at the other blocks.

```rust,ignore
struct VeinMiner {
    busy: AtomicBool,
    vein: Mutex<Vec<[i32; 3]>>,
    kept: Mutex<(Vec<ItemStack>, u32)>,
}

impl Hooks for VeinMiner {
    fn break_block(&self, host: &mut dyn ServerHost, event: &mut BreakBlockEvent<'_>) -> Flow {
        // Our own breaks come through here too.
        if self.busy.swap(true, Ordering::Relaxed) {
            return Flow::Continue;
        }
        let player = host.players().acting_player().unwrap();
        let origin = event.pos;
        let vein = find_vein(host, origin); // the origin first
        *self.vein.lock().unwrap() = vein.clone();
        for pos in vein {
            player.destroy_block(host, pos, false);
        }
        self.busy.store(false, Ordering::Relaxed);
        let (stacks, experience) = std::mem::take(&mut *self.kept.lock().unwrap());
        let level = player.level(host).unwrap();
        for stack in merge(host, stacks) {
            let _ = level.pop_resource(host, origin, stack);
        }
        let [x, y, z] = origin.map(|v| f64::from(v) + 0.5);
        level.award_experience(host, x, y, z, experience);
        // The origin is already broken, by the loop.
        Flow::Cancel
    }

    fn block_drops(&self, _: &dyn LevelHost, event: &mut BlockDropsEvent<'_>) -> Flow {
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        if self.busy.load(Ordering::Relaxed) && self.vein.lock().unwrap().contains(&pos) {
            let mut kept = self.kept.lock().unwrap();
            event.drops.retain(|drop| {
                if let Some(stack) = drop.stack().and_then(|stack| stack.project()) {
                    kept.0.push(stack);
                    false
                } else {
                    true
                }
            });
            kept.1 += std::mem::take(&mut event.experience);
        }
        Flow::Continue
    }
}
```

`merge` folds each stack into the earlier ones it `is_stackable_with`, up to
`host.max_stack_size(stack.item())`, in the order they came. Filter `block_drops` on the
breaker too when other players can break the same blocks while the vein runs. What else a break
sets loose (a torch losing its support, a container's contents) is not the block's own drop and
still drops where it was.

Chained calls have no fixed nesting limit. A listener that breaks the block it is being told
about does not hear itself again: that nested break removes the block without a second
`break_block`, and the outer call then does nothing more. Keep a guard like `busy` when a
listener starts breaks of other blocks that may themselves fire it.
