# Host a server

`sunrise-server` hosts a Minecraft Java Edition 26.3 server with Sunrise's own game as the engine: one native
program, no Java, one settings file, a command line, and a web panel. Vanilla 26.3 clients and Sunrise clients
can join it. This page covers what works on `main` today and says plainly what doesn't yet. The design and the
plan are in [server.md](server.md).

## Before you start

- **Online mode checks accounts** (release v2026.10.17.07 and newer). With `online_mode = true`, the
  default, every player joins with their own account, as on a vanilla server: vanilla players signed in with
  the official Minecraft launcher, Sunrise players with the Microsoft account they signed in with in Sunrise's
  launcher ([microsoft-account.md](microsoft-account.md)).
- **Names you add are accounts** (release v2026.10.17.14 and newer). `init --player`, `sunrise-server
  whitelist add` and `op`, and the console's `whitelist add`, `op` and `ban` look a name up the way vanilla
  does: players already seen (kept in `usercache.json`), then Mojang's profile service. In online mode a name
  with no account is refused. So you can whitelist friends before they first join. On an offline server,
  `sunrise-server whitelist add` always writes the offline UUID, the one the player joins with, also while
  the server runs (it writes the entry and the game reloads the list): the console's own `whitelist add`
  takes the account's UUID when someone owns that name, as vanilla does, and that player is then refused.
- **An offline server** (`online_mode = false`) never checks accounts: anyone who reaches it can join under
  any name. Keep it on your LAN or a tailnet (Tailscale, ZeroTier), or share it only with friends you trust.
  A Sunrise game started without a session (no network at Play) can join only such a server.
- **Secure chat** works as on vanilla's server: in online mode, vanilla clients and Sunrise's (Nightly
  2026-11-01 and newer) sign their chat, and with `enforce-secure-profile=true` (the default) unsigned chat is
  refused. On an offline server, or for a player without a chat session, chat stays unsigned and shows as not
  secure.
- **Nether and End:** each player travels on their own through portals, and operators with `execute in
  <dimension> run tp`. The players in the level they leave stop seeing them, and the ones in the level they
  arrive in start to, as on vanilla (checked against vanilla with two players, by command and through an End
  portal: `tools/test_server_dimension_players_parity.py`). The console can't send another player to a
  dimension yet (`execute in ... run tp NAME`).
- **Movement checks and chunk pacing** (release v2026.10.17.16 and newer). The server pulls back players who
  move too fast ("moved too quickly"), as vanilla does, and sends chunks as fast as each client acknowledges
  them.
- **No Docker image is published.** You build it yourself from the repository (below). Publishing an image
  needs the project owner.
- Nothing of Mojang's is downloaded or shipped. The server reads the game files you prepare from your own copy
  of Minecraft 26.3.

## 1. Install

Linux (x86-64) and macOS (Apple Silicon or Intel):

```sh
curl -fsSL https://raw.githubusercontent.com/sunrise-org/sunrise/main/packaging/server/install.sh | sh
```

Windows (x64 or ARM64), in PowerShell:

```powershell
irm https://raw.githubusercontent.com/sunrise-org/sunrise/main/packaging/server/install.ps1 | iex
```

The script picks the machine's x64 or ARM64 archive, including when PowerShell runs under x64 emulation. It downloads
the current Sunrise Nightly from GitHub, or the release `--version <tag>` names (Windows: `-Version <tag>`): a stable
release's tag, such as `26.3-0.1.0`, puts the server on that stable line, which its updates then follow. It checks it
against the release's signature with a verifier the script pins (it refuses the install if anything doesn't match),
and never asks for sudo or administrator rights. It installs:

- the release in `~/.local/share/sunrise/releases/<version>` (Windows: `%LOCALAPPDATA%\Sunrise`), with
  `current` pointing at the one in use;
- `sunrise-server` and `sunrise-prepare` in `~/.local/bin` (Windows: added to your PATH);
- a `game-files` folder for the files you prepare, kept across updates.

Options: `--version <tag>`, `--dir FOLDER`, `--bin FOLDER` (Windows: `-Version <tag>`, `-Dir FOLDER`). Other systems,
such as Linux on ARM, build from source: `cargo build --release -p sunrise-game --bin sunrise-client -p
sunrise-server` and keep both programs in one folder.

On Nix and NixOS, the repository's flake builds `sunrise-server` from source and has a
`services.sunrise-server` module: [nix.md](nix.md).

The Sunrise launcher also installs releases. Every release folder holds `sunrise-server`, so you can run it
from there too.

## 2. Prepare the game files

Sunrise reads Minecraft's data from your own copy of the game. You need Minecraft Java Edition 26.3, played
once from the official launcher on this computer, a Java 25 runtime, and the 26.3 `server.jar`: it's the
`downloads.server.url` in your launcher's `versions/26.3/26.3.json`. Then:

```sh
sunrise-prepare --prepare --server-jar /path/to/server.jar
```

Add `--minecraft-dir PATH` if your launcher keeps the game elsewhere. The prepared files go to
`game-files`, which updates keep.

**On a headless machine** (a VPS, a home server) with no Minecraft install, the launcher's command line can
download the game for your account and set the server up there:

```sh
sunrise-launcher install latest
sunrise-launcher account sign-in        # shows a code: enter it at the address it prints, on any device
sunrise-launcher prepare --download     # Mojang's files, for the signed-in account that owns Minecraft
sunrise-launcher servers set-up
sunrise-launcher servers create --name "My server" --accept-eula
sunrise-launcher servers run "My server" -- start
```

Or prepare on your own computer instead, with the launcher or `sunrise-prepare`, and copy the prepared `26.3` folder to the server with `sunrise-server import-resources DIR` (it checks the
folder and its version first), or by hand: into
`~/.local/share/sunrise/game-files/sunrise-client/26.3`, or into `resources/` in the server folder, or anywhere
`[engine] resources` in `server.toml` names. A folder copied this way needs release v2026.10.17.07 or
newer: older ones don't keep `registries.json` and `commands.json` with the prepared files, and the
server then stops with "registries.json not found next to the resources". Prepare again with a newer release.

