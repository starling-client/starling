//! The window's first renderer and frames: the renderer built from the prepared textures, the
//! boot screen's frame, and `start_game`, which turns the loaded resources into a `Game`.
use super::*;
use std::borrow::Cow;
use sunrise_client::gfx::RendererDevice;
#[path = "loading_window/starling.rs"]
mod starling;
#[allow(unused_imports)]
pub use self::starling::*;

/// CPU mip chains for the block sprites and world border.
pub(super) fn prepare_renderer_textures(
    pack: &PackResources,
) -> sunrise_client::gfx::PreparedTextures {
    let assets = &pack.render;
    sunrise_client::gfx::PreparedTextures::new(
        &assets.pixels.get(),
        assets.sprite_size,
        assets.texture_layers,
        &assets.sprite_mips,
        (&pack.forcefield.2, pack.forcefield.0),
    )
    .with_pbr(
        &assets.pbr_normals,
        &assets.pbr_specular,
        assets.pbr_size,
        assets.texture_layers,
    )
}

/// Upload pack textures and create the renderer on the window's device, with whether the device
/// made every pipeline of the packs' core shaders it draws (the first refusal, named by its
/// pipeline, fails the pack load as a required shader's does).
pub(super) fn build_renderer(
    window: Arc<Window>,
    offscreen: bool,
    pack: &PackResources,
    gui: &UiResources,
    device: RendererDevice<'_>,
    prepared: Option<sunrise_client::gfx::PreparedTextures>,
    api: sunrise_client::graphics::GraphicsApi,
    translucent_batching: bool,
    vsync: bool,
    // HDR and Smooth Gradients pick the frame, with what the window's display reported to the
    // renderer before (`display`, `Renderer::new`).
    settings: &sunrise_client::graphics::Graphics,
    shader_pack: shader_packs::PickedPack,
    display: sunrise_client::gfx::DisplayReading,
    filtering: (i32, i32, i32),
) -> Result<(Renderer, std::result::Result<(), String>)> {
    let (mut renderer, _) = RendererParts::new(
        window,
        offscreen,
        pack,
        gui,
        gui.atlas.pixels(),
        prepared,
        api,
        translucent_batching,
        settings,
        shader_pack,
        display,
    )
    .build(device)?;
    let required = finish_renderer(
        &mut renderer,
        pack,
        gui,
        filtering,
        (vsync, settings.present_mode),
    )?;
    Ok((renderer, required))
}

/// What a renderer built from `RendererParts` still wants, on the main thread: the pack's models,
/// sprite animations, shaders, sky and glints, the GUI's font told about its new texture, the
/// swapchain's present mode (`present`: VSync and the setting's mode) and the mods' pipelines.
pub(super) fn finish_renderer(
    renderer: &mut Renderer,
    pack: &PackResources,
    gui: &UiResources,
    filtering: (i32, i32, i32),
    present: (bool, sunrise_client::graphics::PresentMode),
) -> Result<std::result::Result<(), String>> {
    let atlas = &gui.atlas;
    // The new renderer's atlas texture starts from `HudAtlas.pixels`, which
    // holds the textures but never a glyph rasterized at run time. The font's
    // cache still answers with the rects it drew into the previous renderer's
    // texture, so forget them: the glyphs that are drawn again are placed into
    // this texture (as `ui_theme` does when its atlas changes). Without this,
    // Chinese and Japanese menu labels go blank when the loading screen hands
    // over to the game.
    atlas.font.reset_cache();
    // The world's renderer is not loaded yet, and the upload holds only standalone models, which
    // no block state's special part changes; `Game` builds its own geometry once it is.
    let (prepared_vertices, prepared_ranges) =
        native_block_entity_draw::StandaloneGeometry::new(pack, gui, None).prepared_upload();
    renderer.install_prepared_models(
        pack.render.generation,
        &prepared_vertices,
        prepared_ranges,
    )?;
    // The loading renderer may have been built before the saved graphics settings were
    // adopted. Configure the world swapchain before its first gameplay frame instead of
    // waiting for `Game::apply_settings` after the handoff.
    renderer.set_present_mode(present.0, present.1);
    renderer.hud.sprite_animations = atlas.renderer_animations();
    // The packs' core shaders: the device refusing a pipeline vanilla requires fails the load,
    // after the renderer is made (`ShaderManager.apply`'s required programs).
    let gui_text = renderer.hud.set_text_shader(&renderer.device, {
        let text = &atlas.text_shaders;
        (text.gui.as_deref()).map(|p| (p, &text.expressions.gui[..]))
    });
    pack.item_renderer.prepare_glint_atlas_with_mips(
        renderer.device.limits().max_texture_dimension_2d,
        (filtering.0, filtering.1),
        filtering.2,
    )?;
    let world_text = renderer.set_world_text_shaders(&atlas.text_shaders);
    let clouds = renderer.set_clouds_shader(atlas.clouds_shader.as_deref());
    let lines = renderer.set_lines_shader(
        (atlas.lines_shader.as_deref()).map(|p| (p, &atlas.lines_expressions[..])),
    );
    let sky = renderer.set_sky_shaders(
        (atlas.sky_shader.as_deref()).map(|p| (p, &atlas.sky_expressions[..])),
        (atlas.stars_shader.as_deref()).map(|p| (p, &atlas.stars_expressions[..])),
    );
    let lightmap = renderer.set_lightmap_shader(
        atlas.lightmap_shader.as_deref(),
        &atlas.lightmap_expressions,
    );
    let required = [world_text, gui_text, clouds, lines, sky, lightmap]
        .into_iter()
        .collect::<std::result::Result<(), String>>();
    renderer.set_custom_sky(&*pack.custom_sky);
    renderer.set_shared_material_maps(&atlas.material_maps);
    let cit_glints = pack.item_renderer.cit.as_ref().map(|c| &c.glint_textures);
    renderer
        .hud
        .set_cit_glints(&renderer.device, &renderer.queue, cit_glints);
    // Last, so a renderer that fails before this marks none of the pack's pipelines invalid.
    // Textured pipelines' sprites are looked up in the atlas the renderer was built with.
    let atlas = &gui.atlas;
    renderer.build_mod_pipelines(&pack.render.mod_pipelines, &|name| {
        let r = atlas.sprites.get(name)?;
        Some([r.u0, r.v0, r.u1, r.v1])
    })?;
    Ok(required)
}

