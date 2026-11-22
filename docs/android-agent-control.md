# Agent control on Android

**Agent Control** lets a program on the phone drive the game. That program can be an AI agent, a
test script or an accessibility tool. Once connected, it can:

- read the game state;
- send commands;
- take screenshots.

Agent Control is off by default. The player turns it on in **Options → Horizon Settings → AI Agent →
Agent Control**, and **No AI** keeps it off whatever else says.

## Where the endpoint is

With Agent Control on, the game listens on `127.0.0.1` (never the network) and writes the endpoint
file `horizon-control.json` in the **game folder**. The game folder is the one the player picked for
Starling's files, such as `/storage/emulated/0/Starling`. When the app was never given storage
access, the file goes to the app's private files instead, where only Starling can read it.

The file holds three things, and changes every time the game starts:

```json
{"transport":"tcp","port":43817,"token":"…"}
```

The game deletes the file when Agent Control is turned off or the game closes. A file left behind
by a crash is replaced at the next start.

**Copy Connection Details** on the AI Agent page copies the file's path and these instructions. It
leaves the token out, so the token never reaches the clipboard.

## Connecting

The first line must be `auth TOKEN`. The game answers `{"ok":true}`, then reads one command per line
and answers each with one line of JSON. Send `help` for the list of commands.

From Termux, with `jq` and `netcat-openbsd` installed and storage access granted
(`termux-setup-storage`):

```sh
f=/storage/emulated/0/Starling/horizon-control.json
port=$(jq -r .port "$f")
token=$(jq -r .token "$f")
{ echo "auth $token"; echo status; sleep 1; } | nc 127.0.0.1 "$port"
```

From a computer over USB, forward the port with `adb forward tcp:PORT tcp:PORT` and use the same
protocol. To use a port that doesn't change between starts, set `HORIZON_CONTROL_PORT`.

## What to keep in mind

The token is the only lock on the endpoint, and the file sits in shared storage so that an agent on
the phone can find it. While Agent Control is on, **any app allowed to read all files can read the
token and connect**. Those are apps holding "All files access", or older apps with storage access.
The AI Agent page says this under the switch.

- Turn Agent Control on only while you use an agent, and off when you're done. The file goes away
  with it.
- Don't grant "All files access" to apps you don't trust while it is on.
- An app without storage access can still open a connection to the port, but without the token the
  game refuses it at the first line.
