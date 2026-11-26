# Item abilities

Ability names are plain, case-sensitive strings with no namespace. They are not registry identifiers.

An item's Part can change the answer to an ability question. The hook receives an [`ItemAbilityContext`] and a borrowed [`ItemStackView`]. The answer depends only on the stack and the ability: the game asks from gameplay action sites and from `neoforge:can_item_perform_ability` loot predicates, and both must get the same answer, so the hook has no host or level (NeoForge's has none either). The context lends only the merged [`Tags`] (`context.tags()`). Behaviour that depends on the world belongs in the hook for that moment, such as block breaking or item use. Call `event.stack.project()` only when you need components or an owned copy.

A client without this mod predicts vanilla here; the server corrects it.

```rust,ignore
#[sunrise_part]
impl ItemPart for CarvingTool {
    fn can_perform_action(
        &self,
        context: &ItemAbilityContext<'_>,
        event: &mut CanPerformActionEvent<'_>,
    ) {
        // Only the merged tags: the same answer for gameplay and loot predicates.
        let _tags = context.tags();
        // Fast metadata reads avoid cloning components; call `event.stack.project()` only if needed.
        let _item = event.stack.item();
        if event.ability == "shears_carve" {
            event.can_perform = true;
        }
    }
}
```

Attach this Part to your item with the registrar's `item_part` method. This grants pumpkin carving only: `shears_harvest` is a separate ability for mob and dispenser shearing. `shield_block` permits blocking and uses the stack's blocking component, or the default component when absent. Leave unknown abilities unchanged.

The game asks these abilities at the following action sites; `host.item_abilities()` lists them:

| Ability | Consumers |
| --- | --- |
| `brush_brush` | Brushing a scute off an armadillo. |
| `firestarter_light` | Priming TNT; lighting a campfire, candle or candle cake, by hand or from a dispenser. |
| `fishing_rod_cast` | A cast line staying out while a hand holds the stack, the line drawn from that hand, and which hand another player's cast rod shows in. |
| `shears_carve` | Pumpkin carving. |
| `shears_dig` | A mined block wearing the stack as shears wear (every block but fire). |
| `shears_disarm` | Disarming a tripwire before breaking it. |
| `shears_harvest` | Shearing sheep, mooshrooms, snow golems, copper golems and bogged; cutting the leads of a mob, a boat or a knot; dispenser shearing; harvesting bee nest honeycombs. |
| `shears_remove_armor` | Shearing a saddle, a llama's carpet, a happy ghast's harness, a sulfur cube's block or a wolf's armor off. |
| `shears_trim` | Cropping the tip of kelp, weeping, twisting or cave vines so it stops growing. |
| `shield_block` | Raising a blocking item and blocking attacks. |
| `shovel_douse` | Putting out a lit campfire. |
| `spyglass_scope` | The zoom, overlay and slower look of the item in use, and an avatar holding it to the eye, on the client and the server. |
| `sword_sweep` | A charged, grounded hit sweeping its neighbours, on the client and the server. |
| `trident_throw` | A trident's throw: a denial stops it; a grant gives no other item a throw. |

The block's side of a fire starter lighting a campfire, candle or candle cake is `tool_modified_state` and
`block_tool_modification` with `ToolAbility::FirestarterLight`, as a shovel's douse is with `ShovelDouse`.

`event.can_perform` starts with vanilla's answer. Set it to `true` to grant an ability or `false` to deny it. The answer controls permission, not the action's world effects or item wear.

## Tool transformation previews

Use `LevelRef::block_transform` to preview a tool's effect on a loaded block in the server's active level. Pass the
live player, hand, stack, position and hit; the call returns an owned `BlockTransformPreview` holding the new block
state, durability cost, sound, optional loot-table ID and particle or level-event ID without applying it.

- Supported tool abilities reach the global `block_tool_modification` hook after the block Part unless the Part
  cancels, with `ToolUse::simulated` set to `true` for preview queries and `false` for actual use. Your listener must
  not change the level during a simulation.
- A stale or unavailable player or level, a player in another dimension, an inactive level, an unloaded position, an
  invalid face, or a tool with no transformation returns `None`.
