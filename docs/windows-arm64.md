# Windows ARM64

Sunrise builds the game and launcher as native Windows ARM64 executables for
`aarch64-pc-windows-msvc`. GitHub Actions builds and checks them on a Windows ARM64 runner.

## Build from source

Use Windows 11 on ARM, or cross-compile from Windows x64. Install stable Rust, the
`aarch64-pc-windows-msvc` Rust target, Visual Studio C++ build tools for ARM64, and a Windows SDK
with ARM64 libraries. Install LLVM/Clang and put `clang.exe` on `PATH`; the `ring` dependency uses
Clang to compile its ARM64 code. The MSVC target also needs the Visual C++ linker and Windows import
libraries.

To run `sunrise-prepare compile-exporters`, install JDK 25 and Python 3.11+; that command compiles
the Java exporters used by the game.

```powershell
rustup target add aarch64-pc-windows-msvc
cargo build --release --locked --target aarch64-pc-windows-msvc `
  -p sunrise-game -p sunrise-client-lib --bin sunrise-client --bin sunrise-ctl `
  -p sunrise-prepare --bin sunrise-prepare -p sunrise-server --bin sunrise-server
```

The launcher is built in its own repository, [sunrise-launcher](https://github.com/sunrise-org/sunrise-launcher); the same cargo commands with `--target aarch64-pc-windows-msvc` run in a checkout of it.

## Releases and limitations

The game archive, `sunrise-windows-arm64.zip`, is attached to the [Nightly release](https://github.com/horizon-rust/horizon/releases/tag/nightly)
and listed in its signed `SHA256SUMS`. The launcher archive, `sunrise-launcher-windows-arm64.zip`,
and `sunrise-launcher-setup-arm64.exe` are attached to the [launcher releases](https://github.com/horizon-rust/horizon/releases)
and listed in their signed `SHA256SUMS`.

Launcher updates always follow the launcher's own process architecture. Game architecture is a
separate choice in Settings on native Windows ARM, detected from the machine even when an x64
launcher runs under emulation. **Automatic** preserves existing behavior: an x64 launcher uses x64,
and an ARM64 launcher prefers ARM64. Either launcher can choose x64 or ARM64 for the game. The
choice is saved and used for installs and updates; when a release has no ARM64 archive, ARM64
selection safely installs the signed x64 fallback. A changed choice installs the selected version
again when automatic updates are on; otherwise use **Install selected build** on the Versions page.
Launching is blocked with an explanation if a manual choice does not match the installed game.
Choosing a game architecture never changes or replaces the launcher binary.

Native-mod compilers remain independent of this setting. The managed compiler currently builds x64
only; an x64 launcher cannot build native mods for an ARM64 game, and the ARM64 launcher has no
matching compiler yet. The launcher refuses mismatched native-mod builds rather than producing a
binary for the wrong game.

On Windows ARM64, `Graphics API: Auto` tries DirectX 12 before Vulkan following a device-loss observed
in testing. Vulkan remains selectable in settings.

The Windows ARM64 game check in `ci.yml` runs only by hand
(`gh workflow run checks --ref <branch>`); the Nightly and launcher releases build these archives on
every run.
