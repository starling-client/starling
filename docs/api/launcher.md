# Launcher API

What a player's agent, a script or a pack maker can do with `sunrise-launcher`: install and run versions, manage
accounts, settings, mods, modpacks and hosted servers, from a terminal or over MCP. The launcher itself:
[the launcher's README](https://github.com/sunrise-org/sunrise-launcher/blob/main/README.md).

## Overview

Every page of the launcher has a command, and every command but three has an MCP tool with the same arguments; a tool
runs the command with `--json`. Tools that change the player's things for good need `confirm: true`; with No AI on,
the MCP server is off.

| | Count |
| --- | --- |
| Commands | 46: versions, installs, play, the server, notes, links, accounts, settings, logs, updates, data collection, modpacks, mods, hosted servers, MCP, benchmark |
| MCP tools | 43, plus the game's tools as `game_<tool>` ([agent-control.md](agent-control.md)) |
| Modpack formats it imports | 3: a Modrinth `.mrpack`, a CurseForge zip, an instance folder |

## Guide

1. `sunrise-launcher versions`, `sunrise-launcher install latest`, `sunrise-launcher play`.
2. Modpacks: `sunrise-launcher modpack import <file>`, then `modpack port` and `modpack play`
   ([modpacks.md](../modpacks.md)).
3. For an agent: `sunrise-launcher agent-config` prints how to connect a client to `sunrise-launcher mcp`.

## Reference

[launcher-commands.md](https://github.com/sunrise-org/sunrise-launcher/blob/main/docs/launcher-commands.md), generated from `https://github.com/sunrise-org/sunrise-launcher/blob/main/src/cli.rs`: every command, its arguments
and its MCP tool.

## Examples

```sh
sunrise-launcher versions --installed --json
sunrise-launcher mods search minimap --kind mod --json
sunrise-launcher servers create --name "Weekend" --player Alex --json
```

## Stability

Preview ([stability.md](stability.md)): command names, arguments and MCP tool names are the contract; `--json`
output shapes are not yet described in the reference.

## Not there yet

- **A Sunrise modpack manifest.** A pack made for Sunrise (not a port of a Java pack) needs its own file: the pack's
  id, name and version; the Sunrise API lines and the release line it needs; its `.smod` mods (id, version, SHA-256,
  where to download it, client, server or both); resource packs; default settings; and whether it has a server side.
  One manifest serves the launcher, `sunrise-server` (a server installs the server side of the same pack) and the mod
  browser. Designed from what a pack maker does, versioned per [stability.md](stability.md), with a schema test.
- **JSON output schemas** for every command, in the generated reference, so MCP tools declare their output.
- **A version** in MCP `serverInfo` and `--json` output, as for agent control.
- **Compatibility tests**: golden `--json` output per command.

## Changelog

- Unversioned so far: the command table and the generated reference are the contract.
