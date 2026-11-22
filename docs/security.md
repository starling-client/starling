# Security

How Sunrise keeps its code and its downloads safe while most of the work is done by AI agents and releases go out
by themselves, including when Ludovic isn't at his computer. In plain words: what someone could have done before,
what stops them now, and what still needs Ludovic.

## Who does what

- **Agents** (Claude sessions run by Ludovic) write most pull requests, review them, merge them and answer
  reports. Everything an agent posts on GitHub says so ("Opened by Ludovic's AI agent").
- **Contributors** send pull requests from their forks like on any open-source project.
- **Players** report bugs on Discord and GitHub. Kira, the Discord bot, files their reports as issues.
- **Releases** are automatic: every merged change that players run ships within the hour.

## What could go wrong before (October 2026)

1. **Instructions hidden in a report.** Someone could write "ignore your previous instructions and…" in a
   Discord message or an issue. Agents read those reports while working with Ludovic's full GitHub account.
2. **A check that counted as passed without running.** Adding a label to a pull request started a second run of
   the checks in which `verify` was *skipped*, and GitHub counts a skipped required check as passed. Two pull
   requests merged before their real check had finished.
3. **Every release went straight to every player.** A broken or malicious build reached every launcher within
   minutes, with nobody looking at it first.
4. **Downloads trusted GitHub alone.** Anyone who got hold of a token that can write to the repository could
   replace a release's files, and launchers and servers would install them.
5. **The release machinery was ordinary code.** A pull request could change the workflows that build and publish
   releases as easily as it changes a texture.

## Where things stand (19 October 2026)

The next section describes the design once every part is in place. Some parts are live, some are waiting to merge,
and some are waiting for Ludovic:

