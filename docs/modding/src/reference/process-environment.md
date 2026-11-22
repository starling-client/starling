# The process, versions and game tests

A mod can ask which kind of process it runs in and which game it runs on, before it declares anything.

## From registration

`RegistrationContext` gives these before any declaration is registered:

| Method | Answers |
| --- | --- |
| `dist()` | `Dist::Client` in the physical client process (its integrated server included), `Dist::DedicatedServer` in a dedicated server |
| `game_version()` | Minecraft's version, exactly `sunrise_core::VANILLA_VERSION` |
| `sunrise_version()` | Sunrise's version, the string crash reports print |
| `game_tests_enabled()` | Whether the process runs with game tests on |

`dist()` names the process, not the logical side of the hook that is running: a hook on the integrated server of a
client process still sees `Dist::Client`. `Dist` and `RuntimeEnvironment` are local reads, not Wire values. If a
protocol needs a process fact, copy that fact into a record defined by the protocol.

## From a host

The `Host` gives the same distribution and versions on both sides, and `game_tests_enabled()` on a server host. All
values are captured once, before mods register.

`server_data_version() -> i32` is also available from `Host` on both sides, before any world is open. It returns
the engine's constant `DataVersion` written into level data and chunks (`sunrise_core::version::DATA_VERSION`),
not a remote server's version or an older save's version.

## Game tests

Game tests are off by default. Turn them on with `--game-tests` on the game's command line (before or after
`--dedicated`), or with `SUNRISE_GAME_TESTS=1` or `SUNRISE_GAME_TESTS=true` in the environment; other values
leave them off. With the mode on, a mod may declare content meant only for tests. `register_game_tests` runs on
every world open whatever the mode, and `/test` is always available. A
[failed registration callback](../concepts/load-phases.md#registration-and-setup) publishes
neither test definitions nor functions. Other listeners can still register those identifiers.

The switch counts only where an option is expected: `--dedicated --world --game-tests` opens the world named
`--game-tests` and leaves test mode off. The dedicated server's `--universe`, `--port`, `--serverId` and
`--pidFile` likewise take the next word as their value.
