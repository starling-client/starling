# Install a local unsigned game build with the Sunrise launcher

`developer-install` installs a game executable built from a trusted checkout. This is a separate
launcher installation command, not a native mod build route. It runs the checkout's packaging
code with your rights and requires `--trust-unsigned`.

Commit tracked changes, build the game and companion programs, and prepare compatible exporters
from your own Minecraft installation before installing. The game program, the `--program` file, is the `sunrise-client` binary of the `sunrise-game` package
(`cargo build -p sunrise-client` builds only the library):

```sh
cargo build -p sunrise-game -p sunrise-client-lib --bins -p sunrise-prepare -p sunrise-server -p sunrise-perf -p sunrise-port-pack
sunrise-launcher developer-install CHECKOUT --program CHECKOUT/target/debug/sunrise-client --trust-unsigned
```

The install needs these files. The checkout's packager (`cargo run -p xtask -- package`, and
`package-server` for a server folder) checks them all before it writes anything, and names every
missing one in one message with the command that makes it. Its list is
`crates/sunrise-mod-package/src/sdk/inputs.json`; a test keeps this table equal to it. In the
commands, `PROFILE` is the cargo flag of the profile the game program was built with (`--release`
for `target/release`, nothing for `target/debug`), `PROGRAM_FOLDER` its folder, and on Windows the
programs end in `.exe`:

<!-- required inputs: crates/sunrise-mod-package/src/sdk/inputs.json, checked by a test -->
| File | Where | Needed by | How to make it |
|---|---|---|---|
| `sunrise-ctl` | beside the game program | `package` | cargo build PROFILE -p sunrise-client-lib --bin sunrise-ctl |
| `sunrise-perf` | beside the game program | `package` | cargo build PROFILE -p sunrise-perf --bin sunrise-perf |
| `sunrise-prepare` | beside the game program | `package`, `package-server` | cargo build PROFILE -p sunrise-prepare |
| `sunrise-server` | beside the game program | `package`, `package-server` | cargo build PROFILE -p sunrise-server |
| `sunrise-port-pack` | beside the game program | `package` | cargo build PROFILE -p sunrise-port-pack |
| `exporters/manifest.json` | in the checkout | `package`, `package-server` | sunrise-prepare compile-exporters --root CHECKOUT, then copy its server, client and manifest.json into CHECKOUT/exporters/ |
<!-- end of required inputs -->

`compile-exporters` compiles against the pinned Minecraft jars that an earlier preparation imported
into the checkout, and it doesn't import them itself. It also needs a JDK of the Java version
`research-lock.json` pins (`java_major`, 25 for 26.3). Prepare first, from your own Minecraft
installation and the server jar you downloaded yourself:

```sh
cargo build -p sunrise-prepare
CHECKOUT/target/debug/sunrise-prepare --prepare --root CHECKOUT --server-jar SERVER_JAR
bundle=$(CHECKOUT/target/debug/sunrise-prepare compile-exporters --root CHECKOUT | tail -1)
mkdir -p CHECKOUT/exporters
cp -R "$bundle/server" "$bundle/client" "$bundle/manifest.json" CHECKOUT/exporters/
```

The preparation also writes the resources to `CHECKOUT/.local/sunrise-client/26.3`, which
`developer-install` copies beside the base (below). `exporters/` is ignored by git, so the developer
install still finds the checkout clean.

The command returns a `developer-…` tag. Select that exact tag and pin an instance to it.
Automatic updates do not replace a local unsigned base. Remove it with
`sunrise-launcher remove DEVELOPER_TAG` when no instance needs it.

## Its resources come from the checkout

A base needs Minecraft's own files prepared under `.local/sunrise-client/<version>` in its folder.
When the checkout has them already — `sunrise-prepare --root CHECKOUT` made them, or you started the
checkout's game once — `developer-install` copies that folder beside the base, and the base is ready
to play: no official launcher install, no signed-in account, and no separate `prepare`, which would
stop at `no Minecraft 26.3 client jar` on a machine that has neither (#4315).

It is a copy, not a link. Preparing the base again rewrites only the base's files, and the base
keeps working once the checkout moves or is deleted. `sunrise-data.bin` is left out because it names
the folder it was written for: the game writes the base's own on its first start.

When the checkout had nothing prepared yet, there are two ways:

- prepare the base as before: `sunrise-launcher prepare --version DEVELOPER_TAG --minecraft <the game folder of your launcher>`;
- or prepare the checkout's folder with `sunrise-prepare --root CHECKOUT` and repeat the same
  `developer-install` command. The tag comes from the checkout's committed sources, so it does not
  change and nothing is installed twice; the resources are copied then.

By hand, that is a copy of `CHECKOUT/.local/sunrise-client/<version>` to
`<launcher folder>/versions/DEVELOPER_TAG/.local/sunrise-client/<version>`, which is what
`developer-install` does for you.

A native mod build needs the prebuilt [SDK](modding/src/reference/sdk.md) of the base's exact commit. For a commit without a
published SDK, build it from the same checkout and name its folder; one folder holds one side:

```sh
python3 tools/prebuilt_game.py build SDK_FOLDER            # add --server for a dedicated server
cargo build --release -p cargo-sunrise-mod                 # the native build tool, from the same checkout
SUNRISE_LAUNCHER_DEVELOPER_SDK=SDK_FOLDER SUNRISE_LAUNCHER_DEVELOPER_TOOL=target/release/cargo-sunrise-mod \
  sunrise-launcher native-build MODS --version DEVELOPER_TAG --trust-native-code
```

The variables are read for developer bases only, and the folder is used where it is. The launcher compiles
through the tool (`cargo sunrise-mod native-build`, [its contract](modding/src/reference/sdk/08-native-build.md)), as it
runs the one a published SDK carries. It must be
that commit's build for this computer, side and the launcher's Rust, as a downloaded SDK must;
anything else is refused. Without it, the launcher uses the commit's SDK release when one is
published, and otherwise says the build has no mod tools. The native mod builder never compiles the checkout itself. For native mixins, the pinned
source must also be available for copying the target functions.