## 3. First run

```sh
mkdir my-server && cd my-server
sunrise-server init
```

`init` asks five things and writes `server.toml`:

1. whether you agree to the [Minecraft EULA](https://aka.ms/MinecraftEULA): only a yes writes `eula=true`;
2. the world's name and an optional seed (an existing world or vanilla server folder is kept and its
   `server.properties` imported);
3. the port, 25565 by default;
4. whether to use a whitelist (yes by default, as in vanilla) and who to add;
5. whether to make the first player an operator.

It then tells you where it found the game and its files. Every question has a flag, so it also runs from a
script:

```sh
sunrise-server init --eula --world survival --seed 12345 --port 25565 --whitelist --player Alex --op Alex --yes
```

`--no-whitelist` turns the whitelist off, and `--offline` turns online mode off (see "Before you start").
The players you add are looked up as Minecraft accounts in online mode, and get offline UUIDs on an offline
server, as vanilla does.

Start the server in the foreground, with the console on your terminal (Ctrl-C stops it cleanly):

```sh
sunrise-server run
```

or in the background:

```sh
sunrise-server start
sunrise-server status
```

```text
Running, up 29 s
TPS 20.0, MSPT 1.0 ms average, 3.1 ms P95
0 of 20 players, 377 chunks loaded
RAM 1.42 GB, CPU 2 % of a core
```

Players join at your computer's address and the port (`192.168.1.20:25565`, or a tailnet name). Every
command takes `--dir FOLDER` to work on a server folder from elsewhere.

## 4. Settings: `server.toml`

You edit one file. The server writes vanilla's `server.properties` from it before every start, so the game
reads exactly what a vanilla server reads, and a vanilla server folder works as it is.

```toml
[server]
motd = "A Minecraft Server"
port = 25565
bind = ""                    # listen on every address, or only on this one (server-ip)
max_players = 20
online_mode = true           # check accounts with Mojang (see "Before you start")
whitelist = true
enforce_whitelist = false
view_distance = 10
simulation_distance = 10
max_viewed_loaded_chunks = 2048             # see "Other levels a mod shows a player" below
max_viewed_ticking_chunks = 441
max_viewed_loaded_chunks_per_level = 16384
max_viewed_ticking_chunks_per_level = 3528
admin_feedback_to_ops = true # operators see each other's commands ([name: ...]); false: only the log

[world]
name = "world"
seed = ""                    # empty: a random seed on the first start
game_mode = "survival"
difficulty = "easy"
hardcore = false

# Any other server.properties key, spelled as vanilla spells it:
[properties]
# spawn-protection = 16
# resource-pack = "https://example.com/pack.zip"   # offered to players as they join; with
# resource-pack-sha1, require-resource-pack and resource-pack-prompt, as on vanilla

[engine]
path = ""                    # the game program; empty: the one beside sunrise-server
resources = ""               # the prepared game files; empty: resources/ here, else the usual places
restart_on_crash = true
low_memory = false           # true: less memory, slightly slower ticks (see below)
parallel_tick = false        # reserved: parallel phases are disabled in this fork (see below)
droppable_clientbound = 262144  # bytes a second of droppable mod payloads sent to each player; 0 turns them off (see below)
droppable_serverbound = 16384   # the same, accepted from each player

[backups]
every = "6h"                 # 30m, 6h, 1d...; "0" turns automatic backups off
keep = 12                    # 1–1000 automatic backups kept; backups made by hand are never deleted
                             # Fix an invalid value directly here: config set must load this file first.

[updates]
keep = 1                     # other releases kept by updates, besides the one in use and the one rollback goes to

[panel]
bind = "127.0.0.1:8130"      # this computer only, or a tailnet address (100.x); "" turns it off
allow_public = false

[api]
bind = ""                    # the control API: a free port on 127.0.0.1, or a fixed one like 127.0.0.1:8131
token = ""                   # 16 characters or more; "" a new random one at every start
allow_public = false
```

The defaults are vanilla 26.3's. Unknown keys and values vanilla would refuse are errors, so a typo stops the
start instead of being ignored. When the game changes a setting itself (`/whitelist on`), the change is copied
back into `server.toml` when it stops, and your comments are kept. Changes to the file apply at the next
restart: there is no live reload yet.

One retired key is the exception: `max_view_distance` under `[server]`, the old far ring, is ignored with a note
at each start, and can be removed. For more reach, raise `view_distance` (up to 32); unlike the far ring, that
loads and, where the world hasn't been generated yet, generates the chunks it sends.

### Operators' commands: `admin_feedback_to_ops`

On by default, as vanilla: when an operator runs a command that reports to admins (`/gamemode`, `/give`, `/tp`...),
the other operators read `[name: ...]` in gray, and the console logs it. Players who aren't operators never see it
either way. `false` keeps it to the console's log (`logs/latest.log`): nobody in game sees an operator's commands,
while each operator still reads their own command's answer.


### Other levels a mod shows a player: `max_viewed_*`

A server mod (a portal mod, for example) can ask for areas of chunks in other levels for one player
(`PlayerRef::set_chunk_areas`). The server keeps them loaded and sends them to a Sunrise client that can
keep other levels; a vanilla client is sent nothing. An area either only keeps its chunks loaded (nothing
ticks there because of it) or ticks them as a player standing there would. An area counts as no player:
no mobs spawn around it, it doesn't count for sleeping, raids or spawners. Terrain generated for it is
saved like any other.

The four caps limit what mods can ask for, counted per level:

