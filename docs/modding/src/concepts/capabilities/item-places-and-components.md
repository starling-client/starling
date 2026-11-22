# Item places and components

Use an `ItemPlace` to read or change a stack where it lives. A place can identify a player's
slot, hand or cursor, a block entity's slot, an item entity, an entity's inventory or equipment,
a menu slot, or a stack inside another item (`ItemPlace::Nested`). For storage you manage,
register a `PlaceProvider` with `r.place_provider` and use `ItemPlace::Mod`. `ItemPlace`,
`ItemPlaceEditError`, and `CraftedByError` are process-local values without `Wire`; encode an
`ItemStack` or your own `Wire` schema when crossing a boundary.

`place.with_stack(host, |stack| stack.item())` reads the item at a nonempty place. The
callback receives an `ItemStackView` for that call only. Read `item()`, `count()` and
`damage()` there without copying a resident stack. If you only need to know whether
the place is occupied, use `place.amount(host) != 0`. For components or an owned
`ItemStack` beyond the callback, project it explicitly:

```rust,ignore
match place.with_stack(host, |stack| stack.project()) {
    None => { /* empty or unavailable place */ }
    Some(None) => { /* occupied, but the stack could not be projected */ }
    Some(Some(stack)) => { /* inspect components or retain the owned stack */ }
}
```

Native reads do not load chunks or initialize a player; reading a chest vehicle's
`ItemPlace::EntitySlot` unpacks its pending loot table without a player, matching vanilla
`ContainerEntity.getChestVehicleItem`. The fill adds loot only to free slots and preserves
any items already there. An item entity's place sees live state, including earlier writes
in the same transaction.
For a provider-backed or decoded place, the view lends the value produced for that read. A
provider-backed menu slot reads its live backing, not the menu's display copy. A dropped
item narrowed to an `ItemEntityRef` has its own
[`with_stack`](../../how-to/world/block-entity-item-drops.md).

## Item capabilities

Register an item's capability with
`r.item_capability(&ITEM_ITEMS, "mymod:backpack", |host, place| Some(Box::new(Backpack)))`.
Your provider returns a `ResourceHandler<ItemResource>` for that place. Use
`r.item_capability_before_game` to answer ahead of the game's provider, which handles
slots declared with `r.item_container(&item, provider)`, then items carrying
`minecraft:bundle_contents`. Bundle handlers use the shared vanilla weight rules and insert
at slot 0, moving the inserted stack to the front. Only slot 0 can be extracted; reads expose
all contents in front-first order. An empty bundle exposes one empty insertion slot.
A mod can return `Some(Box::new(BundleItems::new(place.clone())))` from its own
`ITEM_ITEMS` provider for an item carrying `minecraft:bundle_contents`. All writes exchange
one item through the place and roll back with the caller's transaction. `place.capability(host, &ITEM_ITEMS)` queries these providers in order.
The registered `ItemContainerProvider::slots(&dyn Host)` callback returns `Option<u32>` dynamically;
it is not called by registration-time coverage queries. `ITEM_ITEMS` covers the declared item type
even when that callback currently returns `None`. A closure `|host: &dyn Host| …` can be the provider.
Declarations may target vanilla or mod items. Enabled callbacks run in registration order;
`None` falls through to the next provider, and the first `Some` supplies the slot count.

Keep the contents in the stack and use these methods to access it:

| Method | Result |
|---|---|
| `place.with_stack(host, read)` | The callback's result, or `None` without calling it if the place is empty or unavailable |
| `place.resource(host)` | Its resource without a count, or `None` |
| `place.amount(host)` | Its count, or 0 |
| `place.insert(host, tx, &resource, amount)` | How much was inserted |
| `place.extract(host, tx, &resource, amount)` | How much was taken from a stack matching the resource's item and components |
| `place.exchange(host, tx, &resource, amount)` | How much was exchanged, or 0 if the exchange fails |

Where the game handles extraction directly, an override equal to the item's default does not
change what matches. The resource need not be valid for insertion: for example, a damage
component already on a stack can match even when that stack has no maximum damage.

Each write runs in its own transaction nested in `tx`. Aborting restores changes at every
level of nested items. If a handler answers more than requested, the move is undone and the
game warns once, naming the mod.

## Exchanging one item

Rewrite one item with `place.exchange(host, tx, &changed, 1)`. For example, exchanging one empty
bucket for a filled bucket splits the filled bucket from the stack. Overflow follows the
place's rules:

- A player's place sends it to their inventory, then drops anything that does not fit with no
  thrower. A creative player's hand accepts items and gives them one if they have none
  (`forInfiniteMaterials`).
- An item entity drops it beside itself.
- A block entity, entity or nested item slot tries the container's other slots (`forHandlerIndex`).
- A mod-owned place uses its `PlaceProvider`'s rules.
- A menu-slot insert fills only that slot; excess stays out.

An exchange inside a stack of two backpacks changes one backpack and splits it off. The other
backpack keeps its contents.

## Scratch stacks and wrapped places

These correspond to NeoForge's other `ItemAccess` factories:

