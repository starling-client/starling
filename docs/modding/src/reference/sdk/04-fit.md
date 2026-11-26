# Fitting another build

A mod is **made for** the build it pins. A native mod also **fits** another build when nothing it uses changed
there (the mod API's items, and the game code a `[native]` row or an `internals` list declares), and the launcher
then compiles it against that build on the player's computer. Only the built
binary is exact to the build; the loader's build-id check is unchanged.

## The rule

The [package reader](../manifest.md) validates the manifest before the launcher checks fit. In particular,
`[[native.state]]` is refused with `E_MIXIN_STATE_LAYOUT` regardless of the build pin.

The launcher asks, for each enabled native mod of an instance, in this order:

1. **Made for this build?** Then it can be built without comparing a surface. If it names mixins or published
   loops, `native-check` reads the build data first and reports indexed mixin refusals and unknown loop names.
2. **Rows no list can judge?** A mod with a `[[native.module]]` or `[[native.bridge]]` row names game code that
   no build data describes, so it works on the build it was made for only. No test runs.
3. **Does its package say what it uses?** Its `uses.json` must be one this launcher reads and be made for the mod's
   pin. A package without one (not packed by `cargo sunrise-mod build`, which writes it) works on the build it was made for only.
4. **Is every item it uses unchanged?** Each entry of the list must hold in this build's `surface.json`:
   - the record exists, with the same hash, under every public path the mod writes it by;
   - a type keeps every trait impl it had;
   - a struct the mod builds with a literal can still be built that way, and, built field by field (no `..`), still
     has the same fields;
   - a hook in the list still has the version it had;
   - a trait member the mod reaches through a trait it calls (a sub-host trait's method called on
     `&mut dyn ServerHost`) is still reached from that trait.

   And each piece of game code the mod declares must be, in this build's build data, what the list records.
   `functions.json`, `internals.json` and `loops.json` must all describe this build; the fit check validates the
   function index and the loop paths just as patching does. If a file is missing, fetch this build's SDK again.
   An empty loop catalog is valid; a missing `loops.json` is not. If the caller supplies build data and the
   check reads it, a missing file is an error even when a mod already has an API misfit. To fit:
   - a `[[native.mixin]]` row's target function has the same declaration and body (a replacement written
     for an old body could undo what the game changed in it);
   - an `internals` entry has the same signature; for functions and methods both the declaration and the body
     must be unchanged;
   - a `[[native.loop]]` row's loop is published the same way, its entry and its generic function have the same
     declaration and body, and its slot trait the same signature.

   When both the uses list and the target build have split digests, the report names the function and says
   `body changed` or `declaration changed`. The declaration includes the signature, visibility, attributes and
   enclosing `impl` context. These digests compare tokens, so whitespace and normal comments do not count.
   Both kinds of change mean the mod does not fit; it is left out rather than rebuilt. If either side has only
   the older hashes, the report says `changed` and a byte change inside the source span counts.

   A list with no record of something the mod declares cannot show a fit. Nor can anything show a fit for a
   function the build data holds no body for (it indexes the functions a mixin may target): a mod that declares
   such a function as an internal works on the build it was made for only.

   The mod's `[native] mixins` pin, when it has one, must be the horizon-mixins revision this build's tool builds
   with.
5. **Do the mods it needs fit?** A mod whose library mod is left out is left out too: "it needs Corelib, which
   doesn't fit".
6. **Does it compile?** The compile against the build is the final check. A mod that fits and fails to compile is
   left out of that build, and the other mods build.

