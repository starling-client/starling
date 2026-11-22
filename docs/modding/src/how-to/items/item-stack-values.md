# Owned item values

`ItemStack` is the mod API's own nonempty value, not an engine inventory slot or a borrowed
rendering view. It contains an `ItemKey`, a nonzero count, an optional numeric damage override
and a lazily allocated component patch. Plain stacks allocate no component storage. An empty
slot or a consumed stack is `None`; it is never a zero-count `ItemStack`.

Every loaded item, vanilla or modded, has an `ItemKey`. Resolve identifiers through
`Host::item_key` once when loading data and retain the key. Forward references requested during
registration keep their slots when the load completes; unresolved references refuse the load.
Keys are process-local identities, not persistent names. See [Item values and tag names](../../concepts/registry-keys.md#item-values-and-tag-names)
for retaining a `TagKey` and reading the current membership through `Host::tags`.

## Stored values and effective components

Count and component edits do not consult the game. `new` refuses zero; `set_count` and `grow`
refuse invalid or overflowing counts without changing the original value. Consuming `split`
and `shrink` return optional portions so consuming the last item needs no empty sentinel.

The damage representation has three states:

| Stored state | Effective meaning |
| --- | --- |
| No numeric override and no removal | Inherit the item's prototype, including its default damage of zero when present. A new plain sword needs no `Components` allocation. |
| `Some(0)` | An explicitly present zero-damage component, even on an item without a damage default. `set_damage(0)` creates this state without allocating a patch. |
| No numeric override and `!minecraft:damage` in the patch | The component is explicitly removed, including an inherited default. `clear_damage` creates this state. |

`damage()` returns the stored number or zero. `damage_component()` distinguishes an explicit
zero from no numeric override. `set_damage(u32)` is infallible; the game validates that
damage fits `0..=i32::MAX` when accepting a write. Adding only a maximum-damage override to
an item does not invent a damage component: explicitly adding `Some(0)` is what makes that
component present.

Use full component keys in raw stack patches and resident edits. The write keeps the supplied
key rather than adding a namespace: a bare `damage` entry remains opaque, not a damage override.
Numeric damage overrides live in the optional field. Raw `minecraft:damage` entries use vanilla's
numeric-to-int codec: for example, a Float containing `2.75` becomes damage `2`. A
nonnumeric tag or a negative decoded result refuses the entire replacement. Numeric
values replace old damage in the scalar field, not in the patch. `!minecraft:damage`
clears any prior or incoming damage value. Generic component setters and raw
`set_components` therefore return `Result`. A refusal leaves the stack unchanged. Typed damage
setters do not need an error path.

`apply_components(&patch)` merges over the existing patch rather than replacing it.
Incoming values clear prior removals and incoming `!id` removals clear prior values;
a removal wins if the same patch includes both, including for damage. Damage values replace
previous numeric overrides and are normalized just like `set_components`; malformed damage
leaves the entire stack unchanged.
`copy_component_from(&other, id)` copies only that component's own value or removal,
including explicit zero damage. If `other` has no override, the target inherits too:
no effective prototype value is materialized. `get` followed by `set` covers `update`.
These owned-value operations require no host. To edit a whole stack where it lives, use
server-side [`ItemPlace::edit`](../../concepts/capabilities/item-places-and-components.md#editing-the-whole-stack)
inside a transaction. Its callback borrows an `ItemComponentEditor`, not an owned `ItemStack`,
and cannot use the host borrowed by the edit. `get` sees pending overrides without a
projection. `original().project()` instead gives the pre-edit `ItemStack`, if it can be
projected. The editor has no `set_components`: clear the overrides you want to inherit with
`remove`, then merge the new overrides with `apply_components`. To replace the item or change
its count, use the appropriate transactional slot write or transfer instead.

`ItemResource::with_component` is also fallible for raw component values. Its default-value
elision and `without_component` preserve the normal prototype-plus-patch semantics. The latter
records real NBT removal entries for default components; it is not an unknown-value fallback.

## Game boundaries

A consume or death-protection override whose opaque native effect payload cannot be decoded
refuses the whole stack write with `StackWriteError::InvalidEffectPayload`, before any
resident state changes. This includes children already decoded for the write, such as a
bundle entry. A `use_remainder` or `sulfur_cube_content` template stays saved data until
it is materialized, so a codec refusal there is reported later and that effect does not run.
A `villager_food` patch without positive nutrition is refused as
`StackWriteError::InvalidComponents { reason: StackError::InvalidComponent { id } }`;
`id` names `minecraft:villager_food`. An explicit removal remains valid.
The items host area owns metadata reads such as `max_stack_size(ItemKey)` and
`max_damage(ItemKey)`. A stack does not take a host just to read a maximum, and an unavailable
maximum never turns into a fabricated limit of 64.

The game validates effective count and durability component rules, including overrides, when
accepting a resident write. An effective `max_stack_size` must be in `1..=99`, a carried
`max_damage` must be positive (`1..=i32::MAX`), `damage` must be a non-negative integer
(`0..=i32::MAX`, not capped by `max_damage`), and the presence of `max_damage` forbids a
stack limit above one, even when the current count is one.
A raw `max_damage` override uses the same numeric-to-int conversion and is stored and synced
as an integer. A `max_stack_size` override must be an NBT Int; removing it uses the absent limit
of one. Invalid values produce typed errors, not clamped values. A transfer request is
different: `ItemResource` plus a separate amount can request or return more than one
resident stack's capacity. The resident writes it produces must pass validation.

Game inventory, menus, trades and models keep their engine-owned representations. Where a
native hook or host method takes an owned API snapshot, the game converts only after a listener
guard and stages every mutable result before committing any captured output. Invalid hook
edits retain those captured engine values. The same durability rules govern writable hook
outputs: a valid damage value above that stack's maximum reaches later listeners and the game.
Consuming mutable events such as `bonemeal` and
`item_after_vertical_collision` lend `Option<ItemStack>` so a listener can consume the value by
setting `None`.

The global `Hooks::item_stacked_on_other` hook runs on the authoritative server side only (`StackedOnOtherDirection::Event`), while `ItemPart::item_stacked_on_other` runs on both sides (server clicks, client prediction with authoritative server reconciliation, and creative inventory-tab/hotbar pickups), dispatching first to the carried stack (`CarriedOnOther`) and then — unless a carried vanilla `BundleItem` takes the click — to the clicked stack (`OtherStackedOnMe`). It does not lend the slot: it hands the listener the cursor and
the clicked slot's stack, frozen, and takes a typed answer (`StackedOnOtherResult`: `handled`,
`StackResult::Keep` or `Set` for `carried`, and `ClickedStackResult::Keep`, `Set`, or `Take(count)` for `clicked`) that the game validates once and applies
once. For global listeners, a stack can be set only by a cancelled click that is also `handled` (`ClickedStackResult::Take` is refused), and a replacement cannot change a result slot or take an item from a brewing potion slot, because the replacement does not run the slot's take effects. For `ItemPart` overrides, `ClickedStackResult::Set` performs a direct write without `Slot.onTake` side effects while `ClickedStackResult::Take(count)` takes through `Slot.safeTake` / `Slot.onTake`. If both Parts take from the slot, each take runs its own effects in carried-then-clicked order. A refused replacement keeps
the slot and cursor as they were and undoes the listener's writes. A write through the `Host` to
what the click read likewise makes the click stale and refuses it.

The block-entity
tick, menu-button and menu-quick-move bridges validate their captured snapshot outputs together
before applying them. These are output guarantees, not whole-world transactions: independent
`ServerHost` storage writes, commands, messages and other effects are not rolled back on a
refused output. Carrier-form callbacks such as `bonemeal` and `item_after_vertical_collision`
also retain their captured stack outputs on refusal but do not journal independent host writes.

A client without this mod predicts vanilla here; the server corrects it.

Item-model callbacks receive the owned stack and a separate `bundle_selected` number (`-1`
means none). Runtime selection is not a persistent component. Drawing receives the API-owned
`ItemDisplayContext`, not the renderer's enum.

## Intercept main-hand client input

A [`ClientItemUseBehavior`](../../concepts/parts/item-and-tag-parts.md#intercept-main-hand-client-input)
receives the live typed `ItemStack` from its hotbar slot while the same item kind remains there
(rebound on tick and release like `LivingEntity.useItem`), or the last valid stack of that kind
if the slot was emptied or replaced. These callbacks intercept local input without starting
ordinary item use; see the linked example for registration and typed component reads.

## A stack's name

`host.hover_name(&stack)` is `ItemStack.getHoverName()` as a component, on both sides. The
custom name wins, then a written book's nonblank raw title. Otherwise the item's name is used:
a mod item's `ItemPart::item_name` answer replaces its default; `None` keeps it. Vanilla item
overrides (such as potion contents or a profiled player head) participate in the item-name step;
the base item uses the stack's `item_name` patch or inherits its prototype's value.
Removing `item_name` in the stack's patch gives an empty name. If a mod item's prototype
has no `item_name` (including when the mod removes it), the game uses its item or block
translation key instead. Only a fallback written into a component is kept. The result has
no rarity colour or italics. Each player renders the component in their own language: put it into world
text or a display as it is. Call `host.text_string(&name)` for a string in the language of the
side that calls it. An invalid stack is a `TextFormError`.

## The NBT escape

`ItemStack::from_nbt(host, compound)` and `to_nbt(host)` are the NBT escape. They live on the
plain API value, not as a second engine implementation in the items host area. They use only
typed host key lookup and canonical id metadata. Persistent NBT contains identifiers, never process-local
key slots. Unknown ids, malformed values and zero counts fail decoding; unavailable id
metadata fails encoding.

The escape preserves explicit zero damage, default-component removals and opaque component
NBT. It does not apply resident maxima or materialize prototype defaults. Those remain the
responsibility of the metadata service and game write boundary respectively.
