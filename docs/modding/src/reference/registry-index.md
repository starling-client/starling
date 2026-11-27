# The registry index and `.spack`

This page documents the old registry's schema-1 `index.json` and the `.spack` modpack format. The
[registry repository's CI](https://github.com/sunrise-org/sunrise-mods/actions) builds the old index with
its pinned tool for versions of Sunrise that still use it. To release a content or native mod, follow
[Publishing your mod](../how-to/publish.md) instead.

Releases made that way are listed in the registry's schema-2 index,
`https://sunrise-org.github.io/sunrise-mods/v2/index.signed`: the four lines of its minisign signature, then the
index's exact bytes, signed by the key in the registry's [`registry/index-key.pub`](https://github.com/sunrise-org/sunrise-mods/blob/main/registry/index-key.pub).
Its source files and fields are specified in the registry's
[`docs/registry-v2.md`](https://github.com/sunrise-org/sunrise-mods/blob/main/docs/registry-v2.md).

The old index lists content and native mods (`.smod`), modpacks (`.spack`), resource packs, data packs,
shaders-folder packs and Sunrise shader packs (`.sshader`). Reviewed versions are pinned by SHA-256;
the format also supports unreviewed Nightly source listings. The formats are defined by
`sunrise_mod_package::index` and `sunrise_mod_package::modpack`.

## Repository page fields

For the old index, `sunrise-registry.toml` at the repository root of the newest release tag supplies
`name`, `summary`, `description`, `authors` and `license`. Other keys are refused. The registry entry
takes precedence; this file fills fields the entry leaves out.

## `index.json`

```json
{
  "schema": 1,
  "generated": "2026-10-24T18:00:00Z",
  "projects": [{
    "id": "coords_hud", "kind": "mod", "name": "Coordinates HUD", "summary": "One line.",
    "description": "Markdown.", "authors": ["Sunrise"], "owners": ["octocat"],
    "license": "MIT", "source": "https://github.com/...", "icon": "https://...png", "key": "RWT...", "downloads": 1234, "stars": 56, "updated": "2026-10-24",
    "categories": ["utility"], "tags": ["hud"], "screenshots": ["https://...png"],
    "versions": [{
      "version": "0.1.0", "published": "2026-10-24", "changelog": "",
      "api": "0.2", "side": "client", "capabilities": ["registries"], "game": "26.3", "sunrise": "26.3-0.1.0",
      "dependencies": [{ "id": "jei", "version": "^0.5", "optional": true }],
      "file": { "name": "coords_hud-0.1.0.smod", "url": "https://github.com/sunrise-org/sunrise-mods/releases/download/...",
                "size": 438, "sha256": "e113...", "signature": "untrusted comment: ...\n...\ntrusted comment: ...\n...\n" }
    }]
  }],
  "revoked": [{ "id": "bad_mod", "sha256": "1111...", "reason": "Sent chat messages it didn't declare.", "date": "2026-10-24" }]
}
```

| Field | Meaning |
| --- | --- |
| `schema` | 1. A reader refuses a higher number with "update Sunrise" rather than misread it. |
| `projects[].id` | `[a-z0-9_]{1,64}`, unique among projects and collections; a mod's is its manifest's `id`. Not a page of the mod site (`RESERVED_IDS` in `crates/sunrise-mod-package/src/manifest.rs`, `submit` and `mine` among them): a project's page is `/mods/<id>`. |
| `releases` | Optional top-level list of stable Sunrise releases: `{ tag, commit, line }`. `line` is `<game>-<major>`, e.g. `26.3-0`. |
| `nightly_build` | Optional current game Nightly: `{ commit, date }`, separate from each project's `nightly` source. `date` is `YYYY-MM-DD` (UTC). |
| `unjudged` | Optional top-level list of builds skipped during [fit checks](#fit-on-stable-and-nightly): `{ build, reason }`, where `build` is a stable tag or `nightly`. |
| `collections` | Optional top-level list of [collections](#collections). Each is a page and a list of already listed mod ids, with no file or version of its own. |
| `kind` | `mod` (content), `native-mod`, `modpack`, `resourcepack`, `shader` (Sunrise `.sshader`), `shaderpack` (a `shaders/`-folder zip), `datapack` (a data-pack zip). Other kinds (worlds, themes) may come without a new schema: a reader keeps a kind it doesn't know and doesn't offer it. |
| `summary`, `description` | Required summary of at most 160 characters for lists. Markdown for the project's page: required for a project with versions (50–10,000 characters after trimming), optional for a Nightly-only project. |
| `owners` | GitHub logins allowed to publish updates. |
| `license`, `source` | An SPDX expression and an `https://` address of the source code: required for a project with versions. A Nightly-only project can omit both; in that case the publisher uses the Nightly repository's URL as `source`. |
| `icon` | Required for a project with versions. An HTTPS URL in the generated index, from the registry's `icons/<id>.png` (not a remote image input). |
| `categories` | 1–3 from the kind’s category list for a project with versions. Optional for a Nightly-only project. |
| `tags` | Up to 10 lowercase tags. |
| `screenshots` | Up to 8 HTTPS PNG or JPEG URLs in the generated index, from files kept in the registry under `screenshots/<id>/`. A new project with versions, or an existing one whose pull request adds a version, needs at least one unless it is a `mod` or `native-mod` whose `categories` include `library`. Collections and Nightly-only projects need none. The file must be a PNG or JPEG kept in the checkout and listed here, not a remote address. |
| `key` | The author's [minisign](https://jedisct1.github.io/minisign/) public key. When set, every version must be signed with it; an update signed with another key is refused. Absent: the project is unsigned and the launcher says so. |
| `versions` | Newest first. May be empty for a Nightly-only project. |
| `nightly` | Optional project-level object: `repo` (`owner/name` on GitHub), optional `path` (a directory relative to the repository root), plus optional publisher-supplied `branch` (default branch), `commit` (its head), `pin` (that source's `[native] sunrise`), and `updated` (head commit date, RFC 3339). |
| `downloads` | How many times its files were downloaded, every version together: GitHub's count of each release asset, summed across release assets. Absent when no file has been downloaded. Read-only: never in an entry. The registry's publish workflow reads download and star counts from GitHub each time it republishes the index. |
| `stars` | GitHub stars of the project's source repository, refreshed by the registry. Absent when zero or the source is not on GitHub. Read-only: never in an entry. The launcher shows the count and a button that opens the repository. |
| `updated` | Its newest version's `published` (the latest of `versions[].published`), set by the registry, so a reader can sort by it beside `downloads` without walking every version. Absent when no version has a date. Read-only: never in an entry. |
| `api` | A mod's API line (its manifest's `api`); a native mod's pinned revisions (`native-1/<sunrise>/<mixins>`, without `/<mixins>` when the package pins none); a modpack's manifest format (`1`). |
| `side`, `capabilities` | A mod's, read from its manifest during validation. |
| `game` | The Minecraft version it is for. |
| `sunrise` | Optional version-level stable release tag the package was made for, e.g. `26.3-0.1.0`. |
| `verdicts` | Optional version-level [fit decisions](#fit-on-stable-and-nightly), keyed by stable release tag or `nightly`. Only content and native mods with `sunrise` get these decisions. |
| `dependencies` | A mod or native mod's [manifest dependencies](../concepts/dependencies.md): `{ id, version, optional, side }`, sorted by `id`. `version` is a SemVer range rendered as text. `optional` is omitted unless true; `side` is omitted unless limited to `client` or `server`. The publisher omits this field when a mod has no dependencies. An explicit `[]` asserts that the manifest has none. Never add this field to `mods/<id>.toml`. |
| `requires` | A modpack's exact pins: `{ id, version, sha256 }`, each a version in this index with that hash. These are not mod dependencies. |
| `file` | A plain name with the kind's extension (`.smod`, `.spack`, `.zip`, `.sshader`), an `https://` URL (the registry's release `https://github.com/sunrise-org/sunrise-mods/releases/download/<id>-<version>/<name>`, or an asset of the project's own repository, `https://github.com/<owner>/<repo>/releases/download/<tag>/<name>`: the launcher accepts exactly these shapes), the size (at most 64 MiB), the lowercase hex SHA-256, and the `.minisig` text when the project has a key. |
| `revoked` | Files taken down: one file by `sha256`, or every version of `id` without it. The launcher refuses to install them and turns installed copies off with the reason. A takedown of every version also stops listing the project's Nightly source; one by `sha256` leaves it. |

Readers ignore fields they don't know, so an optional field can be added to schema 1. Renaming or removing a field, or
changing what one means, makes schema 2.

The registry's CI checks the whole index. A reader leaves out individual entries that break file or metadata rules:
a project left with neither versions nor a Nightly source goes too, a modpack version whose mod was left out goes
with it, and an id listed twice
leaves out every project with that id. Dependency failures are diagnosed when planning an install, so the reader can
tell you which mod is missing or why it cannot be installed. The rest of the index still loads; only text that is not
an index, or a newer schema, fails as a whole.

A native version can list `native_binaries`: `{ build, target, file }` for each verified archive in its
[stable build rows](#stable-native-builds). `build` is the exact SDK commit. `target` is the SDK host name:
the Rust target triple with `-server` appended for a dedicated server slice. `file` uses the index's
existing name, URL, size, SHA-256 and signature fields. Its signature is the author's minisign signature
of the archive, verified with the same project key as the source package. There is no separate `side` field.

Build/target pairs are unique. A reader ignores bad optional binary records and keeps the valid `.smod`
version, with a named refusal for an invalid exact build/host pair. It never substitutes another build
or host. Reading the index does not download or run an archive.

## Fit on Stable and Nightly

The index builder judges content and native mods that declare `sunrise` against every listed release on that
release's line, including earlier ones, and against the current Nightly if supplied. Other kinds, including shader
packs, have independent requirements and do not get verdicts. Each decision is an object with `verdict` and
`targets`, the sorted SDK host names with `native_binaries` for that exact build's commit:

- `made-for`: the target commit is the one the version was made for and its build requirements match.
- `fits`: the target uses the same Minecraft version and manifest API, and everything the package uses is unchanged.
  For native mods, the comparison in [`cargo sunrise-mod check --build`](sdk/04-fit.md) checks the API and declared game code.
- `rebuild`: does not fit. Only the bodies of declared game functions changed, and the uses list and target
  SDK both have separate declaration and body digests to prove it. `review` lists their paths so the author can
  check whether a patch overrides a game fix. The version is not listed for this build or rebuilt automatically.
- `breaks`: an API item or declared game code changed, a used item disappeared, or a build requirement no longer
  matches. `changed` names up to five items, including `minecraft` or `content-api` for those requirements. A
  function change names the path and says `body changed` or `declaration changed` when both digests are available;
  with older evidence it reports only that the item changed.
- `exact`: compatibility could not be judged, for example because the package has `[[native.module]]` or
  `[[native.bridge]]` rows, no uses list, or a comparison deferred to compilation. The `reason` says why. On another build Stable lists it only when a
  prebuilt binary exists for that build.

The workflow must authenticate `sdk.json` with the release signature and verify `build-data.tar.xz` against that
index before unpacking it. For each build with at least one version to judge, the workflow supplies
`sdk.json`, `surface.json`, and the unpacked `build-data/{functions,internals,loops}.json` under that commit's
SDK data directory. If no content or native mod version declares `sunrise`, no SDK files are needed.
The current Nightly's build record (`commit` and `date`) is separate from per-project source metadata.

The index builder verifies `surface.json` against the supplied SDK index and checks the commit in each build-data file.
It does not authenticate the index or the unpacked files, fetch SDK data, or require the SDK's compiled
`package_format` to match: fit checks never open the compiled packages. Missing or malformed files, a
surface checksum mismatch, or a file naming the wrong commit fail the build and name the release. An unsupported
surface or build-data schema instead produces one warning per build commit. No version gets a verdict for that
build; the index lists each affected stable tag or `nightly` in `unjudged` with the reason, and publishes the
verdicts for other builds.

## A native mod's list of what it uses

A native mod file carries `uses.json`, the list of what the mod uses, made for the build the mod pins; with it a
newer build can tell whether the mod still fits ([SDK fit](sdk/04-fit.md)). `cargo sunrise-mod build` writes the list
when it can read the mod's source against the pinned SDK, and says what stops it when it cannot.

For old index submissions, CI refuses a native mod file with no list, or with one made for a different build
than the mod pins, when its hash is new in the pull request. A build append verifies the original reviewed source
without requiring a new list. Index publication and installs do not apply this check, so already listed files stay
as they are. A mod with a `[[native.module]]` or `[[native.bridge]]` row is outside this rule: no list can describe
those rows, so its file needs none.

## Dependency checks

`cargo sunrise-mod check <file>` prints the manifest's required and optional dependencies. For the old index, CI
reports those facts for new files. It pins each newly submitted mod version and runs the install planner against
the proposed index for a client and a dedicated server on that version's Minecraft release. A taken-down file
isn't a candidate. The publisher assumes native SDK fit because it doesn't know each player's Sunrise build.

The index checks the dependency rows themselves, not whether their requirements can still be satisfied. Its builder
warns about listed versions that no longer resolve without blocking publication. If a takedown breaks a requirement,
existing listings do not fail the check.

- **The registry does not list it:** list the required mod first, or mark it optional if the dependent actually
  works without it. Optional dependencies can name unlisted mods.
- **No listed version matches:** list a version in range, or correct the range in `sunrise-mod.toml` and rebuild the mod.
- **The range excludes the registry's prereleases:** a range such as `^3.6` won't select `3.6.30-sunrise.1`.
  Use a range that names the prerelease, such as `=3.6.30-sunrise.1`.
- **Required dependency cycle:** read the named chain of versions and break a required link in the manifests.
  Optional links and links that never apply on the same installation do not create a required cycle.

For a new takedown, the review check reports required ranges the taken-down files satisfied, so reviewers can
check whether a usable alternative remains. A takedown may stay listed for review and history; it is not a
candidate for installation.

## Planning installs and removals

`sunrise_mod_package::index::resolve` plans an install without downloading or writing anything:

1. Start with the requested mod ids and any exact versions requested. Follow required dependencies on this
   installation (client or dedicated server). Never install a target just because it is optional. If the target
   is already installed or selected, it must match the applicable optional ranges too.
2. Use one version of each mod for the whole group. Keep an eligible installed version that satisfies the active
   ranges and exact requests. An installed artifact inherits index data only when its id, version and SHA-256 match
   the indexed file. A local copy with a different or unknown hash can be kept in range without index data, unless
   the entire project was taken down. The game loader checks that copy's dependencies; the caller checks takedowns
   against the actual installed file's hash.
3. Otherwise choose the newest fitting version by SemVer, with index order breaking ties. A version must fit the
   Minecraft release, request only known capabilities and not be taken down. For a native mod, the caller also
   supplies eligibility for each version: `native-1/<sunrise>/<mixins>` says what it was made for, not whether
   the used SDK surface is unchanged on this build. Try another eligible version if a later range rules out a
   choice. Do not change a dependency to a version that breaks an installed mod's known range.
4. Try the entire batch against the final versions. A replacement can come from a request or from another mod's
   required dependencies. If the whole batch cannot install, group requests whose selections share a mod or activate
   a dependency constraint across requests, and check each group together. An installed version can block a request
   on its own but not as part of a group that replaces that version; a different candidate version may need a
   different replacement. If a group still fails, retry by omitting requests blocked by installed versions, trying
   larger subgroups first. If the retry search hits its limit, the unresolved requests report that limit and the
   previously validated groups can still be planned separately. A plan gives each request's outcome and version or
   refusal reason, lists what to install, keep or update, and identifies which requests need each dependency. Lists
   are ordered by id.
   An update with `downgrade: true` moves to a lower SemVer; the installer names that downgrade in its status line
   (for example, “Downgraded Sunrise Creatures from 0.2.0 to 0.1.3”).
5. Before removing a mod, use its id and the installed artifacts to find the known installed mods that require
   it directly or through another mod. Optional dependencies do not block removal.

Cross-installer cases are in `crates/sunrise-mod-package/tests/registry-resolution/` in the Sunrise repository.
Each JSON fixture has an index and cases with requests, installed artifacts (`id`, `version`, optional `sha256`),
per-version native fit decisions, and an expected plan or refusal details.

## Before a file is installed

`index::check_file` runs on the downloaded file, before it reaches the game folder:

1. Its size and SHA-256 are the ones the entry lists, and it isn't revoked.
2. With a project key: its signature verifies with that key (minisign's prehashed and legacy signatures both).
3. What the file says matches the entry: a mod's `sunrise-mod.toml` (id, version, API, side, capabilities, and
   dependencies when listed) and its content's fence ([making a content mod](../how-to/content-mods.md#content-mods-api-02)); for code, the same manifest's `[native]`
   (game, the exact Sunrise and horizon-mixins revisions) and checked root Cargo project; a modpack's manifest (id, version, format, game,
   mods); a resource pack's `pack.mcmeta`; a shader pack's `sunrise_shader_api::check` and manifest (id, version,
   shader API).
4. A kind this build can't install is refused with "this Sunrise can't install a … yet".

An absent `dependencies` field makes no claim about the package's dependencies: an older schema 1 index may omit
it. The hash still pins the file; the game loader checks the manifest. When the index lists both the version's
`sunrise` tag and the release table, a native package's `[native] sunrise` pin must equal that release's commit.

## Collections

A collection is one publisher's set of listed mods under one name and icon. How to make one:
[Group your mods into a collection](../how-to/collections.md). Its files are described in the registry's
[reference](https://github.com/sunrise-org/sunrise-mods/blob/main/docs/registry-v2.md).

## `.spack`

A zip holding `sunrise-modpack.toml` and, optionally, `overrides/`. The mods are not inside: the launcher downloads
each from the index and checks it as above.

```toml
format = 1
id = "cozy"                 # [a-z0-9_]{1,64}: the profile's folder
name = "Cozy"
version = "1.0.0"
description = "Coordinates and a calm render distance."
authors = ["Sunrise"]
game = "26.3"

[[mods]]
id = "coords_hud"
version = "0.1.0"
sha256 = "cb6e658445269b2a4d2eb104e83e4b993586bc2e56814fd56454244139f08364"
on = true                   # optional, default true: turned on in the profile's sunrise-mods.txt
```

Unknown keys are refused. `overrides/` may hold only `options.txt`, `sunrise-graphics.txt`, `sunrise-extras.txt`,
`resourcepacks/<name>.zip` and `sunrise-mods/.sunrise-mod-data/<a listed mod>/...` (a mod's own saved settings,
a native mod's saved settings), each at most 64 MiB and 256 MiB in all; anything else refuses the pack. The launcher copies
them into the profile's game folder, which may itself be a link; a destination under it that is a link, or
sits in a linked folder, refuses the import before anything is written.

## Old index project entries

The old index is built from `mods/<id>.toml` files in the registry repository. A project with versions needs:

- A description of 50–10,000 characters after trimming.
- 1–3 categories from your project kind’s list and up to 10 lowercase tags.
- A square PNG icon, 64–1024 pixels and at most 256 KiB, stored in the registry as `icons/<id>.png`.
- 1–8 non-empty PNG or JPEG screenshots stored in the registry as `screenshots/<id>/<name>.png|.jpg`.
  CI requires screenshots for a new project with versions or an existing project that adds a version.
  Collections and `mod` or `native-mod` entries in the `library` category do not need screenshots.

The generated index supplies HTTPS URLs for images. Unknown entry keys are refused. A project’s kind cannot change.
If you set a minisign key, sign every version with it. The registry takes no
new entry that follows a repository's releases (`repo = ...`): every version is listed as its own file.

### Adding a signing key to an unsigned entry

An existing unsigned entry that still has unsigned version rows does not take a `key` as it is.
To switch an unsigned listing to signed releases, keep the existing project entry, edit its `key`, remove its unsigned legacy version rows, and add the new signed row in your registry
checkout. Only unsigned legacy rows without `sunrise` can be removed without a takedown (review notes each removed
row while the project entry stays).

### The entry

```toml
id = "coords_hud"
kind = "mod"                  # mod, native-mod, modpack, resourcepack, shader, shaderpack, datapack
name = "Coordinates HUD"
summary = "Shows where you are."                          # at most 160 characters
description = "Markdown for the project's page, 50 to 10,000 characters."
authors = ["Your Name"]
owners = ["your-github-login"]  # who may publish updates
license = "MIT"                 # an SPDX expression
source = "https://github.com/you/coords_hud"
icon = "icons/coords_hud.png"
categories = ["utility"]        # 1 to 3 from your kind's list
tags = ["hud"]                  # up to 10, lowercase
screenshots = ["screenshots/coords_hud/main.png"]  # 1 to 8, files kept in the registry
key = "RWT..."                 # the project's minisign public key

[[versions]]                    # newest first
version = "0.1.0"
published = "2026-11-16"
game = "26.3"
sunrise = "26.3-0.1.0"
file = "https://github.com/you/coords_hud/releases/download/v0.1.0/coords_hud-0.1.0.smod"
sha256 = "<lowercase hex>"      # of that file
signature = """<the .minisig text>"""
changelog = ""
```

A mod's API, side, capabilities and dependencies are read from the file, not listed. A new version is a new
`[[versions]]` table at the top. Review refuses a change to a listed version's file, hash, signature or source pin;
adding a new binary build does not require another source version.
The category lists are in `crates/sunrise-mod-package/src/index/page.rs`.

CI requires every new or changed version row to declare a stable tag in `sunrise`, such as `26.3-0.1.0`.
The tag matches the Minecraft version in `game`, followed by a hyphen and Sunrise's `X.Y.Z` without prerelease
or build metadata. CI resolves published stable tags from `sunrise-org/sunrise` to their full commits and
verifies the corresponding signed SDKs separately. An index reader leaves out rows whose tags are absent
from the index's release list. When package files are checked, a native package's `[native] sunrise` pin
must match the named commit.

Unchanged legacy rows may still omit `sunrise`. The checks warn about them, and the builder includes them without
a tag. A legacy row may leave its entry without a takedown. For a row published with `sunrise`, the tag cannot
change and removing the row requires a takedown in `revoked.toml` naming the project id and that file's exact
SHA-256. Removing a whole entry is still refused.

An entry with neither versions nor a Nightly source publishes nothing and keeps its id; its id, kind and owners are
still checked. A project with only a Nightly source may leave out its icon and screenshots; the ones it has are
kept in the registry like any other project's.

A `[[versions]]` row with `sunrise` must set `file` to the author's release asset, for example
`https://github.com/you/example/releases/download/v0.1.0/example-0.1.0.smod`. Use the project's own
GitHub repository (`source`, or `repo` if `source` is omitted), one plain tag segment, and the exact
`<id>-<version>.<extension>` asset name: the row's `version`, a full SemVer, and the extension of
the project's kind (`.smod`, `.spack`, `.zip`, `.sshader`). The row's version must agree with the package's
own where the format carries one. A bare filename is refused
on a stable row. The URL is immutable after publication, just like the source pin and digest.
The registry checks the bytes from `files/<sha256>`; the generated index still points to the package
on the registry's `<id>-<version>` mirror. Rows without `sunrise` still use a bare filename.

### Stable native builds

A reviewed standalone native mod's old index entry can contain an author-signed binary build in its
`[[versions]]` row:

```toml
[[versions.builds]]
sunrise = "26.3-0.1.1"
binaries = "https://github.com/you/example/releases/download/v0.1.0/example-0.1.0+26.3-0.1.1.binaries.json"
sha256 = "<lowercase hex of the descriptor>"
signature = """<descriptor .minisig text>"""
```

The source version must be signed and pin a stable release of the same line. Its registry version label
must be the full SemVer from the signed manifest (`1.0.0`, not `1.0`). Older rows without `sunrise` keep their
existing filenames and labels and cannot carry these builds. The package must contain `Cargo.lock` and use the
standalone `prebuilt-api` route: game edits, crates.io libraries and linked mod
dependencies do not produce an independent release library. Builds are optional. Each stable release
gets at most one row, and published rows are append-only. Review reports an owner's append as routine
only after validation and only if the append is the submission's sole registry change. The JSON
report lists these changes under `build_reviews`. That classification covers registry data only. A workflow
that uses it to classify a whole pull request must also check the changed paths and refuse the shortcut
if any change falls outside the append, including changes to the workflow itself.

The descriptor `<id>-<version>+<release>.binaries.json` names `id`, `version`, `build` (the SDK commit),
`rust` (the SDK's Rust release), and `archives`. Every archive record has `target`, `name`, `size`,
`sha256` and `signature` (its `.minisig` text). Its name is `<id>-<version>+<release>-<target>.tar.xz`
and its URL is beside the descriptor in a release of the project's GitHub repository. `source` names that
repository; if `source` is omitted, `repo` supplies it. The archive contains exactly the library and
`identity.json`, both regular root files. `identity.json` has `target`, `file` (the library
name), `sha256` (the library digest) and `identity` (the library's build identity, whose first four fields are `<build> <id> <version> <rust>` and whose last is `unwind`; see the native build's `records`). The same
project key signs the source, every archive, and the descriptor. The descriptor binds each archive's
bytes to the declared project, mod version and SDK host. Validation checks every host the SDK ships,
without running or rebuilding any library. A binary for a package with a known `breaks` verdict is
refused; a verified exact binary can supply a build whose fit verdict is `exact`.

Registry CI fetches the descriptors first, then the archives named by the verified descriptors. It checks the
files from the hash-named store against the release list and SDK metadata. The index points to the verified
archives under the registry's release mirror. A takedown in `revoked.toml` naming an archive's SHA-256 removes
that host from the published index and the files CI requests, without removing the source version, other
hosts, or the signed build declaration.

### Nightly source

An entry may also list a public GitHub repository for unreviewed builds from source:

```toml
[nightly]
repo = "you/coords_hud"
# path = "mods/coords_hud"  # omit when sunrise-mod.toml is at the repository root
```

Only `repo` and `path` are accepted in this table; the entry does not name a branch. With no versions, a Nightly-only
entry needs only `id`, `kind`, `name`, `summary`, `owners` and `[nightly]`. An entry with neither reviewed versions
nor Nightly source keeps the id and owners in the checkout but is not published in the index.

Review reports the addition or edit of just this table by a base owner as routine. A new Nightly-only
entry is routine if the author is in its `owners`. That status classifies the entry edit, not unrelated files or
whether the repository passed the workflow's checks. The JSON report lists these under `nightly_reviews` as
`{ id, status, reason }` (`routine` or `review_required`).

The workflow reads optional display metadata from GitHub for projects with `[nightly]`: the default `branch`,
its `commit` and date (`updated`), and the source's game `pin`. The index builder does not fetch or execute
the repository. `repo` and `path` always come from the entry.

### The files folder

The registry's CI keeps the listed files by hash in `files/<sha256>` (no extension). For a stable source row,
the workflow fetches from the URL in `file`. A row with only a filename predates stable releases: its file is
already in the registry's releases or its `staging` release. Either way, the merged package is published under
`<id>-<version>` with its asset name. CI uses the full checkout and all listed files to plan installations.

For new or changed native mod versions, the review check compares the package's Cargo requirements, including
those of packaged path dependencies, with the pinned SDK's `Cargo.lock`. It warns when a crates.io requirement
excludes every version the pinned game build uses but accepts another version on the same version line as one of
them: the same nonzero major, the same `0.minor` with a nonzero minor, or the same `0.0.patch`. The build refuses
to move the game's pin to
another version on that line. This warning does not rule out a version on a different line that also
satisfies the requirement. For example, `3.0.10` and `=3.0.5` both warn against the game's `3.0.6`; `>=2, <3` does not warn
against `3.1.0`, and `0.0.2` does not warn against `0.0.1`. Loosen the requirement if your mod does not need the other
version. If the game's lock is unavailable, review notes that it skipped
the comparison. A different pin in the mod's own lockfile does not trigger a warning.

To check a GitHub-attested release package on its own, use
[`cargo sunrise-mod check <file.smod>`](sdk/05-commands.md#check-a-released-package).
The command validates package metadata and contents without running the mod's code.
