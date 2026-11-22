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
- Sunrise deleted the Android input modules the base had taken from Starling (c878ccf19f): android_gamepad.rs,
  android_runtime.rs (+tests), keypad.rs, touch.rs and touch/* in sunrise-client-render. The merge dropped them without
  a conflict (Starling hadn't changed them since the base); they are back from Starling's side, declared in lib.rs.
  The list of every file Starling had that the merged tree lacks is in the tools folder (gone-after-merge.txt, 1531
  paths, mostly Sunrise's own moves): check it against what Starling's code still names once the build runs.
- FEATURE TO RE-PORT: the Stats performance overlay (`Overlay::Stats`, stats_overlay.rs, our perf_overlay.rs edits in
  ours-diffs). Sunrise moved the overlay to the crate sunrise-perf-overlay and its key out of sunrise-graphics.txt;
  graphics.rs took that. Anything of ours still naming `g.overlay`/`Overlay::Stats` must go through the new crate.
- ui.rs: `load_skin` (Starling's low-budget skin) is called from startup_menu.rs's set_root; a pack reload in a world must call it too (packs/mod.rs is Sunrise's).
- options_ui.rs: the wardrobe's drag on the skin preview (the `Page::Skin && self.wardrobe_enabled` block of the click
  handler) is out: Sunrise's click handler now returns `(Vec<Event>, bool)` and lost its splitscreen pages. Put the
  block back in that shape (ours-diffs/options_ui.7.wardrobe-click.diff).
- SERVER SIDE IS SUNRISE'S (owner's call, 2026-11-22: only the client's conflicts get hand care). 39 files of
  sunrise-client's gameplay, world, LAN, network messages, mobs and mod hooks took Sunrise's whole; Starling's diff of
  each is in the tools folder (ours-diffs/server/*.diff). What must come back on top, by feature:
  - Create and content mods: mod_content.rs (+103), mod_content/load_conditions.rs (+110), lan/chunks.rs (+47),
    crafting.rs (recipes whose serializer is gone are left out).
  - Old native mods' names (native_hooks/*, game/world/levels.rs, lan_actions/turns.rs, dispenser.rs): the
    compatibility for mods made for Starling 0.8.0.
  - Worlds: save.rs (+246: level.dat_old when level.dat is missing and the like) and game/world/saving.rs (+92).
  - Android in game/world/integrated.rs (+37: worker stacks) and native_hooks/shared.rs (safe zones).
  - mobs/render.rs (+94) and mobs/data.rs: ItemsAdder's custom armour and dyed leather on servers.
  - mobs/ai.rs and mobs/living.rs: the cheaper mob collisions (see PERF TO RE-PORT above).
  - The API backport's host side (stations.rs, menu.rs, native_client_hooks.rs, native_player_*.rs, spill.rs,
    armor_stands.rs, use_hand.rs): Sunrise has its own.
- gfx.rs is Sunrise's; 72 of Starling's 112 edits there are carried into gfx/*.rs by exact context. The 40 left are in
  ours-diffs/crates_sunrise-client-render_src_gfx.rs.N.rej (rejs.py lists them with where their neighbours are now):
  the renderer's Android fields (solid_split, late sky, flat cache, start marker), Adreno 506's depth format, the
  loading screen's lent device, the black first frame, the paused-menu blur frame, ray tracing's world.
- CREATE PORT GOES (owner, 2026-11-22: "não deveria" be in the game). Do not carry or re-port it; remove it while
  making the build pass: game/starling/{arms,basins,bearings,belts,chutes,fans,funnels,kinetic_machines,kinetics,
  machine_entities,processing,saws,falling_trees}.rs, src/kinetics.rs, item_sync/kinetic.rs (about 9,300 lines), their
  fields and calls in game.rs and game/blocks/machines, the `kinetic`/`machine` declarations in sunrise-mod-package's
  content.rs with their tests, models/catalog.rs's kinetic shapes, the Create section of THIRD_PARTY_NOTICES.md and
  licenses/create-MIT.txt. The items above that say "Create" are void; `[assets.fetch]` stays only if another mod
  needs it. (Vanilla's "kinetic weapon", the spear, is not part of this.)
- Ray tracing for native shader packs (#470, experimental): sunrise-shader-api's check.rs took Sunrise's (it split
  validation into `validate`). Ours validated with Capabilities::RAY_QUERY for a `[ray_tracing]` pack, passed that to
  `translate`, and checked the SPIR-V (ours-diffs/shader-api-check.raytracing.diff). The celestial pipeline's kept
  descriptor (for the late sky's far-plane sun and moon) and the `renderer.other_pipelines` startup step are also out:
  Sunrise builds that pipeline in a helper now.
- Touch and the interface's own scale: pad_menu.rs took Sunrise's `projection.window_pointer(at)` where Starling had `interface_window_point`; books, chat and signs keep Starling's interface batch and pointer inside Sunrise's click handlers (which now return bool). Check clicks land where drawn with a separate interface scale.
- Client input, where Sunrise reworked the keys (held keys by device, vanilla mappings): these took Sunrise's side in
  the conflict and need Starling's part put back in the new shape (our hunks are in ours-diffs/client/*.hunks.diff):
  - play_input.rs: `sync_toggle` held sneak down while the touch sneak button is on (`touch_sneaking`).
  - play_keys.rs: on a server, F called `remote_swap_pressed()`.
  - creative_screen.rs: the Bedrock-style catalog (`bedrock_items`) hid the tabs' tooltips and clicks while its panel
    is open; only its empty-cell click is kept.
  - names.rs: name tags turned by `camera_text_axes()`; Sunrise turns them by the camera's orientation.
- Files on Android's shared storage: Starling read and wrote stats, advancements and the like through `sunrise_client::save::read_file`/`write_file`; save.rs is Sunrise's now (`write_durable`). Check those paths still work in shared storage (no hard links, no rename across folders).
  - game/world/autosave.rs also took Sunrise's whole (ours-diffs/server/game_world_autosave.rs.diff).
  - game/network/remote/mod.rs (+964 of ours) took Sunrise's whole too: other players' skins as Starling draws them (3D layers, the wardrobe's previews, Ely.by), old mods' network messages and the API backport's client side were in it.
- THE START (game/runtime/loading_screen.rs, loading_window.rs): Starling's loading flow is kept (the loading screen's
  own renderer `boot_renderer`, `RendererParts` built on a thread, `finish(loaded, built)`, `poll_handoff`), on
  Sunrise's start without the splitscreen wrapper (`run_app(&mut game)`, `finish` returns `Game`). To join with the
  compiler (both sides' hunks are in ours-diffs/client/loading_window.*.diff):
  - Sunrise's `build_renderer` returns `(Renderer, clouds result)` and passes `(draws_flat, starts_wide)`; ours returns
    the renderer alone and builds through `RendererParts`.
  - Sunrise takes the worker's result with `take_worker_result()` (keeps declaration refusals); ours `poll_handoff`
    takes `loaded` itself.
  - The end of `start_game` is Sunrise's (`PendingClientStart`, `apply_pending_client_start`): ours joined the resource
    worker first when a world's mods were to load (`wait_world_rest`), skipped the accessibility onboarding on
    Android, and set `pending_entry = world_content::next_entry()` after the title.
- game/modding/mods/mod.rs is Sunrise's content loading (ContentRequest, admission, ModsReload) with Starling's
  phone parts: no registry names before the game files exist, the cache folder outside shared storage, the mods a
  world's own set is compared with (`world_content::set_running`/`set_loaded`), `display_name`. Left out with the
  Create port: fetched assets (`fetched_assets`, `mod_assets.rs`, the `Fetching` note). To check: a reload from the
  title kept the player's switches (`wanted`), and Sunrise's `apply_mods_reload` sets them to what loaded.
- game.rs is resolved: Sunrise's tick, HUD layer dispatch, pointer projection and mouse-button routing, with
  Starling's fields, performance overlays, phone offers and imports, the keyboard's lift, the world's own content set at
  startup, and a finger's pixel scroll. Starling's edits that sat inside code Sunrise moved are not carried yet (both
  sides' hunks: ours-diffs/client/game.ours.diff and game.theirs.diff):
  - the HUD on a phone: the attack indicator's side (`hotbar_with_attack_side`), the safe-zone margins around the
    sleep overlay and the HUD layers (hunks 12-14), `hud_room` no longer set in `hud` (hunk 11);
  - window events: `platform_keypad_event` first in `handle_window_event_inner`, `platform_held_modifiers` (Android's
    winit sends no ModifiersChanged), the cursor in the interface's own scale on CursorMoved, the title's player
    press/release, `bedrock_held` on a mouse release (hunks 21-28).
- ALL CONFLICT MARKERS ARE GONE (2026-11-22). Every changed .rs file parses (rustfmt). Nothing has been compiled yet:
  next are Cargo.lock (Starling's crates), `cargo check` rounds on the build machine, then the items above.
