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

## Custom enchantment effect kinds

Register a native effect codec in `Hooks::register_enchantment_effects`. Use
`event.effects.register_value` for a value transformation or
`event.effects.register_entity` for a server-side entity effect. The registries
are separate: each refuses duplicate IDs and reserves its vanilla kinds. Use
your mod's namespace so other mods can pick IDs independently.

The `type` in an enchantment selects one codec. Its `decode` method sees the
remaining JSON fields and returns an immutable handler. Read and validate your
fields there; a decode error rejects the definition with its enchantment and
component named. Only the selected data-pack definition is bound. A server's
frozen enchantment definitions do not change on `/reload`; syncing a definition
to a client for display does not require the client to decode its effects.

For example, register `example:scale` with an `EnchantmentValueEffectCodec`
that reads `factor` and returns an `EnchantmentValueEffect`. The `minecraft:damage`
component can compose it with vanilla effects:

```json
"minecraft:damage": [
  {
    "effect": {
      "type": "minecraft:all_of",
      "effects": [
        { "type": "minecraft:add", "value": 2 },
        { "type": "example:scale", "factor": 1.5 }
      ]
    }
  }
]
```

`process(level, random, value)` receives the current value and that stage's
live random stream. In a damage calculation, requirements draw from the level
and value effects draw from the victim. The next effect receives the returned
`f32` unchanged. In `all_of`, each child gets the preceding child's answer.

An `EnchantmentEntityEffectCodec` instead returns an `EnchantmentEntityEffect`.
Its `apply` method receives a `ServerHost` and an
`EnchantmentEntityEffectContext` containing the enchanted item, its holder and
slot when known, the selected entity and application position. A post-attack
context also names the victim, causing entity and direct entity when present.
An arrow's direct entity is the arrow, not the shooter. The handler is called
only if its component runs and its entry passes the requirements; this is not
a broadcast to other mods' handlers.

Entity effects in `minecraft:all_of` run in list order, interleaved with built-ins.
For `minecraft:post_attack`, the victim's equipped effects run before effects of
the attacking item. A refused hit does not enter this stage. A panicking codec
refuses the load; a panicking handler disables that registered effect kind for
the session without disabling other kinds.

When a new set of enchantment definitions is selected, it is decoded into new
handlers. An ordinary decode error does not disable the codec, and the server
does not publish a generation that failed to load.

## Registered potions

A client mod lists the registered potions with `ClientHost::potions()` (`Registries.POTION`), beside
`enchantments()`. Each `PotionInfo` carries the potion's registry id (`minecraft:long_swiftness`), in
registry order: vanilla's 46, from `minecraft:water` to `minecraft:infested`. Content mods cannot
declare potions yet, so none of theirs appear.

The read is `ClientSetup`: call it once a client mod is set up, not every frame. The game builds the
list once; each call returns a copy.

There is no potion stack API. To show a potion, splash potion, lingering potion or tipped arrow per
potion, build the stack and give it a `minecraft:potion_contents` component whose `potion` is the
id, through `ItemStack::set_components`, as a recipe viewer does:

```rust,ignore
for potion in host.potions() {
    let mut stack = ItemStack::new(splash_potion, 1)?;
    stack.set_components(Compound::from([(
        "minecraft:potion_contents".into(),
        Tag::compound([("potion", Tag::String(potion.id.as_str().into()))]),
    )]))?;
    entries.push(stack);
}
```

<!-- section-pages -->
