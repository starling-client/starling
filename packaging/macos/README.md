# macOS packaging

The launcher's own packaging (`package.sh`, `Info.plist`, the .dmg background) moved with the launcher to [sunrise-org/sunrise-launcher](https://github.com/sunrise-org/sunrise-launcher); the sections below describe it and the signing and notarization this folder's `keychain.sh` and `notarize.sh` still do for the game's macOS builds.

`package.sh BINARY VERSION OUTDIR` (in sunrise-launcher's release workflow, on macOS) makes:

- `Sunrise.app`: `Contents/MacOS/sunrise-launcher`, `Contents/Info.plist` (this folder's, bundle
  id `dev.sunrise.launcher`, macOS 11 or later), `Contents/Resources/sunrise.icns`
  (`packaging/icons`, from `cargo run -p xtask -- icons`).
- `sunrise-launcher-macos-arm64.dmg`: the installer. Opening it shows the app and a link to
  Applications over `background.png` / `background@2x.png` (drawn from `background.html`); the
  player drags one onto the other. Built by [dmgbuild](https://github.com/dmgbuild/dmgbuild),
  which writes the Finder layout without a GUI session.
- `sunrise-launcher-macos-arm64.zip`: the same app, zipped with `ditto`. The launcher updates
  itself from this: inside Sunrise.app it replaces the whole bundle (copied beside it, then
  swapped in by renaming, then restarted); a bare `sunrise-launcher` binary, as launchers before
  the app shipped, finds the binary inside the zip and replaces only itself.

## Fork packages

The arismoko fork ships Apple Silicon (arm64) only, not Intel. Its staged packaging
uses `Sunrise Launcher.app`, bundle id `net.playsunrise.launcher`, the fork icon and
executable `sunrise-launcher`. The volume is named `Sunrise Launcher`.
The installer is `sunrise-launcher-macos-arm64.dmg`; self-update uses
`sunrise-launcher-macos-arm64.zip` from sunrise-org/sunrise. Settings, accounts
and game data use Sunrise's own namespace.

The app is **ad hoc signed, not notarized**. Drag it to Applications. On macOS 14
and earlier, right-click the app and choose **Open**, then confirm **Open**. On
macOS 15 and later, open it once, then use **System Settings › Privacy & Security ›
Open Anyway** and confirm. No Apple signing identity or notarization profile is needed. The upstream signing description below does not
apply to these fork packages.

## Signing and notarization

Releases are signed with Ludovic's **Developer ID** ("Developer ID Application: Ludovic Marie
(YYJU63HSD4)"), with the hardened runtime and a secure timestamp, then **notarized** by Apple and
**stapled**, so Gatekeeper opens them on any Mac without a warning, offline too:

- the launcher (`package.sh`): Sunrise.app is signed, notarized and stapled first, so the zip the
  launcher updates itself from and the .dmg both hold it stapled; then the .dmg is signed,
  notarized and stapled. The script ends with what Gatekeeper says of each, which must read
  `accepted` and `source=Notarized Developer ID`.
- the game (`.github/workflows/game-build.yml`): `sunrise-client`, `sunrise-ctl`, `sunrise-perf`,
  `sunrise-prepare` and `sunrise-server` are signed right after they are built (so the checks and the 1500-frame smoke
  run use them as players will), and the zip is notarized. `sunrise-client` embeds
  `packaging/macos/game-info.plist` in `__TEXT,__info_plist` (providing `CFBundleIdentifier`,
  `CFBundleName` and `NSMicrophoneUsageDescription`) and is signed with
  `packaging/macos/game-entitlements.plist` (`com.apple.security.device.audio-input`) so hardened-runtime
  builds can open microphone capture for mods. Programs outside a bundle can't hold a
  ticket: Gatekeeper asks Apple online the first time each one runs.

The other helper binaries need no entitlements. The launcher checks an update's signature before swapping the new
Sunrise.app in, and again where it lands; when it doesn't hold, the old app is put back.

The switch is the repository variable `MACOS_SIGN_IDENTITY`: set to the identity's name, the
builds sign and notarize; empty (as in forks), they are signed **ad hoc** as before.

### The signing keychain on the Mac mini

The release runner (`ludovics-mac-mini-release`, label `sunrise-release`, `MAC_RUNNER`) is a
LaunchAgent whose jobs run in their own security session, where the login keychain is locked.
So signing uses a keychain of its own, `~/Library/Keychains/sunrise-signing.keychain-db`,
holding only the Developer ID identity, Apple's "Developer ID Certification Authority" and the
notarytool profile `sunrise-notary` (an App Store Connect API key of Sunrise's own Apple team,
YYJU63HSD4, the team of the Developer ID above; never a key of another team). Its password is the item "Sunrise signing keychain" in the Agents vault.
`keychain.sh unlock` (each job's first macOS step) reads it with the fleet's 1Password service
account, unlocks the keychain, checks the identity and the profile are there and hands both to
the later steps; `keychain.sh lock` ends the job. Nothing secret is in the repository or on
GitHub, and nothing is printed.

Setting it up again (a new Mac, a renewed certificate):

1. `packaging/macos/keychain.sh create`: the keychain, locked after two hours idle and on sleep.
2. In Keychain Access on the Mac (the login keychain, *My Certificates*), right-click
   "Developer ID Application: Ludovic Marie (YYJU63HSD4)", *Export…*, save it as a .p12 with a
   password; then in the Mac's Terminal `packaging/macos/keychain.sh import FILE.p12`. It asks for
   that password, imports the identity and its intermediate, removes any other identity, lets
   `codesign` use the key without a prompt and deletes the .p12. (The login keychain also holds
   other people's Apple Development and Distribution identities: never those.)
3. `packaging/macos/keychain.sh notary`: the notarytool profile, from team YYJU63HSD4's App Store
   Connect API key in the Agents vault (written to a private temporary file and deleted). Name its
   1Password item with `MACOS_NOTARY_ITEM`: the script's default still points at another team's key,
   which Sunrise must never use.
4. The repository variable `MACOS_SIGN_IDENTITY`.

The certificate and its intermediate expire on 1 February 2027: renew the certificate in the
Apple Developer account before then and repeat step 2.

Signing by hand, as the workflow does:

```sh
export MACOS_SIGN_IDENTITY="Developer ID Application: Ludovic Marie (YYJU63HSD4)"
export MACOS_SIGN_KEYCHAIN=~/Library/Keychains/sunrise-signing.keychain-db MACOS_NOTARY_PROFILE=sunrise-notary
packaging/macos/keychain.sh unlock
packaging/macos/package.sh target/release/sunrise-launcher 0.0.0 out   # run in sunrise-launcher, which holds package.sh
packaging/macos/keychain.sh lock
```

### Gatekeeper with an ad hoc build

An ad hoc build (a fork's, or `package.sh` without `MACOS_SIGN_IDENTITY`) is consistent and runs
on Apple Silicon, but Gatekeeper doesn't know the developer. The first time it is opened after
downloading:

- macOS 14 and earlier: right-click (or Control-click) Sunrise in Applications, choose **Open**,
  then **Open** in the dialog.
- macOS 15 and later: open it once (it is refused), then **System Settings › Privacy & Security**,
  **Open Anyway** next to "Sunrise was blocked", and confirm.

After that it opens normally, and its own updates don't ask again (they aren't downloaded by a
browser, so they carry no quarantine flag).
