# Forking: Sunrise's extras and this fork

Upstream introduced a `sunrise-extras` Cargo feature in
horizon-rust/horizon#12055 to group its cosmetics, emotes, presence, server-list
promotions and settings pages. This fork deliberately differs: excluded features
are removed, not compiled behind a switch.

Cosmetics, emotes, the Wardrobe and the Shop, partner ads, sponsor links and the
official server are not part of this fork: Sunrise has no paid features, shop or
ads. No Cargo feature restores them, and the engine has no hook to draw them: the
player-model hooks cosmetics used (name decorations, the player-list mark, meshes
and poses on player models, the camera pull) were removed with them. CI's public-tree check
(`tools/check_public_tree.py`, run by `verify-static`) fails a branch that
brings back `crates/sunrise-cosmetics`, the game's `cosmetics` module,
`crates/sunrise-official-servers`, the
launcher's `partner`, `shop`, `wardrobe` and `wardrobe_page` sources, or
the launcher's `sculk-*` assets (in sunrise-launcher, which has its own check). It looks at the current tree, not at history, so the
commits from before the removal stay allowed. The Mods and
Sunrise Settings pages remain available independently of the client's default
features; disabling audio or controllers does not hide mod management.

## Build features

The client's default features are `audio`, `controllers` and `friends`.
Play with Friends remains independently selectable. For example, a client without
audio or controllers can be built with:

```sh
cargo build --release -p sunrise-game --no-default-features --features friends
```

The dedicated server uses `--no-default-features`. Neither build mode introduces
a telemetry placeholder or an official-build promotion in the options screen.

## Modding and rendering

| Extra | Where it lives | The gate |
|---|---|---|
| The "Start Sunrise from the launcher" screen of a game without its prepared files, with its Download button (`get_the_game` hook) | crate `sunrise-official` | registered only by the official executable (`sunrise-official`); sunrise-client's own executable has none |
| Sunrise Settings (Extras, PvP & HUD, Studio, AI Agent) and Mods (with the registry browser) | `options_ui.rs` (`Page::Sunrise` and below), `extras_ui.rs`, `extras.rs`, `pvp.rs`, `game/extras.rs` and `game/extras/`, `extras_hud.rs` | the Options screen shows vanilla's greyed Telemetry Data instead (`options_ui::SUNRISE_EXTRAS`, set in `entry.rs`) |
| The Mods screen's Sunrise row (name, version, authors, licence, links) (`sunrise_platform::mod_rows`) | crate `sunrise-official` | registered only by the official executable (`sunrise-official`); sunrise-client's own executable lists Minecraft and the mods only |
| Auto-reconnect ([extras.md](extras.md)) | crate `sunrise-reconnect` | registered only by the official executable (`sunrise-official`); sunrise-client's own executable has none |
| Discord Rich Presence and its switch | crate `sunrise-discord` | registered only by the official executable (`sunrise-official`); sunrise-client's own executable has none |
| Play with Friends | hosting: `game/friends.rs`, `game/friends/screen.rs`; joining: crate `sunrise-friends` (`game`) | hosting: the `friends` feature (`game/friends_off.rs` stands in); joining: only the official executable registers it ([layers.md](layers.md#how-a-sunrise-crate-plugs-in)) |
| The anonymous "I'm playing" ping | crate `sunrise-heartbeat` | not in sunrise-client at all: only the official executable (`sunrise-official`) registers it ([layers.md](layers.md#how-a-sunrise-crate-plugs-in)) |
| The performance overlay (frames per second, frame times and their graph) and its Performance Overlay row on the Extras page | crate `sunrise-perf-overlay` | not in sunrise-client at all: only the official executable (`sunrise-official`) registers it ([layers.md](layers.md#how-a-sunrise-crate-plugs-in)) |

Credits & Attribution then says where the official build is (`sunrise.plain.official`).

`grep -rn sunrise-extras crates/` lists every gate. Apart from the `friends` feature above, the gates
are `cfg!(feature = "sunrise-extras")`: both sides compile in every build, so no stub has to follow
the code it replaces. To remove an extra for good, delete its files from the table and the lines its
gate guards.

## Not behind the feature

- The launcher (`https://github.com/sunrise-org/sunrise-launcher`) holds its themes, the telemetry consent and updates. A build of the game alone has none of them; without the
  launcher's consent the game keeps no crash or performance report (docs/telemetry.md).
- The graphics screen's fullbright, dynamic lights and hiding block-break particles: renderer
  options, off by default (docs/extras.md). A `sunrise-extras.txt` left by an official install is
  still read.
- The mod runtime: nothing installs a mod without the launcher, so it stays idle.
- The `sunrise` brand reply to a Sunrise server, for its protocol extensions (`net.rs`).
- Agent control (`--control`, `sunrise-ctl`), controllers, HDR, shader packs.

## Microsoft sign-in in a fork

Sunrise's Microsoft sign-in runs under one Azure app registration, whose client ID release builds
read from the repository variable `SUNRISE_MSA_CLIENT_ID` (docs/microsoft-account.md). That
registration, and Mojang's approval of it for the Minecraft services, are for Sunrise's own builds
only. They do not pass to forks, ports or derived launchers, whatever their licence.

- A fork, port or derived launcher that signs players in registers its own Azure app and applies for
  its own approval from Mojang/Microsoft (the App ID review in docs/microsoft-account.md).
- Its builds must not ship, embed, download or otherwise fetch Sunrise's client ID, and must not tell
  players to set `SUNRISE_MSA_CLIENT_ID` to it. Sign-ins made with it would appear to Microsoft and
  Mojang as Sunrise's, which the project does not accept.
- Sunrise's licence ([LICENSING.md](../LICENSING.md)) covers the code, not the app registration: copying the code
  gives no right to use Sunrise's client ID.
- Sunrise may restrict, rotate or revoke its client ID at any time, which breaks every build that
  uses it, and reports misuse to Microsoft, Mojang and the hosting platform.

A build made from source without a client ID of its own says that sign-in is not configured; that is
the expected state for a fork until its own app is approved.
