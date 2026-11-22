# Starling performance: what was measured, what was ruled out, what is left

Notes from a session of measurements on 2026-10-28, for whoever looks for a way to make Starling (the
Android port, branch `android`) draw faster. Read this before measuring again: it says what the numbers
are, which ideas they already ruled out, and how to build and try a change on the phone in minutes.

## The phone and how the numbers were taken

- POCO F6 Pro (Snapdragon 8 Gen 2, Adreno 740, 3200x1440, 120 Hz), Android 16, Starling 0.1.2 release
  (`--profile fast`), Vulkan, Mailbox present mode.
- Numbers are the in-game overlay's (F3-style stats in the corner): fps, frame, `cpu` (render thread's
  work per frame), `gpu` (the frame's GPU time from the timestamp queries), chunks, draw calls, CPU MHz.
- A YouTube picture-in-picture window was open during most shots: it uses some GPU and CPU.
- Test APKs installed with the quick profile (`build_apk.py` default) build `horizon-client` and
  `horizon-android` at **opt-level 1**: never measure with them. Use `--profile fast` (what releases use).

## Measurements

Presets (graphics.rs `apply_preset`): Performance = render distance 8, no clouds, no cutout leaves;
Quality = render distance 16, fancy clouds, cutout leaves, biome blend 2, mipmaps 4.

| Scene | Distance | Resolution | fps | cpu | gpu | Notes |
|---|---|---|---|---|---|---|
| Grass field, Performance | 8 | 100% | 138 | 4.6 ms | 5.8 ms | 2 draw calls (flat draws) |
| Island, Performance, no clouds | 8 | 100% | 161 | 3.9 ms | 5.1 ms | |
| Island, Performance, fancy clouds | 8 | 100% | 144 | 4.7 ms | 4.8 ms | clouds ~free at 8 |
| Island, Quality, no clouds | 16 | 100% | 68 | 6.0 ms | 13.5 ms | 40 draw calls, 806 sections |
| Island, Quality, fast clouds | 16 | 100% | 66 | 6.2 ms | 14.0 ms | |
| Island, Quality, fancy clouds | 16 | 100% | 56 | 7.7 ms | 16.5 ms | fancy clouds ~3 ms at 16 |
| Sky only (3 sections) | 16 | 100% | 259 | 2.9 ms | 2.4 ms | sky + HUD + full-screen passes |
| Grass field, Balanced | 12 | 50% | 108-121 | 6.3-7.8 ms | 6.4-6.6 ms | |

Resolution sweep, island, Quality, distance 16, no clouds (on the no-cutout test build below, about
1 ms under the release at 100%):

| Resolution | Pixels | gpu |
|---|---|---|
| 100% | 1 | 12.5 ms |
| 75% | 0.56 | 9.8 ms |
| 50% | 0.25 | 7.5 ms |
| 25% | 0.06 | 6.3 ms |

This fits `gpu = 5.9 ms + 6.6 ms x pixels`: about **6 ms that do not depend on the resolution** (the
terrain's geometry for ~880 sections, plus the HUD and the final upscale; the sky-only shot puts the
non-terrain part at 2.4 ms at most) and **6.6 ms of pixels** at full resolution (7.5 with the cutout).

The CPU never went past **1.6 of 3.19 GHz**: `android_lib.rs` `frame_hint` tells ADPF a target of 120 fps
even with Max Framerate on Unlimited (`cap.unwrap_or(120)`), so the scheduler keeps the clock low while
frames fit.

The "1% low" stayed between 10 and 47 fps and the worst frame between 13 and 29 ms in every shot: frame
spikes, not looked into yet.

## Ruled out

- **Opaque terrain apart from cutout terrain** (so solid blocks draw with a shader without `discard`
  and the tiler's early depth test / LRZ works). Every solid quad draws with `fs_solid`
  (`terrain.wgsl`), which discards below alpha 0.5. A build with **no discard at all** (branch
  `exp/opaque-terrain`, never merged) cut the island's GPU time from 13.5 to 12.4 ms (8%): not worth
  the change across every draw path (flat, indirect, culling phases, shadows, LOD tiles).
- **GPU occlusion culling**: off on Android on purpose (`gfx.rs`, before `cull_on`): measured on an
  Adreno 740 it saved no GPU time, and a block placed or broken sometimes left terrain out.
- **LOD Distance** (`lod_distance`): draws simplified terrain beyond the render distance, which adds
  work, it does not take any away.
- Terrain sampling is `Nearest` without anisotropy; the vertex is ~24 bytes (`TERRAIN_ATTRIBUTES`).

## Cheap, but they do not make the heavy case faster

- ADPF target 240 when uncapped (CPU clock up): helps CPU-bound scenes only.
- LTO for releases (`[profile.release]` is thin LTO, codegen-units 1; releases use `fast`, no LTO,
  because LTO ran the runner out of memory before the swap step): a few percent of CPU.
- `apply_mobile_defaults` (graphics.rs) caps phones at **60 fps** on purpose (cooler). A player who
  changes nothing plays at 60 even on a 144 Hz screen: the likely reason another Android fork was
  reported at 144 fps "with the same settings" against Starling's 60-70.

## Ideas not tried, for the GPU at distance 16

1. **Far rings as LOD meshes**: the LOD tile meshes exist; drawing them for the outer chunks of the
   render distance (say 9 to 16) instead of full sections would cut much of the ~6 ms of geometry.
2. **Greedy meshing**: sprites are layers of a texture array sampled with `Repeat`, so coplanar
   neighbouring faces with the same texture, tint and light could merge into one quad (flat ground,
   water). Smooth lighting and AO limit where it applies.
3. **A smaller vertex** (24 to ~16 bytes): positions section-local in 16 bits; shader packs read the
   layout, so they need care.
4. **The pixels**: whether the full-screen sky (`sky_along_ray` per pixel) or the water costs more
   than it should; a lower default resolution on phones (Bedrock does that).

## What another Android fork does (Vasysik/horizon-android, MIT, read 2026-10-29)

Nothing there cuts the terrain's GPU work (no LOD rings, no merged faces). What differs from Starling:

- **Public APKs use the release profile** (opt-level 3, ThinLTO, codegen-units 1; commit 44998be5),
  where Starling's releases use `fast`.
- **No 60 fps cap on phones**: it has no `apply_mobile_defaults`, so a phone starts with Horizon's
  defaults. With this and LTO, its "144 fps against 60-70" needs no faster renderer.
- **Whole-frame resolution by the window's buffer** (`android/display.rs`, commit b302a6e9): the
  native window's buffer itself is made smaller (presets 100/85/75/67/50/25, default 100), so Android's
  compositor scales it to the screen in its hardware and the GPU draws everything smaller, the HUD too.
  Starling's resolution scale renders the world smaller, then upscales it on the GPU to 3200x1440 and
  draws the HUD at full size: part of the ~6 ms that do not follow the resolution may be that.
- **Bounded per-frame work against spikes**: overdue chunk packets get at most 4 ms a frame on
  Android instead of the whole burst at once (`game/remote.rs` `poll_remote`, commit 57723c4d), and
  mesh uploads have a per-frame budget (2 to 8 ms, 8 MiB; `gfx/upload_budget.rs`, commit 4f9f4f8e).
- An experimental OpenGL ES backend for old phones (compatibility, not speed); no ADPF and no
  `ANativeWindow_setFrameRate`.

## How to try a change on the phone

- **A test APK that installs over the release** (keeps the phone's data):
  `gh workflow run android.yml --repo starling-client/starling --ref starling -f release_key=true`. It signs
  with the release key and builds with the fast profile, does not publish, and the artifact is kept one
  day (`gh run download <id> -R starling-client/starling -n starling-apk`). About 11 minutes. Only the
  `starling` branch and v* tags reach the key's `release` environment: a branch of its own is tried as a
  Starling Preview (any push or `workflow_dispatch`), which installs beside the release.
- **adb**: wireless debugging, the phone at home on 192.168.1.x (its address changes; the port too, the
  owner reads it off Developer options); `adb connect <ip>:<port>`, `adb install -r <apk>`. Xiaomi's
  "Install via USB" sometimes refuses an install without asking (`INSTALL_FAILED_USER_RESTRICTED`):
  try again, or push the APK to `/sdcard/Download` for the owner to open. Warn the owner before
  installing anything; the phone is theirs and in use.
- **Logs**: `/storage/emulated/0/Starling/logs/latest.log` (and `previous.log`) over adb.
- **Measuring**: the same place, the same direction, the same settings, a screenshot of the overlay
  before and after; one change at a time. The island shots above are at a river with a small wooded
  island in a world the owner has.
- The Azure build VM (`starling-build`) builds and checks in a few minutes (`try_ref.sh`, see the
  project notes); deallocate it after use.