- `host.scratch_stack(tx, stack)` (`forStack`) holds a stack outside the world, such as a
  crafting result or a preview, and returns its `ItemPlace::Scratch`. Take it back with
  `host.scratch_take(tx, &place)`. It lives until the outermost transaction closes. Aborting the
  transaction that made it drops it, and aborting a later one puts it back as it was. As in
  `StackItemAccess`, a write never changes its item, only its count and components: a bucket in
  one can't become a water bucket.
- `host.scratch_slots(tx, stack)` holds a stack outside the world in a private two-slot copy
  whose item may change, and returns its `ItemPlace::ScratchSlots`: what
  `fluid_util::dispense_container` runs a dispensed container in, so a bucket can become a water
  bucket. Reads and extracts are the first slot's. An insert fills the first slot where it is
  empty or holds the same item and components, and the rest goes to the second, as a changed
  item split off a stack does. `host.scratch_take_slots(tx, &place)` takes both slots back out.
  It lives and aborts as a scratch stack does.
- `ItemPlace::Strict(place)` (`forHandlerIndexStrict`) refuses what a block entity's, entity's
  or nested item's slot doesn't take, where the plain place would spread it to the other slots.
- `ItemPlace::OneByOne(place)` (`oneByOne`) lends and projects a count of one for a nonempty
  stack without changing its stored count. An extract takes at most one item.
- `ItemPlace::InfiniteMaterials { player, inner }` (`forInfiniteMaterials`) reaches `inner` as a
  creative player does while `player` has infinite materials. Extracts give without taking.
  Inserts are taken whole, and one item goes to the player only if they have none: into their
  inventory, or dropped at the root commit when it is full. Inserting nothing gives nothing.

## Editing the whole stack

To edit components on the **whole stack**, call `place.edit` with a server host and a
transaction. For example, to set the stack's stored damage to an explicit zero:

```rust,ignore
place.edit(host, tx, |editor| editor.set_damage(0))?;
```

The callback borrows an `ItemComponentEditor`, not the host or an owned stack. Read the
item and count but do not change them. Use `editor.get(&descriptor)` to read a typed override,
including changes already staged by this callback; it does not read the item's default.
Use `editor.original().project()` only when you need the original stack's patch or an owned
copy. It never includes pending edits and returns `None` if projection is refused.

`editor.set` stages a value; `editor.remove(&descriptor)` clears an override so the item
inherits its default. To remove the component even if the item has a default, use a
`!id` entry with `editor.apply_components(&patch)`, or `editor.clear_damage()` for damage.
`apply_components` merges without dropping unrelated overrides and refuses a malformed
damage patch before staging any of it. `editor.copy_component_from(&other, id)`
copies the source stack's own value or removal; if the source inherits, the target inherits too.

For a player's slot (including armour and off hand), hand, open menu cursor (`PlayerCursor`),
or a mob's or armour stand's `ItemPlace::Equipment`, the game resolves the slot before calling
the editor. An empty or unreadable slot (`ItemPlaceEditError::NotHeld`), an unavailable backend,
or a closed transaction (`Unavailable`) refuses before the editor runs. After the callback, the
game validates the changed overrides together, including the resulting stack limits. If it returns
`Stack(error)`, none of the edits were applied. Unchanged overrides are not rewritten. The usual
inventory, menu, cursor and equipment sync sends the change, including synchronized component
overrides, without replaying equip sounds,
events or attribute effects. Aborting this transaction or an enclosing one restores the previous
stack.

`Strict` keeps the same edit behavior. `OneByOne` and `InfiniteMaterials` refuse with
`UnsupportedPlace` before the callback; other places use their ordinary exchange rules.
To change an item or count, use the appropriate transactional slot write or transfer instead.

## Component patches

To change **one item** of a stack instead (NeoForge's `ItemResource.with` and `without`), use a resource and exchange it as below. Name a component with `ComponentId::new("minecraft:damage")`. This returns `None` for an invalid
namespaced ID, such as `Damage` or `!minecraft:food`.

| Method | Behaviour |
|---|---|
| `resource.component(&id)` | Reads the stack's own value from its saved patch, not the item's default |
| `resource.with_component(host, &id, value)` | Returns `Result<ItemResource, StackError>` with the value set |
| `resource.without_component(host, &id)` | Returns an `ItemResource` with the component removed |

`with_component` removes a patch value when it equals the item's default from
`host.item_default_component(item, id)`. Equality follows the component codec: vanilla numeric
components compare by value, while raw NBT components such as `custom_data` and mod components
compare by NBT type and value. Invalid component values return `StackError`.

`without_component` removes the stack's own value. If the item has a default value, it also
records a removal such as `"!minecraft:max_damage": {}` in the patch.

These methods return detached resources. Store the result with an exchange of one item:

```rust,ignore
let resource = place.resource(host).ok_or("empty place")?;
let changed = resource.with_component(host, &damage, Tag::Int(5))?;
place.exchange(host, tx, &changed, 1);
```

The exchange damages one sword from a stack, splitting it off when necessary.
