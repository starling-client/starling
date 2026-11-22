# The manifest and the package

A `.smod` is one mod: content, native code, or both. `sunrise_mod_package::Package` reads the shared manifest and
validates the package; `sunrise_mod_package::Content` reads its resources, and `sunrise_mod_package::index::native::Bundle`
reads its code.

## The zip

- A zip archive. Entries are stored or deflate-compressed, the only methods the readers support.
- A path is relative and `/`-separated: no `\`, no `:`, no empty, `.` or `..` part, and it doesn't start with `/`.
  Files must be regular files, not symlinks, devices or executables. Executable payloads are refused even under
  resource paths.
- Each path appears once. Directory entries carry no payload.

## Project and archive layout

```text
my_mod.smod
├── sunrise-mod.toml
├── icon.png                # optional
├── assets/my_mod/
├── data/my_mod/
├── runtime/                # code mods only, optional
├── Cargo.toml              # code mods only
└── src/lib.rs              # code mods only
```

- **Content only:** no `Cargo.toml` or `[native]`. Nothing compiles.
- **Native code:** root `Cargo.toml` and `src/`, with `[native]` in the same manifest. The launcher builds it
  against the pinned prebuilt game; mixins compile into a separate function-replacement patch library. The game itself does not compile. Its resources load through the same content reader as a content mod's.
- **Runtime files:** a native package may carry a `runtime/` folder of data its code opens beside
  itself, listed in a top-level `[runtime]` table (below). A content mod has none.
- Inconsistent code declarations or executable files in the archive refuse the package.
- The full native layout is in [Package contents](../how-to/native-mods/03-data-files.md#package-contents). Packing
  a project folder takes those files and leaves every other file out; in an archive, a file outside the layout
  refuses the package.

There is no mod-id directory level inside the archive. `sunrise-mod.toml` is the only manifest and the only
source of the mod's id and version; Cargo's package name and version are not mod identity. IDs are plain
`[a-z0-9_]{1,64}` names, also used as resource namespaces. The launcher writes `horizon-mixin.toml` in its
staging area when needed. Hand-written tool manifests, `sunrise-native.json`, and input `native/` or `rust/`
folders are unsupported.

## `sunrise-mod.toml`

The shared [manifest fields](../how-to/content-mods.md#the-manifest) include `id`, `name`, and `version`.
`version` is semver (`MAJOR.MINOR.PATCH`, with an optional pre-release and build metadata such as
`3.6.30+refabricated`); an omitted minor or patch is padded (`1` is `1.0.0`, `1.2` is `1.2.0`).
A non-semver version is refused when the manifest is read, naming the value.

`cargo sunrise-mod build` puts `api = "0.2"` and `capabilities = ["registries"]` into the package. A manifest
that writes either field must give that value; any other value is refused, naming both. Unknown fields refuse the
manifest, naming the nearest valid key when one is close. Native mods may declare top-level `internals`, an array of tables with
`path`, optional `hash`, and `kind` when a path names several member kinds. Without `hash`, the build uses the fingerprint from the pinned game’s catalog. An explicitly empty `hash` is refused. Native code adds `[native]`:

| Field | Type | Rule |
| --- | --- | --- |
| `game` | string | not empty, the Minecraft version (`"26.3"`) |
| `sunrise` | string | the exact Sunrise commit, 40 lowercase hex characters |
| `mixins` | string, optional | a horizon-mixins commit, 40 lowercase hex characters; when mixins are used it must match the pinned SDK. If omitted, `build` writes the SDK's revision into the package. See [The pin](sdk/03-in-a-project.md#the-pin). |
| `serves_content` | bool, optional | default `false`; opts this native runtime mod in to serving content packages that name it in top-level `served_by` |
| `module` | array of tables | optional support modules, written as `[[native.module]]` |

A module has a unique, nonempty `id` (shared across `[[native.*]]` rows), a `file` naming the game source
file that receives it, a `name` that is a Rust module identifier, and a `source` payload file under
`src/`, holding the module's items without an outer wrapper. An optional `visibility` is exactly
`"pub"` or `"pub(crate)"`, the default. See [Support modules](../concepts/native-route/5-support-modules.md).

Native source edits use `[[native.mixin]]` with `id`, `target`, uppercase `kind`, `helper` and the
other horizon-mixins options ([Mixins](../concepts/native-route/2-mixins.md)). This package reads only `id`, `target`, `kind` and `helper` of a row (and refuses `hook` and `hook_version`) and forwards every other key to the tool as written; the tool then refuses a key it does not know, so an unknown key on a mixin row fails at build time, not when the manifest is read. The same holds for the keys of `[[native.bridge]]` and `[[native.state]]` rows. An `OVERWRITE` row must carry the tool's `fingerprint`. `[[native.loop]]`, `[[native.bridge]]` and `[[native.module]]` complete the route; `[[native.state]]` is refused. See [The native route](../concepts/native-route.md) for every table, its fields and its refusals.

### `[runtime]`: data files beside the code

A native mod whose code runs a large precompiled artefact (a script bundle, an interpreter image,
a baked lookup table) puts the bytes in a `runtime/` folder and lists each file with its SHA-512:

```toml
[runtime]
"tables/nav.bin" = "<128 lowercase hex characters>"
"image" = "<128 lowercase hex characters>"
```

Each key is a plain relative path below `runtime/` (the rules of every package path: no `..`, no backslash
or colon, no symlink). Any file type is allowed; a runtime file is data, never loaded as code by the game.
Runtime files are not pack data: no datapack or resource pack can replace them and no reload touches them.

| Rule | Refusal |
| --- | --- |
| Every file of `runtime/` is listed | `runtime/<name>: not listed in [runtime]` |
| Every listed file exists | `[runtime] lists <name>, but the package has no runtime/<name>` |
| Each file matches its SHA-512 | `runtime/<name> doesn't match its [runtime] sha512` |
| A content mod has none | `a content mod has no code, so no [runtime]` |

