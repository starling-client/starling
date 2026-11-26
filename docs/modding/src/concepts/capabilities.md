# Capabilities

A capability lets a caller ask an object what it can do without knowing its implementation.
A pipe can ask a chest and a modded machine for `ITEMS` and use either through the same
`ResourceHandler<ItemResource>`. The **provider** decides whether to offer a handler; the
**handler** reads or moves the resource. Selection is read-only, not the transfer itself.

## Register and look up providers

For a block entity, request its type key during registration and offer a handler for its address:

```rust,ignore
r.block_entity_capability(&ITEMS, &pipe, |_, at, _, _, offer| {
    offer.offer(|| Box::new(PipeItems(at)));
});
```

The provider can inspect the supplied level, state, address and block entity. Leave `offer`
untouched to decline. The factory passed to `offer.offer` runs only when the caller needs a
handler. A `Capability` wraps that handler and names its provider through `owner()`.

| Goal | API |
| --- | --- |
| Offer storage by block entity type, including vanilla types | Register with `block_entity_capability`. |
| Offer storage by block type, with or without a block entity | Register with `block_capability`. |
| Get a handler at a position and face | Use `level.capability(host, &ITEMS, pos, side)`; a `None` side is unsided, not all faces. |
| Check for an offer without constructing a handler | Use the read-only `LevelHost::has_item_capability`. |
| Check whether a type is already covered | Use the registrar's coverage queries for a block, entity or item type. |

For block lookups the first offer wins: `_before_game` providers, then the game's provider, then
other providers. Within each mod group, block and block entity type providers run together in
registration order. Disabled mods are skipped. A stale or inactive level cannot lend a handler.

Coverage queries see built-ins and registrations so far, including earlier declarations by the
same mod. They do not run future callbacks or promise that a provider will answer at a specific
position or face. A read-only item probe distinguishes an offer, no offer and an unavailable
position. It does not load chunks or open loot; a client probes its local view, not the server's.
Look up a neighbour again on the next tick, or use a
[kept capability](capabilities/keeping-a-block-capability.md) to handle invalidation explicitly.