| Setting | Default | Range | What it counts |
|---|---|---|---|
| `max_viewed_loaded_chunks` | 2048 | 0 to 65536 | Chunks kept loaded for one player in one level, the loading ring around each area included. A client keeps at most 4489 columns of a viewed level: a higher value is lowered to 4489 at startup, with a warning in the log. A client of an older Sunrise (before revision 4 of viewed levels) gets at most 2048 |
| `max_viewed_ticking_chunks` | 441 | 0 to 65536 | Of them, the ticking ones: 441 is one player's simulation area at distance 10 |
| `max_viewed_loaded_chunks_per_level` | 16384 | 0 to 1048576 | Loaded chunks of every player together in one level |
| `max_viewed_ticking_chunks_per_level` | 3528 | 0 to 1048576 | Ticking chunks of every player together in one level |

0 refuses every area of that kind. A request over a cap is refused whole, and the mod is told which cap and
its value. Areas end when the player leaves.

### Levels a mod opens: `sunrise-max-runtime-levels`

A mod can open levels of its own while the server runs ([runtime levels](modding/src/how-to/world/runtime-levels.md)).
The server keeps at most 64 of them open at once, for all mods together; the 65th request is refused
with `CapReached`. Set another number under `[properties]` in `server.toml`:

```toml
[properties]
sunrise-max-runtime-levels = 128
```

A level frees its place when its mod unloads or deletes it (a delete also removes its folder from `dimensions/`). After 300 ticks with nobody in it, it leaves memory, as any custom level does, but keeps its place: it opens again when someone enters it.

Already have a vanilla server? Run `sunrise-server init` in its folder, or `sunrise-server import
server.properties` to write `server.toml` from it. The world, `ops.json`, `whitelist.json` and the ban lists
keep vanilla's names and formats, so going back to vanilla works too.

## Native server export

A native-mods build exports a standalone engine
(`sunrise-launcher native-build FOLDER --server-output DIR --version TAG --trust-native-code`;
see [Native mods](modding/src/how-to/native-mods/12-standalone-server-export.md)). Point the server at it:

    sunrise-server --dir SERVER config set engine.path=/absolute/export/sunrise-engine
    sunrise-server --dir SERVER restart

Use the absolute engine path printed by the build. Build on the same OS and architecture as the server
(no cross compilation), using a prepared installed launcher release. Updates do not rebuild a pinned `engine.path`: build a new
export folder, set `engine.path` to it, restart, and keep the old folder for manual rollback. The supervisor does
not auto-update or rebuild a pinned engine path, and there is no compatibility handshake yet (follow-up #6304).
Native mods run unsandboxed: only build code you have read and trust.

## 5. Commands

```text
sunrise-server init                      first run
sunrise-server run                       in the foreground, with the console; Ctrl-C stops it
sunrise-server start | stop | restart    in the background (its output goes to logs/supervisor.log)
sunrise-server status [--json]           up or not, TPS, MSPT, players, chunks, RAM, CPU
sunrise-server cmd COMMAND...            one console command, and what it answers
sunrise-server logs [-f] [-n N]          the log; -f follows it
sunrise-server console                   attach: the live log, TPS/players at the top, type commands; Ctrl-D detaches
sunrise-server mcp                       an MCP server on stdio, so an AI agent can run and moderate the server
sunrise-server report [SECONDS]          record a performance report (below)
sunrise-server op|deop NAME
sunrise-server whitelist on|off|list
sunrise-server whitelist add|remove NAME
sunrise-server players [--json]          who is online, the operators, the whitelist and the bans
sunrise-server kick|ban NAME [REASON]    on the running server
sunrise-server pardon NAME
sunrise-server ban-ip ADDRESS|NAME [REASON]
sunrise-server pardon-ip ADDRESS
sunrise-server backup [NAME]             back up now
sunrise-server backups [--json]          list the backups
sunrise-server restore BACKUP [--dimension ID] [--yes]
sunrise-server service install|uninstall|print [--system]
sunrise-server panel                     a one-time login link for the web panel
sunrise-server import FILE               server.toml from a vanilla server.properties
sunrise-server import-resources DIR      copy prepared game files into place (step 2)
sunrise-server config [--json]           the settings in server.toml
sunrise-server config set KEY=VALUE... [--json]
                                         change settings, checked first (below)
sunrise-server update [--check [--json]] [--version TAG] [--timeout SECONDS]
                                         install the newest release and restart on it (below);
                                         --archive FILE --version NAME installs an archive you have
sunrise-server rollback [--timeout SECONDS]
                                         back to the release used before
```

While the server runs, `op`, `whitelist` and `cmd` go to the game's console, so they act exactly as vanilla's
console commands do. While it's stopped, they edit vanilla's files directly. `kick` runs the vanilla command
on the running server, through its control API; `ban`, `pardon`, `ban-ip` and `pardon-ip` do the same while it
is running and edit the vanilla files directly while it is stopped. `players` reads the
lists from the files when the server is stopped. Any console command works through
`cmd`: `sunrise-server cmd ban Griefer`, `sunrise-server cmd say Restarting in 5 minutes`.
To generate the world around spawn ahead of the players, Sunrise has its own `pregen` command, off until
`server.toml` has `sunrise-pregen = true` under `[properties]`: `sunrise-server cmd pregen start 2500`
([pregeneration](pregeneration.md)).
To let only the Sunrise client join, set `sunrise-clients-only = true` under `[properties]` (off by default).
Other clients are turned away while they connect with "This server is for the Sunrise client:
playsunrise.net", and the console says which client it was. The check reads the brand a client names itself
by (`minecraft:brand`), which any client can fake: it keeps out players who didn't install Sunrise, not
cheaters. The server's own checks (reach, movement) are what protect it.

Sunrise clients can get their far terrain (Video Settings → LOD Distance) from the server: set
`sunrise-lod-distance` under `[properties]` to how far, in chunks (1024 at most), and the server samples the
tiles each player's client asks for out to there. It is off (0) by default: far tiles show the world's terrain
far past the view distance, which helps players find the seed. A player's own LOD Distance applies when it is
shorter. Each player may have 8 tiles waiting and ask for 64 a second on average, and one background thread
samples them for everyone in turn, behind the ticks, so far terrain never slows the server down; a tile past the
distance or over a player's limits is refused, and a client that keeps breaking the limits is disconnected.
Tiles are sampled again for each player who asks (there is no cache yet). Vanilla clients never hear of it.

`sunrise-server report` records the running server for 60 seconds (`report 300` for five minutes, up to 600) into
`reports/profile-<time>Z.hzperf`, a capture the performance tools read ([perf-platform.md](perf-platform.md)), and
prints a short summary (TPS, MSPT, CPU, memory, players and the heaviest tick phases) and where it went. On the console it is `sunrise-report [SECONDS]`; `sunrise-report stop` ends it now and
`sunrise-report status` says how far it is. Nothing restarts: the report runs beside the game.

The capture holds the settings, the machine, the Sunrise switches and the few game rules that change what a tick
costs, the health history from before it (TPS and MSPT over the last 10 minutes to 7 days), a sample each second,
the counters and the slow ticks with their steps. It says where the ticks' time went, phase by phase (mobs, block
ticks, block entities, the network and the chunks it sends, saving...), in blocks of five seconds and as totals,
with each dimension's share of the tick (`level_*`) and the 16 slowest ticks with their whole step tree. It counts
the chunks, entities and block entities of each dimension at the start and at the end, and gives what each mob type
costs per dimension (one tick in four times every mob: ms a tick, mobs, µs a mob), the chunk workers' CPU by job
(load, generate, decorate, finish, light), the bytes each player sends and receives in each block, and what recording
cost the tick (measured, usually well under 1 % of MSPT). It holds no names, UUIDs or addresses (players are
numbered in the order they joined), no seed and no world name, so you can attach it to an issue as it is. Past 30
files or 1 GiB the oldest `profile-*.hzperf` is deleted; a file you renamed is never touched. On Linux the tick thread is also sampled 250 times a second (about 0.05 % of the tick thread), and the capture
holds where the CPU time went, by function and by what the server was doing (step, entity type, block, dimension,
chunk); `--no-samples` leaves that out. Finishing the file after the window can take a few seconds and does not stall
ticks. The old `perf-<time>.json` and `.txt` files are no
longer written; the ones already in `reports/` stay.

