# Native mods: building and shipping

A native mod is Rust source in a `.smod`. The launcher compiles it on the player's machine, with its own pinned Rust
toolchain, against the prebuilt game of the exact Sunrise build the mod pins in `[native]`. The game itself is never
rebuilt for a mod: the mods are compiled and linked against that prebuilt game, and the instance then plays the
result.

Native code runs with the player's rights, outside any sandbox. Reviewed registry releases install without a
warning after the launcher verifies their signature and hash; unreviewed sources (**Add from source**, local `.smod`
files, and CLI `--trust-native-code`) require explicit confirmation before building or running native code.

If your mod calls another mod's Rust library, put the requirement in `sunrise-mod.toml`. In the add-on's folder,
use `cargo sunrise-mod check --with ../library` for a local copy, then use the same flag with `build` or `run`.
Without `--with`, the tool looks for required libraries in the Sunrise mod registry. Use `--with` to
include optional libraries. The built `.smod` contains only your add-on; see
[dependencies between mods](../concepts/dependencies.md) for the wiring and version rules.

The pages in this section follow a mod from its source to a player's game and a server:

<!-- section-pages -->
