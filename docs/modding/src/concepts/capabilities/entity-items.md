# Entity capabilities

Use `entity.capability(host, &ENTITY_ITEMS, ())` to access an entity's items as a
`ResourceHandler<ItemResource>`. This is NeoForge's `Capabilities.Item.ENTITY`.
A stale handle or an entity outside the active level returns `None`. A retained
game handler cannot read or change items after its entity is removed or its world closes.

## Game providers

| Entity | Inventory exposed |
| --- | --- |
| Player | Main inventory, armour and off hand through `ItemPlace::PlayerSlot`. |
| Chest or hopper minecart, chest boat, chested donkey, mule or llama | Container slots through `ItemPlace::EntitySlot`. |
| Living non-player entity without a container, including an armour stand | Equipment in `EquipmentSlot` ordinal order, through `ItemPlace::Equipment`. |

Player slot indices are half-open ranges:

| Indices | Slots |
| --- | --- |
| `0..36` | Main inventory; `0..9` is the hotbar. |
| `36..40` | Armour, feet to head. |
| `40` | Off hand. |

An armour slot accepts one item whose `equippable` component permits that slot
and the player's type, by ID or tag. Bound armour can be removed only in creative.
`insert` merges matching stacks in the selected slot, off hand and main
inventory, then uses empty main-inventory slots; it does not fill armour slots.

For non-player equipment, hands accept a stack. Armour and body slots accept
one item whose `equippable` permits the slot and entity type.

## Custom providers and transactions

Register with `r.entity_capability(&ENTITY_ITEMS, "mymod:robot", |host, robot, ()| …)`.
Providers registered with `r.entity_capability_before_game` run before the game's
provider; ordinary providers run after it. Within each group, registration order
applies and the first handler returned wins.

Give, take and count player items through this capability. For example, this
trade aborts if an offer changes or either recipient lacks room. The caller gets
`TransactionError::Aborted(())` for those failures, or `TransactionError::Unavailable`
if the scope can't be opened:

```rust,ignore
// A trade: each offer is (index, the exact stack it must still hold). A stale offer or no
// room for one aborts the transaction, and every move in it is put back.
let (a, b) = (seller.capability(host, &ENTITY_ITEMS, ())?, buyer.capability(host, &ENTITY_ITEMS, ())?);
host.transaction(|host, tx| {
    for (from, to, offer) in [(&a, &b, &seller_offer), (&b, &a, &buyer_offer)] {
        for (index, stack) in offer {
            let offered = ItemResource::of(stack);
            let still = from.resource(host, *index).as_ref() == Some(&offered)
                && from.amount(host, *index) == stack.count();
            if !still
                || from.extract_at(host, tx, *index, &offered, stack.count()) != Ok(stack.count())
                || to.insert(host, tx, &offered, stack.count()) != Ok(stack.count())
            {
                return Err(());
            }
        }
    }
    Ok(())
})
```

Cauldron interactions outside a transaction retain vanilla's effect order.
Inside a transaction, statistics (including `player_stat_award`), sounds and
overflow item tosses or spawns wait for the outermost successful commit. An abort
emits none of those effects.

## Entity automation

Use `entity.capability(host, &ENTITY_AUTOMATION, side)` to access the inventory an
entity exposes to hoppers and droppers. This is NeoForge's
`Capabilities.Item.ENTITY_AUTOMATION`, distinct from the broader `ENTITY_ITEMS` capability.

Register a provider with
`r.entity_capability(&ENTITY_AUTOMATION, "mymod:robot", |host, robot, side| …)`.
Use `r.entity_capability_before_game` to answer ahead of the game's provider for
a minecart or boat. The context is `Option<Direction>`; `None` means no particular side.

- Chest and hopper minecarts and chest boats expose their container slots.
- Players and chested donkeys, mules and llamas have no game-provided automation handler.
- A pushing hopper requests the side it reaches the entity from (`Up` when the
  hopper is above it). A pulling hopper requests `Down` from the entity above it.
  A dropper requests the side it drops onto.
- At a target, automation chooses randomly among the game's container entities
  and entities whose mod providers answer this capability, using vanilla's
  `getEntityContainer` selection draw.
- A mod handler takes the container's place. Automation moves one item per
  transfer in a transaction, as it does through a block's `ITEMS` handler.

## Entity energy and fluids

Look up an entity's handlers with `entity.capability(host, &ENTITY_ENERGY, side)`
or `entity.capability(host, &ENTITY_FLUIDS, side)`. Register providers with
`r.entity_capability(&ENTITY_ENERGY, "mymod:robot", |host, robot, side| …)`
and the equivalent call for `ENTITY_FLUIDS`. Neither key has a game-provided handler.

| Key | Handler | NeoForge counterpart |
| --- | --- | --- |
| `ENTITY_ENERGY` | `EnergyHandler` | `Capabilities.Energy.ENTITY` |
| `ENTITY_FLUIDS` | `ResourceHandler<FluidResource>` | `Capabilities.Fluid.ENTITY` |

The context is `Option<Direction>`: `Some(direction)` identifies the side reached
by the lookup, and `None` requests no particular side. Your provider receives this
context and can return a different handler for each side. `ENTITY_AUTOMATION`
uses the same context; `ENTITY_ITEMS` uses `()`.

Use `move_energy` or `resource_util::move_resources` inside a [transaction](../transactions.md)
to transfer between entity, block and item handlers. Pass its borrowed `tx` to `move_energy`,
or `Some(tx)` to `move_resources`.
