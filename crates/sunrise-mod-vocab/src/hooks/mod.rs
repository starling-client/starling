//! Declarations of the mod API's hooks and draw contexts: the catalog.
//!
//! This module is never compiled (`#[cfg(any())]` in `lib.rs`): `sunrise-mod-api-gen` reads these
//! files with `syn` and builds the `Hooks` trait, its dispatch, the game's entry points and the
//! rustdoc from them. Lint checks these files directly with `rustfmt`. What the game does for a mod, the host
//! methods and the handles', is not declared here but written by hand, one file per area, in `../host/<area>.rs`.
//! Each host method's default and the game's forwarder are generated. Implement the same-named
//! inherent `Game` method, without a `native_` prefix. Hook entry points keep their `native_` names.
//!
//! Each hook has its own file: a bodiless `fn name(parameters);` with its rustdoc. List the
//! parameters in vocabulary types, before they become event fields, a reply, or a cancellation.
//! Do not repeat the receiver or a phase context in the parameter list. Only a parameter used by
//! the engine but not by the listener is marked `#[engine]`, such as a level hook's tick queue.
//! Separate files hold draw contexts and general notes. The declaration traits are:
//!
//! - `Hooks`: `#[hook(side = "...", contract = "...", ...)]`, one per moment the game tells mods about.
//! - `DrawContexts`: `#[draw_context(surface = "...", name = "...")]`, a drawing surface hooks draw on.
//! - `Notes`: `#[notes = "..."]`, prose for the hook-notes page that belongs to no one hook.
//!   A hook's own prose is a `#[notes]` beside its declaration; a host method's, a `Notes` entry.
//!
//! A hook's arguments:
//!
//! | Argument | Meaning |
//! |---|---|
//! | `side` | `"server"`, `"client"` or `"both"`; required |
//! | `contract` | `"listen"`, `"cancel"`, `"collect"` or `"replace"`; required |
//! | `out` | the event field a cancel, collect or replace hook writes; not for `"listen"` |
//! | `staged_surface` | Engine-only marker naming a flat listen hook's mutable surface, such as a registrar, a GUI draw context or a Part's effect sink. The fire adapter hands its owner one listener thunk at a time, under that listener's mod scope. Only an accepted callback publishes its contributions. This marker is not a mod-facing parameter. |
//! | `scope`, `key` | a Part method: the kind of Part (`"block"`, `"block_entity"`, `"item"`, `"menu"`, `"fluid"`, `"entity"`) and the hook argument naming what it is attached to |
//! | `part` | an additional Part method on a plain global `"cancel"` hook (e.g. `"item"`): keeps `Hooks::<name>` and also emits `<Kind>Part::<name>` plus `dispatch_<name>_part(host, part_key, ...)` sharing the same `HookId`. Prefer `scope` + `key` unless one `HookId` must combine a global event with per-part overrides whose dispatch key varies by phase (such as `item_stacked_on_other` dispatching on carried and clicked stacks). Cannot be combined with `scope`, per-listener event construction such as `event_per_listener` or `independent_listener`, `answered_by`, `owner`, `output_validator`, or a phase `context` |
//! | `answered_by` | `"mod"`: a global replace hook whose output also names the mod that answered, or a global collect hook whose output is `Vec<Answered<T>>`: each mod pushes `T`s into its own empty batch and dispatch names that mod on each entry |
//! | `output_validator` | A path on a global cancel, plain global collect or plain global listen hook with shared `Copy` outputs (refused on a collect hook with `answered_by` or `registration_append`). `fn(&accepted, ..., &proposed, ...) -> Result<(), &'static str>` takes both sets in declaration order. It checks only completed proposals. A refusal rejects the whole proposal, including any cancellation, reports the mod and field every time, and does not disable the callback. Later eligible listeners see the accepted outputs; validated cancel hooks do not clear an accepted cancellation. |
//! | `host`, `context` | `"read"` or `"level"`; `"registration"`, `"data_load"`, `"loop"` or `"worldgen"` |
//! | `neoforge`, `fabric`, `paper` | the counterpart's name, or a list of them, for the rustdoc and the gap lists |
//! | `cancellable` | whether the NeoForge counterpart event is cancellable (documentation, not the contract) |
//! | `version` | the hook's contract version; left out when it is 1 |
//! | `target` | the function that fires the hook; defaults to `Game::native_<name>` in the `native_hooks` module with the same filename as the declaration |
//! | `receiver` | a game target borrows `&mut self` by default; `"read"` borrows `&self`, `"associated"` has no receiver |
//! | `dispatch` | a game target takes the generated engine carrier by default; existing flat, already-converted (`"api_carrier"`), and direct bindings name their shape explicitly |
//!
//! Every hook needs `///` lines: they are the rustdoc of its trait method, and a hook without them
//! is refused. `HookId` values are process-local dispatch indexes, not serialized or persistent
//! API identities (SDK records identify hooks by name as `hook:<name>`).
//! Native binaries rebuild for the selected game build. The generator discovers `*.rs` declaration
//! files (other than `mod.rs`) in lexical file-name order, and each file's declarations in in-file
//! order, without a shared list or ordinal pins.
//! Each new hook goes in a file of its own.
//!
//! A new game hook takes its generated `fire::<Name>Call` carrier, built from these declarations
//! and the generator's engine value and draw mappings. A hook with no event parameters needs no
//! carrier. Start the method body with `let _ = call;` (or `let _ = (every, parameter);` with
//! additional inputs); `cargo run -p xtask -- mod-api` fills in the call to the typed `fire_*`
//! adapter. A mismatch fails to compile at that call.
