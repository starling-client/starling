# Read-only equipment predicates inside menu loops

> The native contract below still applies. WebAssembly mods were removed; the parts about guests are history.
> The borrowed predicate does not require guest preparation or missing-export refusals.
> See [`ItemPart::can_equip`](../../crates/sunrise-mod-vocab/src/hooks/can_equip.rs) for the current native API.

Implemented boundary for #2669 and #3549. The historical conversion analysis below
explains why the engine stack and API stack remain distinct owned types.

## Can we just borrow the existing stack?

**Not through the existing boundary.** The menus already borrow an engine stack,
not an API stack: `crates/sunrise-client/src/menu.rs:615–636` and
`crates/sunrise-client/src/lan/menu.rs:460–501`. Both belong to client-lib (whose
manifest uses these source paths). `Game::menu_ops` delegates through `menu_session`
(`crates/sunrise-client/src/game.rs:9506–9585`); its `Env` lends only loop context
and player identities (`menu.rs:88–97`). `LoopContext` contains registry/settings
snapshots, not entity storage (`crates/sunrise-world/src/native_load_context.rs:193–215`).

The engine stack owns `Arc<str>` and boxed extra data
(`crates/sunrise-items/src/items.rs:417`); the API stack owns keys and an optional
`Arc<Components>` (`crates/sunrise-mod-vocab/src/item/item_stack.rs:62`). They cannot
be reborrowed as the same type. The existing slot adapter converts before borrowing
(`game/modding/api_values.rs:1192–1208`, under the client source root). Even ordinary
Host-backed borrowed-stack hooks are not a zero-copy precedent: `native_use_item`
(`game/native_hooks/base.rs:3477`) calls generated dispatch whose crossing creates
an API value (`crates/sunrise-mod-api-gen/src/dispatch.rs:909–942`). The generator
passes Game itself as Host, not a newly allocated Host
(`crates/sunrise-mod-api-gen/src/host_param.rs:109–115`).

| Option | Engine/bridge allocations per candidate slot |
| --- | --- |
| Existing adapter, no subscribers | 0, but no readable target entity contract |
| Existing adapter, subscribed | Payload-dependent, not a fixed count: component-bearing stacks allocate an encoded compound and an API component owner, plus nested payload allocations; plain stacks may allocate 0 |
| Borrow the engine stack directly as API `&ItemStack` | Not type-correct; a cosmetic Host does not fix it |
| Prepare one API stack before slot traversal, then borrow it | 0 during traversal; preparation is payload-dependent and still needs a real Host/entity boundary |

Conversion evidence: `api_values.rs:711–728`,
`crates/sunrise-items/src/item_data.rs:1110`, and API `set_components` at
`item/item_stack.rs:337`. Thus **hoisting conversion makes borrowing sufficient for
the stack cost**, without a new public stack representation, but does not remove
the need for the following small Host boundary.

## Implemented boundary

The native `can_equip` predicate runs synchronously against the live `&Game` lent as a
read-only `&dyn mod_api::Host` (through `NativeHookHost { host: self }`), with an
API-owned `LivingRef` target handle and a borrowed `&mod_api::ItemStack`. There is no
pre-captured `EquipSnapshot`, no cached `EquipHost`, no thread-local revision/pointer
stamp table, no `EquipDecision` wrapper and no apply-time snapshot recheck: the read-only
`Host` borrow makes live entity and world reads directly available while the synchronous
decision and apply run under the same game state (matching NeoForge's
`IItemExtension.canEquip` and vanilla's `LivingEntity.isEquippableInSlot`).

Before converting a stack or calling into mod parts, every caller checks
`item_listens(HookId::CanEquip, &stack.item)`. When no `ItemPart` listens on that item,
the stack's resolved `equippable` component answer stands immediately with zero
conversion and zero entity lookup.

Where one unchanged stack is tested across multiple candidates in a single synchronous
operation, conversion is hoisted locally:
- Dispenser (`EquipmentDispenseItemBehavior`) and mob pickup (`Mob.equipItemIfPossible`)
  convert once with `Game::equip_stack` into a local `EquipStack` and reuse it for each
  candidate entity or slot in that call.
- Inventory menus (`Menu::move_item_stack_to` and `Input::QuickCraft` distribution)
  borrow the unchanged `&ItemStack` through `CanEquip::with_stack`, which holds a
  stack-local `OnceCell<EquipStack>` in `ScopedMenuEquip` keyed only by that immutable
  borrow. The conversion runs lazily on the first armor candidate and is reused with zero
  extra conversions or allocations for every remaining candidate in that loop.

A candidate that asks a listening `ItemPart` refuses a missing or despawned target handle
instead of aliasing a recycled entity ID. Client prediction uses the replicated client
entity view; the server evaluates authoritatively against live server state.

The required engine/bridge budget **per candidate** after local conversion is 0 heap
allocations and 0 stack or component payload clones/copies. Setup conversion of a
component-rich stack happens at most once per synchronous candidate loop and is covered
by `equip_answers::tests` (including a four-armor-slot `QuickCraft` menu pass and warmed
allocation measurement) and `menu::equip_movement::tests`.

## Historical guest preparation

The removed guest route encoded the stack and read view once per participating guest
instance, then passed call-scoped tokens, numeric slot IDs and scalar answers without
engine pointers or per-slot stack encoding. It rechecked revisions when sharing a
prepared payload across candidates (#3549); generation and release checks invalidated
stale preparations, and registration refused missing prepared exports. The old loopback tests
compared native and guest answers and checked token refusals. The guest copy probes
measured preparation bytes separately from scalar asks. None of these guest exports,
refusals or probes is required for native code.
