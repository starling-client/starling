# `sunrise-server engine-command`

The game command a server folder would start, described without starting it. The launcher runs the installed
version's own `sunrise-server` for it, so it never links the server's code: the program, the arguments and the
folders come from the same `server.toml`, install layout and command builder as `sunrise-server run`.

## Overview

| | |
| --- | --- |
| Format | `"format": 1` in every reply |
| Level | experimental ([stability.md](stability.md)): its first user is the launcher, after the launcher split |
| Covers | `sunrise-server [--dir DIR] engine-command --json`: the reply, its error kinds and the exit codes |

```sh
sunrise-server --dir ~/servers/survival engine-command --json
```

It starts nothing, writes nothing (not even `server.properties`), runs no preflight and downloads nothing.
Without `--json` it prints the same command for a person.

## Reference

### Reply (exit 0)

One JSON object on stdout:

```json
{
  "format": 1,
  "program": "/opt/sunrise/current/sunrise-engine",
  "args": ["--resources", "/srv/mc/resources", "--control-socket", "/srv/mc/.sunrise/engine.json", "--dedicated", "--nogui"],
  "cwd": "/srv/mc",
  "env": {"SUNRISE_NO_AUDIO": "1", "MIMALLOC_ALLOW_THP": "0"},
  "resources": "/srv/mc/resources",
  "resources_pending": false
}
```

- `program`: the game program `run` would start: `engine.path`, else the one beside `sunrise-server` (the
  release `current` points at when installed), else one on the `PATH`. An absolute path.
- `args`: its arguments, in order.
- `cwd`: the folder it runs in, absolute.
- `env`: the variables it sets on top of the inherited environment. A `null` value means the variable is removed.
- `resources`: the game-files folder the supervisor selects (`--resources`), absolute. Startup, this
  description and `import-resources` all use the same version and profile checks.
- `resources_pending`: `true` if the supervisor needs to prepare its managed server profile at
  `resources`; otherwise `false`. The probe does not prepare it.
- Every path is a string. A path that isn't valid Unicode is an `io` error, not a lossy string.

### Errors

One JSON object on stdout, with the message for a person:

```json
{"format": 1, "error": {"kind": "resources", "message": "the prepared game files in /srv/mc/custom are incomplete or damaged: …"}}
```

| `kind` | Means | Exit |
| --- | --- | --- |
| `resources` | the selected folder cannot be used or prepared by `run`, for example an explicit `engine.resources` folder that is not ready | 3 |
| `configuration` | `server.toml` doesn't parse or fails its checks | 1 |
| `program` | no game program beside `sunrise-server`, on the `PATH` or at `engine.path` | 1 |
| `io` | a file couldn't be read, or a path isn't valid Unicode | 1 |

These four are every kind `engine-command` writes: each comes from one case of `EngineCommandError::kind`
(`crates/sunrise-server-launch`), and there is no catch-all. A path in the reply that isn't valid Unicode, or a
command with no working folder, is `io`.

A refused command line (anything after `engine-command` but `--json`) exits 2 with a line on stderr and nothing on
stdout.

### Starting it

If `resources_pending` is `true` for a configured server that has already accepted the EULA, run
`sunrise-server --dir DIR init --yes`. This prepares the selected managed slot without changing
`server.toml` or starting the engine. Probe again: the same path is now ready and `resources_pending`
is `false`. Only then take launch leases and send Start.

The reply is what to run; how `sunrise-server run` starts it is fixed, so a launcher that starts the engine itself
does the same (`command_with_transfers` in `crates/sunrise-server-launch/src/lib.rs`):

- **stdin, stdout and stderr piped.** The supervisor sends console commands on stdin (`stop` among them, to end
  the game cleanly) and reads the game's log from stdout and stderr.
- **Its own process group.** Unix: `process_group(0)` (a new group led by the game). Windows: the creation flags
  `CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW`. A terminal's Ctrl-C then reaches only the supervisor, which stops
  the game cleanly through its console (`stop`) instead of the game dying mid-save, and on Windows no console
  window opens.
- **The environment** is the inherited one with `env` applied on top, and the working folder is `cwd`.

### Reading it

- Readers ignore fields they don't know, and an unknown error `kind` is treated as `io`.
- A reply whose `format` is newer than the reader knows is refused: the reader says to update the launcher.

## Stability

Experimental. A new field or error kind keeps `format`; a changed or removed one makes the next `format`, and the
release contract in `sunrise-release.json` moves with it. The tests in `crates/sunrise-server/src/engine_command.rs`
pin the reply's shape.

## Changelog

- format 1: `engine-command [--json]`, the error kinds above and exit code 3 for unusable game files.
  The additive `resources_pending` field distinguishes preparation by the supervisor from a resource error.
