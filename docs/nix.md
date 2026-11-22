# Nix and NixOS

The repository is a flake (`flake.nix`, and the game's packages in `nix/`). It builds Sunrise from source with nixpkgs'
`rustPlatform` and holds none of Mojang's files: the game files still come from `sunrise-prepare` and your own
copy of Minecraft 26.3, as everywhere else ([hosting.md](hosting.md), step 2).

| Package | What | Systems |
|---|---|---|
| `sunrise-server` | `sunrise-server`, with `sunrise-engine` (the game without sound or controllers) beside it | Linux, macOS (Apple Silicon) |
| `sunrise-engine` | The server build of the game alone | Linux, macOS |
| `sunrise-prepare` | `sunrise-prepare`, with a JDK 25 for the exporters it compiles | Linux, macOS |
| `sunrise-client` | The game and `sunrise-ctl` | Linux, macOS |

```sh
nix run github:sunrise-org/sunrise#sunrise-server -- --version
nix profile install github:sunrise-org/sunrise#sunrise-server
```

`overlays.default` adds them to `pkgs`; on Linux, `nix develop` gives the build environment of the workspace.

Sunrise is under the PolyForm Shield licence ([LICENSING.md](../LICENSING.md)), which nixpkgs counts as unfree. The
flake's own packages, checks and module build as they are. With the overlay, your nixpkgs decides: allow these
packages, for example with `nixpkgs.config.allowUnfreePredicate = pkg: lib.hasPrefix "sunrise-" (lib.getName pkg);`.

## Preparing the game files

`sunrise-prepare` works in a writable folder (its pinned hashes, the exporters' sources, scratch files). The
Nix wrapper copies one to `$SUNRISE_PREPARE_ROOT`, by default `~/.local/share/sunrise-nix/prepare`, unless you
pass `--root`. Write the files where you want them with `--output`:

```sh
sunrise-prepare --prepare --server-jar ~/Downloads/server.jar --output ~/sunrise/26.3
```

## A server on NixOS: `services.sunrise-server`

```nix
{
  inputs.sunrise.url = "github:sunrise-org/sunrise";

  outputs = { nixpkgs, sunrise, ... }: {
    nixosConfigurations.box = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        sunrise.nixosModules.sunrise-server
        {
          services.sunrise-server = {
            enable = true;
            eula = true;                      # you agree to https://aka.ms/MinecraftEULA
            openFirewall = true;              # port 25565 by default
            resources = "/srv/sunrise/26.3";  # the folder sunrise-prepare wrote, copied here
            init = { world = "survival"; players = [ "Alex" ]; op = "Alex"; };
            settings = { "world.difficulty" = "hard"; "server.max_players" = 10; };
          };
        }
      ];
    };
  };
}
```

- The server folder is `/var/lib/sunrise-server` (`dataDir`), owned by the `sunrise-server` user. The first
  start writes `server.toml` from `init`, as `sunrise-server init` would. Every start then sets `port`,
  `resources` and `settings` with `sunrise-server config set`, which checks them and keeps the rest of the
  file: the settings you don't name stay yours to edit.
- Give paths as strings, not Nix paths: a Nix path would copy the game files into the world-readable store.
- The service runs `sunrise-server run` and stops it with SIGTERM, which saves the world first. Manage it as
  that user: `sudo -u sunrise-server sunrise-server --dir /var/lib/sunrise-server status` (or `cmd`, `logs`,
  `backup`...).
- Nix decides the release: the update check is off (`SUNRISE_NO_UPDATE_CHECK=1`) and `sunrise-server update`
  refuses, since this isn't an install.sh install.
- The unit is hardened (read-only system, no home folders, no capabilities, its own `/tmp` and devices). A
  `resources` folder under `/home` isn't visible to it: keep it elsewhere, such as `/srv`.

### Preparing on the server

Instead of `resources`, `prepare` runs `sunrise-prepare` before the server starts, from files you put on the
machine yourself: the 26.3 `server.jar` and a copy of the official launcher's game folder with 26.3 played
once (`versions/26.3`, `libraries`, `assets`), readable by the `sunrise-server` user.

```nix
services.sunrise-server.prepare = {
  enable = true;
  serverJar = "/srv/sunrise/server.jar";
  minecraftDir = "/srv/sunrise/minecraft";
};
```

It writes `/var/lib/sunrise-server/game-files`, and runs again only when the `sunrise-prepare` package or
those paths change. The `sunrise-server-prepare` unit has no network at all: nothing is ever downloaded.
