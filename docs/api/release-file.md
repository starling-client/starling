# `sunrise-release.json`

What a game build is, in one small file a launcher reads before it runs anything in the build: the release contract
it needs, the Minecraft version, and the Rust its native mods build with. It replaces values a launcher used to
compile in from the game's sources (`sunrise_core::version::VERSION_ID`, `tools/ci/rust-version`), so the launcher
can live in its own repository.

## Overview

| | |
| --- | --- |
| Contract | `"contract": 1` |
| Level | experimental ([stability.md](stability.md)): its first reader is the launcher, after the launcher split |
| Where | every Nightly, Dev and Stable release, listed in `SHA256SUMS` and so covered by its signatures |
| Written by | `tools/release_file.py write`, in `nightly-build.yml`'s `pick` job, from the built commit |

SDK releases (`sdk-<sha>`) have no `sunrise-release.json`: their signed `sdk.json` covers them, with the same facts
(`build.commit`, `toolchain.rust`) and the assets. A reader of an SDK release reads `sdk.json`.

## Reference

```json
{
  "contract": 1,
  "launcher_min": "1.0.0",
  "channel": "nightly",
  "build": "nightly-2026-11-16-0c02067",
  "commit": "0c02067…",
  "version_id": "26.3",
  "vanilla": "26.3",
  "protocol": 777,
  "java_major": 25,
  "rust": "1.99.1"
}
```

| Field | Means |
| --- | --- |
| `contract` | What a launcher must understand to install and run this build (below). An integer |
| `launcher_min` | The oldest launcher version that may install it, even when it knows the contract |
| `channel` | `nightly`, `dev` or `stable` |
| `build` | `<channel>-<date>-<short commit>`: the version `SHA256SUMS.v2.sig` is signed for and the release title names. A stable release's is its tag, `<target>-<semver>` (`26.3-0.1.0`), which its `SHA256SUMS.sig` is signed for |
| `commit` | The full id of the commit built |
| `version_id` | Minecraft's `WorldVersion.id()`: the version the player's files are prepared for and the launcher shows |
| `vanilla` | Minecraft's `WorldVersion.name()` |
| `protocol` | The network protocol version the game speaks |
| `java_major` | The Java the preparer runs |
| `rust` | The Rust toolchain native mods for this build compile with |

### What a launcher does with it

1. Downloads `SHA256SUMS` and its signatures, verifies them, then downloads this file and checks its SHA-256 there,
   as for any asset. An unsigned or mismatched file is refused, as an archive would be.
2. A `contract` it doesn't support: the build needs a newer launcher, and the launcher says so ("update the
   launcher") instead of installing it. A `launcher_min` above its own version: the same.
3. Otherwise it takes `version_id` and `rust` from here, not from values compiled in.
4. Readers ignore fields they don't know.

### When `contract` moves

`CONTRACT` in `tools/release_file.py` goes up by one in the pull request that changes, in a way an older launcher
can't follow:

- the release archives' layout (names, folders, which programs are there);
- the runtime holder protocol or the markers the launcher scans game executables for;
- `mod_list` or the other files the launcher writes in the game's home;
- the content manifest's required fields;
- the `format` of a tool the launcher runs: [engine-command.md](engine-command.md),
  [prepare-extras.md](prepare-extras.md), [port-pack.md](port-pack.md), and the native build tool when it ships.

An addition an older launcher can ignore moves nothing. Such a pull request links the launcher's change, so a
launcher that supports the new contract is released first. `launcher_min` moves only when an older launcher would
misbehave on builds of a contract it claims to know.

## Stability

Experimental. A new field keeps `contract`; a changed or removed field is a contract change itself.
`tests/test_release_file.py` pins the fields, and `tests/test_optional_release_assets.py` that the file is signed
and published with each build.

## Changelog

- contract 1: the file, on Nightlies and Dev builds.
