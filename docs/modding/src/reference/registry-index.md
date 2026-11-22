# The registry index and `.spack`

Use the registry to distribute content and native mods (`.smod`), modpacks (`.spack`),
resource packs, data packs, shaders-folder packs and Sunrise shader packs (`.sshader`).
A native mod's version is written by `cargo sunrise-mod publish --sunrise <stable release tag>`
([Publishing your mod](../how-to/publish.md#publishing-a-stable-source-version)), which checks the package's
`[native] sunrise` pin against that release. For every other kind, a content-only mod included, write the
`[[versions]]` row by hand: `sunrise`, your release asset's URL as `file`, its SHA-256, its signature, and the
project's `key` on the entry ([The entry](#the-entry)).
Reviewed package versions are pinned by SHA-256. The registry can also list unreviewed source for
Nightly builds. The launcher and website read schema 1 of `index.json`.
The format is defined by `sunrise_mod_package::index` and `sunrise_mod_package::modpack`.

## Repository page fields

Place `sunrise-registry.toml` at your repository root in the newest release tag
to supply `name`, `summary`, `description`, `authors` and `license`. Other keys
are refused. The registry entry takes precedence; this file fills fields the
entry leaves out.

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
| `downloads` | How many times its files were downloaded, every version together: GitHub's count of each release asset, summed across release assets. Absent when no file has been downloaded. Read-only: never in an entry. The registry's publish workflow reads the counts from GitHub and hands them to `cargo sunrise-mod registry build --counts <file>` (`{"downloads": {"<file address>": n}, "stars": {"<owner>/<repo>": n}}`), so both refresh each time the index is republished. |
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

`registry build` judges content and native mods that declare `sunrise` against every listed release on that
release's line, including earlier ones, and against the current Nightly if supplied. Other kinds, including shader
packs, have independent requirements and do not get verdicts. Each decision is an object with `verdict` and
`targets`, the sorted SDK host names with `native_binaries` for that exact build's commit:

- `made-for`: the target commit is the one the version was made for and its build requirements match.
- `fits`: the target uses the same Minecraft version and manifest API, and everything the package uses is unchanged.
  For native mods, the comparison in [`cargo sunrise-mod fits`](sdk/04-fit.md) checks the API and declared game code.
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
`--sdk-data <dir>` with `sdk.json`, `surface.json`, and the unpacked `build-data/{functions,internals,loops}.json`
under `<dir>/<commit>/`. If no content or native mod version declares `sunrise`, no SDK files are needed.
`--nightly-build <file>` takes JSON `{ "commit": "<40 hex>", "date": "YYYY-MM-DD" }`; it does not replace `--nightly`,
which supplies per-project source metadata.

`registry build` verifies `surface.json` against the supplied index and checks the commit in each build-data file.
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

`cargo sunrise-mod check <file>` refuses a native mod file with no list, or with one made for another build than the
mod pins. `publish` checks new source submissions; on a build append it verifies the original reviewed source
and checks fit without requiring a new list for that source. `registry release` checks new files, and `registry check`
applies the rule to files whose hash is new in the pull request. `registry build` and installs do not apply it,
so files already listed stay as they are. A mod with a `[[native.module]]` or `[[native.bridge]]` row is outside
this rule: no list can describe those rows, so its file needs none.

## Dependency checks

`cargo sunrise-mod check <file>` prints the manifest's required and optional dependencies. `registry check` reports
those facts for new files. It pins each newly submitted mod version and runs the install planner against the proposed
index for a client and a dedicated server on that version's Minecraft release. A taken-down file isn't a candidate.
The publisher assumes native SDK fit because it doesn't know each player's Sunrise build. Both check commands offer
`--json` for machine-readable reports.

The index checks the dependency rows themselves, not whether their requirements can still be satisfied. `registry build`
warns about listed versions that no longer resolve without blocking publication. Existing listings do not fail `registry check`
when a takedown breaks a requirement.

- **The registry does not list it:** list the required mod first, or mark it optional if the dependent actually
  works without it. Optional dependencies can name unlisted mods.
- **No listed version matches:** list a version in range, or correct the range in `sunrise-mod.toml` and rebuild the mod.
- **The range excludes the registry's prereleases:** a range such as `^3.6` won't select `3.6.30-sunrise.1`.
  Use a range that names the prerelease, such as `=3.6.30-sunrise.1`.
- **Required dependency cycle:** read the named chain of versions and break a required link in the manifests.
  Optional links and links that never apply on the same installation do not create a required cycle.

Run `registry review <base> <head> --author <login> --files <dir>` with the head checkout's packages in the same
SHA-256–named file store used by `registry check`. For a new takedown it reports required ranges the taken-down
files satisfied, so reviewers can check whether a usable alternative remains. A takedown may stay listed for
review and history; it is not a candidate for installation.

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

A collection is a named selection of existing `mod` and `native-mod` projects. It has no file or version of its own.
Unlike a `.spack`, it pins nothing: each selected mod is installed, updated and removed on its own. A collection and
project may display the same name, but they cannot share an id, and the registry's reserved ids aren't available.

The generated `collections` list takes each page from `collections/<id>.toml`: `id`, `name`, `summary`, optional
Markdown `description`, GitHub `owners`, `icon` as the published HTTPS address, and ordered `members`. Each member
has `id` and may set `default = false` to start unticked. The registry uses the same title, summary and PNG icon
rules as a project. It requires at least two distinct listed mods and at least one selected by default; the upper
bound is `sunrise_mod_package::index::MAX_COLLECTION_MEMBERS`. A collection cannot name another collection as a
member. Checkout member rows reject unknown keys, so a typo in `default` does not change the selection silently.
An older index without `collections` has no collections. Index readers ignore unknown fields and skip malformed
collection rows with a reason, without discarding the other collections or projects.

An installer submits all selected members to [install planning](#planning-installs-and-removals) together. Required
dependencies are selected even if not on the collection's list; a member needed by another selected mod stays
selected and identifies the mod that requires it. An unfit member is shown with its reason. A collection does not
record where a mod was installed from.

`registry check`, `registry build` and `registry review` refuse unlisted members, including the removal of a project
that remains a member. Review reports which collections list a project with a new takedown. An edit to membership
alone is routine when the pull request's author owns the collection and every added or removed project. Review
matches the author's GitHub login against the base checkout's `owners`, as it does for project updates. See
[publishing a collection](../how-to/publish.md#a-collection).

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

## Submitting a project

A project is a file `mods/<id>.toml` in [sunrise-org/sunrise-mods](https://github.com/sunrise-org/sunrise-mods),
added by a pull request that a person reviews. Write it with `cargo sunrise-mod publish` and open the pull request
yourself; see [Publishing your mod](../how-to/publish.md) for the steps. A project with versions needs:

- A description of 50–10,000 characters after trimming.
- 1–3 categories from your project kind’s list and up to 10 lowercase tags.
- A square PNG icon, 64–1024 pixels and at most 256 KiB, stored in the registry as `icons/<id>.png`.
- 1–8 PNG or JPEG screenshots stored in the registry as `screenshots/<id>/<name>.png|.jpg`. `registry review` refuses a new project with none, and a pull request that adds a version to a project that still has none. Existing listings the pull request leaves without a new version stay valid, and collections need none. `registry check` and `registry review` apply the same rule, and the listed file must be a non-empty PNG or JPEG in the checkout. A `mod` or `native-mod` in the `library` category needs no screenshot: it does not take a placeholder image.

The generated index supplies HTTPS URLs for images. Unknown entry keys are refused. A project’s kind cannot change.
If you set a minisign key, sign every version with it. The registry takes no new entry that follows a repository's
releases (`repo = ...`): every version is listed as its own file.

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
`[[versions]]` table at the top. `registry review` refuses a change to a listed version's file, hash, signature
or source pin; adding a new binary build does not require another source version.
The category lists are in `crates/sunrise-mod-package/src/index/page.rs`.

`registry review` requires every new or changed version row to declare `sunrise = "26.3-0.1.0"`. The tag is
the Minecraft version (dotted numbers, matching `game`), a hyphen, and Sunrise's `X.Y.Z` without prerelease or
build metadata. `registry check`, `registry review` and `registry build` accept `--releases <file>`: a JSON array
of objects with `tag` and `commit` (40 hex digits, published as lowercase), one per stable release. A declared tag
must be in the list; the commands refuse it when no list is given. An index reader leaves out rows whose tags
are absent from the releases listed in that index. When package files are checked, a native package's
`[native] sunrise` pin must match the named commit.

Unchanged legacy rows may still omit `sunrise`. `registry check` warns for each such row, and `registry build`
still includes it without a stable release tag. `registry review` lets a legacy row leave its entry without a
takedown. For a row published with `sunrise`, the tag cannot change and removing the row requires a takedown
in `revoked.toml` naming the project id and that file's exact SHA-256. Removing a whole entry is still refused.

`cargo sunrise-mod registry releases --out releases.json` discovers published, non-draft stable releases
from `sunrise-org/sunrise` (no prereleases), resolves lightweight and annotated tags to full commits, and
sorts the rows by tag. A failed listing or tag lookup leaves the output unchanged. The command uses
`GH_TOKEN`, then `GITHUB_TOKEN`, if set; public releases can also be read without a token. Discovery does
not verify an SDK signature. The workflow must verify the corresponding signed SDK separately before
supplying its data and the release list to the offline `registry check`, `review` and `build` commands.

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
The registry checks the bytes from `--files/<sha256>`; the generated index still points to the package
on the registry's `<id>-<version>` mirror. Rows without `sunrise` still use a bare filename.

### Stable native builds

A reviewed standalone native mod can add an author-signed binary build to its `[[versions]]` row:

```toml
[[versions.builds]]
sunrise = "26.3-0.1.1"
binaries = "https://github.com/you/example/releases/download/v0.1.0/example-0.1.0+26.3-0.1.1.binaries.json"
sha256 = "<lowercase hex of the descriptor>"
signature = """<descriptor .minisig text>"""
```

The source version must be signed and pin a stable release of the same line. Its registry version label
must be the full SemVer from the signed manifest (`1.0.0`, not `1.0`). Older rows without `sunrise` keep their
existing filenames and labels; publish a new stable mod version to add builds rather than renaming the old row. The package
must contain `Cargo.lock` and use the standalone `prebuilt-api` route: game edits, crates.io libraries and linked mod
dependencies do not produce an independent release library. Builds are optional. Each stable release
gets at most one row, and published rows are append-only. `registry review` reports an owner's append as
routine only after validation and only if the append is the submission's sole registry change. The JSON
report lists these changes under `build_reviews`. That classification covers registry data only. A workflow
that uses it to classify a whole pull request must also check the changed paths and refuse the shortcut
if any change falls outside the append, including changes to the workflow itself.

The descriptor `<id>-<version>+<release>.binaries.json` names `id`, `version`, `build` (the SDK commit),
`rust` (the SDK's Rust release), and `archives`. Every archive record has `target`, `name`, `size`,
`sha256` and `signature` (its `.minisig` text). Its name is `<id>-<version>+<release>-<target>.tar.xz`
and its URL is beside the descriptor in a release of the project's GitHub repository. `source` names that
repository; if `source` is omitted, `repo` supplies it. The archive contains exactly the library and
`identity.json`, both regular root files. `identity.json` has `target`, `file` (the library
name), `sha256` (the library digest) and `identity` (`<build> <id> <version> <rust> unwind`). The same
project key signs the source, every archive, and the descriptor. The descriptor binds each archive's
bytes to the declared project, mod version and SDK host. Validation checks every host the SDK ships,
without running or rebuilding any library. A binary for a package with a known `breaks` verdict is
refused; a verified exact binary can supply a build whose fit verdict is `exact`.

`registry list <checkout>` prints `sha256 id version file`, with `file` as written in the row. For build assets, use
`registry list <checkout> --build-assets` to list descriptors, then repeat with `--files <dir>` to list
archives from verified descriptors. Both print JSON rows with `id`, `version`, `sunrise`, `url`, `name`,
`sha256`, and `size` (null for a descriptor). The workflow supplies the listed files under their digests,
along with `--releases` and `--sdk-data`, to `registry check`, `registry review` and `registry build`. The
index points to the verified archives under the registry's release mirror, not the temporary hash-named
files. The tool itself does not fetch or upload assets. A takedown in `revoked.toml` naming an archive's
SHA-256 removes that host from the published and read index, without removing the source version,
other hosts, or the signed build declaration. The offline asset list no longer requests that archive.

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

`registry review` reports the addition or edit of just this table by a base owner as routine. A new Nightly-only
entry is routine if the author is in its `owners`. That status classifies the entry edit, not unrelated files or
whether the repository passed the workflow's checks. The JSON report lists these under `nightly_reviews` as
`{ id, status, reason }` (`routine` or `review_required`).

`registry build --nightly <file>` adds optional display metadata keyed by project id, for example
`{"coords_hud":{"branch":"main","commit":"<commit>","pin":"<Sunrise commit>","updated":"2026-11-20T18:00:00Z"}}`.
The workflow reads this metadata from GitHub; the tool uses it only for projects with `[nightly]` and does not fetch
or execute the repository. `repo` and `path` always come from the entry.

### The files folder

The registry's CI keeps every listed file in a folder named by hash, `files/<sha256>` (no extension), which
`registry check` and `registry build` read with `--files`. For a stable source row, the workflow fetches
from the URL in `file`. A row with only a filename is from before stable releases: its file is already in the
registry's releases or its `staging` release. Either way, the merged package is published under
`<id>-<version>` with its asset name. `cargo sunrise-mod publish` uses the same hash-named layout locally
for the new file, but its output is not a complete checkout. `registry check` needs the full checkout and
all listed files to plan installations.

For new or changed native mod versions, `registry review --files <dir>` compares the package's Cargo
requirements, including those of packaged path dependencies, with `<sdk-data>/<commit>/Cargo.lock` from
`--sdk-data <sdk-data>`. It warns when a crates.io requirement excludes every version the pinned game build
uses but accepts another version on the same version line as one of them: the same nonzero major, the
same `0.minor` with a nonzero minor, or the same `0.0.patch`. The build refuses to move the game's pin to
another version on that line. This warning does not rule out a version on a different line that also
satisfies the requirement. For example, `3.0.10` and `=3.0.5` both warn against the game's `3.0.6`; `>=2, <3` does not warn
against `3.1.0`, and `0.0.2` does not warn against `0.0.1`. Loosen the requirement if your mod does not need the other
version. If the game's lock is unavailable, review notes that it skipped
the comparison. A different pin in the mod's own lockfile does not trigger a warning.

Check a package on its own with:

```sh
cargo sunrise-mod check my_mod.smod
```

Use `--key` and `--signature` to check its signature. The command validates package
metadata and contents without running your mod’s code.

## Native dependency tools

Run these commands in your native mod’s project folder.

| Command | Behaviour |
| --- | --- |
| `cargo sunrise-mod internals sync` | Checks a trusted native mod's library against an isolated copy of its pinned game source with the Rust version named by its signed SDK. Replaces top-level `internals` with compiler-discovered paths, kinds and hashes, including uses inside globs and macros. Removes unused declarations, preserves retained entries and prints additions/removals. Skips discovery when the mod neither depends on `sunrise-internals` nor declares `internals`. Never runs automatically on build. |
| `cargo sunrise-mod internals add <path>…` | Adds each declared internal and its tool-written hash to top-level `internals` in your mod's `sunrise-mod.toml`. Reads `internals.json` from the pinned SDK, or discovers a catalog with the mod's `[[native.module]]` tables applied. Unknown paths, shape-only entries and paths available only under a `cfg` are refused before the manifest is changed. Existing declarations are refreshed, not duplicated. |
| `cargo sunrise-mod update [--sunrise <build>]` | Moves the mod's pin to the current Nightly's build (or the build given: a release tag such as `nightly-<date>`, `26.3-0.1.0` or `dev-<date>-<short sha>`, or a sha); the next command fetches its SDK into `.sunrise/sdk/`. A native mod's pins are `[native] sunrise` and `[native] mixins`: `update` fetches the destination SDK's `sdk.json` (into `.sunrise/cache/<build>/`), writes its `mixins.rev` beside the new build, and changes nothing when that file is refused; `Cargo.toml` names the SDK by path, so it does not change. For a native mod's every declared internal, prints `missing`, `changed` (old signature → new signature), or `unchanged`, then refreshes the available hashes. Missing or gated declarations keep their old hash and must be removed or replaced before building. Build names: [build names](sdk/03-in-a-project.md#build-names). |

The native-mod commands run in the mod's project folder and preserve the manifest's comments and
layout. `sync` performs a metadata-only `cargo check` on the current host, using default features;
Rust build scripts and procedural macros run with your rights, but the game does not run. It keeps
a separate source and check cache under `discovery/` in the tool's per-user cache (`SUNRISE_MOD_CACHE` can
set its root), never in the normal game or prebuilt target directories. The pin's source must be cached or
downloadable; an unavailable source is `E_INTERNAL_SOURCE_UNAVAILABLE`, not a fallback to another
pin. Compiler or catalog errors leave the manifest untouched. Discovery may prepare source and
metadata-check game dependencies; it does not compile or link a game executable. This separate
check cache is not a native mod build route. `[[native.state]]` is refused with
`E_MIXIN_STATE_LAYOUT`; use attachments for per-object mod data.

Without support modules, `internals add` reads `internals.json` from the pinned SDK's build data
(`build-data.tar.xz`). The tool verifies the SDK release, `sdk-<build id>`, against its signed `SHA256SUMS`,
using `SUNRISE_SDK_BASE` when set. No game or mod code is run.
With `[[native.module]]` tables, `internals add` uses the Rust named by the signed SDK to stage the pinned source
with the mod's modules applied and save the catalog under `target/sunrise-mod/catalog-snapshots/`. Only use
source discovery on mods you trust. It does not run the game.
The separate `update` command compares the SDKs' build data for the old and new pins. Each build's data is kept
in `.sunrise/cache/<build id>/build-data/`, so `update` can show old signatures after that release expires.
If a needed catalog is neither cached nor downloadable, the command fails without changing the pin
or declarations. A mod with no declared internals needs no catalogs to update. Quote paths containing
Rust lifetimes or other shell metacharacters, for example:

```sh
cargo sunrise-mod internals add 'sunrise_client::game::Game' "sunrise_client::game::maps::Game<'_>::dimension_id"
cargo sunrise-mod internals sync
cargo sunrise-mod update
```
