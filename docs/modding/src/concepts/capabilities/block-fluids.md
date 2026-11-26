# Block fluids

Look up `level.capability(host, &FLUIDS, pos, side)` to access a block's
`ResourceHandler<FluidResource>`. Transfer fluids inside a transaction with
`resource_util::move_resources`, as you transfer items. `FLUIDS` corresponds to NeoForge's
`Capabilities.Fluid.BLOCK`. Amounts are millibuckets (mB); one bucket is 1000 mB.

## Resources and tanks

`FluidResource` identifies a fluid and its data components without an amount.
Use `resource.to_stack(amount)` to create a `FluidStack`, and
`FluidResource::of(&stack)` to obtain its resource identity. A tank accepts fluid
when empty or when its contents match both the fluid and components.

| Game provider | Rules |
| --- | --- |
| Mod block tanks (`r.block_fluids(&block, tanks)` with `ModFluidTanks`) | Access depends on side; face/input/output indices must be within `0..tanks.len()`. Capacities are kept as declared (zero-capacity tanks accept no fluid). Saved tanks reject fluids with components. |
| Water cauldron | One bucket in three levels; insertion and extraction use whole levels. |
| Lava cauldron | One bucket, inserted or extracted whole. |
| Registered cauldron content | Uses the whole levels specified through `register_cauldron_fluid_content`. |
| Powder snow cauldron | No fluid handler. |

Cauldrons are position-based providers, not block entities. They accept registered
fluids regardless of components and discard those components; extracted fluid is
plain. An empty cauldron accepts any fluid with registered cauldron content.

The source must extract exactly what the destination accepted. If it cannot, the
step rolls back. For example, a full water cauldron cannot supply exactly 500 mB:
request 334 mB to move one level into a tank with 500 mB of room.

## Cauldron contents

`register_cauldron_fluid_content` publishes a listener's entries only after [successful registration](../load-phases.md#registration-and-setup).

To inspect a cauldron without probing its capability, use the `fluids` host reads
`host.cauldron_content_for_block(block_key)` and `host.cauldron_content_for_fluid(fluid_key)`.
Both return `Option<CauldronContent>`: typed `block` and `fluid` keys, full `total` in
millibuckets, `max_level`, and optional `level_property`. Water answers 1000, 3, and
`Some("level")`; lava answers 1000, 1, and `None`. `host.cauldron_level(state)` reads a
typed state's current level (water at level 2 answers 2); an always-full content
answers its maximum. Unregistered contents, including powder snow, answer `None`.
These `WorldOnly` reads work on both sides, over the existing registration table,
including mods and rebuilt resource generations. A client that has not built that table
answers `None`. The lookup does no work unless called. A content lookup scans the existing content table without a mod
scan, clones its shared snapshot, and allocates only the returned level-property string
when present; the scalar level read allocates no result.

## Item containers

Use `place.capability(host, &ITEM_FLUIDS)` for the fluid handler at an `ItemPlace`.
Register a provider with `r.item_capability(&ITEM_FLUIDS, "mymod:canister", …)`.
Changes exchange one item, splitting the changed item from a larger stack.

Vanilla buckets expose one 1000 mB tank. Empty buckets accept plain water or lava
and become filled buckets; filled buckets drain whole and become empty buckets.
`move_resources` transfers between item and block handlers: a full cauldron can
fill a bucket in a player slot, but a cauldron holding two levels cannot.

## Player interaction

Call `fluid_util::interact_with_fluid_handler(host, player, hand, level, pos, side)`
from your block's `use_item_on` part, and return success when it returns `true`.
This corresponds to NeoForge's `FluidUtil.interactWithFluidHandler`.

The helper looks up the block's `FLUIDS` and the held item's `ITEM_FLUIDS` through
`ItemPlace::PlayerHand`. It first tries filling the item, then emptying it if no
fluid moves. It transfers the first fluid that can move, as much as fits in one
operation. From three buckets, it fills and splits off one; a creative player's
hand stack is unchanged.

After a successful transfer, the helper emits the sound and game event at the
block's centre:

| Operation | Game event | Fluid sound |
| --- | --- | --- |
| Fill the item | `minecraft:fluid_pickup` | `FluidKey::pickup_sound` |
| Empty the item | `minecraft:fluid_place` | `FluidKey::empty_sound` |

A tank-only fluid emits the event without a sound.
`fluid_util::interact_with_handler` performs the interaction against a handler you
supply. `fluid_util::trigger_sound_and_game_event` emits only the sound and event.
These effects are emitted before the call returns: call these helpers outside
an enclosing transaction that you might abort.

## World fluid

