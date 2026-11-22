# Build log

The full log of a build is `native/build.log`, beside the build's own folders under the launcher's data
folder. Each new attempt replaces it. Compiler output and the final result
stay in the log, including failures before compilation or while selecting or exporting the built game.

Choose **Build log** at the top right to open the latest build's log. The action is available
whenever a log exists, including after restarting the launcher. Each build replaces that log. The
panel shows a bounded preview of the end of the log and says when it truncated; **Copy log** copies the
whole file, **Open full log** opens it outside the launcher, **Show in folder** reveals it, and
**Refresh** reads it again (also after a build finishes). The panel is a snapshot read on open and
refresh, not a live stream: a running build's later output appears only on the next refresh. Nothing is
uploaded; the log stays on the player's computer.

`cargo sunrise-mod build` keeps its own log and prints that log's path when it ends, built or failed.

The launcher's `native-build` command writes build progress to stderr. A terminal keeps
detailed download updates; redirected stderr reports only the first and completed updates for each
Rust toolchain or mod-tools download. Build phases, compiler progress and failures are still
reported in order, and the full build log is unchanged.


## When a build fails on Linux

- **A linker error naming `-ludev` or `-lasound`.** The game links udev and ALSA, so their development files must be
  installed: `libudev-dev` and `libasound2-dev` on Debian and Ubuntu, `systemd-devel` and `alsa-lib-devel` on Fedora,
  `udev` and `alsa-lib` in the shell's packages on NixOS. The build's error leads with this when it happens.
- **A C or C++ dependency that cannot find its own headers under Nix.** Nix's compiler wrapper, when
  `NIX_ENFORCE_PURITY` is `1`, drops every include folder outside the Nix store, and a dependency's sources are in the
  build's own Cargo folder. The builds the tool starts set `NIX_ENFORCE_PURITY=0`, so this affects only a `cargo`
  you run yourself in such a shell.