The hashes are checked once, when the package opens: at install, at a dedicated server's or a game's start
(also for a folder edited by hand), and by `native-build` and `cargo sunrise-mod` packing. The package is
refused by name; nothing else is skipped.

Top-level `[dependencies]` declares other mods by ID: a semver requirement string, or a table:

```toml
[dependencies]
base = { version = "^1", optional = false, side = "both", order = "after", reason = "Provides the shared API", url = "https://example.org/base" }
```

Versions refer to `sunrise-mod.toml`, not Cargo metadata. `optional` defaults to false;
`side` defaults to `"both"`; `order` is `"after"` (default), `"before"` or `"none"`.
After means the dependency's listeners/content precede the owner's; before reverses that
**dispatch** edge; none removes it. `reason` is a string, and `url` must start with
`https://`. Unknown fields and malformed values are reported when the manifest is parsed;
there is no `kind` key or alternate optional spelling.

Side is the **installation**, not the logical side executing a hook: an integrated server
belongs to the client installation, a dedicated server to the server installation.
Inactive declarations are excluded from code dependency selection at build time and from logical
missing/version/order checks at startup. A missing required dependency, incompatible installed
version or dispatch cycle refuses loading; an absent optional dependency is allowed. Content and
native manifests share the installed-set rule resolver.

**Physical linking is separate from dispatch policy.** Native builds wire only supplied code
packages; a content-only target has no Cargo alias. At startup, the loader and compiled package
inventory enforce physical requirements. Dispatch order, side filters and instance overrides cannot
remove those requirements or bypass ABI and build identity checks.
See [Dependencies between mods](../concepts/dependencies.md).

Top-level `served_by = "<native_mod_id>"` lets a content package delegate code-side namespace authority to an opting-in native runtime mod (for example a bytecode or script host that compiles once and serves generated `.smod` packages). Both sides opt in:

- The content package sets top-level `served_by = "<native_mod_id>"` (refused on `[native]` packages, self-references, invalid IDs, and reserved namespaces or mod-site IDs).
- The native host sets `serves_content = true` under `[native]`.
- At load, the target must be an installed `[native]` mod with `serves_content = true`, its `side` must cover the content package's `side` (`side = "both"` content requires a `side = "both"` host), and the content layer orders after its host. `dependency-overrides.toml` cannot drop this requirement.
- `PackageInfo::namespaces()` stays each package's own namespaces (keeping namespace collision checks intact), while `PackageInfo::served_by()` exposes the host ID. Every namespace-owned native registration—blocks and items, block entity types, fluids, general entity types, attributes, custom stats, game rules, recipe types and serializers, recipe book types, menu recipe books, menu screen dispatch, recipe property sets, render pipelines, armour models, arm poses, attachment types, biome modifiers, datapack registries, client dynamic textures (still bounded by the calling host's per-mod budget), and runtime levels—together with mob-effect tick ownership (`MobEffectPart`), delegates uniformly to the serving host through the pre-resolved namespace authority. Only filesystem paths keyed strictly by package ID (such as `config/<mod_id>.toml`) stay keyed to the host's own ID. Client-required declarations (blocks, items, block entity types, world fluids, general entity types, attributes, custom stats, synced attachments, datapack registries, or required client-bound payload channels) into a `side = "server"` served package's namespace are refused even when the host itself is `side = "both"`. Reserved namespaces (`minecraft`, `c`, `sunrise`) stay unclaimable, and ordinary mods see no change.
- **Host-runtime trust contract:** setting `serves_content = true` allows any installed content package to name the host in `served_by`. The loader verifies package syntax, opt-in, side coverage, and load order, not provenance; the host runtime is responsible for checking which `served_by` packages in `context.mods()` it actually generated or understands before registering parts or declarations for their namespaces.
- A served content package failing to load (missing jar, unreadable file, or content refusal) refuses startup so host code never runs without its served data. Toggling either the served package or its serving host on the Mods screen requires a restart; when a serving host is disabled at startup, its served packages are withheld (with a Mods-screen note naming the disabled host) rather than loaded without it.

Top-level `[properties]` holds free-form values other mods read: strings, integers, floats and booleans only.
Unsupported values, such as arrays or tables, are refused when the manifest is parsed. The complete
`sunrise-mod.toml` is limited to 32 MiB; there is no separate limit on property count, property size or
reason length. A mod reads any loaded package's manifest through
`context.mods().get(id)`: `PackageInfo` gives `name`, `description`, `authors`, `license`, `homepage`,
`issues`, `icon_path`, `kind`, `served_by`, `dependencies` and `property(key)`. Each read-only
`PackageDependency` exposes `id()`, `version()`, `kind()`, `order()`, `side()`, `reason()` and
`url()`. `DependencyKind` is `Required`, `Optional`, `Incompatible`, `Discouraged` or `LoadBefore`;
`DependencyOrder` is `After`, `Before` or `None` and still reads and writes the lowercase manifest
strings through Serde. The package metadata has no Wire encoding of its own. To send a fact from
it, copy just the fields your protocol needs into your own Wire record. The declarations include
inactive sides and do not reflect the effective override-rewritten graph. `LoadBefore` is
ordering-only: its version `*` is a marker, order is `Before` and side
is `Both`, not an API library declaration. The metadata is the same on client and server (NeoForge's
`ModContainer.getModInfo`, `IModInfo` and `getModProperties`).

Load rules name other mods without linking against them (NeoForge's `INCOMPATIBLE`, `DISCOURAGED` and
`ordering = "BEFORE"`). The game checks them alongside ordinary dependencies for the selected content and loaded code:

```toml
load_before = ["minimap"]            # their hooks run after ours when they are loaded

[incompatible]
old_lib = "<2"                       # refuse to start beside old_lib below 2.0

[discouraged]
shaders = { version = "*", reason = "Flickers with the water shader", url = "https://example.org/w", side = "client" }
```

An `[incompatible]` or `[discouraged]` entry is a semver requirement on the other mod's version, or a
table with `version` (every version when left out), `reason` (a string), `url` (`https://`
only) and `side` (`"both"`, the default, `"client"` or `"server"`, the dedicated server). A rule
about a mod that isn't loaded holds nothing. A matching incompatible mod refuses startup with the
owner's reason and link; a matching discouraged mod loads with a warning in the log and on
both affected installed rows in the Mods page. Every notice remains visible in the scrollable
panel, even when a row is off or failed; registration notices are retained too.

