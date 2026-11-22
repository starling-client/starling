# Capabilities

Use capability providers to expose a block's storage to pipes, machines and hoppers. Register a
provider for a block entity type, including a vanilla type, or for a block with or without a block entity.
For example, during registration, obtain `pipe` from your declaration builder or
`r.block_entity_type_key("mymod:pipe")`, then supply your handler:

```rust,ignore
r.block_entity_capability(&ITEMS, &pipe, |_, at, _, _, offer| {
    offer.offer(|| Box::new(PipeItems(at)));
});
```

Here, `PipeItems` is your implementation of `ResourceHandler<ItemResource>`. Its `BlockSide` context
contains the level, position and requested side. A provider checks the supplied state, entity and
read-only level, and leaves `offer` untouched to decline. The closure passed to `offer.offer` runs
only when the caller needs the handler. Select before offering; do transfers and writes later,
through the handler's `ServerHost` and transaction. This registration corresponds to NeoForge's
`RegisterCapabilitiesEvent`.

## Register and look up providers

| Operation | Behaviour |
|---|---|
| `r.block_entity_capability(&ITEMS, &kind, provider)` | The provider receives `&dyn LevelHost`, `BlockSide`, `BlockState`, the matching `BlockEntityRef`, and the offer. |
| `r.block_capability(&ITEMS, &block, provider)` | The same inputs, but the `BlockEntityRef` is optional. Obtain `block` with `r.block_key("mymod:void_bin")`. |
| `r.block_entity_capability_before_game(...)` / `r.block_capability_before_game(...)` | Register a provider ahead of the game's provider. |
| `level.capability(host, &ITEMS, pos, side)` | Look up a handler on a `LevelRef`; `side` is `Option<Direction>`, with `None` meaning no particular side. The result is `Option<Capability<dyn ResourceHandler<ItemResource>>>`. |
| `view.has_item_capability(pos, side)` | Test whether a handler would be offered on a read-only `LevelHost`, without making one. |
| `r.is_block_capability_registered(&KEY, &block)` | Whether any game or mod provider registered so far covers this block type. |
| `r.is_entity_capability_registered(&KEY, &entity_type)` | The same type-coverage query for an `EntityTypeKey`, obtained with `r.entity_type_key("minecraft:pig")`. |
| `r.is_item_capability_registered(&KEY, &item)` | The same query for an `ItemKey`, obtained with `r.item_key("minecraft:stick")`. |

A block provider can expose a trash can that voids items or forward a multiblock port to its controller.
A block with a block entity can have both kinds of provider.

Lookup uses this order:

1. Providers registered with `_before_game`.
2. The game's provider: for `ITEMS`, container slots exposed by side using the rules hoppers follow,
   including slots declared with `r.block_container(&block, container)` (`ModContainer`)
   and a composter's state-backed ports.
3. Other registered providers.

Within each mod-provider group, block and block entity type providers form one list in registration
order. The first offer wins; handlers are not merged. Use `_before_game` when you need priority over
the game's handler, but do not assume precedence over another mod at the same priority.
Disabled mods' providers are skipped. A stale level handle or a level that is not in place returns
`None` without asking providers.

