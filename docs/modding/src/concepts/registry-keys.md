# Registry keys

A registry key is a typed, opaque identity for the running mod load. An `ItemKey` identifies
an item without carrying its name or definition, and can't be mistaken for a `BlockKey`.
Compare keys on hot paths. Save or send canonical names, not key slots, local ordinals or
engine numeric IDs. Resource reloads preserve key identity, not world mappings or tag membership.

## Request now, resolve later

In `Hooks::register`, request entries you know your mod needs through the `Registrar`:

```rust,ignore
let stone = r.block_key("minecraft:stone");
let water = r.fluid_key("minecraft:water");
// Keep the keys in your mod or Part for later calls.
```

Requests are canonicalized and deduplicated. A bare **request** uses `minecraft:`; a bare name
in a mod-owned declaration, such as `declare_fluid`, uses that mod's namespace. Requests resolve
after all declarations, so you can request another mod's content. An unknown request refuses the
load and names the requesting mod and entry, even if no Part uses it.

## Registry lookup and enumeration

Use `host.block_key("minecraft:stone")` (and similar string-taking lookups such as `effect_key`
and `fluid_key`), or `host.item_key(&id)` with a parsed `Identifier`, when a full name arrives from
config, data or a command. These read the completed registry and return `None` for unknown names
or unavailable tables. Registration callbacks see a startup snapshot, not the completed table.
When you need a whole table, enumerate through the host's typed key methods once at load time;
order follows the owning registry and can differ by registry. Block enumeration yields blocks,
not their states. Similarly, read an entity's type key from its live handle and compare it with a
retained key instead of formatting its name. A stale handle can no longer answer.

An engine registry number, such as `ItemKey::registry_id(host)`, is for engine behaviour on that
side. It is neither the key slot nor a protocol ID. When working with raw packets, use the
connection's registry mapping instead; it may not be available without a connection.

## Pinned registry names

Many keys' `id(host)` methods borrow their canonical name from the host. Others, such as
`EntityTypeKey`, `MenuKey` and `EnchantmentKey`, return an optional `RegistryName`; the
[generated API reference](../reference/api.md) describes each key's exact return type.

A `RegistryName` keeps its text available without borrowing the host or holding a registry lock.
Use `as_str()` to borrow it, or compare it directly with the full namespaced name. Cloning shares
the text. A pinned name remains readable across a reload but does not reflect a later publication.
Persist the name's text, not the key or the snapshot that produced it.

## Block states

A `BlockState` is an owned, copyable `BlockKey` paired with a block-local state ordinal. The ordinal
keeps its meaning when the world's numeric state IDs are reordered. Start with the default state
or look up the block's `BlockStateRange` with `host.block_states().states_of(key)` (via `HostBlockStates`).
The range yields owned states in local ordinal order, not world numeric IDs. Membership checks
both the block and its ordinal. Persist a block's name and properties rather than the ordinal.

