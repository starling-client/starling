# Install the mod tool

Every mod starts with `cargo sunrise-mod`, the tool that makes a project, fetches the mod SDK, checks the mod, builds
it and packs it. Each SDK release ships the tool for three computers, built from that release's own source. Take the
one from the release the Nightly names, so the tool, the SDK your mod builds against and the game players run are the
same build. A fix to the tool that has been published since is in the [newest tool from main](#newest-tool-from-main).

You also need Rust 1.95 or newer (`cargo`, from [rustup.rs](https://rustup.rs)). The tool runs `cargo` itself.

## The prebuilt tool

All of Sunrise's releases are prereleases, so there is no "latest" to ask for. The `nightly` release's `sdk.json`
names the current Nightly's build, and the `sdk-<build>` release of that build holds the tool. Each block below
names the release it takes the tool from in its `RELEASE` line: `nightly` means that build's. It
reads the build, downloads the tool, `SHA256SUMS` and its signature, stops unless Sunrise's key signed the sums and the
tool's hash is in them, and only then puts `cargo-sunrise-mod` on your `PATH`. The key in each block is the one in
[`release-keys.txt`](https://github.com/sunrise-org/sunrise/blob/main/crates/sunrise-release-sign/release-keys.txt); the signature is checked with OpenSSL
(3.0 or newer, for `pkeyutl -rawin`), which is what [how a download is checked](../reference/sdk/01-release.md#how-a-download-is-checked)
describes. macOS's own `openssl` cannot check it: the block uses Homebrew's (`brew install openssl@3`). On Windows
it uses the `openssl.exe` that Git for Windows installs.

### Linux (x86_64)

```sh
(
  set -eu
  RELEASE=nightly
  BASE=https://github.com/sunrise-org/sunrise/releases/download
  KEY=r53xb3cuafGxZSicLz06KC+xXp9NJdPH8snNmXo5pAc=
  OPENSSL=openssl
  TOOL=tool-x86_64-unknown-linux-gnu.tar.xz
  if [ "$RELEASE" = nightly ]; then
    COMMIT=$(curl -fsSL "$BASE/nightly/sdk.json" | sed -n 's/.*"commit": *"\([0-9a-f]\{40\}\)".*/\1/p' | head -n 1)
    [ -n "$COMMIT" ] || { echo "could not read the Nightly's build" >&2; exit 1; }
    RELEASE=sdk-$COMMIT
  fi
  cd "$(mktemp -d)" || exit 1
  curl -fsSLO "$BASE/$RELEASE/$TOOL" || exit 1
  curl -fsSLO "$BASE/$RELEASE/SHA256SUMS" || exit 1
  curl -fsSLO "$BASE/$RELEASE/SHA256SUMS.sig" || exit 1
  printf 'horizon release %s\n' "$RELEASE" | cat - SHA256SUMS > message
  { printf '\060\052\060\005\006\003\053\145\160\003\041\000'; printf '%s' "$KEY" | "$OPENSSL" base64 -d -A; } > key.der
  signed=
  while read -r line || [ -n "$line" ]; do
    printf '%s' "$line" | "$OPENSSL" base64 -d -A > signature
    "$OPENSSL" pkeyutl -verify -rawin -pubin -keyform DER -inkey key.der -in message -sigfile signature > /dev/null 2>&1 && signed=1
  done < SHA256SUMS.sig
  [ -n "$signed" ] || { echo "SHA256SUMS is not signed by Sunrise" >&2; exit 1; }
  grep " $TOOL\$" SHA256SUMS | sha256sum -c - || { echo "$TOOL does not match SHA256SUMS" >&2; exit 1; }
  tar -xJf "$TOOL" || exit 1
  install -Dm755 cargo-sunrise-mod "$HOME/.local/bin/cargo-sunrise-mod"
)
export PATH="$HOME/.local/bin:$PATH"
```

The `export` lasts for this shell. Add `export PATH="$HOME/.local/bin:$PATH"` to your shell's startup file to keep it.

### macOS (Apple silicon)

<!-- quickstart-test: skip, another computer's block -->
```sh
(
  set -eu
  RELEASE=nightly
  BASE=https://github.com/sunrise-org/sunrise/releases/download
  KEY=r53xb3cuafGxZSicLz06KC+xXp9NJdPH8snNmXo5pAc=
  OPENSSL="$(brew --prefix openssl@3)/bin/openssl"
  TOOL=tool-aarch64-apple-darwin.tar.xz
  if [ "$RELEASE" = nightly ]; then
    COMMIT=$(curl -fsSL "$BASE/nightly/sdk.json" | sed -n 's/.*"commit": *"\([0-9a-f]\{40\}\)".*/\1/p' | head -n 1)
    [ -n "$COMMIT" ] || { echo "could not read the Nightly's build" >&2; exit 1; }
    RELEASE=sdk-$COMMIT
  fi
  cd "$(mktemp -d)" || exit 1
  curl -fsSLO "$BASE/$RELEASE/$TOOL" || exit 1
  curl -fsSLO "$BASE/$RELEASE/SHA256SUMS" || exit 1
  curl -fsSLO "$BASE/$RELEASE/SHA256SUMS.sig" || exit 1
  printf 'horizon release %s\n' "$RELEASE" | cat - SHA256SUMS > message
  { printf '\060\052\060\005\006\003\053\145\160\003\041\000'; printf '%s' "$KEY" | "$OPENSSL" base64 -d -A; } > key.der
  signed=
  while read -r line || [ -n "$line" ]; do
    printf '%s' "$line" | "$OPENSSL" base64 -d -A > signature
    "$OPENSSL" pkeyutl -verify -rawin -pubin -keyform DER -inkey key.der -in message -sigfile signature > /dev/null 2>&1 && signed=1
  done < SHA256SUMS.sig
  [ -n "$signed" ] || { echo "SHA256SUMS is not signed by Sunrise" >&2; exit 1; }
  grep " $TOOL\$" SHA256SUMS | shasum -a 256 -c - || { echo "$TOOL does not match SHA256SUMS" >&2; exit 1; }
  tar -xJf "$TOOL" || exit 1
  install -d "$HOME/.local/bin"
  install -m755 cargo-sunrise-mod "$HOME/.local/bin/cargo-sunrise-mod"
)
export PATH="$HOME/.local/bin:$PATH"
```

Add the `export` line to `~/.zshrc` to keep it.

### Windows (x64, PowerShell)

```powershell
& {
  $ErrorActionPreference = "Stop"
  $Release = "nightly"
  $Base = "https://github.com/sunrise-org/sunrise/releases/download"
  $Key = "r53xb3cuafGxZSicLz06KC+xXp9NJdPH8snNmXo5pAc="
  $OpenSsl = Join-Path $env:ProgramFiles "Git\usr\bin\openssl.exe"
  $Tool = "tool-x86_64-pc-windows-gnu.tar.xz"
  $Dir = (New-Item -ItemType Directory (Join-Path $env:TEMP "sunrise-tool-$PID")).FullName
  $At = { param($Name) Join-Path $Dir $Name }
  if ($Release -eq "nightly") {
    Invoke-WebRequest "$Base/nightly/sdk.json" -OutFile (& $At "sdk.json")
    $Commit = (Get-Content (& $At "sdk.json") -Raw | ConvertFrom-Json).build.commit
    if ($Commit -notmatch '^[0-9a-f]{40}$') { throw "could not read the Nightly's build" }
    $Release = "sdk-$Commit"
  }
  foreach ($Name in $Tool, "SHA256SUMS", "SHA256SUMS.sig") {
    Invoke-WebRequest "$Base/$Release/$Name" -OutFile (& $At $Name)
  }
  $Prefix = [byte[]](0x30,0x2a,0x30,0x05,0x06,0x03,0x2b,0x65,0x70,0x03,0x21,0x00)
  [IO.File]::WriteAllBytes((& $At "key.der"), $Prefix + [Convert]::FromBase64String($Key))
  $Head = [Text.Encoding]::UTF8.GetBytes("horizon release $Release`n")
  [IO.File]::WriteAllBytes((& $At "message"), $Head + [IO.File]::ReadAllBytes((& $At "SHA256SUMS")))
  $Signed = $false
  foreach ($Line in Get-Content (& $At "SHA256SUMS.sig")) {
    if ($Line.Trim() -eq "") { continue }
    [IO.File]::WriteAllBytes((& $At "signature"), [Convert]::FromBase64String($Line.Trim()))
    & $OpenSsl pkeyutl -verify -rawin -pubin -keyform DER -inkey (& $At "key.der") -in (& $At "message") -sigfile (& $At "signature") *> $null
    if ($LASTEXITCODE -eq 0) { $Signed = $true }
  }
  if (-not $Signed) { throw "SHA256SUMS is not signed by Sunrise" }
  $Want = ((Select-String -Path (& $At "SHA256SUMS") -Pattern "  $([regex]::Escape($Tool))\s*$").Line -split " ")[0]
  if (-not $Want -or (Get-FileHash (& $At $Tool) -Algorithm SHA256).Hash.ToLower() -ne $Want) { throw "$Tool does not match SHA256SUMS" }
  tar -xf (& $At $Tool) -C $Dir
  if ($LASTEXITCODE -ne 0) { throw "tar could not unpack $Tool" }
  $Bin = Join-Path $env:USERPROFILE ".sunrise\bin"
  New-Item -ItemType Directory -Force $Bin | Out-Null
  Move-Item -Force (& $At "cargo-sunrise-mod.exe") $Bin
  [Environment]::SetEnvironmentVariable("Path", "$Bin;" + [Environment]::GetEnvironmentVariable("Path", "User"), "User")
}
$env:Path = "$env:USERPROFILE\.sunrise\bin;$env:Path"
```

The `Path` change is saved for your user, so new terminals find the tool.

### Check it

```sh
cargo sunrise-mod --help
```

It prints a short list of the everyday commands, which shows it is installed (`cargo sunrise-mod help all`
lists every command's usage, and `cargo sunrise-mod <command> --help` says one command's):

```text
usage: cargo sunrise-mod <command> [options]

  new     Makes a mod project in <path>
  build   …
```

A tool built before `--help` existed prints the same usage after `error:` and exits with 1; that also means it is installed. `error: no such command: sunrise-mod` means the folder with `cargo-sunrise-mod` is not on your `PATH`.

The tool checks every SDK it fetches against Sunrise's release signature before it uses it, whatever the tool's own
origin.

To move to a newer tool later, run the same block again.

## Newest tool from main

The release `mod-tool` holds the newest tool published from the repository's main branch, for the same three
computers and signed with the same key: `tool-x86_64-unknown-linux-gnu.tar.xz`, `tool-aarch64-apple-darwin.tar.xz`,
`tool-x86_64-pc-windows-gnu.tar.xz`, `tool.json` (the commit it is built from), `SHA256SUMS` and `SHA256SUMS.sig`.
To install it, run your computer's block above with its `RELEASE` line changed to `RELEASE=mod-tool` (in PowerShell,
`$Release = "mod-tool"`). The block then checks Sunrise's signature for that release instead of the Nightly's.

Which to take:

- **The Nightly's tool**, by default. It is built from the same source as the SDK your mod pins and the game players
  run.
- **The newest tool from main**, when you need a fix to the tool that merged after the Nightly and is in that
  release, or when the Nightly's tool is older than this book: if `cargo sunrise-mod --version` answers with the
  usage instead of a commit, that tool cannot yet `build` or `run` a native mod on its own, so take `mod-tool` for
  [Your first native mod](first-mod.md). The project still pins the Nightly's SDK either way. It builds against a
  pinned SDK as long as that SDK has what the tool needs; when it does not, `build` stops and names both ways out:
  `cargo sunrise-mod update --sunrise nightly`, or the tool in that SDK's own release.

`cargo sunrise-mod --version` prints the commit a tool is built from, or `dev` for a tool you built yourself without
naming one (`cargo install`); a tool older than that option answers with its usage instead. The newest tool's commit is `commit` in
[`tool.json`](https://github.com/sunrise-org/sunrise/releases/download/mod-tool/tool.json).

For [tagged native releases](../how-to/publish.md#tagged-native-releases-in-github-actions), use a signed `mod-tool`
that supports both `cargo sunrise-mod release plan` and `cargo sunrise-mod release scripts`. The generated workflow
verifies the tool and `tool.json` against the signed sums before running it, pins the same tool commit across all
jobs and extracts the release scripts shipped in that tool. The mod's stable SDK is a separate pin, even if that
SDK ships an older tool.

## With `cargo install`

If no prebuilt tool fits your computer, build it from source:

<!-- quickstart-test: skip, the prebuilt tool is the one under test -->
```sh
cargo install --locked --git https://github.com/sunrise-org/sunrise cargo-sunrise-mod
```

This compiles the tool and the crates it needs from the game repository, and puts it in
`~/.cargo/bin`, where `cargo` finds it. It builds the tool from the repository's newest commit, which can be newer
than the Nightly. A mod pinned to the Nightly still builds against the Nightly's SDK, and `new` and `update` write
the pins from that SDK, not from the tool.

## On NixOS

The prebuilt tool is a glibc binary linked for a standard Linux, which NixOS does not have. Run it through
`steam-run` (`steam-run cargo sunrise-mod …`), or turn on `nix-ld`, or use the `cargo install` path above. The tool
runs `cargo`, so Rust has to be installed in the same environment.

## Next

- [Your first native mod](first-mod.md)
- [Your first content mod](first-content-mod.md)
