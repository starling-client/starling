# Sync Sunrise 1: deferred items

- tests/test_screen_mouse_dragged_hook.py, tests/test_world_input_capture.py: took Sunrise's side; our fixtures
  (interface_point from hud_room.rs, regrab_after_run) must be put back once window_handler.rs is merged.
- tests/test_nightly.py: Sunrise's new nightly classes need our SUNRISE_RELEASES skip (no nightly-build.yml here).
- docs/sunrise-parity.md removed: Sunrise moved the audit to docs/status.md (tests/test_parity.py reads it).
- crates/sunrise-assets/src/model.rs: took Sunrise's from_json_with_loaders (native composite models); our
  flatten_composite is now unused: remove it, or keep it only if NeoForge composites still need it (test).
- crates/sunrise-protocol/src/entity_data.rs: Sunrise dropped the legacy wolf/painting quirks; ours kept only OptGlobalPos.
- PERF TO RE-PORT: "Cheaper mob collisions, with the same results" (b6ac272312, a3fc54328a): MovedBlock/Collider/Collides
  in core geometry movement.rs and world_movement.rs. Sunrise added OrientedCollider through the same functions, so
  the three files took Sunrise's whole. Re-apply on top (generic `S: Collides` beside `oriented`) and measure.
  (also block_collision.rs / block_collision_scan.rs: the full-block flag and next_collider.)
- crates/sunrise-mod-vocab: Sunrise's whole. Our API backport (5cdd4312ec, "the items 25 catalog native mods name")
  is in Sunrise under its own shapes (entity_body_yaw, player_destroy_block, player_place_block, gui_entity, the two
  hooks). The host side of our backport in sunrise-client (stations.rs, menu.rs, native_client_hooks.rs, lan/*) must
  follow Sunrise's signatures; check the catalog mods still fit (hz_fit.py) after the build.
- sunrise-mod-host: Sunrise dropped typed slot listeners (mod_points), the guest stack (prepare_main_stack,
  exit_if_compile_child) and CodeRuntime. `open()` returns entries + built only; `load_code(mods, native, launch)`
  returns nothing. android_lib.rs must follow (no slots, no code_runtime, entry::start's new signature).
  Our wasm refusal text (starling.mods.wasm_removed) is gone: Sunrise refuses a wasm manifest at parse.
- #836 (hide-wasm-mods, sync-horizon-7) must be carried to sync-sunrise-1: starling_store::not_played and both catalogs' filters.
- FEATURE TO RE-PORT: fetched assets (`[assets.fetch]`, crate::fetch, Content::read_with, FileReader's `fetched`
  overlay) in sunrise-mod-package content.rs. Sunrise rewrote the content source (ContentSource, AdmittedContentSource,
  FileReader over the opened archive), so content.rs took Sunrise's reader and kept only our declarations (kinetic,
  machine, models) and tests. Our whole diff is in ours-diffs/mod-package.diff. Create's assets depend on this.
- .hmod LAYER (Horizon's registry and packages on a Sunrise base), to rebuild on purpose:
  - sunrise-mod-package index.rs took Sunrise's check; ours let a native mod be `.hnmod` and read capabilities it
    doesn't know (wasm mods in the index) without refusing the whole index.
  - rebrand.py turned our Horizon-registry code into "sunrise" names: launcher/src/browser/sunrise.rs (was
    horizon.rs, the Horizon registry browser; our 12-line diff is in ours-diffs), `.hmod` -> `.smod`,
    `horizon-mod.toml` -> `sunrise-mod.toml`, the folders `horizon-mods`/`horizon-native-mods`.
  - Players' folders from 0.9.0 keep the old names: migrate or read both.
- launcher/: Sunrise moved its launcher to another repository (43d0b834b4). starling-catalog compiles
  launcher/src/browser*, so that tree (17 files, as Starling had it) is back and is Starling's to carry now.
- README.md and CONTRIBUTING.md are Starling's as they were, renamed by rebrand.py: they must say the fork is based on
  Sunrise (with the link), unofficial, under Sunrise's licence (LICENSE is Sunrise's file), and credit Horizon where
  the text now wrongly says Sunrise. Links rebrand.py broke: Vasysik/sunrise-android is Vasysik/horizon-android
  (THIRD_PARTY_NOTICES.md, sync-check.yml, gfx/*.rs comments, starling-account).
- Cargo.lock is Sunrise's: add Starling's crates back with `cargo metadata` on the build machine (not --locked once).
- .github/workflows: ours as they were, renamed by rebrand.py; check every name it changed that isn't a crate (env vars HORIZON_* -> SUNRISE_*, artifact names, branch names fixed by hand) when CI first runs.
- Shader pack crate (sunrise-render-shaderpack): Starling's edits to pack_post, pack_uniforms, pack_entities and
  pack_lines are carried (exact context, then five by hand: Pass.prepass, shadow_every/shadow_wait, programs()/report(),
  the kept shadow map in shadow_view, now `&mut self` in client-render's PackPostExt). Still owed there:
  - tests: `a_layout_lists_groups_up_to_the_last_used` and `alpha_cut: None, prepass_uv: None` in three Interface
    literals (ours-diffs/pack_post.left.rej hunks 6-9); a Frame literal's `menu_blur_cache: false`.
  - gfx.rs has a second caller of shadow_view (`.as_ref()` must be `.as_mut()`), once that file is merged.
- HUD: all 14 atlas/batch edits and the 10 tests are in hud/*.rs and hud/tests/atlas.rs. Sunrise already ignores PNG
  checksums (our line dropped as a duplicate) and no longer swaps the title banner to "RUST EDITION".
- Compile store (mod-builds, 2026-11-22): the base is rebuilt in the sandbox's environment under cache key store2-;
  mod.yml/catalog.yml read it once branch store2-readers is merged.