Pass the state itself to metadata reads, scene and structure snapshots, and world writes. A
restricted level read can return vanilla's air state even at an unloaded position. An unavailable
mapping returns no state, not fabricated air; check residency separately. A typed world write
can distinguish an unavailable level or invalid state from a valid write that vanilla refuses.
See [Block-state facts](../how-to/world/block-state-facts.md) and [Level hosts](hooks-and-the-host.md#level-hosts).

## Item values and tag names

An `ItemStack` owns an `ItemKey`, a positive count and optional component changes. Use `None` for
an empty slot. A read-only `ItemStackView` lets you inspect a resident stack without copying its
components; project it when you need an owned value. Read or draw engine-legal stacks as they
are, but don't assume they're valid to insert elsewhere. The engine checks the effective count,
damage and components on write and refuses invalid values rather than clamping them. See
[Owned item values](../how-to/items/item-stack-values.md) for the boundary rules and examples.

On a creative client, `host.give_stack(&stack)` inserts into the local player's inventory,
keeping the stack's components. It sends every changed slot to the server immediately,
even when a server menu is open. `Ok(None)` means the whole stack fit; `Ok(Some(remainder))`
returns what didn't fit. Nothing is dropped. A `CreativeGiveError` leaves the inventory
unchanged: `Unavailable` means there's no ready client world or game backend, `NotCreative`
means the local player isn't in creative mode, `FeatureDisabled` means the connected server's
enabled feature flags do not enable the item, and `Stack(error)` means the game refused the
stack (for example, an unknown item, a count above the effective stack limit, or invalid
components). By contrast, the server's [`PlayerRef::give_stack`](../how-to/entities/server-services-and-players.md#inventory-ender-chest-and-saved-data)
gives to a server player as `/give` does, normally dropping overflow for that player instead
of returning it.

A `TagKey<K>` identifies a **name**, parameterized by the registry key type `K` it queries
(`TagKey<ItemKey>`, `TagKey<BlockKey>`, `TagKey<FluidKey>`, `TagKey<EntityTypeKey>`,
`TagKey<BiomeKey>`, `TagKey<EnchantmentKey>`, `TagKey<StructureKey>`, `TagKey<PotionKey>`,
`TagKey<DamageTypeKey>`, `TagKey<WorldClockKey>`), so a block tag cannot be passed to an item
query. Sunrise provides const-built convention tag constants under `mod_api::tags::c::*` (for
example `mod_api::tags::c::items::INGOTS` and `mod_api::tags::c::blocks::ORES`), seeded beneath
vanilla, data packs and mods so `values`, `remove` and `replace` work as usual. For custom tags,
request an untyped `TagKey` during registration with `r.tag_key` or look it up with `tags.key(&id)`,
then convert it with `.typed::<ItemKey>()` (or `.typed()` when the query fixes `K`). A missing tag
can still have a key; all registries share the name table but have separate membership. A reload
replaces membership, not the key. Clients use the authoritative tags applied from the server, not
locally prepared tag data.

| Membership result | Meaning |
| --- | --- |
| `Missing` | No tag bound to this name in this registry; a missing required entry can leave it unbound. |
| `Members(keys)` | The bound members in vanilla order. This list can be empty. |
| `Unavailable` | The authoritative table or a required member mapping isn't available; no partial list is returned. |

Use `host.tags()` to test membership or list the members. A test can return `Some(false)` for
both a missing and a bound-but-empty tag; enumerate to tell them apart. Leave a fallback arm
when matching the non-exhaustive `TagMembers` enum. See [Load phases and access](load-phases.md)
for when to refresh caches built from these tables.

## Owned values

Use API values rather than engine types. Owned records retain facts, not live access; a handle
inside a snapshot can still go stale. Structured text keeps translation keys, styles and children
until display. Keep opaque metadata as NBT `Tag` data when its types matter: JSON conversion loses
numeric widths and typed-array identity and can refuse values it cannot represent.

| Value | What it represents |
| --- | --- |
| `TooltipDisplay` | Effective tooltip choices, including vanilla defaults. |
| `LootContext`, `LootStack` | Roll facts and loot candidates; counts are signed and component patches preserve removals. |
| `ItemBar`, `ItemRarity` | Item resource display properties. |
| `Side` | Package metadata, not the executing logical `HostSide`. |
| `Suggestion` | A completion and its byte range in the input. |
| `Block`, `Item` | Registration declarations; `None` in a component patch removes that component. |
| `BlockEntityTypeDecl` | A block entity type declaration, not a resident entity. |
| `GuiBlitRegion`, `GuiSpriteRegion` | In-process source regions for [canvas blits](../how-to/client/gui-paint.md#blits), not serialized draw commands. A refused call leaves earlier draws intact; return its `GuiDrawError` from the drawing callback to discard the group. |

## Fluids and effects

Tank-only fluids have keys but no world state. For a fluid that flows, declare a world fluid with
its block, flowing form and bucket supplied by the game. See [Content mods](../how-to/content-mods.md)
for both declaration forms and [Block fluids](capabilities/block-fluids.md) for tank behaviour. A
saved tank whose fluid's mod is absent retains its saved name as an `UnresolvedFluid`, without inventing a
key or admitting it to vanilla storage: its stack's `key()` is `None`, `UnresolvedFluid::id()` borrows the
name, and a payload sends the name, which the receiving game refuses when it has registered it. Active
effects use `EffectKey`, not an enchantment effect-component name or a tag string.

## Where to go next

- [Item and tag Parts](parts/item-and-tag-parts.md) covers scoped hooks. The
  [particle provider guide](../how-to/client/particles-and-movement-queries.md#particle-providers)
  distinguishes retained registry keys from typed particle options.
- [Recipe caching](../how-to/items/reload-stable-recipes.md) distinguishes reload-owned definitions
  from live matching. Engine matching uses the server's recipe type and input shape, not the
  serializer, and consumes nothing. No match is not the same as an invalid input or a type with no
  engine matcher. For a mod-owned type, cache the published definitions and match in the mod;
  only enumerate ingredient keys when the recipe really is expressible as plain item membership.
- [Hook notes](../reference/hook-notes.md) covers hook-specific snapshots and live loans. The
  [generated API reference](../reference/api.md) gives constructors, traits and typed refusals.