/// The loading screen's renderer where there is no window to draw on (`--offscreen`): one atlas
/// (the boot screen's until the pack's GUI atlas is ready) over one opaque white block-sprite
/// layer, an empty border texture and the title panorama once the menu bundle is ready.
pub(super) fn loading_renderer(
    window: Arc<Window>,
    offscreen: bool,
    atlas: Option<&sunrise_client::hud::HudAtlas>,
    panorama: Option<&Arc<sunrise_client::gfx::panorama::Faces>>,
    previous: Option<&Renderer>,
    api: sunrise_client::graphics::GraphicsApi,
) -> Result<Renderer> {
    static FORCEFIELD: [u8; 4] = [0; 4];
    static EMPTY_HUD: [u8; 4] = [0, 0, 0, 255];
    static EMPTY_GLINT: [u8; 4] = [0; 4];
    let panorama = panorama
        .cloned()
        .unwrap_or_else(|| Arc::new(sunrise_client::gfx::panorama::Faces::colour([0; 4])));
    let (pixels, width, height) = atlas.map_or((EMPTY_HUD.as_slice(), 1, 1), |a| {
        (a.pixels.as_slice(), a.width, a.height)
    });
    let (glint_pixels, glint_width, glint_height) = atlas
        .map_or((EMPTY_GLINT.as_slice(), 1, 1), |a| {
            (a.glint.0.as_slice(), a.glint.1, a.glint.2)
        });
    // A glyph rasterized into an earlier texture is not in these pixels; the
    // new texture gets it again when the character is drawn (see
    // `build_renderer`).
    if let Some(atlas) = atlas {
        atlas.font.reset_cache();
    }
    #[cfg(target_os = "android")]
    crate::diagnostics::checkpoint(
        "renderer_init",
        || serde_json::json!({"api":format!("{:?}",api)}),
    );
    #[cfg(target_os = "android")]
    let marker = start_marker(previous);
    let mut renderer = pollster::block_on(Renderer::new(
        window,
        offscreen,
        &[255, 255, 255, 255],
        1,
        1,
        1,
        &[],
        &[],
        (pixels, width, height),
        atlas,
        (glint_pixels, glint_width, glint_height),
        (glint_pixels, glint_width, glint_height),
        (&FORCEFIELD[..], 1),
        panorama,
        None,
        previous.map_or(RendererDevice::New, RendererDevice::Reuse),
        api,
        // It draws no terrain.
        false,
        (false, false),
        // An 8-bit frame, which HDR output takes as well: HDR off until the settings apply.
        &sunrise_client::graphics::Graphics::default(),
        Default::default(),
        previous.map(|r| r.display_reading()).unwrap_or_default(),
    ))?;
    #[cfg(target_os = "android")]
    if let Some(marker) = marker {
        renderer.forget_start_marker(marker);
    }
    if let Some(atlas) = atlas {
        renderer.hud.sprite_animations = atlas.renderer_animations();
    }
    Ok(renderer)
}

