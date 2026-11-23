# Developer acceptance workflow

Use a disposable server folder and fresh homes when developing:

```sh
SCRATCH=$(mktemp -d)
export SUNRISE_HOME="$SCRATCH/game" SUNRISE_LAUNCHER_HOME="$SCRATCH/launcher"
export XDG_DATA_HOME="$SCRATCH/data" XDG_CONFIG_HOME="$SCRATCH/config"
export SUNRISE_NO_AUDIO=1 SUNRISE_LAUNCHER_NO_SELF_UPDATE=1
export SUNRISE_LAUNCHER_TRASH="$SCRATCH/trash"
export SUNRISE_LAUNCHER_CLIPBOARD="$SCRATCH/clipboard"
export SUNRISE_LAUNCHER_DESKTOP="$SCRATCH/desktop"
```

Install and prepare the base release in that scratch launcher home first, then test your own licensed bundle:

1. `sunrise-launcher native-build bundle/ --server-output $ENGINE --version TAG --trust-native-code` (scratch homes,
   `SUNRISE_NO_AUDIO=1`, `SUNRISE_LAUNCHER_NO_SELF_UPDATE=1`).
2. `sunrise-server --dir SERVER config set engine.path=$ENGINE/sunrise-engine`; `sunrise-server --dir SERVER run`.
3. Join with the same mod's client half where the hook needs one; test lifecycle, tick, command and net hooks.
4. Headless compatibility: the mod's server code must not require an audio device; run the server audio-free (`SUNRISE_NO_AUDIO=1`).