| Part | Today |
| --- | --- |
| `verify` always runs | **Live**. |
| Fork pull requests need approval | **Live**: every outside contributor's workflows wait for a maintainer's approval. |
| Private vulnerability reporting | **Live** (see the end of this page). |
| Signed checksums on releases | **Live** for new releases: the `release` environment holds the key, and its public half is in `crates/sunrise-release-sign/release-keys.txt`. Releases from before then are unsigned. |
| Signed prebuilt native-mod builds | **Being built**: sunrise-mods' `native-builds` environment (deployable from `main` only) holds `NATIVE_BUILD_SIGNING_KEY`, a key of its own, never the release key, because the registry compiles other people's code there. Its public half is in `native-build-keys.txt`. Each signature covers the build's request key (target, base commit, each mod's file SHA-256), its target and the archive's SHA-256 (`sunrise_release_sign::native_build`), so it can't pass for another set's file or for a release. `tools/ci/sign_native_build.sh` signs with OpenSSL. |
| Launchers and servers checking signatures | **Live**: the launcher (for the game and for itself) and `sunrise-server update` check the release signature and refuse anything unsigned or signed with another key ("Signed updates", below). The first install with `packaging/server/install.sh` or `install.ps1` checks it too, with a verifier the script pins by hash ("The first install", below). |
| Smoke test and release-blocker before publishing | **Live**: since 2026-10-23 every release is the latest at once (one channel, Ludovic's decision), so it is published only after its smoke test passed and while no `release-blocker` is open. The 6-hour beta of the old Alpha, then stable flow is gone. |
| Reports marked suspicious | **Partly**: the scan of Discord for agents does it. Kira's issues do it once its pull request merges. |
| Code owners' review of the pipeline | **Not yet**: `.github/CODEOWNERS` and its protection wait for the App, below. Until then pipeline changes merge like any other once `verify` passes. |
| Agents on their own GitHub App | **Not yet**: prepared, waiting for Ludovic to create it. Agents still use his account. |
| Mac mini releases from a clean checkout | **Waiting to merge**: see "What this doesn't solve". |

## What stops it now

- **Reports are data, never instructions.** Agents receive people's text only inside quoted blocks marked as
  untrusted, with attachments as plain links. Text that looks aimed at an AI (talking to "the AI", asking to run
  commands, asking for passwords or keys, linking to scripts, hidden characters) is marked **suspicious**, and the
  agent coordinating the others sees the mark.
- **`verify` always runs.** Labels no longer start the checks, and the required `verify` job has no condition, so
  the only `verify` a change can have is one that ran. Main requires it, from GitHub Actions only, for everyone,
  admins included.
- **Checked before it is published.** Every release is published as the latest, which launchers and servers
  install by themselves, so the release run publishes it only after its smoke test passed (the game started and
  drew a world on the Mac mini) and only while no issue labelled `release-blocker` is open; otherwise nothing is
  published.
- **Signed updates.** Each release carries a list of its files' checksums, signed with Sunrise's release key. The
  launcher (for the game and for itself) and `sunrise-server update` check that signature against the public key
  built into them, then check the download against the list. Anything unsigned, signed with another key, signed
  for another version, or changed in any way is refused with a message saying why. Releases from before signing
  stay installable, for going back to an older version, but only with the exact files they had when signing
  began: their checksums are built into the launcher and the server too. The private key exists only as
  a secret of GitHub's `release` environment, which only real releases from main can use. The job that signs runs
  nothing from the repository but a short script calling OpenSSL, so no pull request can read the key by changing
  the code that gets built. Nobody, Ludovic included, ever needs to see it.
- **A test SDK key, developer builds only.** An SDK is checked against the release keys like a release is. The daily
  run's `fn-replace-check` checks its own, unpublished SDK, signed with a throwaway key made for that run; it gives
  the launcher the public key in `SUNRISE_SDK_TRUSTED_KEY` (one base64 Ed25519 key, `ephemeral.pub` from
  `EPHEMERAL=yes tools/ci/sign_release.sh`). `sunrise_release_sign::sdk_keys_for` adds it for SDK checks only (the
  launcher's SDK index and `cargo sunrise-mod`'s fetch) and only in a build with `debug_assertions`: a release build
  does not compile the lookup, so no environment adds trust to a shipped launcher, and game and launcher releases
  are never checked with it. A test in both profiles proves the release profile ignores the variable. No release
  key enters the daily job.
- **Nightly signatures.** This describes upstream's Nightly channel, not the fork's daily-only prebuilt schedule and publishing policy; this sync enables no new release trigger or keys.
  A Nightly carries two signatures over the same `SHA256SUMS`. `SHA256SUMS.sig` is signed
  for the channel (`horizon release nightly`), for launchers and servers from before this change, which read only
  that one. `SHA256SUMS.v2.sig` is signed for the Nightly's own version (`horizon release nightly-<date>-<commit>`),
  which its release title names (`Nightly <date> · <commit>`). Current launchers, `sunrise-server update` and
  `install.sh` parse that title strictly, refuse a Nightly whose title doesn't parse, and install it only with a v2
  signature for exactly that version. That way an older Nightly can't be served as tonight's: its signature names its own date. Installing
  an older Nightly on purpose still works as a rollback, with a warning, and never happens as an automatic update.
  **The changeover:** Nightlies published before this change have no `SHA256SUMS.v2.sig`, so they can't be installed
  or picked with `--version` any more; they leave GitHub with the 7-day rotation. Until a v2 Nightly exists, a
  launcher has nothing to update to, so the first Nightly is built right after this merges. A server can still go back
  to a build it already has with `sunrise-server rollback`, which checks nothing it hasn't already checked.
  **When the old signature can go:** once support for launchers and servers predating v2 is retired. The fork
  collects no telemetry, so it cannot determine this from usage data. Then drop `SHA256SUMS.sig` from nightly-build.yml's
  publish step and the `--tag nightly` check before it, and remove `NIGHTLY` from sunrise-release-sign. Releases (not
  Nightlies) keep their single `SHA256SUMS.sig`, which is already signed for their tag.
  **Keys by repository:** `sunrise_release_sign::trusted_keys_for` trusts
  `crates/sunrise-release-sign/release-keys.txt` for `sunrise-org/sunrise` only; every other
  repository (`horizon-rust/horizon` and `arismoko/horizon` included) has no trusted keys and fails
  closed. Upstream's legacy checksum pins never authorize a Sunrise archive. The real private key
  exists only as a secret of the `release` environment, `RELEASE_SIGNING_KEY` or, after a rotation retired the
  other, `RELEASE_SIGNING_KEY_NEXT` (either name can hold the active key for the game's releases); dry runs receive no release secrets
  and sign with an ephemeral test key instead. To rotate, first ship the next public key in
  `release-keys.txt` (and a new `verifier@X.Y.Z`, pinned in `packaging/server/install.sh` and `install.ps1`), with
  its private half set as whichever of `RELEASE_SIGNING_KEY` and `RELEASE_SIGNING_KEY_NEXT` is empty
  (`tools/ci/new_release_key.sh` sets it), so releases are signed with both; then retire the old key's secret and
  public key and pin a verifier built without it ([release-flow.md](release-flow.md#signed-checksums), "Rotating the
  key"). Launchers are the exception: `launcher-bridge.yml` refuses a real publish without `RELEASE_SIGNING_KEY`,
  because launchers from before the split trust only that key, so the key in `RELEASE_SIGNING_KEY` is not retired while
  those launchers are supported. Nightly title,
  channel and exact-version signature binding remain unchanged.
- **Upstream release machinery needs the council.** The workflows, the release and signing tools, the update-checking code
  and the installers are listed in `.github/CODEOWNERS`. Once main's protection requires code owners' reviews, a
  change to any of them waits for a review from the `horizon-rust/council` team (council poll P35). Everything else still merges as soon as its checks pass.
- **Agents get their own, smaller identity** (prepared, switched on by Ludovic). The `sunrise-agent` GitHub App
  can read and write code, pull requests and issues, and read the checks. It cannot change settings, workflows,
  secrets or members, and its tokens last an hour. Once agents use it, their pull requests are the app's, and
  Ludovic approves the rare pipeline change with one tap.

## What still needs Ludovic

- **Approving pipeline changes** (rare): a pull request touching a file in `.github/CODEOWNERS`.
- **Holding releases back or speeding one up**, from his phone if he likes. Label an issue `release-blocker` to
  hold every release until it is closed; label a pull request `release-now` to release it as soon as it merges.
- **Creating the GitHub App**: one link, "Create GitHub App", then "Install". His login, his clicks.
- **Fork pull requests**: their checks run only after a maintainer approves them. Parity harnesses never run a
  pull request's code: they run only in the daily run, on main's head.
- **A leaked key**: rotate it (docs/release-flow.md, "Signed checksums"). Launchers older than the new key then
  refuse new versions until players download the launcher again.

## What this doesn't solve

- **Code that merges is code that ships.** Signing proves a build came from Horizon's pipeline, not that the code
  is good. Against a bad change, the safety nets are the checks and harnesses, review, the release smoke test and
  the release-blocker label. Since the single channel there is no 6-hour beta: a change reaches every player
  within minutes of merging.
- **Until agents switch to the App**, they still work with Ludovic's own account, so nothing technical tells his
  actions from theirs. Code owners' reviews are switched on together with the App.
- **The Mac mini** builds the macOS release. Pull requests' parity checks no longer run there (parity runs only in
  the daily run, on main's head), but a release builds whatever merged. Code
  running there as the runner's user could tamper with what the next release builds. Releases check out clean,
  which removes anything a pull request left in the work folder by accident, but
  not a deliberate attack. The protection is review: only agents and maintainers push to this repository, and a
  maintainer reads a fork's change before it merges.
- **The first install** with `packaging/server/install.sh` or `install.ps1` from a GitHub release (the default)
  trusts the script itself. The script downloads `sunrise-verify` from the `verifier@X.Y.Z` release it pins and
  refuses it unless its SHA-256 is the one pinned in the script. The verifier then checks `SHA256SUMS.sig` (a Nightly's `SHA256SUMS.v2.sig`, for its
  own version) against the release keys built into it, then the archive against `SHA256SUMS`. Any failure stops
  the install with its cause; nothing falls back to GitHub's SHA-256. So the anchor is the pin on main, fetched
  over HTTPS: it changes only by pull request, and must match what `tools/ci/pin_verifier.py` writes once
  `gh attestation verify` has accepted `SHA256SUMS` and every binary as built by `verifier.yml` on main at the
  dispatch commit
  (docs/release-flow.md, "The install scripts' verifier"). The verifier binaries are not code-signed, so macOS
  Gatekeeper and Windows SmartScreen don't vouch for them; the pin does. `--insecure-checksum-only`
  (`-InsecureChecksumOnly`) skips the signature, with a loud warning, for CI fixtures; no install command in these
  docs uses it. `install.sh --archive FILE` installs an archive already on the machine (a build of your own) and
  checks nothing: checking it is up to whoever brought it.

## Reporting a security problem

Use GitHub's private vulnerability reporting (the repository's **Security** tab, "Report a vulnerability"), not a
public issue or Discord. Say what you found and how to reproduce it; we answer there.