pub(super) fn loading_frame(hud: &HudBatch) -> Frame<'_> {
    Frame {
        weather: &[],
        weather_time: sunrise_client::weather::Time::new(0, 0.0),
        camera: glam::DVec3::new(0.5, 100.0, 0.5),
        yaw: 0.0,
        pitch: 10.0,
        roll: 0.0,
        fov_degrees: 85.0,
        cull_fov_degrees: 85.0,
        eye: glam::DVec3::new(0.5, 100.0, 0.5),
        eye_rotation: [0.0, 10.0],
        first_person: true,
        bob: glam::Mat4::IDENTITY,
        view_transform: glam::Mat4::IDENTITY,
        spin: glam::Mat4::IDENTITY,
        sky: sunrise_client::environment::Sky::default(),
        shadow_scene_revision: (0, 0),
        shadow_cache: false,
        distance: 12.0,
        detail_distance: 12.0,
        shadow_distance: None,
        depth_far: 1000.0,
        underwater: false,
        cave_culling: false,
        title: true,
        sky_pass: false,
        sky_blocked: false,
        skybox: false,
        end_sky: false,
        end_sky_sprite: None,
        boss_world_fog: false,
        environment_fog: None,
        water_fog: [0.0; 3],
        fog: None,
        dark_disc: false,
        outline: Vec::new(),
        outline_after_terrain: false,
        outline_high_contrast: false,
        pvp: Default::default(),
        debug_lines: &[],
        debug_lines_on_top: &[],
        debug_crosshair: &[],
        hud,
        glint_alpha: 0.75,
        glint_ticks: 0.0,
        nether_lighting: false,
        entities: &[],
        explicit_particles: None,
        static_block_entities: &[],
        visible_static_block_entity_sections: &[],
        static_block_entity_stats: Default::default(),
        entity_motion: &[],
        entity_motion_posed: &[],
        entity_spans: &[],
        skin: None,
        glowing: &[],
        dynamic_lights: &[],
        colored_lights: &[],
        emissive_layers: &[],
        clouds: None,
        lightning: Default::default(),
        additive: &[],
        see_through: &[],
        world_primitives: Default::default(),
        shadow_only: &[],
        celestial: &[],
        celestial_bodies: &[],
        stars: None,
        hand: &[],
        hand_items: &[],
        hand_fov_degrees: 70.0,
        screen: &[],
        pack: Default::default(),
        border: None,
        game_time: 0.0,
        shader_game_time: 0.0,
        end_portal_sprites: [[0.0; 4]; 2],
        crumbling_sprite: [0.0; 4],
        ticks: 0,
        darkness: (0.0, 0.0),
        night_vision: 0.0,
        block_flicker: 0.0,
        boss_darken: 0.0,
        chunk_fade: 0.0,
        post_effects: sunrise_client::post_chain::Request::none(),
        menu_blur: None,
        menu_blur_cache: false,
    }
}