A mod that doesn't fit stays installed and is left out of the build, with the changed items named (at most five; the
build log has them all). The texts players see, the check before a game update and what happens to worlds saved with
such a mod are in [What the launcher does](../../how-to/native-mods/04-what-the-launcher-does.md#mods-made-for-another-build).

What fits does not promise: that the behaviour behind an unchanged declaration is the same. That is the API's
contract, as for any library.

## `uses.json`

`cargo sunrise-mod build` packs `uses.json` beside `sunrise-mod.toml` in every code mod's package, except a mod with
a row no list can judge (step 2), or one whose source the scan cannot judge (the build says so and packs none). The list holds
every surface item whose name occurs as an identifier in the mod's source or that its manifest names, then every
item those items' signatures reference, transitively. A member counts only when its owner is reached. A mixin's or
a loop's helper file is not scanned: it is compiled inside the game's code, where a name is the game's own. The
game code the manifest declares is listed as `game:fn:<path>` (a mixin's target),
`game:internal:<kind>:<path>` (the kind the manifest names, or `any` when it names none) and `game:loop:<name>`,
each with its hash at the pinned build, read from that build's build data. A misfit can also name `game:tool:horizon-mixins`.

A helper that the mod's own source also loads (an `include!`, a `#[path]`, or a `mod` declaration that may name
its file) is both game code and mod code: the scan says it cannot judge the mod, and no list is packed.

A helper can call game code its row does not declare. The compile catches a changed signature there; nothing
catches a changed body in a function the mod did not declare. How the scan
reads source, and what it leaves to the compile, is in
[Checking a mod against a build](../../how-to/native-mods/14-checking-a-mod-against-a-build.md).

| Field | What it is |
|---|---|
| `schema` | The table's schema number: 4 when an entry has `game_code`, otherwise 3, which the tools of earlier builds read. A reader also takes schemas 1 and 2 ([Lists an older tool wrote](#lists-an-older-tool-wrote)). It refuses any other number, unknown fields, repeated ids and a table over 4 MiB. |
| `made_for` | The build id the hashes come from: the mod's pin. |
| `entries[].id`, `entries[].hash` | A record the mod uses, and its hash at `made_for`. |
| `entries[].game_code` | Where the pinned `functions.json` provides split digests: `functions` maps each declared function path to `declaration` and `body` SHA-256 digests. For a loop, `loop_contract` also records how the loop is published and its slot trait's hash. |
| `entries[].impls` | A type's trait impls at `made_for`. |
| `entries[].built` | How the mod builds a struct it names: `full` (some literal or pattern lists its fields with no `..`), `base` (every one ends in `..`) or `none`. |
| `entries[].fields_hash`, `entries[].fields` | With `full`: the struct's field set at `made_for`, and its field names, to name what a build gained. |
| `entries[].built_at` | `file:line` of the first literal that builds it. |
| `entries[].paths` | The public paths the mod writes the item by, besides its id's. |
| `entries[].reached_from` | For a trait's member: the traits the mod calls that lead to its owner. |
| `entries[].version` | For a hook: its version at `made_for`. |

A struct built with `..` (for example `..Default::default()`) keeps fitting when the struct gains a field; one built
field by field does not. `cargo sunrise-mod check --uses` says which of the mod's literals are which.

A registry or a release page can tell which builds a published package fits from the package and each build's
`surface.json` alone, or with the build's build data too for a mod that declares game code:
`cargo sunrise-mod check <file.smod> --build <name>`. For a current list nothing from the build
the mod was made for is needed, so pruning that build's compiled files never strands the mod. The fit report first checks
that the list was made for the build the package pins (`[native] sunrise`). If the pin is missing, is not a full
commit SHA, or does not match `made_for`, the list cannot show which other builds the package fits. If the
function index refuses a declared mixin target, the fit report names the target, error code and reason
rather than reporting the function as removed.

The version promise covers the hooks in the list. A hook entry that holds no version cannot be shown to mean what
the mod was written for, whatever the target's version is: when nothing else changed, the answer is "not known",
naming the hooks, never "fits". `native-check` reports it as `no-list` with `why: "hook-versions"`.

### Lists an older tool wrote

Schemas 1–3 do not have split digests for game functions. The fit check still compares their original
fingerprints, but cannot tell a body-only change from a declaration change.

Schemas 1 and 2 were written by the older scan, which could attribute a mod's own name to the API. The fit check
accepts the older hashes for unchanged signatures. For those lists only, absent API records and lost impls that
refer to a removed API type do not refuse fit. The fit report lists the deferred IDs in one `not checked` line;
the native compile decides whether the mod actually needs them. Changed signatures and hook versions still refuse.

A schema 1 list also differs in two ways:

- **It holds no hook versions.** `cargo sunrise-mod check --build` fetches the `surface.json` of the build the list was
  made for only when a comparison with another build would otherwise be "not known" because a required hook
  version is missing. That file stays published when a build's compiled files are pruned. If it cannot be fetched,
  the answer is still "not known", with the reason. `native-check` and `sunrise-stage fits` do not fetch that file:
  both still report definite misfits and otherwise report "not known" when a required hook version is missing.
- **It recorded a struct as built wherever the scan was in doubt**, with nothing to tell a doubt from a literal the
  scan had placed. A reader does not read how such a list says a struct is built. This is a chosen trade-off: a
  struct the mod really builds field by field, which gained a field, is judged to fit and then fails the compile,
  which leaves the mod out by name. The other choice refuses mods that would have worked.

A list with a schema newer than the tool reads is `no-list` with `why: "newer-schema"`: a tool from a newer build
reads it.
