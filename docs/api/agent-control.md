# Agent control API

How a program drives a running game: an AI agent through MCP, a script through `sunrise-ctl`, a test through the
control socket. Design and inventory: [agent-control.md](../agent-control.md); for players:
[playsunrise.net/docs/agents](https://playsunrise.net/docs/agents).

## Overview

The game listens on a local control socket (a Unix socket, or localhost TCP with a token on Windows), takes one
command per text line and answers one JSON line. `sunrise-ctl` wraps it: it starts the game, sends commands and
serves the same commands as MCP tools. The launcher's MCP server offers them too, as `game_<tool>`.

| | Count |
| --- | --- |
| `sunrise-ctl` commands | 96, in 8 groups (session, observe, move and look, act, screens and chat, multiplayer, world and cheats, scripting and benchmarks) |
| MCP tools | 88 (8 commands are CLI only) |
| Destructive commands (need `--confirm`) | 3 |
| Command names the game's socket answers | about 160, test hooks included (`sunrise-ctl help --game`) |

## Guide

1. Build `sunrise-ctl` (`cargo build --profile fast -p sunrise-client-lib --bin sunrise-ctl`), then
   `sunrise-ctl --socket $PWD/ctl.sock launch --world W --creative --wait --resources <prepared files>`.
2. `sunrise-ctl --socket … state`, `keys`, `turn`, `command`, `screenshot`, `quit`.
3. For an agent: `sunrise-ctl mcp`, or the launcher's `sunrise-launcher mcp` with `agent-config` to connect a client
   ([launcher-commands.md](https://github.com/sunrise-org/sunrise-launcher/blob/main/docs/launcher-commands.md)).

## Reference

[sunrise-ctl-commands.md](../sunrise-ctl-commands.md), generated from the command table in
`crates/sunrise-client/src/bin/sunrise-ctl/commands.rs`: every command, its MCP tool and its arguments.

## Examples

- `tools/agent_control_e2e.py`: starts a game and drives it end to end through `sunrise-ctl` and MCP.
- `tools/lan_e2e.py`: two games over LAN, both driven through their sockets.

## Stability

Preview ([stability.md](stability.md)). The commands in `sunrise-ctl`'s table and their MCP tool names are what the
API offers; the other commands the game's socket answers are test hooks and experimental.

## Not there yet

- **Reply schemas.** Each command's reply described in the table (fields, types, units), so the reference documents
  replies as well as arguments and MCP tools declare an output schema.
  `hello` and `status` are described (the reference's "Replies" section; MCP `outputSchema` with `structuredContent`);
  the other groups follow one at a time.
- **Events as a stream.** `events --since` polls today; a subscription that pushes events as they happen.
- **Test hooks apart.** The hooks only harnesses use move under their own prefix, out of the agent's tool list.
- **Compatibility tests.** A transcript per protocol version: commands and the reply keys each must keep.

## Changelog

- 0.1.0: the first numbered line. `hello` answers `protocol`, `release` (the game's version, which MCP `serverInfo`
  carries too) and `features` (the command groups the game serves). Before it the command table and the generated
  reference were the contract.
- `sunrise-ctl --api 0.1` (a tool change, the protocol stays 0.1.0): calls `hello` before the first command and refuses a
  game on another line or one without `hello` (exit status 5, `kind: "api"`). `0.1` takes 0.1.x; a full version must
  match exactly.