/// The post-load setup once the resources are ready: `Game::new` and the
/// options, packs, mods, control socket, benchmark and world selection the
/// old `run` did after loading. The launching path hands over its live
/// settings. `None` means a harness or dedicated run finished here instead.
#[allow(clippy::too_many_arguments)]
pub(super) fn start_game<'a>(
    res: &'a Arc<Resources>,
    pack: PackResources,
    gui: UiResources,
    options: Options,
    pack_error: Option<String>,
    audio: Audio,
    vanilla_root: PathBuf,
    started: Instant,
    rest_loader: Option<WorldRestLoader>,
    mut settings: Settings,
    prepared: Option<GameCpuResources>,
    client_applies: Vec<client_reload::PreparedClientReload>,
    entity_models: Option<Arc<sunrise_client::mobs::EntityModelDocument>>,
    window: Option<(Arc<Window>, Renderer)>,
) -> Result<Option<Game<'a>>> {
    let handoff = prepared.is_some();
    if let Some(d) = options.distance_override {
        settings.render_distance = d;
    }
    // A synchronous launch has no handoff worker: build its CPU resources
    // here, from the same entity-model document the rest worker read.
    let prepared = match prepared {
        Some(prepared) => Some(prepared),
        None => Some(GameCpuResources::load(
            &sunrise_assets::resource_source::ResourceRoot::new(
                options.resources.clone(),
                packs::cache_dir(),
                pack.source.clone(),
            ),
            entity_models.as_deref(),
            pack.render.model_library.as_deref().ok(),
            !options.execution_role.is_dedicated_server(),
        )?),
    };
    let mut initial = None;
    let stage = options.is_client_start().then_some(&mut initial);
    let mut game = Game::construct(res, pack, gui, options, settings, audio, prepared, stage)?;
    // `new DebugRenderer()` refreshes its list.
    game.refresh_debug_renderers();
    game.rest_loader = rest_loader;
    game.ui.vanilla_root = vanilla_root;
    if game.options.benchmark.is_some() {
        // The same conditions on every machine: no VSync, no frame cap, no
        // pause or idle limit without focus. In memory only, never saved.
        game.settings.vsync = false;
        game.settings.max_fps = 260;
        game.settings.inactivity_fps_limit = "minimized".into();
        game.options.no_pause = true;
    }
    game.apply_settings();
    // Benchmarks measure the game alone.
    if game.options.benchmark.is_none() {
        game.start_mods();
    }
    game.refresh_loop_context()?;
    // After the mods: their templates are among the ids it lists.
    game.preload_registries();
    game.start_agent_control()?;
    if let Some(scenario) = game.options.benchmark.clone() {
        let control = game
            .control
            .get_or_insert_with(sunrise_client::control::Server::local);
        sunrise_client::benchmark::start(
            control.sender(),
            sunrise_client::benchmark::Setup {
                scenario,
                started,
                app: sunrise_bench::App {
                    name: "sunrise-client".into(),
                    version: sunrise_client::crash::version(),
                    commit: option_env!("SUNRISE_COMMIT").map(str::to_owned),
                    mods: Vec::new(),
                },
                simulation_distance: game.settings.simulation_distance as u32,
                vsync: false,
                fps_cap: None,
                offscreen: game.options.offscreen,
                game_dir: std::env::current_dir()?,
                set: game.options.benchmark_set.clone(),
                profile: game.options.benchmark_profile,
            },
        );
    }
    if game.options.execution_role == execution_role::ExecutionRole::DedicatedServer {
        let result = game.run_dedicated();
        game.native_game_shutting_down();
        result?;
        return Ok(None);
    }
    let pending = initial.map(|models| initial_client_models::PendingClientStart {
        models,
        applies: client_applies,
        pack_error: pack_error.clone(),
        handoff,
    });
    if let Some((window, renderer)) = window {
        game.resize_state = Some(game.resize_state_of(window.inner_size()));
        game.window = Some(window);
        let ready = game.apply_pending_client_start(
            pending.expect("loading screen has a client"),
            Ok(Some(renderer)),
        )?;
        Ok(ready.then_some(game))
    } else if let Some(pending) = pending {
        // These synchronous fixtures finish inside startup; they never enter a window loop.
        let startup_fixture = [
            "SUNRISE_NATIVE_CLIENT_PARTICLES_FIXTURE",
            "SUNRISE_BOAT_RIDER_FIXTURE",
            "SUNRISE_BOAT_AUTHORITY_FIXTURE",
            "SUNRISE_SCREEN_BUILD_FIXTURE",
            "SUNRISE_DISPENSE_FIXTURE",
            "SUNRISE_SCULK_FIXTURE",
            "SUNRISE_HOT_PATHS",
        ]
        .iter()
        .any(|name| std::env::var_os(name).is_some())
            || (std::env::var_os("SUNRISE_COMMAND_SCRIPT").is_some()
                && std::env::var_os("SUNRISE_COMMAND_OUTPUT").is_some())
            || (game.options.server.is_some() && std::env::var_os("SUNRISE_NET_REPORT").is_some());
        if game.options.headless || startup_fixture {
            let ready = game.apply_pending_client_start(pending, Ok(None))?;
            Ok(ready.then_some(game))
        } else {
            game.pending_initial_models = Some(Box::new(pending));
            Ok(Some(game))
        }
    } else {
        let ready = game.finish_start_game(pack_error, handoff)?;
        Ok(ready.then_some(game))
    }
}

