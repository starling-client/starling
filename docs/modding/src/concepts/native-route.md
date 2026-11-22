# The native route

The native route lets a native mod change the game's own code: a function's body, a value or call inside
it, or a hot loop. You declare it with `[[native.*]]` tables written after `[native]` in `sunrise-mod.toml`,
and the build applies them to the prebuilt game without rebuilding the game.

## When to use it

Use a [hook](hooks-and-the-host.md) or a [Part method](parts.md) when it exposes what you need. If the
API is missing a hook, describe the game action, its side, the values you need, and whether the hook needs
to observe, change or cancel the action in a [hook request](https://github.com/sunrise-org/sunrise/issues/new).

Take the native route when you need to change a game function or hot loop or reach private game types or
fields that the API doesn't expose. To store data on an entity or block, use an [attachment](attachments.md).

## The tables

| Table | Use it to | Page |
| --- | --- | --- |
| `[[native.mixin]]` | run code at the start or end of a game function, change a value or a call inside it, or replace its body | [Mixins](native-route/2-mixins.md) |
| `[[native.loop]]` | run code before and after each entry of a hot loop the game publishes, such as the block entity tick | [Published loops](native-route/3-loops.md) |
| `[[native.bridge]]` | read, write or call a private field or method of a game type from your mixin code | [Bridges](native-route/4-bridges.md) |
| `[[native.module]]` | add a module of your own code inside a game source file, for your mixin code to share | [Support modules](native-route/5-support-modules.md) |
| `[[native.state]]` | add a field to a game type: refused, use an attachment | [Native state](native-route/6-native-state.md) |

Every row of these tables has an `id`. No two rows of one mod share an `id`.

## It is tied to one game build

A mod on this route builds against one exact game build: the commit in `[native] sunrise`. Your code gets
the game's own types, such as `Game` and its fields, and these change between builds without notice.

- To move to a new build, change `sunrise` and rebuild. A target that was renamed or moved fails the build
  with an error naming your mod.
- A change elsewhere in a target's body is not reported. Reread the functions you target on each new build.
- Read the game's source at your `sunrise` commit on [GitHub](https://github.com/sunrise-org/sunrise). Every
  target path, field and type on these pages comes from it.
- This code runs with the player's rights. It is not sandboxed.

## The `[native]` table

| Field | Value |
| --- | --- |
| `game` | The Minecraft version, `"26.3"`. |
| `sunrise` | The game build: the exact Sunrise commit, 40 lowercase hex characters. |
| `mixins` | The exact commit of the [horizon-mixins](https://github.com/horizon-rust/horizon-mixins) tool, which applies `[[native.mixin]]` rows; 40 lowercase hex characters. |

`cargo sunrise-mod new path/to/mod --template native` writes all three; to move an existing mod, edit them. Otherwise the mod is an ordinary native mod
([Your first native mod](../start/first-mod.md)).

## Building

Build as for any native mod, with `cargo sunrise-mod build`
([Your first native mod](../start/first-mod.md#5-build-against-the-prebuilt-game)). A player's launcher builds it
the same way from the package ([What the launcher does](../how-to/native-mods/04-what-the-launcher-does.md)).

A build with a `[[native.mixin]]` row obtains the horizon-mixins tool for your `mixins` commit.

The prebuilt tool is checked against its release's `SHA256SUMS` and against the commit, patch and platform the
build pins. Where no prebuilt exists for your platform or pin, the build compiles the tool from source.

Read the build log, whose path `cargo sunrise-mod build` prints, for the number of patched functions and
mixin rows. A player reads the same log through the launcher's **Build log**
([build log](../how-to/native-mods/05-build-log.md)).

How the build works is on [How the native route works](native-route/7-how-it-works.md).
