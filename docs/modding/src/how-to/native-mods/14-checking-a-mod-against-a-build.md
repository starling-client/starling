# Checking a mod against a build's surface

In a mod folder, `cargo sunrise-mod` does this for you from the SDK it fetched into `.sunrise/sdk/`:

- `cargo sunrise-mod check` runs `cargo check` for this computer, checks the package rules,
  and prints what the mod uses: `Greeter uses 14 items of the API (6 hooks, 5 methods, 3 types). Made for
  nightly-2026-11-13 (afe471c).` With `--uses` it lists each item with the line that brought it in.
- `cargo sunrise-mod build` packs the list into the package as `uses.json`, with each item's fingerprint at the
  pinned build, and each mixin target, loop and internal the manifest declares with its fingerprint there. A mod
  with a `[[native.module]]` or `[[native.bridge]]` row is made for exactly one build and gets no list.
- `cargo sunrise-mod check --build nightly` asks whether the mod fits the current Nightly.
  Repeat `--build` for a report on each named build, or pass a package:
  `cargo sunrise-mod check greeter.smod --build nightly-2026-11-20`.
  This is a fit report instead of the ordinary check; it takes only the build names and optional package path.
  It exits 0 when every build fits, 1 when one does not, and 2 when it cannot tell or the question does not
  apply (a mod made for one build, a content mod, or a package with no `uses.json`).
- `cargo sunrise-mod pin [<build>]` reports what moving the pin breaks, where the mod uses each changed item,
  and the closest replacement it can find, then moves the pin. With no build given it pins the current Nightly.
  It prints the change notes (why and `migrate`) of every SDK between the two pins for the items the mod uses
  that changed; `cargo sunrise-mod check` prints the same notes after a compile error they explain. Each SDK's
  release page lists the notes since the SDK before it.
- `cargo sunrise-mod hooks <text>` searches the pinned build's hooks by id, doc and their NeoForge, Fabric and
  Paper names, offline; `--all` searches every item. `cargo sunrise-mod sdk diff <build> <build>` prints what
  changed between two builds, from their small files alone, so it works for builds whose game files are pruned.

Every command and its exit codes are in [Commands](../../reference/sdk/05-commands.md), and the rule the check
applies in [Fitting another build](../../reference/sdk/04-fit.md). The rest of this page is how the same check runs
by hand.

Every build's SDK carries its surface export (`surface.json`). Given the surface
of the build a mod was made for and the mod's folder, write the mod's uses table, then check it against
another build's surface:

```sh
sunrise-stage uses MADE_FOR_SURFACE MOD_DIR --json uses.json --made-for BUILD
sunrise-stage fits uses.json TARGET_SURFACE
```

`uses` reads [`sunrise-mod.toml`](../../reference/manifest.md) with the package reader when the file is
present, including the default `registries` capability when the manifest leaves `capabilities` out.
With no manifest, it scans the Rust sources alone.

`uses` prints each API item the mod uses with the place that brought it in, then how the mod builds each struct
it builds with a literal. The table it writes records, per item, its hash at the made-for build. It also records
the type's trait impls and the public paths the mod writes it by. For each struct the mod names, it records
whether the mod builds it in full, with `..`, or not at all, and the field set when built in full. For a trait's
member, it records every host trait in the mod's list that leads to the member's trait.

`sunrise-stage fits` exits 0 when the mod fits (`fits (N items, made for BUILD)`). It exits 1 when it does not, printing
`does not fit: K of the N items it uses changed` and one line per changed item, for example
``the method `PlayersHost::player` (removed)`` or
``the type `DimensionSettings` gained the field `ultrawarm`, and `src/world.rs:12` builds it field by field``.
It exits 2 when an input cannot be read or judged.

The check reads the mod's source with a Rust parser; it is not a compiler and does not expand macros. A name
it cannot resolve with certainty counts as used. A struct literal written with a
path (`world::DimensionSettings { … }`) counts for the API's struct of that name, unless the segment before the
name is a type (`Self`, a type the mod defines, or an API type: `Shape::Pos(…)` and `TextHover::Item { … }` are
variants, not the structs `Pos` and `Item`), which the scan reads through the mod's `use` items first, so
`use mod_api as Pos; Pos::Settings { … }` is still the API's `Settings`; an unqualified one counts as
the mod's own only in the file that defines it outside any `#[cfg]`, when nothing in that file (a `use`, a glob,
an inline module, a macro or attribute that is not standard or the API's) could change what the name means and
the mod has no `include!`. The parser reads a macro body's tokens as they
stand: an API struct name followed by `{` or `(` is a build, in full unless it ends in `..`, wherever it stands
(`a < Pos(1, 2)`, `for Pos(x, y) in v`). A tuple or unit struct's name with no bracket after it is a build too,
except where only a type stands or as a type written after `name:` in parameters, a `let` or a struct's fields
(`.map(Pos)`, `{ Marker }` and `make: Pos` count).

A positional read counts when the scan can tie it to a tuple struct path: `p.0` on a typed parameter or
binding, a read on a constructor result (directly or through a binding), `self.0` in a known `impl`, or the
positions a tuple pattern lists before `..`. If the path is proven to name a mod-local type or constructor,
or an enum variant, it does not count for a same-named API tuple struct. An unresolved path keeps its
plausible API fields.

What the scan cannot place is left to the compile, and records nothing:
- a literal of a name that is no API struct's: a macro's `$t { … }`, another crate's type;
- a `Self { … }` the scan cannot tie to an impl, and a literal of a `type` alias whose target is not a plain path;
- whatever a macro, derive or attribute from outside the mod and the API expands to;
- a literal or pattern under the name of a struct the pinned build keeps closed to literals: a mod that compiles
  cannot be building it, so the name is another type's (your own enum's variant `Source::Block(b)`).

The real compile on the player's machine is always the final check. A mod judged to fit whose compile fails is
left out by name, with a note that it needs an update from its author, and the other mods still build.

Two changes always refuse, because no compile can see them or repair them: an item in the list that is removed,
renamed or changed, and a hook in the list whose version changed (the same signature with a new meaning). The
promise covers the hooks the list names: a subscription that a macro from outside the mod and the API writes is
not in it. A list that holds no version of a hook, as one an older tool wrote, gets "not known" for that hook
([Lists an older tool wrote](../../reference/sdk/04-fit.md#lists-an-older-tool-wrote)).

Some sources cannot be judged at all:
- a file that does not parse as Rust (the message gives its line);
- source nested deeper than 256 levels. The count is cautious, so one bracket holding more than about 250
  comparisons between plain names (`[a < b, c < d, …]`) is also refused, though it would compile;
- a `use` statement nested deeper than 64 groups;
- a symbolic link;
- a `#[path]` or `include!` that leaves the mod's folder or is computed, or an `include!` renamed by a `use`.

The parser runs first in a separate process, the same program started again with the mod's source on its
input, never on its command line. If that process runs out of stack, or takes longer than 15 seconds, the
source cannot be judged, and the program that asked carries on. On Linux the process's memory is capped at
4 GiB, and on Linux and macOS its processor time too; Windows and macOS have no memory cap, so there only the
15 seconds bound it.

This by-hand check covers the mod API's items. The game code a mod declares is compared by
`cargo sunrise-mod check --build <name>`, from the target build's build data; `sunrise-stage` refuses such a mod
and such a list by name. A mod with a `[[native.module]]` or `[[native.bridge]]` row is made for exactly one build, and no check
applies to it. A mod that fits is still compiled against the target build, and the binary built is exact to that build.