impl Game<'_> {
    pub(super) fn apply_pending_client_start(
        &mut self,
        pending: initial_client_models::PendingClientStart,
        renderer: Result<Option<Renderer>>,
    ) -> Result<bool> {
        self.apply_initial_model_generation(pending.models, pending.applies, renderer?)?;
        self.world_renderer_ready = self.renderer.is_some();
        self.finish_start_game(pending.pack_error, pending.handoff)
    }

    fn finish_start_game(&mut self, pack_error: Option<String>, handoff: bool) -> Result<bool> {
        let game = self;
        if let Some(directory) = std::env::var_os("SUNRISE_NATIVE_CLIENT_PARTICLES_FIXTURE") {
            game.run_native_client_particles_fixture(std::path::Path::new(&directory))?;
            return Ok(false);
        }
        if let Some(directory) = std::env::var_os("SUNRISE_BOAT_RIDER_FIXTURE") {
            game.run_boat_rider_fixture(std::path::Path::new(&directory))?;
            return Ok(false);
        }
        if let Some(directory) = std::env::var_os("SUNRISE_BOAT_AUTHORITY_FIXTURE") {
            game.run_boat_authority_fixture(std::path::Path::new(&directory))?;
            return Ok(false);
        }
        if let Ok(spec) = std::env::var("SUNRISE_SCREEN_BUILD_FIXTURE") {
            game.screen_build_fixture(&spec)?;
            return Ok(false);
        }
        if let Ok(spec) = std::env::var("SUNRISE_DISPENSE_FIXTURE") {
            game.run_component_harness(integrated::ComponentHarness::Dispenser(spec))?;
            return Ok(false);
        }
        if let Ok(spec) = std::env::var("SUNRISE_SCULK_FIXTURE") {
            game.run_component_harness(integrated::ComponentHarness::Sculk(spec))?;
            return Ok(false);
        }
        if let Ok(output) = std::env::var("SUNRISE_HOT_PATHS") {
            game.run_component_harness(integrated::ComponentHarness::HotPaths(output))?;
            return Ok(false);
        }
        // Finish Setup -> Config -> World before any screen, connection, input or tick hooks.
        game.native_configs_client_start();
        if let Some(error) = pack_error {
            game.open_options_at(true, sunrise_client::options_ui::Page::Packs);
            game.ui.options.resource_error = Some(error);
        }
        match game.options.world.clone() {
            _ if game.options.server.is_some() => {
                let (address, name) = (
                    game.options.server.clone().unwrap_or_default(),
                    game.options.player_name(),
                );
                game.connect_server(&address, &name)?;
                if let Some(report) = std::env::var_os("SUNRISE_NET_REPORT") {
                    game.run_net_harness(report.as_ref())?;
                    return Ok(false);
                }
            }
            Some(folder) => {
                let (seed, creative) = (game.options.seed, game.options.creative);
                if std::env::var_os("SUNRISE_COMMAND_SCRIPT").is_some() {
                    // The loaded-command fixture pins the server's valid minimum.
                    // Client options reject simulationDistance2 (their UI starts at5),
                    // so an options.txt value cannot establish this prerequisite.
                    if std::env::var_os("SUNRISE_HARNESS_LOADED_FIXTURE").is_some() {
                        game.settings.simulation_distance = 2;
                    }
                    // Headless, but with the same server and memory connection as play.
                    game.open_integrated(&folder, None, seed, creative, None)?;
                } else {
                    // Start after the window is handed over, so --world uses the
                    // same responsive initialization path as the title screen.
                    let title = game
                        .new_title(std::env::current_dir()?.join("saves"))
                        .with_user(|| game.options.player_name());
                    game.show_title(title);
                    game.pending_entry = Some(PendingWorldEntry::Local {
                        folder,
                        name: None,
                        seed,
                        creative,
                        create: None,
                    });
                }
            }
            None if handoff => {}
            None => {
                let mut title = game
                    .new_title(std::env::current_dir()?.join("saves"))
                    .with_user(|| game.options.player_name());
                title.last_server = game.settings.file.string("lastServer");
                title.skip_multiplayer_warning = game.settings.file.bool("skipMultiplayerWarning");
                // Gui.addInitialScreens: the accessibility onboarding comes first. Starling: not on
                // Android, which has no narrator, and whose players' options all still ask for it.
                if !cfg!(target_os = "android") && game.settings.file.bool("onboardAccessibility") {
                    title.start_onboarding();
                }
                game.show_title(title);
                // Starling: started again for a world's own mods (the start without a loading
                // screen: the menu's takes it in startup_menu.rs).
                game.pending_entry = world_content::next_entry();
            }
        }
        if std::env::var_os("SUNRISE_COMMAND_SCRIPT").is_some()
            && std::env::var_os("SUNRISE_COMMAND_OUTPUT").is_some()
        {
            game.wait_command_harness()?;
            return Ok(false);
        }
        Ok(true)
    }
}
