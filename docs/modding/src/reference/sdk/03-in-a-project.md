# In a mod project

A code mod's project names the mod API by the path where `cargo sunrise-mod` keeps the pinned build's SDK:

```toml
[dependencies]
sunrise-mod-api = { path = ".sunrise/sdk/api/crates/sunrise-mod-api" }
```

`sunrise-internals` is named the same way, by `.sunrise/sdk/api/crates/sunrise-internals`. The
SDK does not carry a `sunrise-internals` crate: `cargo sunrise-mod build` and `internals sync` generate the
mod's own facade in its place. A plain `cargo check` cannot resolve it. Features
(`features = ["testing"]` on a dev-dependency) are kept.

`cargo sunrise-mod new` writes that path. A native build refuses every other form under `E_SDK_SOURCE`,
with the line to write instead:

| Written | Why it is refused |
|---|---|
| `{ git = "…", rev = "…" }` | The SDK is no longer named by git. |
| `"0.1"` or `{ version = "0.1" }` | A version names no source, so Cargo would take the crate from crates.io, where anyone can publish that name. |
| `registry`, `registry-index`, `branch`, `tag`, `rev` or `workspace` | It does not name the SDK. |
| `{ path = "…" }` anywhere else | The path must be the pinned SDK the tool fetches. |

## The pin

The build a native mod is made for is its pin, a full commit SHA in `[native] sunrise` in
`sunrise-mod.toml`.

`cargo sunrise-mod new` writes the current Nightly's build, or the one selected with `--sunrise`, in any of the
[forms below](#build-names). The pin is always the full SHA.

A native project normally leaves `[native] mixins` out. When a mod uses mixins, builds and discovery use the
horizon-mixins revision and patch recorded in the pinned SDK's signed `sdk.json`. The mod tool must support that
exact pair, and any explicit `[native] mixins` revision must match it. If the tool and SDK disagree, install the
tool from that SDK or run `cargo sunrise-mod update` to move to an SDK that matches your tool. If the explicit
revision disagrees, remove it to use the SDK's revision.

A native build writes the revision into the package, not the project. For a mod that doesn't use mixins, the
package keeps an explicit revision or records the SDK's revision if none was written, without requiring a match.
A project in a game checkout uses the checkout's API surface instead of fetching one into the project, but its
native compile and package revision still come from the pinned SDK. `cargo sunrise-mod update` moves the Sunrise
build pin after checking the destination SDK; it leaves any explicit mixins revision alone.

### Build names

Wherever a build is given (`--sunrise` for `new` and `update`, `sdk diff`, `fits`), these forms are accepted:

| Written | Means |
|---|---|
| `nightly` | The current Nightly's build. |
| `nightly-YYYY-MM-DD` | That day's Nightly's build. |
| A Stable tag such as `26.3-0.1.0` | That release's build. |
| A Dev tag `dev-YYYY-MM-DD-<short sha>` | That Dev build's; the build must start with the short SHA. |
| 40 hex digits | That build. |
| 7 to 39 hex digits | The one SDK whose build id starts with them, among the releases listed: this repository's 100 newest on GitHub, or a folder base's entries. Another URL base takes the full sha only. |

A release name is read from `<base>/<name>/sdk.json`. A Stable or Dev release's `sdk.json` must match its
`SHA256SUMS`, signed with Sunrise's release key for exactly that tag (a Dev build's `SHA256SUMS.v2.sig`), so a mirror
can't put another build under its name. A release whose build has no SDK says so, and asks for a build that has one.

## `.sunrise/sdk/`

In a code mod, `new`, `build`, `check`, `hooks`, `fits` and `update` fetch the pinned build's SDK
into `.sunrise/sdk/` after the [checks](01-release.md#how-a-download-is-checked). It holds `sdk.json`,
`surface.json`, the unpacked API source under `api/`, and `build`, a stamp written only after the fetch finishes.
The tool fetches again if the stamp is missing or if the pinned build or trusted SDK keys have changed.

`fits` and `update` fetch no SDK for a mod made for exactly one build: its manifest says so before one is needed. Any of these commands makes a fresh clone of a mod work, `check` included, since it fetches before Cargo
reads the path. An editor needs that first command too: until it runs, `sunrise-mod-api`'s path does not exist.

`sdk diff`, `fits` and `update` read other builds' small files (`sdk.json` and `surface.json`) into
`.sunrise/cache/<build id>/`, with the same checks.

`.sunrise/` is in the template's `.gitignore`, and the package walker skips it: it is never packed, copied by the
launcher or sent to a registry.

## Mirrors and offline use

`SUNRISE_SDK_BASE` replaces the releases' base for the SDK fetches of the tool and the launcher alike:

- a URL: a mirror with the same layout, `<base>/sdk-<build id>/<file>`, and `<base>/<tag>/sdk.json` for a
  [release name](#build-names);
- anything else: a folder holding `sdk-<build id>/` folders, and `<tag>/` folders with their `sdk.json`.

A Stable or Dev tag's folder also holds that release's `SHA256SUMS` and its signature, as published. The launcher also reads game releases' folders there, for their build ids.

Every SDK base is checked the same way, against Sunrise's signature. For a [local SDK built from a
checkout](../../how-to/native-mods/07-local-developer-builds.md), a debug build of the mod tool can also check
against the public key set in `SUNRISE_SDK_TRUSTED_KEY`. The launcher's own copies are described in
[Cache and disk](../../how-to/native-mods/10-cache-and-disk.md).

`internals add` and pin changes that need catalogs read `internals.json` from the pinned build's signed SDK.
The tool unpacks `build-data.tar.xz` into `.sunrise/cache/<build id>/build-data/`. `internals sync` checks
pinned game source instead, as does `internals add` when the mod declares `[[native.module]]` rows. Source
discovery uses the Rust named by the signed SDK, but does not need its compiled game slices.
See [Native dependency tools](../registry-index.md#native-dependency-tools).
