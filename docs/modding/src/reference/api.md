# The mod API reference

The complete reference for the mod API is the rustdoc of the `sunrise-mod-api` crate (library name `mod_api`):
every hook of `Hooks`, every event, Part trait, handle and `Host` method, with its side, its NeoForge 26.3
counterpart where it has one, and whether a mod may cancel it.

Build it from a Sunrise checkout:

```sh
cargo doc -p sunrise-mod-api --no-deps --open
```

## Finding your way in the reference

| Start at | For |
| --- | --- |
| `Hooks` | Every moment a mod can implement, one method each |
| `BlockPart`, `ItemPart`, `BlockEntityPart`, `MenuPart`, `EntityPart` | Per-object behaviour ([Parts](../concepts/parts.md)) |
| `Host`, `ServerHost`, `ClientHost` | What a hook can ask of the game, split by side |
| `Registrar` | What a mod declares in `Hooks::register`: keys, Parts, settings |
| `AttachmentDescriptor`, `AttachmentKey` | Data a mod keeps on objects ([Attachments](../concepts/attachments.md)) |
| `Wire` | Encoding typed payloads and other API data ([Wire types](../concepts/wire-types.md)) |


## Block contact callbacks

`BlockPart::entity_inside` is called as the entity sweeps across the block. Use `event.apply` to
request a vanilla `InsideEffect`. Use `event.run_before` and `event.run_after` to run a closure at an
effect slot, even if nothing requested the effect itself. The closure receives an `InsideEntityView`
when the sweep flushes. Each step runs in effect order and stops when the swept entity dies.

Read and change the entity and its level through the view. For example, queue a block replacement
before extinguishing:

```rust,ignore
let pos = [event.pos.x, event.pos.y, event.pos.z];
event.run_before(InsideEffect::Extinguish, move |view| {
    view.set_block(pos, replacement);
});
```

Read the `bool` that a write returns inside the closure. A refused or unchanged write returns
`false`; it doesn't prevent another call from succeeding. For sourced damage, obtain an `EntityRef`
from `host.entity(id)` in a server callback that has a `ServerHost`, and capture that handle in the
closure. `hurt_with_source` checks the explicit handles against the live level when called. If a
handle is stale or in another level, that hit returns `false` instead of becoming an unattributed hit.

If the part call panics, what it queued is discarded. If a deferred callback panics, its already
completed live writes are not undone unless an existing transaction already journals them. Either
panic disables that mod's `entity_inside` callback for the session, logs the mod's ID, and queues an
error toast for the client. Other mods' callbacks and the built-in effects still run while the
entity is alive.

[API reference](api-reference.md) indexes the catalog-backed `Hooks` and Part hooks and the
area-declared `Host` / `ServerHost` / `ClientHost` methods; build the rustdoc above for the
complete traits and types. [Hook notes](hook-notes.md) holds longer explanations that go with
groups of hooks, and [NeoForge counterparts](neoforge-counterparts.md) lists each hook's NeoForge
event or method.
