# Items, recipes and loot

Pages for mods that work with items and the data around them: recipes and ingredients, enchantment and consume
effects, loot, data maps and conditional data.

## When a player's item is used up

Implement `Hooks::player_destroy_item` to observe the last item consumed by use or by an entity,
a completed food/drink use, or a durability break.
Its `PlayerDestroyItemEvent` contains `player` (UUID), `player_entity_id`, `original` (an immutable full
stack snapshot taken before consumption or wear), and `hand` (`Some(Hand::Main)`, `Some(Hand::Off)`,
or `None` for worn equipment and inventory ammunition). Count, pre-wear damage and component patches
are preserved; a returned bottle or bucket does not replace `original`.

The callback receives `HostSide`: the server sees its applied action, including its local host player,
and a client sees its own prediction or completed use. An equipment-break status packet still plays
its sound and particles, but does not send this notice: it has no original stack. Inventory updates
do not send notices either.

This is a notification after the change, so it cannot cancel the action. A mining break notifies
after the block's drops. A completed use notifies after consumption and before a remainder such as
an empty bottle enters the hand. Partial consumption, refused interactions, and creative use that
preserves the item do not notify. Equipping an item into an empty armour slot moves it rather than
consuming it.

See the [API reference](../reference/api.md) for the generated event and `Hooks` method.

<!-- section-pages -->