Default block storage declarations provide engine-managed item slots, fluid tanks or energy
buffers for mod blocks. Use a provider to extend a vanilla block instead. Item containers can
also target vanilla items. See [Block entity containers](../how-to/content-mods.md#block-entity-containers)
and [Item places and components](capabilities/item-places-and-components.md).

## Proxy metadata

A block capability is non-proxyable by default. An explicit proxyability declaration tells a
forwarding mod whether it can safely pass requests on to another block. Repeating the same value
is allowed; conflicting explicit values refuse registration. Reading the default is not a declaration.
The registrar sees metadata discovered so far; runtime readers see published snapshots.
Proxyability neither changes lookup order nor promises a provider will answer. See
[A mod's own capability](#a-mods-own-capability) for keys with no built-in provider.

## Read and move items

`ItemResource` identifies an item and its components **without a count**. The handler uses the
host it is passed rather than keeping a borrow of the game. Keep state in journal-managed
storage so a failed transfer can restore it.

| Handler operation | What to expect |
| --- | --- |
| `size`, `resource`, `amount` | Read zero-based slots, their optional resource and its amount. |
| `capacity`, `is_valid` | Capacity is a limit, not free space or admission. Enforce acceptance rules on insert. |
| `insert_at`, `extract_at` | Move up to the requested amount at one index using the current transaction. |
| `insert`, `extract` | Let the handler choose the index; the default walks indices in order. |

For the game's `ITEMS` handler, `amount` reads the resident stack's count without copying the stack or
encoding its components; `resource` still returns an owned, count-free `ItemResource` with the item's
components. A restricted level hook reads the same resident slot through
[`LevelHost::with_container_stack`](hooks-and-the-host.md#level-hosts) without asking registered
capability providers.

Use `resource_util::move_resources` rather than moving the two sides independently. It probes
extraction and rolls it back before insertion, then commits a step only if the source can give
exactly what the destination accepted. An overanswer rolls back the whole helper call, even if
the caller later commits the enclosing transaction. Pass capability references to keep provider
names in an overanswer diagnostic. Stacking variants try occupied destination slots first.
See [Capability answer limits](#capability-answer-limits) and the `resource_util` module in
the [generated API reference](../reference/api.md) for scans, wrappers and composition.

To try a transfer without keeping it, use a [transaction](transactions.md) and abort. Pass
`Some(tx)` for the innermost frame, or `None` to open a root when no frame is open. A wrong
context or an unavailable transaction is an error, not a successful zero transfer.

### Capability answer limits

If a handler answers more than requested, `resource_util::move_resources` rolls back the
whole move and returns `Err(TransactionError::Aborted(Overanswered))`. If a needed scope
cannot open, the move returns `Err(TransactionError::Unavailable)`. Use `?` to abort the
enclosing transaction too when its other writes depend on the move succeeding; an error
returned from its closure is wrapped in the outer transaction's `Aborted` variant.

- `Overanswered.owner` is `Some` with the mod ID or `"the game"` when a capability can
  attribute a slot-less error. A direct handler or a raw indexed answer may leave it `None`.
  Capabilities still warn once, naming their provider, when an indexed answer exceeds the request.
- Handlers must not move more than the amount passed to `insert_at`, `extract_at`, `insert`,
  or `extract`. Journal their writes with the supplied `&TransactionContext` so an abort can undo
  them. If you open a nested transaction, forward the new context inside it, not its parent's.
- Partial acceptance is not an overanswer. The helper attempts to extract exactly the accepted
  amount. If the source cannot supply it exactly, the step rolls back.
- The separate `move_energy` helper keeps its source-first order: partial acceptance rolls
  back its attempt and retries at the smaller amount.

### Hopper and dropper attempts

Vanilla containers, including composter containers and container entities, keep their direct
automation paths. Other blocks can be reached through storage providers. Automation can probe
and roll back an attempt, so never keep transfer state outside the journal. Engine-managed slots
defer their implicit change notice to root commit; explicit notifications are immediate and
survive abort. A provider that ignores the journal can lose an item on a refused transfer.
A composter shows why the journal covers more than inventory slots: its capability ports
journal the block state and a composter-local random stream, and publish fill feedback only
on commit. An aborted probe does not shift the result of the next attempt. For caller-specific
hopper pull/push and dropper rules (including ordinary refusal vs. invalid overanswer, source-slot
reservation before callbacks, and hopper-minecart item-entity fallback), see `ITEMS` in the
[generated API reference](../reference/api.md).

## Keep addresses and values local

Handlers, `BlockSide` and local resource records are not network formats. To send a block
address, send its position, face and canonical dimension name, then resolve the named level on
receipt. Do not send a process-local `DimensionKey`. Use owned `ItemStack`, `FluidStack` or
mod-defined [Wire types](wire-types.md) for payload values, and the registered codec for
[attachments](attachments.md).

For other resource families, see [Block fluids](capabilities/block-fluids.md),
[Block energy](capabilities/block-energy.md) and [Entity capabilities](capabilities/entity-items.md).

## A mod's own capability

You define your own capability, as Mekanism defines chemicals: a resource type and a key, published from
its root crate ([Dependencies between mods](dependencies.md)) for other mods to use.

```rust,ignore
// The chemistry mod's root crate
#[derive(Clone, PartialEq)]
pub struct Chemical(pub String);
pub const CHEMICALS: CapabilityKey<dyn ResourceHandler<Chemical>> = CapabilityKey::new("chem_api:chemical");
```

Declare the defining mod under `[dependencies]` to build against the same `Chemical` and `CHEMICALS`. Your providers,
`r.block_entity_capability(&chem_api::CHEMICALS, &vat, …)` (where `vat` is a declared type key or
`r.block_entity_type_key("chem_pipe:vat")`), answer the same capability, and
`level.capability(host, &chem_api::CHEMICALS, pos, side)` finds either mod's handler. `resource_util::move_resources` moves a
chemical as it moves items. Pass `Some(tx)` from the caller's innermost transaction, or `None` when
no transaction is open. Provider write methods take the borrowed `&TransactionContext`. Only mod
providers answer a mod's key; a lookup with none registered costs one table check. A handler that
keeps its amount in its block entity's saved data (`BlockEntityRef::merge_data`) is covered by the
game's journal, so an aborted transaction puts it back. `ItemCapabilityKey::new` and `EntityCapabilityKey::new` define item and entity
capabilities the same way.

A key is known by its name, which must be namespaced (`chem_api:chemical`), and a name has one handler type. Two
mods that each declare their own `Chemical` under one name, rather than sharing the defining mod's, would never see
each other's handlers. So the game refuses that at startup, naming both mods and both types, and it refuses a key whose name has no
namespace. Both checks run once, when the mods register.

Storage for a holder without a block entity (an entity, a player, a chunk, the level) comes stock, kept in an
attachment the mod registers, as NeoForge mods keep a `ItemStacksResourceHandler` or `SimpleEnergyHandler` in one:
`AttachedItems::new(&slots_key, holder, size, capacity, valid)` (returns `None` for more than 4096
indices) and `AttachedFluids` (`AttachedStacks<R>`, a
`ResourceHandler<R>` over an `AttachmentKey<StackList<R>>`, `single` for one index) and
`AttachedEnergy::new(&energy_key, holder, capacity, max_insert, max_extract)` over an `AttachmentKey<u32>`. Give the
item or fluid attachment descriptor `.serialize(ITEM_STACKS)` or `.serialize(FLUID_STACKS)` to save it. Every write is an attachment write, so
the transaction journal, which keeps each holder's attachment map, undoes it on an abort; sync follows the attachment
descriptor's `.sync(...)` codec, and keys from an earlier installation refuse reads and writes. A mod's own resource type implements `StackedResource` to be held. `capacity` stays the physical
limit of an in-range index while `valid` refuses only inserts (extraction never asks it), indices past the handler's
`size` that the attachment keeps are never deleted by a smaller view, and the codecs refuse a saved `Slot` past 4096
indices or an amount past what a signed `Int` saves, rather than honoring or wrapping it.
