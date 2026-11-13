<!-- chapter: Item and tag Parts -->
<!-- separator: 1 -->

For an item-specific Part, request its canonical registry key once in `register`:
`let stick = r.item_key("minecraft:stick"); r.item_part(stick, Sharp);`. Bare names use the
`minecraft:` namespace. Requests may refer to content declared later; the final item registry refuses
unknown IDs with the requesting mod. `ItemKey` equality uses its load-owned slot, and `key.id(host)`
reads its canonical diagnostic name. Item capability registration and item helpers still use strings;
their migration is a separate change.

`r.block_part_tag("minecraft:logs", Rotates)` and `r.item_part_tag("minecraft:axes", Sharp)` attach a part to
every member of a tag (its name without `#`). Each game resolves them against its own tags when its world
opens, again on `/reload`, and when a client joins a server, so the window's game and its server thread each
answer from the tags they play with; a block that leaves the tag is no longer asked. A mod's parts on one
thing run in the order it registered them, by key or by tag. A part a tag gives a thing
that already has that kind of part warns, naming both mods, and the earlier mod's part stays. A tag that
doesn't exist warns. Opening a world fails, naming the mod, when a part is attached to a block or item the game
doesn't have. Scoped hooks run on a game's thread; one dispatched from any other thread (which no game hook
does) can't see tag parts, and warns once.

## Instant item use

`ItemPart::use_item` receives `UseItemEvent { stack, hand }`. `hand: Hand` is the
interaction hand of this click (`Hand::Main` or `Hand::Off`), including air clicks
with identical stacks in both hands. Read it directly when choosing which held stack
to shrink. `living_used_item_hand` instead answers only during active item use and
cannot identify an instantaneous click. Returning `Flow::Cancel` claims the use and
swings that interaction hand without running the item's own use. The field has the
same meaning on native and wasm and corresponds to NeoForge's
`PlayerInteractEvent.RightClickItem.getHand()`.

## Item wear

`ItemPart::damage_item` is `IItemExtension.damageItem`: the wear a stack of the part's item is about to take
(`ItemStack.hurtAndBreak`), after Unbreaking, as NeoForge passes it. The part reads `event.amount`, the points
about to be lost, and changes it (0 or less leaves the item whole) or returns `Flow::Cancel` for no wear at all. It
runs on every wear path: a player's tools, armour, shield and mace, a tool a dispenser uses, and a mob's armour. It is
not asked when Unbreaking already saved the item or for a creative player's tools, and an item with no part costs
one table lookup. With no part, the vanilla amount applies.

The global `item_damaged` hook is gone: this method is its one moment, and a part acts only on its own item. What the
global hook could do and a part cannot is change the wear of another mod's or vanilla's items, which no part is
attached to. Do that with an enchantment effect in data (a `minecraft:item_damage` effect on the enchantment, the
component Unbreaking uses), or with an injection. A part does not get the wearing entity;
`host.players().acting_player()` names the player on the paths a player takes.