A player can override a rule in `sunrise-mods/dependency-overrides.toml`: a table per mod that owns
the rule, mapping the other mod's ID to `"drop"` (remove its logical validation/dispatch edge)
or `"discourage"` (warn instead of refusing). This also applies to ordinary required/optional
manifest dependencies; the physical code-library requirements above remain enforced. Every
applied override is logged and shown on both affected installed Mods rows. Missing files mean
no overrides; malformed files refuse loading with their path in the diagnostic. Owner and target
IDs use the same validated ID syntax as manifests.

The game reads this file from its game directory. Native builds read the policy and selected
content from `sunrise-mods` beside the source folder (the normal source folder is
`sunrise-native-mods`), and apply the target installation side. A standalone server export
has a separate runtime instance: its operator supplies that instance's policy if overrides
are needed at runtime; source-instance policy is not an implicit global setting.

```toml
[my_mod]
old_lib = "discourage"
```

## Original Java jar layers

Content and code mods can list ordered `[[jar]]` references. Use
[`cargo sunrise-mod use-jar`](../how-to/content-mods.md#java-jars-as-external-assets) to record the
original `id`, `version` and exact file's `sha512`, without copying it into the mod.

**Pack order.** From bottom to top, assets use vanilla, then each mod's jars in
`[[jar]]` order, then that mod's own files, with later mods above earlier ones and
player resource packs above all mods. The mod's own files win over its jars for
the same path (the log lists each duplicate asset path once at load). Files every
resource pack stacks (atlases, lang, sounds, blockstates) merge in that same order
through the normal pack merger.

Data uses vanilla, then each mod's jars in `[[jar]]` order and that mod's own
`data/`, then the world's enabled datapacks. Later mods override earlier ones,
tags merge with their normal replacement rules, and a later recipe replaces an
earlier recipe at the same id. A jar may use any namespace. Only its passive
`assets/` and `data/` participate: Java classes and the jar's own pack metadata are
not loaded.

The game reads exact originals from `<game home>/jars/<sha512>.jar`; no bytes are copied into a mod.
For development, [`cargo sunrise-mod run`](../how-to/content-mods.md#java-jars-as-external-assets) checks and
stages these files in its own game home. The ZIP readers write only the decoded resource cache.
The complete ordered reference list,
including version, contributes to the composed root's stamp, so changing it
rebuilds on the next resource load. Server `/reload` uses the accepted inputs of
that composed root, not another package scan.

**Unavailable originals.** `cargo sunrise-mod run` refuses before starting the game and names the mod,
the original jar and its expected cache path. Direct game resource or world loading with a missing native
requirement fails as a whole; it never publishes code without the matching data.
A missing input during `/reload` reports the typed failure to the command source
and keeps the previous data and composed root. Content-only mods have no code to
strand: each whole mod can instead be refused, with the reason in its Mods screen panel, including
every owner of a shared missing file. The game never downloads jars.

## Source paths

A code package contains root `Cargo.toml`, optional `Cargo.lock` and `build.rs`, `.rs` and `.toml` files under `src/`,
and root text files named `README*`, `LICENSE*`, `LICENCE*`, `COPYING*` or `NOTICE*`.
Your root `impl Hooks` uses `#[mod_api::sunrise_mod]`; the attribute generates the factory
([Writing hooks](../start/writing-hooks.md)).

Folder packaging selects source and resource files, excluding `.git/` and `target/`. It validates selected
files before copying them. Whole archives are checked, not treated as source folders with ignored extra entries.

## Content paths and icon

`assets/` or `data/`, then at least two parts of `[a-z0-9_.-]` (vanilla's resource-location characters).
Code and content use the same top-level resource trees; there is no separate native data folder.

`icon.png` is optional for both kinds. It has no manifest key. The package reader fully validates it once:
a PNG, nonzero square dimensions, at most 512 × 512 pixels and 256 KiB encoded. The launcher shows it in
the mod list. The scaffold does not create a default icon.

## Limits

| What | Limit |
| --- | --- |
| A registry file, as downloaded | 64 MiB |
| Selected native source files, excluding the manifest, icon and resources | 65,536 files, 64 MiB unpacked in total |
| Content files | 65,536 files, 16 MiB each, 512 MiB in total |
| `sunrise-mod.toml` | 32 MiB |
| `icon.png` | 256 KiB encoded, 512 × 512 pixels |
