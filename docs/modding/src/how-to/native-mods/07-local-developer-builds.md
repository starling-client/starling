# Build a local mod SDK

Use a local SDK to build a native mod for a Sunrise commit that has no published SDK. The mod tool
reads a signed `sdk-<commit>` folder on your machine instead of looking for a release.

## Build the SDK from your checkout

Commit the game changes you want the mod to use. Every SDK input comes from a temporary checkout
of `HEAD`, so local edits and untracked files are left out. Use the Rust version
in `tools/ci/rust-version`. For this Linux example you also need Bash,
Python 3.11 or later, OpenSSL with Ed25519 support, and the ALSA and udev development headers
(`libasound2-dev` and `libudev-dev` on Debian and Ubuntu). No Minecraft files are needed to build the SDK.

From the root of the Sunrise checkout, pick a new output folder:

```sh
commit=$(git rev-parse HEAD)
sdk_base="$PWD/target/developer-sdk-$commit"
tools/developer_sdk.sh "$sdk_base"
```

This builds the client libraries for your machine and assembles the compiled SDK, API source and
hook catalog into `sdk-<commit>/`. It signs the SDK with a throwaway key and saves the **public** key
as `developer-key.pub`. The base also contains a `nightly/sdk.json` pointer so commands that default
to Nightly find your local build.

## Use the SDK from a mod

Build the mod tool in **debug** mode. Only a debug build accepts `SUNRISE_SDK_TRUSTED_KEY`; a release
build trusts only Sunrise's release keys. The debug tool still checks the signature and the assets'
checksums. Trust only a key from an SDK you built or otherwise trust.

```sh
cargo build --locked -p cargo-sunrise-mod
export PATH="$PWD/target/debug:$PATH"
export SUNRISE_SDK_BASE="$sdk_base"
SUNRISE_SDK_TRUSTED_KEY=$(cat "$sdk_base/developer-key.pub")
export SUNRISE_SDK_TRUSTED_KEY
```

If you build Cargo output elsewhere, use its `debug/` folder on `PATH` instead. `SUNRISE_SDK_BASE`
points to the **parent** of `sdk-<commit>/`, not the release folder itself. Keep these settings in the
shell you use for the mod. In a folder outside the Sunrise checkout:

```sh
cargo sunrise-mod new greeter --template native --sunrise "$commit"
cd greeter
cargo sunrise-mod check
cargo sunrise-mod hooks player_logged_in
cargo sunrise-mod build
cargo sunrise-mod fits
cargo sunrise-mod update --sunrise "$commit"
cargo sunrise-mod check
cargo check
```

`new` pins the mod to the commit and unpacks the SDK's API source into `.sunrise/sdk/api`. The
generated `Cargo.toml` points there, so plain `cargo check` and your editor can use it without linking
the game. In a fresh clone or after `update`, run `cargo sunrise-mod check` to fetch the pinned source
again. `build` links against the compiled SDK and writes the `.smod` under `target/sunrise-mod/`.

Here `fits` and `update` check the mod's existing pin. To compare two builds, their SDKs must be
available from the same `SUNRISE_SDK_BASE`. Each invocation uses one base and trusts at most one
extra key. This recipe makes a separate base and key for each local SDK, so it cannot compare two
independently built local SDKs as they are.

To start the game with the mod, see [Run it](../../start/first-mod.md#6-run-it); that step needs
resources prepared from your own copy of Minecraft.