`python3 tools/server_bench.py --profile` records each Sunrise run's measure window this way (`--profile=--no-samples`
passes flags to `sunrise-report`) and adds the run's record, MSPT, RSS, CPU and chunk figures, to the capture with
`sunrise-perf attach FILE --name NAME --body JSONFILE`, which keeps whatever else the file holds. Build
`-p sunrise-perf` beside the server first; the capture's path is the run's `profile` in the report.

To see a slow stretch tick by tick, `sunrise-trace [SECONDS]` (30 by default, up to 600) writes a capture with every
thread's timed steps to `reports/profile-<time>Z.hzperf`, one row per thread. `sunrise-trace stop` ends it early, and
`sunrise-report --timeline` adds the same steps to a report. The steps are recorded for the first 60 seconds of a
capture, or until they pass 256 MiB; the capture then says when they stopped. `SUNRISE_TRACE=FILE` in the server's
environment writes such a capture to FILE from the first tick. It holds step names and times only.

To profile your server from a game, join it as an operator of level 4 and run `sunrise-ctl profile server start` (or `profile start`, which captures your game and the server in one file). Another operator level is refused and nothing is written. One server capture runs at a time. The server writes its copy to its own `reports/`, logs who started it and when, and sends the file to your game in the background, at most 256 KiB a second and after gameplay packets; the game says where it saved it. Open a capture with `sunrise-perf` ([perf-platform.md](perf-platform.md)), which you build from this repository (`cargo build --release -p sunrise-perf`).

