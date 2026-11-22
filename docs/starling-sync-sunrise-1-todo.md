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
- Cargo.lock: Starling's crates are in it again (taken from the build machine after build 14).
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
- The Stats overlay is back on its own (stats_overlay.rs keeps its frames, game/starling/perf.rs its switch): the
  three-finger tap and `overlay stats|off`. Still owed: the choice is not saved (it was `overlay:stats` in
  sunrise-graphics.txt, a key the crate sunrise-perf-overlay owns now), and the compact frames counter under the touch
  Back button over menus is gone (Sunrise's layer draws at the top right, over the utility buttons: check on a phone).
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
- BUILD ROUNDS (2026-11-22): sunrise-client-render compiles for Android. Dropped on the way: the old .hnmod index and
  the fetched assets (sunrise-mod-package), Starling's protocol edits (item components for servers' custom armour,
  the old mod messages `mod_net`), `starling_mod_api.rs` (names for mods made for 0.8.0), `kinetics.rs` (Create).
  `stats_overlay.rs` is in the tree but not compiled (it reads `perf_overlay::FrameSummary`, which Sunrise's
  sunrise-perf-overlay crate doesn't have). Create World's kept options no longer keep the world type across the
  restart for a world's content mods.

## What the build rounds dropped (builds 6 to 14), to put back or decide

`cargo ndk check -p sunrise-android` passes at build 14. These went to get there:

- Create port: removed whole (kinetics, machines, basins, the 15 files of game/starling, their fields and draws).
- Old mod hooks (native_compat, starling_mod_api, old_mod_net): removed. Mods built for 0.8.0's hooks don't load.
- Server stick mode (`ServerStick` per server, the prompt dialog, stick_memory): the connection code is Sunrise's, so
  `remote_stick_mode` is gone and the touch stick asks nothing (`picked.unwrap_or(false)`). stick_memory.rs is still
  there, unused. Re-port onto Sunrise's remote/mod.rs.
- SRV lookups on Android (`sunrise_protocol::set_name_servers`, resolver's NAME_SERVERS): the protocol crate is
  Sunrise's. A server address that needs its SRV record may not resolve on a phone. Re-port (small).
- Custom name tags of other entities on a server (`remote_name_tags`, `r.names`) and `camera_text_axes`: names.rs is
  Sunrise's whole.
- Deferred file writes (`save::later`, `save::read_file`): gone with Sunrise's save code; advancements read the file.
- 3D skin layers (`three_d_skin_layers`, `skin_3d`): Sunrise removed them; the wardrobe no longer prepares them.
- Mods page: the Modrinth-assets line and "Its assets could not be downloaded" (mod_assets), and the loading screen's
  "building" note for content mods; `.hnmod` bundles are no longer listed (only Sunrise's flat package reads).
- Split screen (split_guest, split_window, the pad router): Sunrise removed it; Starling's guards on it are gone.
- The launcher row leaves the game through `begin_closing_save` (Sunrise's title has no event loop to exit).
- HUD: the safe area starts at the hotbar; the spyglass, crosshair and sleep overlay take the whole screen. Sunrise
  draws the crosshair in its layer walk; Starling's two conditions are on it (touch hides it, Bedrock creative's gap).
- Toggle keys: sneak and sprint held by the touch controls go through `touch_set_toggle` onto the vanilla mappings'
  state. Check on a phone: sneak button modes (hold, toggle), flying down, sprint button, a keyboard's Shift/Ctrl.
- Background save: a typed request to the server thread (`OwnerRequest::SaveForBackground`), since Sunrise's server
  takes no closures outside tests.
- Bedrock Inventory in creative: `Creative::show` in place of writing `selected`. Check both windows and the tooltips.
- The player's preview on the title screen (wardrobe): `MobData` as the render data when there is no world.
- pad/mod.rs: Starling's version over Sunrise's (touch pad, keypad). Compare once more with Sunrise's for what it
  added since (the disabled-controllers path calls `pad_menu_frame` there).

## Inventory of what the merged tree lacks (2026-11-22, tools: dropped.py, dropped.txt)

`dropped.py` lists, per file Starling changed since the base, its added lines found nowhere in the merged tree.
Without the Create port and the old hooks (dropped on purpose), about 1,500 lines. By weight:

- game/network/remote/mod.rs (430): Starling's whole work on the connection. Sunrise split the file (session.rs,
  tick.rs, sync.rs, skin.rs, packets_*.rs, use.rs, chat.rs, menu_interactions.rs); Starling's diff is kept in the
  tools folder (ours-diffs/network/network.diff). To re-port, by feature:
  - the stick's modes on servers (Free Direction's steering and held-back hits, Keys, Analog; the setback count, the
    question dialog, stick_memory) and `interaction_rotation` for the look sent;
  - the logs that explain a player who "cannot walk" (movement gaps, rate, trail, setbacks, the chunk under the player);
  - the server's attributes and abilities for a player made after their packets (respawn, first teleport);
  - custom names of any entity (`names`, drawn by names.rs's remote_name_tags);
  - a proxy's reconfiguration (clear_remote_world, reset_listener, joined again);
  - the creative screen's grid, armour and off hand sent as SetCreativeModeSlot (creative_menu);
  - skins: Ely.by's for offline servers, the player's own skin and the wardrobe's cape in `skins` resolution;
  - legacy section-sign codes in server text and chat decoration (`resolve_legacy`);
  - a phone's budget for an overdue chunk backlog (OVERDUE_CHUNK_PACKETS_PER_FRAME);
  - key.swapOffhand sent at the tick (swap_presses); the touch shield in remote_use_hands/RemoteUseWorld;
  - remote_menu_clicks (a mod's slots.click), the account's name only on real servers, touch_vibrate on a break.
- sunrise-protocol item.rs (275) and entity_data.rs (37): items and entity data as plugin servers send them.
- save.rs (111): deferred writes (`later`). random_tick_sections.rs (75). core geometry movement.rs (118) and
  world block_collision*.rs: the cheaper mob collisions (already listed above).
- hud.rs (83), gfx.rs (31), options_ui.rs (30), ui.rs (11): check each against hud/*.rs and gfx/*.rs.
- mobs/render.rs (36), spawn_rules.rs (33), ai.rs (19), living.rs (16), armor_stands.rs (30), names.rs (29),
  stations.rs (30), menu.rs (40), item_sync.rs (11), crafting.rs (9), net.rs (15), integrated.rs (19),
  autosave.rs (8), world_rest.rs (25, Create), machines/mod.rs (50, Create), mod_content*.rs (90, fetched assets).
- native-mods/wrench (240): the wrench mod's mixins.
The full list is dropped.txt in the tools folder; each line must end as re-ported, already in Sunrise, or dropped
on purpose with a reason.

### State of that inventory (2026-11-22, after the re-ports; `cargo ndk check` passes at 4c7133f014)

Re-ported: SRV name servers; the connection (remote/starling.rs: the stick's modes with the question and memory,
Free Direction's held-back hits, the movement, setback and chunk-wait logs, the creative grid; and in Sunrise's
files: `resolve_legacy`, abilities after a respawn, names with remote_name_tags, the overdue chunk budget on a phone,
Ely.by and own skins with the wardrobe's cape, the touch shield, the account's name on real servers only, the look
a placement tells).

Already in Sunrise under its own shapes (nothing to bring; check on a server): attributes for a player made after
the packet (respawn_attributes.rs), key.swapOffhand at the tick (play_keys.rs), a proxy's reconfiguration
(raw_ingress.rs drops the level and the player), a stack's own `equippable` and `use_cooldown` (item_sync.rs,
sunrise-items cooldowns.rs), armour and off hand in the creative sync (CREATIVE_SYNCED = 41), the grindstone and
shulker hooks (mod API).

Server side, Sunrise's whole by decision; Starling's are speed-ups to measure on a phone before re-porting:
deferred world files (`save::later`), random_tick_sections' position cache, spawn_rules' spot table, mobs ai.rs
and living.rs, the cheaper mob collisions.

Still to decide or re-port:
- Custom armour from server packs: custom_armor.rs is in the tree and nothing calls it (mobs/render.rs's
  own_equipment and armor atlases, armor_stands.rs's layer_look). Sunrise draws a stack's own equipment asset through
  equipment_piece.rs and cit_armor: try ItemsAdder/Nexo armour first, re-port only what is missing.
- 3D skin layers; `remote_menu_clicks` for a mod's slots.click; fetched assets; the wrench mod's mixins.
- The small files of dropped.txt (under 30 lines each) not yet read one by one.

## First phone test (owner, 2026-11-22), on the APK of 8dc5da1

Fixed at 513ed8b547:
- "Sunrise terrain" failed to parse (`hdr_normal`): Starling's fs_cutout_static(_sdr) wrote a private Sunrise
  removed. The world's renderer never came up.
- The launcher's Mods page: Sunrise's registry (sunrise-org.github.io/sunrise-mods), no Modrinth tab, no
  "Java mods - browsing only" status.
- Play offline: the name the player already has is in the field.
- Clicks landing beside the buttons in Options: the cursor is in the interface's space again (`in_interface`,
  re-port 3); to confirm on the phone.

Open:
- Horizon's registry beside Sunrise's on the Mods page (the owner's ask). Its index parses but every entry is left
  out: `.hmod` is not the kind's extension (`.smod`), the manifest is `horizon-mod.toml`, and native mods name
  horizon-* crates. That is the `.hmod` layer above, plus the Compile store renaming a Horizon mod's source
  (tools/rebrand.py) before building it against this game.
- "Starling is ready: Launcher / Game" asked again after the launcher already asked (ChooseSetupDestination saves
  `Chosen`, and `finish_saved_onboarding` should answer for it). Not reproduced by reading; needs the phone's log.
- Buttons showing their lang keys for a moment before the text.
- Controls: the search box moves while the list is dragged.
- The setup still offers sign-in when the launcher has an account selected (only the offline name is prefilled).
- "Configurações do Sunrise..." and other Sunrise names in the game's own menus.
