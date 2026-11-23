# Sunrise Fork Launcher (arismoko)

This is the arismoko fork, not the official Sunrise launcher. It uses separate
settings, game data and account storage and installs and updates only from
[arismoko/horizon](https://github.com/arismoko/horizon).

Microsoft sign-in is configured using the application ID upstream authorized the
fork to use. Signed Nightly builds require a signed-in account that owns Minecraft.
Account and token storage remain separate from the official launcher.

## Downloads

- **Linux x86_64:** download `sunrise-fork-launcher-linux-x86_64.AppImage`, make it
  executable and run it. The `.tar.gz` archive is also available.
- **Windows x64:** download `sunrise-fork-launcher-setup.exe`, or extract
  `sunrise-fork-launcher-windows-x64.zip` and run `sunrise-fork-launcher.exe`.
  This fork is **not Authenticode-signed**. Expect a SmartScreen or unknown-publisher
  warning; signed checksums do not remove Windows warnings.

These launcher releases contain Linux x86_64 and Windows x64 packages only.
The launcher checks signed updates from this fork automatically.
Older launchers build native mods from source against newer Nightlies until they update.

## Verify the download

Download `SHA256SUMS`, `SHA256SUMS.sig` and all four packages from this release
into one folder. The signature binds the checksums to this fork and the exact
release tag; comparing a checksum alone does not authenticate the download.

With Rust installed, use the verifier from the fork's tagged source checkout:

```sh
git clone --branch '{tag}' --depth 1 https://github.com/arismoko/horizon.git sunrise-fork-source
cd sunrise-fork-source
cargo run --locked -p sunrise-release-sign -- verify --repository arismoko/horizon --tag '{tag}' --publishing /absolute/path/to/downloads
```

The verifier uses the fork's built-in public keys and checks all four packages
against the signed manifest. An invalid signature, a wrong release tag, a missing
package or a changed package is refused. The launcher performs the corresponding
signature and checksum checks automatically for each download.
