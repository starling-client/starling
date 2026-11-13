# Sync with Horizon 5 (temporary, delete after the merge lands)

Upstream commit merged: `c98a51a664` (2026-11-08). Base `31f83499b6`. Starling before the merge: `121742ad89`.
Only this branch goes to `starling-client`: nothing merges into `starling` or `android`, nothing is released.
The release comes first; before merging this, `starling` has to be merged in again and the owner tests it.

The merge commit `06d7176a62` holds its conflict markers on purpose (307 hunks in 83 files); each group was
resolved in a commit of its own.

How to work and tools: `sync5/` in the session notes folder
(`%APPDATA%/Claude/side-session-notes/local_4c49e3d5-8a78-4799-b072-15ba12d5ea8a/`): `BRIEF.md`,
`COMPILE.md`, `FOLLOWUPS.md` (what fell, in detail), `vm.sh MODE` (check, all, android, clippy,
clippy-android, fmt, test, raw CMD). On the VM: `~/sync5`, `~/sync5-target`, `~/sync5-sweep`, `~/sync5.log`.

## Steps
- [x] Merge with markers: `06d7176a62`
- [x] A. Docs, CI, manifests, languages: `7731f158b7`
- [x] B. Render, shaders, assets: `cc75569f5c`
- [x] C. Screens, options, mods: `c7e91f5431`
- [x] D. Network, gameplay, input: `6b56ffa0e4`
- [x] E. game.rs and crate roots: `e42338e694`
- [ ] **Next: desktop and Android build** (`vm.sh check`, `vm.sh android`); uncommitted work may be in the worktree
- [ ] Put back on upstream's new pages what fell from them: the Shader Packs rows (Resolution, FPS Limit, Auto
      profile) on `shader_ui.rs`, the Android Controller list and stick rows on `controller_ui.rs`
- [ ] `MIXINS_REV` to upstream's `5a64af938ec86b00d53d16a408aabc0038fb3e57` (`native_build.sh`, `launcher/src/native_mods.rs`)
- [ ] fmt, clippy (desktop and Android), tests
- [ ] Draft PR `[NÃO MESCLAR]` against `starling` for the org's CI and a Preview APK; no auto-merge
- [ ] Sweep "after" of the site's 104 native mods; compare with "before" (`~/sync5-sweep/before/job/out`)
- [ ] Check the last sync's regressions: accessibility screen on first run, focus and keyboard in
      `enter_singleplayer`, the loading stall (`follow_loader`, `Loading::retry`), upscale and FPS on Mali

## What fell so far (details in FOLLOWUPS.md)
- Create's OBJ-shaped blocks: upstream's OBJ loader replaced Starling's reader; they need porting to it.
- Starling's own block predictions: upstream's `Prediction` does the same.

## Resuming
The VM keeps running what was started detached (`~/sync5-sweep/before/run.sh`). `~/sync5-holdwatch.sh` takes
`~/modbuild/hold` away after 30 minutes without sync work; every `vm.sh` run touches it again.