A fluid container can pick up and place fluid in the world as a bucket does
(NeoForge's `FluidUtil.tryPickupFluid` and `tryPlaceFluid`):

| Helper | What it does |
| --- | --- |
| `fluid_util::try_pickup_world_fluid(host, destination, player, level, pos)` | Takes the source at `pos` (a fluid source, a waterlogged block's water, a bubble column) into `destination`, the whole bucket or nothing |
| `fluid_util::try_pickup_fluid(host, destination, player, level, pos, side)` | The same, and where `pos` has no source, one move from the block's `FLUIDS` at `side` |
| `fluid_util::try_place_fluid(host, source, player, level, pos, validate)` | Takes a bucket of the first fluid of `source` that comes out whole and places it at `pos`; `validate` refuses a waterloggable block that won't take it |
| `fluid_util::first_stack_contained(host, place)` | Reads what the container at an `ItemPlace` holds at its first non-empty index |

The three that move fluid each run in their own transaction, nested in one you have
open. The world-fluid writes they use take the innermost `&TransactionContext` and
return `Err(TransactionError::InvalidContext)` for a different context without making a
change. `level.pickup_fluid(host, pos, player, tx)` (returning
`Result<Option<FluidStack>, TransactionError<()>>`) and
`level.place_fluid(host, pos, fluid, player, validate, tx)` (returning
`Result<bool, TransactionError<()>>`) decide everything at the
call against the world as the transaction has it and write the block at once, so a
second pickup of the same source in the transaction gets `Ok(None)`. A block the fluid
replaces has its drops made at the call, with the block as it is and its block entity kept
by the journal first: `block_drops` listeners, which only read, see the old block, and a
cancelled drop keeps the replacement (it suppresses the drops and the experience, not the
removal). The new block is written after the drops are made.
The block's updates, the fluid tick, those drops, the broken block's particles and
`block_destroy`, then the bucket's sound and `fluid_pickup` or `fluid_place` follow at the
outermost commit. Where water evaporates (the Nether), placing it only hisses. The tank
fallback's fill sound and event also wait for that commit
(`level.play_fluid_effects(host, pos, fluid, pickup, player, tx)`). An aborted pickup or
placement leaves the world unchanged, with no drop, sound or event, where NeoForge's may
leave it changed.

## Dispensing a fluid container

`fluid_util::dispense_container(host, event)` is NeoForge's `DispenseFluidContainer`: a
dispenser's answer for a fluid container. Return it from the container's `dispense_item`
part. It is no default, and vanilla's buckets keep their own behaviours.

```rust,ignore
fn dispense_item(&self, host: &mut dyn ServerHost, event: &mut DispenseItemEvent<'_>)
    -> Option<Dispensed> {
    fluid_util::dispense_container(host, event)
}
```

The dispensed container runs in a lent two-slot copy (`HostTransaction::scratch_slots`,
yielding an `ItemPlace::Scratch` for the callback scope), so an empty container may become a
full one of another item, as a bucket does. Its handler reaches
the copy one item at a time (`ItemPlace::OneByOne`), so it sees one container whatever the
stack. It always tries
`try_pickup_fluid` on the block in front first (its `FLUIDS` reached from the dispenser's side);
if pickup fails, its nested transaction aborts and restores the lent slots before
`try_place_fluid` (unvalidated, as a bucket) runs as the fallback. The updated slots are
published only when a transfer commits.
A single container is replaced in its slot (the final first slot, or the second slot when the
first was emptied). When both slots remain populated, the final first slot stays in the
dispenser's active slot and the second slot's remainder goes into the dispenser's other slots,
else out of its front with a second click (`DefaultDispenseItemBehavior.addToInventoryOrDispense`).
The helper's fluid and slot changes are provisional while a transaction is open in the
**dispenser's level**. Block updates, fluid ticks, drops, sounds, game events, and slot
notices publish when that level's outermost transaction commits. If only another level
has an open transaction, the dispenser's changes commit independently.
This includes the pickup event from a `FLUIDS` transfer and both vanilla waterlogging ticks
and ticks scheduled through `LevelWriteHost` by a `place_liquid` Part. An abort publishes none
of them. If a fluid game-event listener opens another transaction during commit, it does not
delay attachment retirement for block entities removed after the outermost close.

`LevelRef::dispense_with_remainder` (resolve the dispenser's level with
`host.level(event.dimension)`) likewise returns a detached active-slot
remainder immediately, but any other slot it fills is provisional, and overflow drops, click
and smoke wait for the outermost commit in the dispenser's level. Inserting the remainder into an empty slot notifies; growing
a compatible stack does not. The detached active stack keeps its count until the dispenser
writes its slot, using the effective stack-size component (one if absent). An inert Part or a
refused reply leaves the slot untouched and sends no slot change notice.
Where neither pickup nor placement moves anything, one unchanged container is dispensed as
`DefaultDispenseItemBehavior` does (`LevelRef::dispense_default`). Where the helper returns `None` (for example, when a
transaction is unavailable or stack validation fails), return `None` from the Part rather than
calling `dispense_default` yourself.