Between reports, the server keeps its last 256 slow ticks in memory, each with its eight largest steps. A tick
counts as slow from 250 ms (`SUNRISE_SPIKES=MS` in the engine's environment sets another threshold), and every
capture carries the ring as it stood at its start and adds the slow ticks of its own window. The log no longer has a
line for them: `sunrise-perf query spans --slow` says which steps they spent their time in, and `query ticks --worst 5`
lists the five worst with their step trees (the newest capture, or name a file last). When a slow tick spent 10 ms or more on one player's packets or one event, the log says so in a `Sunrise: … ms tick was busy with:` line with the player's name; the capture never holds names. A server more than two seconds behind still says `Can't keep up!` as
vanilla's does, at most every 15 seconds.

`config set` changes `server.toml` without opening it: `sunrise-server config set world.difficulty=hard
server.max_players=10`. Each key is a section and a name from the file (`properties.spawn-protection=0` for any
other vanilla key). The values are checked as the server checks them, and nothing is written if one is wrong.
A running server takes the difficulty, the default game mode, the whitelist, the message of the day, the
player limit and the backup settings at once; the rest applies at its next start, and `config set` says which. `--json` prints that as `saved`, `applied`
and `restart`.

A crash save keeps the last saved files (`players/data/<uuid>.dat`, stats and advancements) of a player whose turn
the panic interrupted, such as a mod's hook panicking while it handled that player's action: the half-done turn can
hold an item taken but not yet placed. The log says so with a `player's turn` warning, and the panic still
ends the server. Single player keeps the owner's file the same way.

The server restarts the game by itself if it crashes: after 5 s, then waiting longer each time, up to 5
minutes. It keeps the crash report and gives up after 5 crashes in 10 minutes, and `status` says why.

**Updates** (a server installed with the install script). The server checks for a newer release a minute
after it starts and every 6 hours after, and says so in its log, in `status` and in the panel; it never
updates by itself. `sunrise-server update --check` (add `--json` for machine output) asks once. `sunrise-server update`:

1. downloads the newest build of the server's channel and checks it against its signed SHA-256: on a stable release
   (`26.3-0.1.0`), the newest stable release of its line (the same Minecraft version and major, `26.3-0.x`),
   never a Nightly or the next major; on any other build, the current Nightly;
2. stops the game, backs up the world (`backups/<time>-before-<release>.zip`) and switches to the new
   release;
3. starts the game on it and waits up to 5 minutes (`--timeout SECONDS`) for it to come up.

If the new game crashes or doesn't come up in time, the server goes back to the release it was on by itself
and puts the world back as it was just before the switch (the world the new game left is kept in
`backups/`). `sunrise-server rollback` goes back to the release before by hand, keeping the world as it is
(it backs it up first). `--version <tag>` installs a given release (moving to another line or channel too), and `--archive FILE --version NAME` an
archive you have, such as your own build. A server still on a release from before the last dated one doesn't know
that 0.2 comes after the dated releases and says it is up to date: run `sunrise-server update --version v0.2.0`
once ([versioning](versioning.md)). A stopped server is switched and starts on the new release next
time. Set `SUNRISE_NO_UPDATE_CHECK=1` in the server's environment to turn the check off.

Release checks and downloads, and the Discord webhook, use the first nonblank setting in this order:
`HTTPS_PROXY`, `https_proxy`, `ALL_PROXY`, `all_proxy` for HTTPS, or `HTTP_PROXY`, `http_proxy`,
`ALL_PROXY`, `all_proxy` for HTTP. A proxy address without a scheme means HTTP. Supported schemes are
`http://`, `socks4://`, `socks4a://`, `socks5://`, `socks5h://` and `socks://` (an alias for SOCKS5).
An `https://` proxy address is not supported, even for HTTPS downloads. HTTP proxy credentials are sent
only on HTTPS CONNECT, not for plain HTTP requests.

Local addresses bypass the proxy. `NO_PROXY` (or `no_proxy` if `NO_PROXY` is unset) is a comma-separated
list: `*` bypasses all hosts; a host or domain (`example.org`, `.example.org`, `*.example.org`) matches
that name and its subdomains. The list accepts IP literals, including bare or bracketed IPv6, but not CIDR.
An entry with a port (`example.org:8443` or `[2001:db8::1]:8443`) matches only that destination port,
including the URL scheme's default port when no port is written. Release checks and downloads
select the route again at each redirect.

## 6. Web panel

The panel runs inside `sunrise-server` while the server runs. It shows live TPS, MSPT, RAM and CPU graphs for
the last 10 minutes, the players with their ping (and a kick button), the console with a command line, and the
backups (back up now, restore with a confirmation), with Restart and Stop. When a newer release is out it says
so, with an Update button that runs `sunrise-server update` (section 5), and shows how it went.

```sh
sunrise-server panel
```

prints a login link that works once, for 10 minutes. The panel listens on `127.0.0.1:8130`, this computer
only. To reach it from your other devices:

- put your tailnet address in `[panel] bind` (for example `bind = "100.101.102.103:8130"`), or
- forward the port over SSH: `ssh -L 8130:127.0.0.1:8130 your-server`, then open the link on your computer.

Any other address, such as `0.0.0.0` or a public IP, is refused unless you set `allow_public = true`, and
even then the panel warns at every start. With `bind = "0.0.0.0:8130"` it answers on every address of this
computer, so a LAN whose router hands out a new IP keeps working: open `http://<its LAN IP>:8130`, and the
login link names that IP. It has no passwords to leak: whoever can read the server folder can
make a login link, and no one else can. Never put it on the open internet.

### A bot or a tool of your own

Everything the panel does goes through the control API ([api/server.md](api/server.md)), which a moderation bot
(a Discord `!ban`), a monitoring tool or a script can call too. It listens on a free port of `127.0.0.1` and
writes the address and a fresh token to `.sunrise/api.json` at every start. For a program that needs a fixed
address, set `bind` in `server.toml`:

```toml
[api]
bind = "127.0.0.1:8131"      # or your tailnet address, 100.x.y.z:8131
token = ""                   # empty: generate a fresh token at every start
```

If the program needs a fixed token too, set `token` to a private random string of at least 16 printable ASCII
characters without spaces.

The same rules as the panel's hold: a tailnet or loopback address, unless `allow_public = true` (the token is then
all that guards the server, and it warns at every start). The API can't change `[engine]`, `[panel]`, `[api]` or
`[discord]`, nor turn on RCON or another remote console.

### Chat in a Discord channel

`[discord]` posts the server's chat, joins, leaves and deaths to a Discord channel. In the channel's settings,
Integrations → Webhooks → New Webhook → Copy Webhook URL, then:

```toml
[discord]
webhook = "https://discord.com/api/webhooks/..."   # a secret: whoever has it can post in the channel
events = ["chat", "join", "leave", "death"]        # and "kick", "announce" (lines native mods post)
enabled = true                                     # false pauses it and keeps the webhook
```

The lines of each 2 seconds are drawn as one picture of the in-game chat (transparent, shadowed letters, colours
included), and a channel gets at most one post every 2 seconds (more when Discord asks for it: a burst beyond 20
lines is counted, not drawn). Only `chat` is forwarded. Posts
never ping: they allow no mention, and an `@everyone` typed in game reaches Discord defused. Every post comes
from the webhook's own name and avatar (set them in Discord), never a player's; players' addresses are never sent,
and a player is always named by their account's name (a nickname only next to it). The server only makes outgoing HTTPS calls to Discord;
the API and the panel show the webhook as `(set)`. The letters are Pixelify Sans (SIL Open Font License,
`crates/sunrise-theme/fonts`), not Minecraft's font.

### Tools written for vanilla's management protocol

Vanilla 26.3 has its own management protocol, JSON-RPC over a WebSocket, and Sunrise's server speaks it too, so a
tool written for a vanilla server manages a Sunrise one. It's off by default; turn it on in `server.toml`:

