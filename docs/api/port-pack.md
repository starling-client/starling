# `sunrise-port-pack --json`

How a launcher imports, ports and checks Java modpacks and mod jars without linking the porter: it runs the
`sunrise-port-pack` that ships in the game's release folder, beside `sunrise-client`, and reads JSON lines. What the
steps do is in [modpacks.md](../modpacks.md#the-pipeline-sunrise-port-pack).

## Overview

| | |
| --- | --- |
| Format | `"format": 1` in every reply |
| Level | experimental ([stability.md](stability.md)): its first user is the launcher, after the launcher split |
| Covers | `--json` on `import`, `inventory`, `fetch`, `carry`, `report`, `port`, `port-work`, `status`, `profile` and `check`; `--version`; the program in the release folders |
| Not in | `upgrade-datapack` and `events`, which refuse `--json` (exit 2) |

The launcher's steps, each one process:

```sh
sunrise-port-pack import PACK WORK --json          # Import: pack.json
sunrise-port-pack inventory WORK --json            # then: Modrinth metadata, classes and tiers (--offline reuses it)
sunrise-port-pack report WORK --json               # then: coverage.json
sunrise-port-pack port-work WORK --resources PREPARED --json   # Port: jars, data packs, carry, report again
sunrise-port-pack profile WORK --json              # after Import and Port, and before Play: the profile's resource packs
sunrise-port-pack status WORK --json               # a page that opens: read only
sunrise-port-pack check JAR OUT --json             # the mod browser's check of one jar
```

## Reference

### Lines

With `--json`, every line on stdout is one JSON object:

- `{"message": string}`: a log line, for the player to read, as the step goes.
- The last line is the reply, which has `format`: a work folder's status, a check, or an error.
- Readers ignore keys they don't know and lines with no key they know.

Exit 0 with a reply. Exit 1 with an error reply, and the message after `error:` on stderr. It has the
`{"kind", "message"}` shape every launcher-split contract uses (`sunrise-server engine-command` and
`sunrise-prepare extras` too):

```json
{"format": 1, "error": {"kind": "fetch", "message": "…"}}
```

`kind` is the step that failed, one per step, with no catch-all:

| `kind` | The step | From |
| --- | --- | --- |
| `import` | reading the pack into `pack.json` | `import`, `port` |
| `inventory` | metadata, jars and classes into `inventory.json` | `inventory`, `port` |
| `fetch` | getting the portable mods' jars (or reading `inventory.json` for it) | `fetch` |
| `carry` | carrying the content mods' data and assets over | `carry` |
| `report` | writing the coverage report | `report` |
| `check` | checking one mod jar | `check` |
| `port` | the port step (fetch, carry, report and the profile's packs) | `port`, `port-work` |
| `profile` | installing the staged resource packs into the profile | `profile` |
| `status` | reading the work folder back for the reply | every command that ends with a status |

Exit 2, with the usage on stderr and no JSON: a command line it doesn't understand (an unknown command, a missing
argument, an option without its value), and `--json` on `upgrade-datapack` or `events`.

### Status reply

`import`, `inventory`, `fetch`, `carry`, `report`, `port`, `port-work`, `profile` and `status` end with the work
folder's status, read from the files the steps wrote:

```json
{"format": 1, "work": "/home/a/.local/share/sunrise/ports/cozy",
 "pack": {"name": "Cozy", "format": "mrpack", "minecraft": "1.21.1", "loader": "neoforge", "loader_version": "21.1.1",
          "files": [], "overrides": {}},
 "rows": [], "summary": {"needing": 0, "carried": 0, "partial": 0, "ready": 0, "blocked": 0, "later": 0},
 "fetched": {"files": []}, "mods": [], "left_off": [], "portable": 0, "resourcepacks": 0, "indexed_resources": 0,
 "datapacks": 0, "resource_reports": {"staged": {"files": {}}, "profile": {"files": {}}}}
```

- `pack`: `pack.json` as `import` wrote it. `files[]` has `path`, `client` and `server` (`required`, `optional`,
  `unsupported` or empty) and, when known, `sha1`, `downloads`, `curseforge` (project and file ids), `mod_id`,
  `name`, `version`, `jar`.
- `rows`: `coverage.json`, one per file, empty before `report` ran: `key`, `name`, `class`, `tier`, `license`,
  `license_kind`, `status` (`ported`, `partial`, `not-yet`, `not-needed`), `reason` (for modders), `plain` (for
  players, without the dependency sentence), `waiting` (unported dependencies, for the launcher to translate),
  `old_layout` (only when true).
- `summary`: the headline's numbers. `needing` is every mod but the ones not needed; `carried` the ported and partial
  ones, `partial` those of them that are partial; `ready`, `blocked` and `later` split the rest (data-tier content
  that `port` can carry; mod support Sunrise lacks; not sorted yet).
- `fetched`: `fetch.json`, empty before a fetch: `files[]` with `key`, `file`, `how` (`present`, `downloaded`,
  `local`, `needs-key`, `not-found`, `failed`) and `detail`.
- `mods`: the carried-over mods the game loads together, in load order (absolute paths of `mods/<id>/carry`).
- `left_off`: carried-over mods the game leaves off, each with why (a namespace another one owns).
- `portable`: the mods `port` would get and carry over (data-tier content, made for 1.21 or newer).
- `resourcepacks`: resource packs staged for the profile; `indexed_resources`: the ones the pack lists that `port`
  can get; `datapacks`: data packs from jars that hold nothing else, on in the profile's new worlds.
- `resource_reports`: `resourcepacks.json` (`staged`) and `profile-resourcepacks.json` (`profile`): `files`, by
  path, with `state` and `detail`.

### Profile

`profile WORK` copies each resource pack staged in `WORK/resourcepacks/` into the profile's game home,
`WORK/home/resourcepacks/`, and writes what happened to each to `WORK/profile-resourcepacks.json` (the reply's
`resource_reports.profile`). A pack already in the profile is never overwritten or merged into.

### Check reply

`check JAR OUT --json` carries one jar over into `OUT/carry/`, as the mod browser installs it, and ends with:

```json
{"format": 1, "check": {"mod_id": "coords", "name": "Coords", "version": "1.0", "loader": "neoforge", "license": "MIT",
 "verdict": "works", "summary": "…", "ports": ["12 blocks"], "skipped": [], "depends": [], "namespaces": ["coords"],
 "code": {}}}
```

`check` is `OUT/check.json` as the step writes it; `verdict` is `works`, `partly` or `no`.

### `--version` and `--help`

`sunrise-port-pack --version` prints `sunrise-port-pack <version>`, the release's version as `sunrise-prepare` and
`sunrise-server` print theirs. `--help` prints the usage. Both exit 0.

### Where it is

Every game release folder has `sunrise-port-pack` (`.exe` on Windows) beside `sunrise-client`
(`sunrise-mod-package`'s `package` route). The server folder doesn't.

## Stability

Experimental. A new field or line key keeps `format`; a changed or removed one makes the next `format`, and the
release contract in `sunrise-release.json` moves with it. The fields inside `pack`, `rows`, `fetched` and `check`
are the work folder's files, so a change to those files is a change here. `status::tests` pins the status reply.

## Changelog

- format 1: `--json` on the launcher's commands, `port-work`, `status`, `profile`, `--version` and `--help`; the program
  ships in the game's release folders.
