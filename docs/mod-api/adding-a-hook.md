# Adding a hook

Declare a new hook in `crates/sunrise-mod-vocab/src/hooks/<name>.rs`: add `///` lines and a
`#[hook(...)]` attribute to its bodiless `fn name(parameters);`. Write the parameters in vocabulary
types and keep the receiver and phase context out of the list. For example:

```rust,ignore
#[hook(side = "client", contract = "cancel", out = "cancel")]
fn play_hit_sound(state: BlockState, pos: BlockPos, face: Direction, cancel: &mut bool);
```

The game method takes `super::fire::PlayHitSoundCall<'_>`. The generator maps its fields across the
engine boundary: `state` is a `BlockStateId` in the carrier, for example. Put the method in the
same-named `game/native_hooks` module or set `target` to its actual Rust path. Use `receiver = "read"`
for `&self` or `receiver = "associated"` for an associated function; otherwise use `&mut self`.
See the catalog's `mod.rs` for the declaration arguments.

You can start a method under `native_hooks/` with `let _ = call;` (or `let _ = (every, parameter);`
if there are other inputs). Run `cargo run -p xtask -- mod-api` to fill it with the call to the
`fire_*` adapter. That command refuses an unrelated body. Commit the declaration and the game method,
not the `OUT_DIR` files. Builds generate from the catalog; rustc checks the engine method's call to
the adapter. Mixin injections still go at the head of the method, before dispatch.

A hook that stands in for a vanilla block, block entity, item or menu method also gets a `scope` and
`key`, such as `scope = "block", key = "state"`. The key's type selects the Part; for a block,
`BlockState` routes by its prepared block key. See [Parts](../modding/src/concepts/parts.md).

Free-function targets keep their hand-written wiring. A crate that can call a generated dispatcher
calls it directly; other crates call an installed callback such as `NativeLoadHooks` in
`sunrise-world` or `NativeClientHooks` in the client library. Raw render hooks keep their replaceable
functions and the renderer calls them through generated typed adapters.

Mutable owned stack-list arguments and collection outputs use generated per-listener validation, not
a hook-specific write-back rule. Single-stack and optional-stack adapters keep their existing policy.
Dispatch retains the preceding accepted stack list and validates
each edited output before leaving the listener's ownership scope. Refusal logs the mod, hook,
output and conversion error without discarding scalar/cancel outputs. The engine wrapper captures
the active immutable item-registry owner before lending its mutable host, and caches accepted
engine conversions for final commit. Untouched listeners neither clone another checkpoint nor
revalidate unchanged stacks. Keep this policy in the generator when adding mixed-output hooks;
the general generated hook notes describe the author-visible rule once.