```toml
[properties]
management-server-enabled = true
management-server-port = 8132          # 0: a free port, logged at the start
# management-server-secret: 40 letters and digits; left out, the server makes one and writes it here
# management-server-tls-keystore = "management.p12"   # your certificate (PKCS12); left out, see below
# management-server-tls-keystore-password = "..."     # or MINECRAFT_MANAGEMENT_TLS_KEYSTORE_PASSWORD
```

It listens on `localhost` (`management-server-host`). A client sends the secret as `Authorization: Bearer <secret>`,
or, from a browser, in the WebSocket subprotocol (`minecraft-v1,<secret>`) from an origin listed in
`management-server-allowed-origins`. `rpc.discover` lists the methods; Sunrise answers more of them with each release
([api/server.md](api/server.md#not-there-yet)).

TLS is on, as in vanilla (`wss://`). The certificate and its key come from a PKCS12 keystore, as vanilla's do:

- `keytool -genkeypair -storetype PKCS12 ...` makes one, or `openssl pkcs12 -export` turns a PEM certificate and
  key into one.
- Its password comes from the `MINECRAFT_MANAGEMENT_TLS_KEYSTORE_PASSWORD` variable, else from
  `management-server-tls-keystore-password`.

Where vanilla refuses to start without a keystore, Sunrise makes one:

- a self-signed certificate for `localhost`, `127.0.0.1` and `management-server-host`, saved in
  `.sunrise/management-keystore.p12`, which only the server's user can read;
- its path and password are written to `[properties]`;
- each start logs the certificate's SHA-256 fingerprint, which a client pins or adds to its trusted certificates.

Keep the protocol on `localhost` or your tailnet even so. `management-server-tls-enabled = false` turns TLS off.
As in vanilla, a wrong secret or keystore stops the server from starting.

### Tools that speak RCON

Hosting panels, Discord bridges and scripts often run commands through RCON, vanilla's remote console, and Sunrise's
server serves it. It's off by default and needs a password; turn it on in `server.toml` (the dotted keys are
quoted in TOML):

```toml
[properties]
enable-rcon = true
"rcon.port" = 25575
"rcon.password" = "a long random string"
```

It listens on the address `[server] bind` names (`server-ip`; every address when it's empty), as vanilla's does.

A client logs in with the password and each command runs as one typed in the server's console, with the same
output (`sunrise-server cmd` does the same from the shell). RCON isn't encrypted: the password and every command
travel in clear text, and it gives full control of the server. Keep its port closed to the internet: a firewall, a
tailnet or an SSH tunnel. As in vanilla, an empty password or a port outside 1 to 65535 leaves RCON off with a
warning in the log, and a port already in use is a warning too, not a server that doesn't start. The settings are
read when the server starts. To turn it off again, set `enable-rcon = false`: taking the lines out of
`server.toml` leaves what `server.properties` already holds, as for any other key.

Where it differs from vanilla: a connection that hasn't logged in within 30 seconds is closed, at most 16 are open
at once, and a command a client's TCP splits in two still works.

A command runs from a source named `Rcon`: operators in game see what it did as `[Rcon: ...]` (gray, italic) unless
`broadcast-rcon-to-ops` is `false`, and a `say` shows as `[Rcon] ...`.

### Tools that read the query

Server lists, status pages and monitoring tools (`mcstatus`, Prometheus exporters, hosting panels) often read the
MOTD, the version and who is online through vanilla's GS4 query, a UDP port, and Sunrise's server serves it. It's off
by default; turn it on in `server.toml`:

```toml
[properties]
enable-query = true
"query.port" = 25565
```

It listens on UDP on the address `[server] bind` names (every address when it's empty), as vanilla's does: the same
number as the game's TCP port is fine, they are different ports. The query tells anyone who can reach the port the
player names and the version, and has no password: open the port only if you want that. As in vanilla, a port
outside 1 to 65535 leaves it off with a warning in the log, and a port already in use is a warning too. The
settings are read when the server starts. To turn it off again, set `enable-query = false`.

Where it differs from vanilla: while the game isn't running (starting, restarting) nothing answers, and who is
online is read from the game at most once a second.

## 7. Backups

```sh
sunrise-server backup before-the-update
sunrise-server backups
sunrise-server restore before-the-update
```

A backup zips the world, vanilla's files and `server.toml` into `backups/`. A running server saves first and
stays up: players don't notice. Automatic backups run on the `[backups] every` schedule while the server runs,
and the oldest automatic ones beyond `keep` are deleted. Backups you make by hand are never deleted.

`restore` takes a backup's file name, a unique start of it, or the name you gave it. It moves the current
world aside to `backups/replaced-<time>/` (never deleted), puts the backup in place, and restarts the server if
it was running. Add `--dimension ID` (like `minecraft:the_nether`) to bring back only that dimension and leave
the rest of the world as it is. The panel does the same with one click.

### A smaller world: `[prune_chunks]`

Players who travel far leave a trail of chunks nobody comes back to. This deletes the chunks players spent
less than a minute near (vanilla's `InhabitedTime`, 1200 ticks), so they generate again from the seed, as they
were, if someone returns. Back up first. Add to `server.toml`, then restart:

```toml
[prune_chunks]
mode = "dry-run"             # "dry-run": only the report, the game starts at once; "delete": before the game starts
min_inhabited_ticks = 1200   # chunks players spent less than this near go
spawn_radius = 32            # chunks this close to the spawn stay (0 0 in the Nether and the End)
ring = 1                     # chunks this close to a kept chunk stay, so no seam opens beside a build
keep_files = ["sunrise-smp/claims.json", "sunrise-smp/places.json"]   # in the world folder: claims, homes
```

After the run, `mode` is set back to `"off"` and `logs/prune-chunks-<time>.txt` says, per dimension, how many
chunks went (or would go) and how much disk was freed. The chunk's entities and points of interest go with it;
region files are rewritten compacted (vanilla still reads them) and a file left empty is deleted. Chunks that
can't be read, files in `keep_files` (keys or text like `minecraft:overworld 12 -3`, objects with `dimension`,
`x` and `z`), level.dat and player data are never touched; a `keep_files` file that exists but can't be read
stops the prune before anything is deleted.

Deletion stages each region's terrain, entities and POI together, keeping originals in the dimension's
`.sunrise-prune` folder until it commits. An interrupted region is rolled back before the next server
start, even though `mode` is now `"off"`; committed earlier regions stay pruned. A failed prune or failed
recovery stops startup and reports an error instead of reporting planned deletions as successful. Keep
the recovery folder if recovery fails; do not delete it to force the server to start. Staging retains
hard links to the originals and needs room for the compacted files of one region group. The three
folders must support hard links on the same filesystem. Unix flushes files and directories; Windows
flushes files and uses write-through renames. This does not replace a backup or promise recovery from
filesystem corruption or storage hardware that ignores flushes.

### Mods' biome regions: `[biome_regions]`

A world made with a mod that adds a [biome region](modding/src/how-to/content-mods/biome-regions.md) saves that region with the
world. If the mod is later removed, the server refuses to start and names the mod and region. To start anyway with
vanilla's biomes where that region was (chunks that already exist stay as they are), add to `server.toml`, then
restart:

```toml
[biome_regions]
missing = "vanilla"      # "refuse", the default: don't start
adopt_installed = false  # true explicitly adopts installed regions for new generation
```

The choice for a missing mod is saved in the world, so it holds even if the setting is removed later.
Substitution keeps the missing region's slot and weight; a returning mod does not undo it.
Adoption keeps saved weights for returning regions, takes declared weights for new regions and
drops absent regions. Completed chunks keep their saved biomes and blocks. New generation may
differ in surface material, carving, features and structures. The
[design note](design/mod-api/mod-biome-regions.md) lists record limits and fixture coverage.

### Worlds saved with mods that are gone: `[missing_mods]`

A world records the mods it was saved with: code mods and content mods, and the namespaces of the mods' blocks.
If one of them is no longer loaded, opening the world turns that mod's blocks into air and removes what they hold,
and the next save makes it permanent. So the server refuses to start that world: it logs one line per missing mod
and a last line naming the switch, and changes nothing in the world. To open it anyway, back the world up, then add
to `server.toml` and restart:

```toml
[missing_mods]
allow = true   # false, the default: don't start
```

Started as `sunrise-client --dedicated` directly, `--allowMissingMods` does the same for one start. With either one,
the server logs the same list as a warning and opens the world. A mod whose `mod_mismatch` listener marks a missing
mod resolved (a migration) takes it off the list.

## 8. Start it with the computer

```sh
sunrise-server service install
```

- **Linux:** a systemd user unit (`--system` for a system unit, with sudo from your own account: the unit runs
  the server as that account, never as root). To keep a user unit running while
  you're logged out: `loginctl enable-linger $USER`.
- **macOS:** a launchd agent in `~/Library/LaunchAgents`.
- **Windows:** a task that starts at logon (a real Windows service comes later).

The service runs `sunrise-server run` through the installed `current` release, so an update applies at the
next restart. `service print` shows what it would install, `service uninstall` removes it and leaves the server
folder alone.

## Hosting panels (Pterodactyl, AMP)

The egg installs the server build of the current Nightly (nothing of Mojang's) and runs `sunrise-server`
through `packaging/server/panel-start.sh`: the first start writes `server.toml` from the panel's settings, and the
port always follows the panel's allocation. Prepare the game files on your own computer (section 2) and upload the
prepared `26.3` folder's contents to `resources/` in the server's files. Set the EULA setting to true before the
first start: until then the server stops and says why. The settings (world, seed, whitelist, players, operator,
online mode) are read on the first start; afterwards edit `server.toml`. The panel's Stop sends `stop`, which saves
the world first.

Import `packaging/server/pterodactyl-egg.json` under Nests. Its Docker image is
`ghcr.io/ptero-eggs/yolks:debian`. The `SUNRISE_VERSION` variable picks the release to install (`nightly`, or a tag);
it is read at install and reinstall.

**AMP** (CubeCoders): `packaging/server/amp/` is a template for AMP's generic module, the same setup with AMP's
words. Its update runs the egg's install script (the release named by "Sunrise release", checked against its
SHA-256) and the start runs the same `sunrise-start.sh`, with the port from AMP's "Server Port". AMP reads templates
from the root of a configuration repository: put the five files at the root of a repository of yours (a fork of
CubeCoders/AMPTemplates, for example), add it under Configuration → Instance Deployment → Configuration Repositories,
then Fetch latest, and "Sunrise" is in the list of applications for a new instance. Linux only, x86-64.

## 9. Docker (build it yourself)

No image is published: build one from the repository. The image holds none of Minecraft's files; you mount the
folder you prepared (step 2) read-only.

```sh
git clone https://github.com/sunrise-org/sunrise && cd sunrise
docker build -f packaging/server/Dockerfile -t sunrise-server .

docker run -d --name sunrise -p 25565:25565 --stop-timeout 180 \
  -v sunrise-data:/data \
  -v /path/to/26.3:/opt/sunrise/.local/sunrise-client/26.3:ro \
  -e SUNRISE_EULA=true -e SUNRISE_WHITELIST=true -e SUNRISE_PLAYERS=Alex,Steve -e SUNRISE_OP=Alex \
  sunrise-server
```

- `SUNRISE_EULA=true` accepts the [Minecraft EULA](https://aka.ms/MinecraftEULA) for this server. Without it
  the container stops and says why.
- The first start writes `/data/server.toml` from `SUNRISE_EULA`, `SUNRISE_WORLD`, `SUNRISE_SEED`,
  `SUNRISE_PORT`, `SUNRISE_WHITELIST`, `SUNRISE_PLAYERS`, `SUNRISE_OP` and `SUNRISE_ONLINE`. Later starts use
  the file.
- The mounted folder must come from release v2026.10.17.07 or newer (see step 2).
- Manage it with `docker exec sunrise sunrise-server status` (and `cmd`, `backup`, `logs`, ...).
  `docker stop` saves and stops cleanly; `--stop-timeout 180` gives it the time.
- The panel stays on 127.0.0.1 inside the container.
- The image still carries ALSA and udev libraries because the game links them, even though a server never plays
  a sound. A smaller engine without them is planned.

## 10. How fast is it

The latest bot benchmark compares Sunrise with vanilla 26.3, Paper and Fabric + Lithium:
[benchmarks/server-four-way-2026-10-19](https://github.com/sunrise-org/sunrise/blob/main/benchmarks/server-four-way-2026-10-19/README.md).
It ran release v2026.10.19 on a 4-thread laptop CPU (i7-7500U), with 1 to 50 players walking, exploring and
standing around spawn, the bots on another machine. All four servers did the same work. Against
Pumpkin: [performance/server-vs-pumpkin.md](performance/server-vs-pumpkin.md). In short:

- **Memory:** 0.7 GB with one player and 1.1 GB with 50, against 3.4 to 4.5 GB for the Java servers
  (each with a 4 GB heap): 3.8 to 4.8 times less.
- **Start:** ready in 8 to 9 s, against 15 s for vanilla, 17 s for Fabric and 19 to 22 s for Paper.
- **CPU:** from 10 players up, the least of the four (1.4 cores at 50, against Paper's 2.1 and Fabric's 2.4).
- **Average tick:** Paper's is shorter at every count (10.3 against 13.7 ms with one player, 46.9 against
  47.8 with 50), and Fabric + Lithium's with 1 and 10 players. Sunrise's is shorter than vanilla's (equal
  with 10), and than Fabric's from 25 players up.
- **Slow ticks:** from 10 players up, Sunrise's slowest ticks (99th percentile) are the shortest of the
  four (73 ms with 50, against 110 for Paper and 174 for Fabric).
- **50 players:** Sunrise and Paper keep 20 TPS, Fabric + Lithium drops to 17, and vanilla's watchdog
  stopped it.

For more than 50 active players, give it more cores.

### Less memory: `low_memory`

Off by default. On Linux, the game's allocator (mimalloc) backs its memory with transparent huge pages
(2 MB each) where the system allows them. Many of those pages are only partly used, so the server holds
more memory than it needs, but the tick runs a little faster. Under `[engine]`, `low_memory = true` starts
the game with huge pages off. Measured on a 4-core EPYC VPS with 20 bots playing (2026-11-01):

| | Default | `low_memory = true` |
|---|---|---|
| Memory, idle after start | ~300 MB | ~245 MB (−60 MB) |
| Memory, 20 players | ~1180 MB | ~980 MB (−190 MB) |
| Average tick, 20 players | 27.1 ms | 27.5 ms (+1.2 %) |
| Start | 2.1 s | 2.2 s |

Turn it on when memory is what you're short of, for example on a small VPS. It applies at the next start.
A game started without `sunrise-server` (`sunrise-client --dedicated`, a container of your own) gets the same
effect from the environment variable `MIMALLOC_ALLOW_THP=0`. On macOS and Windows it changes nothing.

### Parallel tick: `parallel_tick`

**Fork limitation:** all opt-in parallel server phases are currently disabled, including when this
setting is true or `SUNRISE_PARALLEL_TICK` is set. A request logs a warning and uses serial execution.
The upstream behavior described below is retained for reference, not currently available on the fork.
Typed hook-aware parallel execution is tracked in [#1532](https://github.com/sunrise-org/sunrise/issues/1532).

Off by default. Under `[engine]`, `parallel_tick = true` starts the game with its parallel tick on: the
phases of the server tick that give the same result in any order share their work among helper threads (at
most 8, as many as the machine has cores). It applies at the next start. It is the same switch as the
environment variable `SUNRISE_PARALLEL_TICK=1`, for a host that gives you no way to set one (a game panel
with a fixed startup command). With the setting off, a `SUNRISE_PARALLEL_TICK` already in the server's
environment still applies.

### Droppable mod payloads: `droppable_clientbound` and `droppable_serverbound`

A mod can declare a payload channel as droppable (voice chat, live data): under congestion the game drops
those sends, instead of letting them queue behind chunks and arrive late ([the modding
book](modding/src/how-to/client/networking.md)). Under `[engine]`, `droppable_clientbound` is the bytes a
second of them the server sends to each player (default 262144, 256 KiB) and `droppable_serverbound`
the bytes a second it accepts from each player (default 16384, 16 KiB). For voice, tune these budgets
for the traffic your mod's codec and relay produce; `0` turns droppable delivery off in that direction.
A player over the limit loses those sends, which are counted, and is not kicked; nothing else on the
connection is ever dropped. Vanilla's `rate-limit` (off by default) still counts droppable payloads as
packets. Changes to the budgets apply at the next start.

## When something goes wrong

- **"no prepared game files"**: `init` and `run` list the folders they looked in. Prepare (step 2), or set
  `[engine] resources`.
- **"registries.json not found next to the resources"**: the prepared folder is from an older release. Prepare
  again with a newer one.
- **Nobody can join**: the whitelist is on by default (`sunrise-server whitelist add NAME`), and the port must be
  open in your firewall. On a tailnet, use the server's tailnet address.
- **It keeps restarting**: `sunrise-server status` shows the last crash and `sunrise-server logs -n 200` the log
  before it. Crash reports are in `crash-reports/`.
- Report problems at <https://github.com/sunrise-org/sunrise/issues>, with the end of the log.
