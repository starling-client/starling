# Writing hooks

Every native mod has the same shape: one crate whose root implements `mod_api::Hooks`.
This page gives the rules that shape follows.

## The crate

- `sunrise-mod.toml` is the only source of the mod's id and version. Cargo's package name and version are not
  the mod's identity.
- `Cargo.toml` depends on `sunrise-mod-api` (lib name `mod_api`), never on the game's own crates. Other
  dependencies follow [The mod's crate](../how-to/native-mods/01-the-mods-crate.md).
- `src/lib.rs` holds the one marked `impl Hooks`, at the crate root.

## The mod type and its hooks

```rust,ignore
use mod_api::{self, Hooks};

#[derive(Default)]
struct Mod;

#[mod_api::sunrise_mod]
impl Hooks for Mod {
    // The hooks this mod needs.
}
```

- The mod type implements `Default`; the game makes one value of it per mod.
- `#[mod_api::sunrise_mod]` generates the mod's `hooks()` factory and subscribes the mod to exactly the methods
  written in the block. Do not write a factory yourself.
- The marked impl must be at the crate root. If the build reports "cannot find function `hooks`", the marked
  impl is in a submodule.
- Hook methods are written out in the block; a method a macro would generate inside the impl is not seen.

## Events and hosts

A hook method takes the side's host and the moment's event:

```rust,ignore
fn player_logged_in(&self, host: &mut dyn ServerHost, event: &mut PlayerLoggedInEvent<'_>)
```

- The **event** carries what happened. Fields a mod may change are writable, and a hook that can be cancelled
  has a way to say so in its event or return value ([Hooks and the Host](../concepts/hooks-and-the-host.md)).

## What can this hook do?

Read the hook's signature. The host parameter tells you which game operations you can call:

| Host parameter | What it gives you |
| --- | --- |
| `&dyn Host` | Shared reads, such as resolving handles and looking up registry entries. It doesn't give you server write methods. |
| `&mut dyn ServerHost` | Shared reads and server writes. |
| `&dyn ClientHost` | Shared reads and client-only methods that take `&self`, such as `host.local_player()`. Some hooks instead give `&mut dyn ClientHost` for methods that need a mutable client host. |
| `HostSide<'_>` | Match on `HostSide::Server(host)` and `HostSide::Client(host)`; each branch gets that side's mutable host. Do server-only work in the server branch. |
| `LevelSide<'_>` | Match on `LevelSide::Server(level)` and `LevelSide::Client(level)`. These are level-only hosts, with no `open_menu` or commands. |

With a world open, a mutable `ServerHost` can open a chest-style menu
through `MenusServerHost::open_menu` (`host.open_menu(…)`).
`ServicesServerHost::commands` gives you the `Commands` service: call
`host.commands().run_command(line)` to execute a server command, or
`host.commands().run_command_with_output(line)` to keep the feedback.
Neither operation is available from a client host.

Handles in the event (a player, an entity, a level) have methods of their own.
Read the method's signature too: a server write on a handle takes a mutable
server host. Not every hook has one: for example, a registration hook gets a
registration context instead of access to a live world. See
[Load phases and access](../concepts/load-phases.md) for when services are available.

From your mod project, run [`cargo sunrise-mod hooks <query>`](../reference/sdk/05-commands.md)
to find matching hook names in the pinned SDK, for example:

<!-- quickstart-test: in first-mod/greeter -->
```sh
cargo sunrise-mod hooks player_tick_post
```

With the SDK already fetched, this works offline. The search prints names and short
descriptions, not Rust signatures. For the exact signature, go to the matching
method on `mod_api::Hooks` in your editor, using the pinned SDK under
[`.sunrise/sdk/api/`](../reference/sdk/03-in-a-project.md#sunrisesdk).
For a scoped hook, look at the relevant Part trait instead.

The engine declares the parameters of each hook in vocabulary types in
`crates/sunrise-mod-vocab/src/hooks/`. The listener method and event come from that declaration.
When adding a hook to the engine, give its game function the generated `fire::<Name>Call`
carrier rather than copying the parameter list. See [Adding a hook](https://github.com/sunrise-org/sunrise/blob/main/docs/mod-api/adding-a-hook.md).

## Order between mods

Mods run in dependency order: a mod after the mods it depends on. A hook method may carry
`#[listener(priority = <i32>, receive_cancelled)]`, both optional:

- mods run by descending `priority` (0 when omitted), then in dependency order;
- with `receive_cancelled`, the mod is still called after an earlier mod cancels the event, and can read and
  clear `event.cancelled`. It compiles only on a hook a mod may cancel.

## Per-object behaviour

A moment that belongs to one block, item, block entity, menu or entity type is a method of a **Part**, not a global
hook: the game calls it only for the objects your mod attached the Part to. See [Parts](../concepts/parts.md).