Registration queries see the game's coverage, earlier mods in registration order, and providers already
added by the current mod; they do not see later registrations. For example, `ITEMS` covers
`minecraft:chest` without a mod provider. Use these queries to add a fallback only when no provider
covers the type. This is type coverage, not a promise that a lookup at a particular position, side,
entity or item place will return a handler.
Declarations made in a later phase, such as `register_cauldron_fluid_content` after registry resolution,
are not visible yet; these queries never run future registration callbacks.
The same registrar owns default storage declarations: `block_container(&block, ModContainer)`,
`block_fluids(&block, ModFluidTanks)`, `block_energy(&block, ModEnergyBuffer)` and
`item_container(&item, provider)`. Their declarations supply both the game's runtime handlers and
registration-time coverage; the item provider's dynamic slot callback is never invoked by a coverage query.
`ModContainer::insert_rules` gives slots a default admission rule: an empty-only slot uses
`ModContainerInsertRule::OnlyWhenEmpty`, while a same-kind slot uses `ModContainerInsertRule::SameKind`
and accepts an empty slot or a stack with the same item and components. Missing entries are
unrestricted. In world insertion, a `BlockEntityPart::can_place_item` answer replaces this rule
when it returns `Some`. The default `ITEMS` handler's `capacity` remains a limit, not a promise of
admission; `insert_at` applies the rule. See [Block entity containers](../how-to/content-mods.md#block-entity-containers)
for the distinction from menu slots.
The three block-storage declarations accept mod blocks only, not `minecraft` blocks; use
`block_capability` to expose storage on a vanilla block. The first enabled declaration for a block
type supplies its default storage. Item-container declarations may target vanilla items too, and
their callbacks run in registration order until one returns `Some`.

`LevelHost::has_item_capability` asks the same enabled providers in the same order on either side.
`None` as the side means unsided, not a search over the faces. `Some(true)` means a provider
answered even if its handler would be empty or have no slots. `Some(false)` means no provider
answered; `None` means the view could not resolve a resident block at that position. The lookup
never loads a chunk or opens deferred loot. A client's answer uses only the state, entity and mods
available to that client; the server still decides the actual placement or transfer.

Declare your own capability with `CapabilityKey::new("mymod:chemicals")`. Only registered providers
answer it; it has no game-provided handler. A `Capability` wraps the handler and identifies its
provider through `owner()`.

## Proxy metadata

Block capabilities are non-proxyable by default. The defining mod can call
`r.set_capability_proxyable(&KEY, true)` to declare that requests can safely be forwarded to other
blocks, or explicitly mark `false` when forwarding is unsafe. Repeating the same explicit value,
including from another mod, is allowed; an opposite explicit value refuses registration rather than
letting the last writer win. Merely reading the default `false` does not explicitly mark it.

During registration, `r.block_capability_names()` and `r.block_capability_proxyable(name)` expose
the capabilities discovered so far. After publication, use `block_capability_names(host)`,
`block_capability_proxyable(host, name)` and `block_capability_proxyable_names(host)`; the last
returns precisely the known names whose proxyability reads `true`. These owned snapshots use the
published metadata. Proxyability is metadata for forwarding mods, not a
change to provider lookup order or a guarantee that a provider will answer.

These rules follow NeoForge 26.3's
[`BlockCapability`](https://aldak0.ru/javadoc/26.3.x/net/neoforged/neoforge/capabilities/BlockCapability.html)
and [`RegisterCapabilitiesEvent`](https://aldak0.ru/javadoc/26.3.x/net/neoforged/neoforge/capabilities/RegisterCapabilitiesEvent.html).

## Read and move items

`ITEMS` handlers implement `ResourceHandler<ItemResource>`, corresponding to NeoForge 26.3's
`ResourceHandler<ItemResource>`. An `ItemResource` identifies an item and its components without a count.
Each method receives the host; the handler holds no borrow of the game.

| Method | Meaning |
|---|---|
| `size` | Number of indices, numbered from 0. |
| `resource` | Resource at an index, or `None`. |
| `amount` / `amount_int` | Amount held at an index; `amount_int` caps it at `i32::MAX`. |
| `capacity` / `capacity_int` | Limit for `Some(&resource)`, or the general limit for `None`. An unknown general limit is `u32::MAX`; `capacity_int` caps the answer at `i32::MAX`. This is not free space or permission to insert. |
| `is_valid` | Whether the resource is allowed at an index. The default checks for nonzero capacity; override it for separate acceptance rules and enforce those rules in `insert_at`. |
| `insert_at` / `extract_at` | Move up to the requested amount at one index, using a `&Transaction`. Return the actual amount moved. Insertion can refuse even with nonzero capacity. |
| `insert` / `extract` | Let the handler choose where to move the resource. The default visits indices in order. Prefer these unless you need to target a specific index. They return `Result<u32, Overanswered>`. |

Keep handler state in transaction-managed storage such as block entity slots, saved compounds or
attachments. Your handler can look up and call a neighbour's capability through the host.

Use `resource_util::move_resources(host, &from, &to, filter, max, Some(tx))` to transfer matching
resources, up to `max`, in source index order. The helper probes an extraction and rolls the probe
back before asking the destination to insert. A step commits only if the source can then extract
exactly what went in. An overanswer rolls back the entire helper call, even if you handle the error
and commit the enclosing transaction. Use `Some(tx)` for the current transaction or `None` to open a
root transaction; a token for a different transaction, or `None` while one is open, moves nothing.
Pass capability references to keep their provider names in `Overanswered` errors. For a handler
you compose directly, pass it as `&dyn ResourceHandler<R>`; the helper does not infer a provider.

`move_resources_stacking` tops up matching destination indices before filling empty ones.
`move_first` and `move_first_stacking` move only the first resource the destination accepts,
including more of the same resource from later source indices, and return which and how much.

`index_of`, `contains`, `is_empty`, `is_full` and `is_valid` query the whole handler. `is_full`
uses general capacity at empty indices, so an empty, zero-capacity index is already full.
`comparator_signal` averages the fill over all indices: empty and zero-capacity indices contribute
zero and overfilled indices contribute at most one. `insert_stacking` tops up matching indices
before filling empty ones; an overanswer rolls it back and returns `Overanswered`. `extract_first`
takes the first extractable resource matching a filter. `find_extractable_resource` probes each
candidate with its full int amount in a separate transaction that always rolls back. These three
helpers also require the current transaction token, or `None` when opening a root.

Compose handlers from the same module: `CombinedResourceHandler` assigns indices in order and fixes
its size at construction; `RangedResourceHandler::of` and `of_single_index` return `None` for
out-of-bounds selections, including empty ranges. Use `EmptyResourceHandler::default()` to refuse
everything, `InfiniteResourceHandler::new(resource)` for an endless supply of one resource, or
`VoidingResourceHandler::default()` to discard what comes in. To delegate to another handler unchanged, forward its trait methods as in
`RangedResourceHandler`'s API example.

There is no `simulate` parameter. To try an operation without keeping its changes, run it in a
transaction and return `Err` to abort. See [Transactions](transactions.md).

`BlockSide`, `ItemResource`, `FluidResource`, `Overanswered`, and the other local capability and
Part values and errors are native in-process types and do not implement `Wire`. Use `ItemStack`,
`FluidStack`, or a mod-owned `#[derive(mod_api::Wire)]` schema at network or recipe boundaries;
attachment storage keeps using its own attachment codecs.

To send a `BlockSide` address, include `pos`, `side`, and the canonical dimension name from
`level.dimension().id(host)`. Do not encode the address if the name is unavailable. Do not send
`DimensionKey`: custom dimension slots can differ between processes. Resolve the name with
`host.level(&name)` on receipt, and reconstruct the address only if the level is available.

### Hopper and dropper attempts

Vanilla containers, composter-provided containers and container entities keep their direct
automation paths even if a mod also exposes storage. Only blocks without a vanilla container
use the storage attempt path.

- A pull skips only a fully stacked hopper. It visits every nonempty index of the source's
  bottom-face storage in order, extracts one in a fresh transaction, then chooses the first
  receptive hopper slot (empty or matching item and components with room). An empty lower
  index wins over a matching later stack. It commits success; otherwise it aborts and continues.
- A push skips only storage whose slots are all at capacity. One root transaction covers the
  hopper slots in order: snapshot and remove one, ask the destination's indexless insertion,
  restore a refused source slot by direct write, and continue. Commit the first success or
  abort all refusals. The destination face is opposite the hopper's facing.
- A dropper inserts one in one transaction and always commits the attempt, including refusal.
  An accepted item is removed from the source before root-commit callbacks run, so a writable
  destination callback cannot move the same source item a second time.
- Hopper ticks push before pulling; a push can make room for a pull in the same tick. A push
  makes no decrement/commit changed mark. A pull marks after writing its hopper slot, before
  commit. The single final mark and eight-tick cooldown happen only after an actual move.
- A failed block-hopper pull does not try item entities. A hopper minecart still tries item
  entities after failure and has no block-hopper transfer cooldown.

Notifications your extract/insert issues happen immediately and survive abort. The default
`ITEMS` and item `STORAGE` handlers defer a single notice per backing block to root commit
for vanilla containers and engine-managed mod slots. On the default block `ITEMS` handler, a
successful `set_max_stack_size` call queues the backing container's change notice at root commit,
even if no items moved. If a root commit runs during `menu_quick_move`, those implicit notices
wait until the clicked slot's accepted remainder is in place, or until the hook finishes without one.
A deferred notice does not run if the original backing container was removed or replaced. A write
from an active `contents_changed` callback to its own backing block still applies without queuing
another notice. Direct writes by mods to their own storage and rollback restores do not
issue notices. Keep storage in the transaction journal: a handler
that ignores it can lose an extracted item on abort. The hopper does not guard or repair that loss.

The ignored `sunrise-client` library tests under
`game::machines::native_items::storage_attempt_tests::` exercise real ticks and Parts storage,
including literal attempt/commit/abort/restore/mark traces, cooldowns, comparator scheduling,
direct vanilla/saved-pot paths, writable notification callbacks and the minecart fallback. They require prepared `SUNRISE_TEST_RESOURCES`
and scratch homes/cwd.
The comparator regression uses a supported comparator in a forced, lit chunk with its 3x3
neighbours loaded. It drives the real scheduler before each hopper tick, retaining literal
comparator executions on game ticks 3 and 5 rather than manually firing those ticks.

Composters need no block entity. Their top `ITEMS` port exposes one empty input
slot: below level seven, each insertion consumes **one** compostable item, even when offered a
stack, and evaluates the same vanilla layers provider and random draw algorithm as hopper insertion.
Their bottom port exposes one bone meal only at level eight; extraction empties the composter.
Unsided and horizontal requests have no game-provided handler. Retained handlers recheck the current level.
State changes are visible inside the transaction, but fill feedback, game events, maturation
scheduling and outward block updates wait for the outermost commit. Aborting restores the
level without those effects. The capability also journals its composter-local random stream,
so an aborted transfer attempt and its retry have the same chance outcome, without rewinding
unrelated world randomness. This ownership follows NeoForge's public
[`TransactionalRandom` contract](https://aldak0.ru/javadoc/26.3.x/net/neoforged/neoforge/transfer/item/TransactionalRandom.html).
Sunrise initializes each position's stream once by copying the current level random state;
capability composting then neither advances nor rewinds the ordinary level stream. This
initialization is Sunrise's mapping, not a claim about NeoForge's unspecified initial seed.
Cached streams retire with their chunk or world epoch. Direct hand, villager and vanilla
container paths keep their existing level-random behavior.

Maturation still takes twenty server ticks after committing level seven. These ports have a
fixed one-item limit and refuse `set_max_stack_size`.

Look up a neighbour once per tick and reuse the `Capability` for that tick's moves. Look it up again
on the next tick because the block entity may have been removed.
