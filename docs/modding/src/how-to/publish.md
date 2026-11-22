# Publishing your mod

Sunrise's registry, [sunrise-org/sunrise-mods](https://github.com/sunrise-org/sunrise-mods), lists mods, modpacks,
resource packs and shader packs at [playsunrise.net/mods](https://playsunrise.net/mods). Every new or changed
registry version row must name a stable Sunrise release in `sunrise`. A native mod submits a version through
[the stable source submission](#publishing-a-stable-source-version); for any other kind, a content-only mod
included, write the row by hand as [the registry reference](../reference/registry-index.md#the-entry) shows. The steps below show how to prepare and check
a local draft, and what review does with a submission.

## 1. Build and write the submission

Set `version` in `sunrise-mod.toml` to a SemVer without `+build` metadata, at most 64 characters after
padding missing parts (for example, `1.2` becomes `1.2.0`). The registry uses that version in the release name.

In your mod's folder:

```sh
cargo sunrise-mod publish --summary "One line about the mod." --owner your-github-login --categories utility
```

It builds the package as `build` does, which may download the SDK, toolchain and dependencies. It does not contact the
registry. The submission goes into `target/sunrise-mod/publish/` (change it with `--out <dir>`):

| Path | What it is |
| --- | --- |
| `mods/<id>.toml` | The registry entry, filled from `sunrise-mod.toml`: name, description, authors, license, `homepage` as the source, and the version's file name, SHA-256 and date |
| `<id>-<version>.smod` | The package: attach exactly this file to a release of your repository |
| `files/<sha256>` | The same file under the name the registry's checks read |
| `icons/<id>.png` | For a new listing, your `icon.png` from beside `sunrise-mod.toml` |
| `PULL_REQUEST.md` | The text of the pull request |

What the manifest cannot say comes from flags: `--summary` (at most 160 characters), `--owner <login>` (repeat it for
several; the GitHub accounts allowed to publish updates), `--categories a,b` (1 to 3 from [your kind's
list](../reference/registry-index.md#submitting-a-project)), `--tags a,b` and `--changelog <text>`. An entry or page
field still missing is written as a value that starts with `TODO(publish)`, and the command lists what remains. The
registry refuses an entry that holds one, so edit `mods/<id>.toml` (or the manifest, then run `publish` again) until the
list is empty.

Save `icon.png` beside `sunrise-mod.toml` as a square PNG, 64 to 512 pixels per side and at most 256 KiB, so it fits
both the package and the registry.

## 2. Check the new file

Run the `cargo sunrise-mod check <file>` command that `publish` prints, from your mod's folder. For example:

```sh
cargo sunrise-mod check 'target/sunrise-mod/publish/my_mod-0.1.0.smod'
```

This checks the new package and, for a signed project, its signature. `publish` also lists anything left to fill in
on the entry or page. Registry CI checks the whole checkout and plans installations with all listed
files available. Your submission folder does not contain previously published packages or an existing project's images.

For a first listing, you can also try an index containing just your project once the entry and icon are complete:

```sh
cargo sunrise-mod registry build 'target/sunrise-mod/publish' --files 'target/sunrise-mod/publish/files' \
  --out 'target/sunrise-mod/publish/index.json' \
  --release 'https://github.com/sunrise-org/sunrise-mods/releases/download/'
```

## 3. Open the pull request

A row without `sunrise` is refused by review. For a native mod, give `publish` the flags of
[the stable source submission](#publishing-a-stable-source-version) before opening the pull request; for any other
kind, write the row by hand as [the registry reference](../reference/registry-index.md#the-entry) shows.

1. Create a release of your repository and attach `<id>-<version>.smod` to it. Its address is the artifact URL.
2. Fork [sunrise-org/sunrise-mods](https://github.com/sunrise-org/sunrise-mods) and copy in `mods/<id>.toml`.
   For a first listing, copy in `icons/<id>.png` too, and at least one screenshot: save a PNG or JPEG (320×180 to
   3840×2160, at most 1 MiB) as `screenshots/<id>/<name>.png` (or `.jpg`) in your fork and list it in the entry:
   `screenshots = ["screenshots/<id>/<name>.png"]`. A new project without one is refused. A `mod` or `native-mod`
   in the `library` category needs none. Do not commit the package or `files/`.
3. Commit with `git commit -s`: every commit needs a `Signed-off-by:` line.
4. Open the pull request with the text of `PULL_REQUEST.md`, after pasting the artifact URL into it.

## 4. What review does

- The `check` workflow downloads each listed file, compares its SHA-256 with the entry's, and runs
  `registry review`, `registry check` and `registry build` on the result. It fetches a new version from the
  release asset its row names; nothing is staged by hand.
- A reviewer reads the change and merges it. The `publish` workflow then releases the file as `<id>-<version>` and
  adds the project to the index.
- The hash is checked again before any download reaches the game folder, so a file changed after
  review is refused.

For a native mod, review warns if a crates.io requirement in your packaged Cargo manifests excludes the
pinned game build's versions of that crate but accepts another version on the same version line (the
same nonzero major, the same `0.minor` with a nonzero minor, or the same `0.0.patch`). If your mod
doesn't need the other version, loosen the requirement before packing so it can use the game's version.

## 5. A new version

Raise `version` in `sunrise-mod.toml`, then run `publish --registry <your checkout of sunrise-mods>` with an output
folder separate from the checkout. For a signed project, [build and sign first](#signing). `publish` adds the new
version row to the entry from the checkout, leaving all existing fields and rows in place. The printed local check
reads only the new file, so you do not need to download the old files or copy over the icon and screenshots.
A new version of a listing that has no screenshots needs one too: add the file under `screenshots/<id>/` and list it
in the entry's `screenshots` in the same pull request, or the check refuses it (a `mod` or `native-mod` in the
`library` category is exempt, and a listing that gains no version stays valid as it is). Repeat steps 2 to 4.

`--registry <checkout>` also checks a first listing's id against the checkout's collection ids, and lists among what
remains each required dependency of the package that no listed mod version for the same game satisfies, with the
versions the checkout does list; a version taken down in `revoked.toml` does not count. These lines are printed only,
not written into the entry. The dependencies are read from the package itself, so after changing `[dependencies]` in
`sunrise-mod.toml`, build again (and sign the new file, for a signed project) before running `publish`. Without
`--registry`, the registry's CI checks both when the pull request opens; it also checks sides and what the dependencies
themselves require.

Published source files never change. Removing a row published with `sunrise` requires a takedown by the
registry's maintainers in `revoked.toml` naming the project id and file hash. Legacy rows without `sunrise`
can be removed without a takedown, but the project entry must stay. The only update `publish` makes to an
existing version is an [append of signed native builds](#publishing-native-binaries).

## Signing

Signed projects use an Ed25519 key in [Minisign](https://jedisct1.github.io/minisign/) format. The registry entry
holds the public key; every version of that project must be signed with the same private key.

Run `cargo sunrise-mod keygen` once in your mod project. It reads the mod ID from `sunrise-mod.toml`, asks for a
passphrase without echoing it, and prints the registry public key. It saves the encrypted `signing.key` and a text
copy of the public key (`signing.pub`) in the mod's per-user data folder:

- Linux: `$XDG_DATA_HOME/sunrise-mod/keys/<mod-id>/` (default data home: `~/.local/share`).
- macOS: `~/Library/Application Support/sunrise-mod/keys/<mod-id>/`.
- Windows: `%LOCALAPPDATA%/sunrise-mod/keys/<mod-id>/` (fallback: `%USERPROFILE%/AppData/Local`).

Use `--out <dir>` to choose a different folder, either outside the project or under its top-level `.sunrise/`,
`target/`, or `.git/` directory, which are excluded from packages. The tool does not change Git's ignore rules.
Back up the private key securely; never commit it or upload it as an asset.

Build and sign before calling `publish`, including on the first listing. For example, for a mod with ID `my_mod`,
set `public_key` to the value printed by `keygen`:

```sh
cargo sunrise-mod build
cargo sunrise-mod sign target/sunrise-mod/my_mod.smod
cargo sunrise-mod publish --summary "..." --owner you --categories utility \
  --key "$public_key" --signature target/sunrise-mod/my_mod.smod.minisig
```

`sign` writes a detached `.minisig` beside the input. It uses the current mod's per-user `signing.key` unless
`--key <path>` is given. Existing Minisign key files and signatures work too. On Unix, `keygen --no-password` explicitly
opts in to generating an unencrypted key. On Windows, `keygen` requires encryption because key files inherit the
folder's permissions. Use `sign --no-password` to sign with an existing unencrypted key file. Neither command
overwrites an existing output. After rebuilding a file at the same path, remove its old `.minisig` before signing
the new bytes. Keep the existing signature when retrying publication of unchanged, verified bytes.

For CI, store the full private key file text as a protected secret named `SUNRISE_MOD_SIGNING_KEY` and, for an
encrypted key, store its passphrase separately as `SUNRISE_MOD_SIGNING_PASSPHRASE`. Set those only in the
signing step's environment and run `cargo sunrise-mod sign <file> --key-env`. With a passwordless CI key, leave
the passphrase variable unset. The command never prompts in this mode. Do not put either secret on the command
line, in a log, or in a build artifact. The tool clears its own in-memory copies of keys and passphrases after use;
it cannot clear the process environment or the copies held internally by Minisign and the password prompt library.

For a new version of an already-signed project, use the same key and replace the page flags (`--summary`, `--owner`,
`--categories`) with `--registry <your checkout of sunrise-mods>`. With `--key` and `--signature`, `publish` checks
and copies the exact bytes you signed, without rebuilding.

## Publishing a stable source version

A native mod can publish its signed `.smod` without binaries, including a mod that is not eligible to ship
them. `publish --sunrise` checks the package's `[native] sunrise` pin, so it does not write a row for a
content-only mod or a pack.
Set `homepage` in the manifest to your project's GitHub repository. Pin the project to the stable Sunrise
commit, build and [sign the source package](#signing), and pass a release list (`releases.json`, an array of
`{ "tag", "commit" }` records). Use `--asset-base` for your GitHub release's download URL without the asset name:

```sh
cargo sunrise-mod publish --summary 'A brief summary' --owner you --categories utility \
  --sunrise 26.3-0.1.0 --releases releases.json \
  --key "$public_key" --signature target/sunrise-mod/my_mod.smod.minisig \
  --package target/sunrise-mod/my_mod.smod \
  --asset-base 'https://github.com/you/my_mod/releases/download/v0.1.0'
```

Upload only the author-release assets listed in `PULL_REQUEST.md`: the exact `.smod` and `.smod.minisig`.
Submit `mods/<id>.toml` to the registry with `icons/<id>.png` if an icon was staged. The `file` field already
points to your release asset. Registry CI downloads it from there, verifies it and mirrors it after merge.
For a new version of an existing listing, use `--registry <checkout>` instead of the page flags.

An entry that still lists unsigned versions from before stable releases cannot take a key through
`--registry`. Run `publish` without it for the signed package, then edit the entry in your checkout by hand:
add the `key`, remove the unsigned rows and add the new row. Review notes each removed row and needs no
takedown for it.

## Publishing native binaries

Builds are optional. They are available only to a standalone API native mod: no game patches, crates.io dependencies
or declared mod dependencies. Keep the signed `.smod` as the source release even when players can download binaries.
Set the manifest's `homepage` to your project's GitHub repository. Use the stable release list (`releases.json`,
an array of `{ "tag", "commit" }` records). To publish binaries, put the verified SDK index (`sdk.json`),
`surface.json`, and extracted `build-data/` under `sdk-data/<commit>/` for both the original source package's
pin and the target commit. To describe the archives with `--describe-binaries`, only the target's verified
`sdk.json` is needed under `sdk-data/`.

1. Pin your checkout to the stable SDK commit. On each SDK host, build the archive with:

   ```sh
   cargo sunrise-mod build --binaries release-assets --sunrise-release 26.3-0.1.0 --releases releases.json
   ```

   Run it with `--server` as well for each dedicated-server host the SDK lists. Every run writes a `.smod` and the
   selected host's `<id>-<version>+<release>-<host>.tar.xz`. Keep one `.smod` to sign as the source for a new version.
2. Sign every archive with `cargo sunrise-mod sign <archive>`. Gather the archives and their `.minisig` files
   in `release-assets`, then assemble and sign the descriptor with the same key:

   ```sh
   cargo sunrise-mod build --describe-binaries release-assets --sunrise-release 26.3-0.1.0 \
     --releases releases.json --sdk-data sdk-data
   cargo sunrise-mod sign release-assets/my_mod-0.1.0+26.3-0.1.0.binaries.json
   ```

   The descriptor embeds the archive signatures; it does not sign the archives for you.
3. Prepare the submission. For a new listing:

   ```sh
   cargo sunrise-mod sign target/sunrise-mod/my_mod.smod
   cargo sunrise-mod publish --summary 'A brief summary' --owner you --categories utility \
     --sunrise 26.3-0.1.0 --releases releases.json --sdk-data sdk-data \
     --key "$public_key" --signature target/sunrise-mod/my_mod.smod.minisig \
     --package target/sunrise-mod/my_mod.smod \
     --binaries release-assets --asset-base 'https://github.com/you/my_mod/releases/download/v0.1.0'
   ```

   For an existing listing with a new mod version, replace the page flags with `--registry <checkout>`. For a later
   Sunrise build of an **already published** mod version, build archives from a temporary checkout of that source
   repinned to the new commit. Use `--registry <checkout>` and pass the **original published `.smod`** to `--package`,
   not the temporary build's package. Omit `--sunrise`, `--key`, and `--signature`: `publish` uses the source row's
   existing stable pin, key and signature and adds one build row without touching them. Keep `--asset-base`
   on the original author release. `publish` checks the signed assets and the registry's fit verdict before writing output.
   Use the output only after `publish` succeeds.
4. Upload and verify only the author-release assets listed in `PULL_REQUEST.md` on the release at `--asset-base`
   before opening the pull request. These are the archives, the descriptor and its `.binaries.json.minisig`, and,
   for a new mod version only, the source package and its `.smod.minisig`. The descriptor embeds the archive
   signatures, so no separate archive signature uploads are needed. If an upload fails partway through, upload
   the missing files from the same verified output rather than rebuilding or re-signing. The tool does not upload anything.
   Submit `mods/<id>.toml` with `icons/<id>.png` if an icon was staged; do not commit the release assets. See the
   [registry format](../reference/registry-index.md#stable-native-builds) for offline verification and takedowns.

## Tagged native releases in GitHub Actions

New native projects include `.github/workflows/release.yml`, `.github/workflows/rebuild.yml`, a shared workflow
and an action that verifies and installs the release tool. The repository does not carry the release scripts: every job extracts them
from its verified tool with `cargo sunrise-mod release scripts`. Development still starts on Nightly. When
you are ready to publish a stable version from GitHub:

1. Set `homepage` to your mod's GitHub repository and pin the mod to the commit of a stable Sunrise release.
   The tag you push must be `v` followed by the full version in `sunrise-mod.toml`, such as `v0.1.0`. Check in
   your reviewed `Cargo.lock` if the project has one; CI does not update an existing lock for you.
2. Run [`cargo sunrise-mod keygen`](#signing) once. Back up the private key outside the repository. Set the
   Actions variable `SUNRISE_MOD_PUBLIC_KEY` to the printed public key. For an existing registry listing it
   must match the key in that entry. Don't generate a fresh key for another release of the same project.
3. Create the `sunrise-mod-signing` environment. Restrict it to your trusted release tags and require review
   according to your repository's release policy. If you use rebuilds, also allow your trusted default branch.
   Store `SUNRISE_MOD_SIGNING_KEY` and, if needed,
   `SUNRISE_MOD_SIGNING_PASSPHRASE` there, not in repository variables or on the command line. Keep this
   environment off pull requests. Only the signing step sees the secrets, and it never builds or runs mod code.
4. For a first listing, set `SUNRISE_MOD_REGISTRY_OWNER` (your GitHub login), `SUNRISE_MOD_SUMMARY` and
   `SUNRISE_MOD_CATEGORIES` (comma-separated). Fill the manifest's description and license and put `icon.png`
   beside it. See the [registry page requirements](../reference/registry-index.md#submitting-a-project) for
   anything else to add to your submission. Set `SUNRISE_MOD_RELEASES=true` in Actions variables to enable the workflow.

A tag build verifies Sunrise's rolling signed [`mod-tool`](../start/install-tool.md#newest-tool-from-main), then
plans against the tag's independently signed SDK. All jobs require the same authenticated tool commit. A
standalone API mod builds an archive for each host in that SDK. Other valid native mods get a signed source
package only, including mods with required content dependencies. If your code imports another mod's library,
the standalone check cannot supply it and the tag workflow stops before signing. Dependency-aware tagged
releases are tracked in [#3885](https://github.com/sunrise-org/sunrise/issues/3885). A failed check or binary
build stops the release; it does not publish a source-only substitute.

The workflow keeps the verified signed batch as a run artifact before uploading. If an upload stops partway,
rerun the failed jobs of that run while the original signed batch is available. Identical assets are reused;
only missing ones are uploaded. A different remote file stops the workflow. Recover the original signed batch
rather than rebuilding or re-signing, even if the original source checkout did not change. A rolling tool update
can also stop a retry; in that case finish the release manually with the saved, verified assets.

Once the registry job succeeds, the `sunrise-registry-submission` artifact and the run summary give you the entry,
any staged icon and the pull request draft. You can always submit these yourself. To let Actions open or update
that pull request, set `SUNRISE_MOD_REGISTRY_FORK` to your own fork of `sunrise-org/sunrise-mods` (`owner/name`).
In a separate `sunrise-mod-registry` environment, set `SUNRISE_MOD_REGISTRY_TOKEN` to a token for your listed
GitHub owner account that can push to the fork and open registry pull requests. It is not the author release's
token and is not available to the build, sign or upload jobs. If the registry needs page fields or screenshots
that were not staged, complete the attached draft by hand. The workflow does not commit the author release assets.
If the registry job fails on the fork or token, fix that setup or remove both settings, then rerun the registry job.

## Rebuilding binaries for a newer stable release

Once the registry has reviewed and published a version with binaries, `rebuild.yml` can add a build for a newer
stable Sunrise release on the same line. It runs on the default branch with the same `SUNRISE_MOD_RELEASES`
opt-in and signing key as the tag workflow. On its daily schedule it takes at most one candidate: the newest
eligible reviewed mod version whose line's current stable release is not already covered. It does not walk back
through older releases, and no candidate is a successful no-op. To try a specific pair, run **Rebuild reviewed
native mod source** on the default branch with both the exact mod version (such as `0.1.0`) and the stable Sunrise
tag (such as `26.3-0.1.1`). The same eligibility rules apply.

A rebuild downloads the exact signed `.smod` from the original author release and verifies it against the
reviewed source row. It builds from that package and its existing `Cargo.lock`, not the current branch's Rust
source. Only the manifest in the disposable build folder is repinned; neither a branch nor the tag moves. If
the lock no longer works unchanged or the mod does not fit the newer SDK, publish a new mod version instead.
Planning skips withdrawn or ineligible versions. Build jobs use the plan snapshot; preflight stops publication
if eligibility changes or either stable Sunrise tag moves or is withdrawn after planning.

Only the target's signed archives and descriptor are added to the existing author release. The original source
and its signature never change. The submission adds one build row to the reviewed version without editing its
old builds or page fields. If the exact row was already accepted, nothing is submitted. If the source, key, owners
or existing target changed since planning, stop and replan rather than rewriting the reviewed entry. Recover the
original signed batch after an interrupted upload, as for a tagged release. Without a registry fork token, submit
the staged append yourself from the run artifact, which is attached once the registry job succeeds.

## A collection

Add `collections/<id>.toml` and a square PNG at `icons/<id>.png` in one pull request to
[sunrise-org/sunrise-mods](https://github.com/sunrise-org/sunrise-mods). A collection is not a package: there's no file
upload or version to publish.

```toml
id = "sunrise_creatures"
name = "Sunrise Creatures"
summary = "Fireflies, butterflies and their shared library."
owners = ["octocat"]
icon = "icons/sunrise_creatures.png"
members = [
  { id = "creature" },
  { id = "fireflies" },
  { id = "butterflies", default = false },
]
```

Members are selected by default unless you set `default = false`. You can add a Markdown `description` and record
numeric `owner_ids` in the same order as `owners`. `cargo sunrise-mod registry check <base> <head> --files <dir>`
checks the proposed registry, including the collection; see the [full rules](../reference/registry-index.md#collections).

To add a new mod, list it in the registry, then add its id to `members` in one collection pull request. If you own both
the collection and the mod, this is a routine membership edit; changes to the page, defaults or owners wait for review.

## Integrity and takedowns

- **Integrity.** Every download is checked against the SHA-256 the index lists before the file reaches the game
  folder. Unsigned local files stay allowed, with an "unsigned" mark.
- **Takedowns.** A file the registry takes down (`revoked.toml`) is refused and turned off where installed. What a
  listed project agrees to is in the registry's `MOD-POLICY.md`.

Once a project is published, players can rate it on its page after signing in with GitHub. A player can change their
rating later. Owners can't rate their own project, and nobody sees who rated.
