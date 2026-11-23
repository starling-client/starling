//! Startup shared by the gameplay library and its executable.
#[path = "sunrise_look.rs"]
mod sunrise_look;
use crate::{crash, game, logging};
use sunrise_mod_package::selection::{Argument, LaunchArguments};
pub(crate) mod dedicated_arguments;
pub(crate) mod runtime_environment;

/// Validated startup state. No native hooks or workers may run before prepare returns.
pub struct Startup {
    launch: LaunchArguments,
    options: game::Options,
    _capture: Option<sunrise_mod_package::output::Capture>,
}

impl Startup {
    /// Validated launch context used by the platform's code loader.
    pub fn launch(&self) -> &LaunchArguments {
        &self.launch
    }
}

/// Holder mode runs first. The game's sole Options validation reads arguments/environment
/// and resource marker metadata only: no logging, account loading, mods or worker threads.
/// Account IO, cwd changes and command registration remain in Options::read, after leases.
pub fn prepare() -> Startup {
    // The holder (mod lease process) is built for desktop only.
    #[cfg(not(target_os = "android"))]
    sunrise_mod_package::holder::enter();
    let result: crate::Result<_> = (|| {
        let mut command = std::process::Command::new(std::env::current_exe()?);
        #[cfg(not(target_os = "android"))]
        command.args(std::env::args_os().skip(1));
        // The process has no command line: `sunrise-args.txt`'s, read by `android::main`.
        #[cfg(target_os = "android")]
        command.args(crate::android::args());
        let launch = LaunchArguments::for_command(&command)?;
        let options = game::Options::validate(launch.clone())?;
        Ok((launch, options))
    })();
    let (launch, options) = result.unwrap_or_else(|error| {
        eprintln!("Sunrise: {error}");
        std::process::exit(game::exit_code::USAGE);
    });
    let Some(options) = options else {
        std::process::exit(0);
    };
    #[cfg(not(target_os = "android"))]
    let capture = sunrise_mod_package::holder::acquire(&launch);
    #[cfg(target_os = "android")]
    let capture = None;
    Startup {
        launch,
        options,
        _capture: capture,
    }
}

/// Runs once the log is open (Android's start-up notes, its platform set-up that logs); set before [`start`].
static AFTER_LOG: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>> = std::sync::Mutex::new(None);

/// Register what runs once the log is open (see `AFTER_LOG`).
pub fn set_after_log(hook: impl FnOnce() + Send + 'static) {
    *AFTER_LOG
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Box::new(hook));
}

/// Refuse startup if any merged copy could not be installed, before loading any mod.
/// Cells cannot roll back: each copy merges all requesting rows, so stopping the whole
/// start is how we ensure none of a refused mod's rows run.
pub fn installed(patched: &[(&str, &[&str], bool)]) {
    let refusals = installation_refusals(patched);
    for line in &refusals {
        eprintln!("{line}");
    }
    if !refusals.is_empty() {
        std::process::exit(game::exit_code::ERROR);
    }
}

fn installation_refusals(patched: &[(&str, &[&str], bool)]) -> Vec<String> {
    patched.iter().filter(|(_, _, installed)| !installed).map(|(target, mods, _)| {
        format!("Sunrise: {}: {target}: a patched copy was already installed; the game was not started", mods.join(", "))
    }).collect()
}

/// Start the engine with dependency-ordered code supplied by the host.
pub fn start(
    native: sunrise_mod_package::runtime::Runtime,
    load: impl FnOnce(&sunrise_mod_package::runtime::Runtime, &LaunchArguments),
    startup: Startup,
) {
    let Startup {
        launch,
        options,
        _capture,
    } = startup;
    runtime_environment::capture(&launch);
    sunrise_threads::configure_allocator();
    sunrise_look::give();
    // OptiFine's and Polytone's pack formats (platform, docs/layers.md), before any pack loads.
    sunrise_client::pack_formats::install();
    // sunrise-extras (docs/forking.md): without the feature, no Sunrise Settings or Mods page.
    if !cfg!(feature = "sunrise-extras") {
        sunrise_client::options_ui::SUNRISE_EXTRAS
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
    sunrise_client::telemetry::mark_start();
    // Opt-in telemetry keeps each crash report (docs/telemetry.md), whatever the mode.
    crash::on_report(sunrise_client::telemetry::keep_crash);
    // Crates above the engine registered their hooks before this (sunrise-official's `main`);
    // none registers after this point.
    sunrise_hooks::engine::seal();
    // The text crates ship (`sunrise_hooks::lang`), loaded with the game's own languages.
    sunrise_client::lang::install_crate_files(sunrise_hooks::lang::engine::all());
    sunrise_client::spans::startup::init(std::time::Instant::now());
    logging::set_game_home(
        launch.context.home.clone(),
        launch.cache_home().to_path_buf(),
    );
    // The dedicated server logs in its own folder, as vanilla's does; the game in its home.
    let (home, side, thread) = if launch
        .rest
        .iter()
        .any(|arg| matches!(arg, Argument::Dedicated(_)))
    {
        (launch.context.game_dir.clone(), "server", "Server thread")
    } else {
        // A player's own log stays separate; crash reports follow the shared home.
        if let Some(dir) = launch.rest.iter().find_map(|arg| match arg {
            Argument::Value(name, value) if name == "--player-dir" => Some(value),
            _ => None,
        }) {
            logging::set_player_dir(std::path::absolute(dir).unwrap_or_else(|_| dir.into()));
        }
        (launch.context.home.clone(), "client", "Render thread")
    };
    logging::init(
        logging::player_dir_override().unwrap_or(home.as_path()),
        thread,
    );
    // The log's first line names the build, so a bug report that carries the log says which one it was.
    // Starling: on a phone the APK's own version (`0.9.0-preview.<commit>`), which is what a report needs;
    // Sunrise's crate version beside it.
    #[cfg(target_os = "android")]
    match crate::android::app_version() {
        Some((apk, _)) => log::info!("Starling {apk} (Sunrise {})", crash::version()),
        None => log::info!("Sunrise {}", crash::version()),
    }
    #[cfg(not(target_os = "android"))]
    log::info!("Sunrise {}", crash::version());
    // Mods' startup bars and lines (`RegistrationContext::progress`), before they register.
    game::startup_progress::install(side == "server");
    // Mod loading emits refusals; both client and dedicated logs must exist first.
    load(&native, &launch);
    let save_failure_diagnostics = options.save_failure_diagnostics.clone();
    let panic_diagnostics = save_failure_diagnostics.clone();
    crash::install(home.clone(), side, move || {
        panic_diagnostics.suppressed_summary()
    });
    if let Some(after_log) = AFTER_LOG
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    {
        after_log();
    }
    let _ = sunrise_world::loot::types::install(crate::game::loot_types::capture);
    if sunrise_world::native_load_hooks::install(
        sunrise_world::native_load_hooks::NativeLoadHooks {
            item_ability_default: sunrise_items::item_abilities::vanilla_can_perform,
            loot_ability: crate::game::api_values::loot_values::dispatch_loot_ability,
            loot_table_load: |context, id, table| {
                if !mod_api::__private::listened(mod_api::HookId::LootTableLoad) {
                    return;
                }
                let reader = sunrise_client_render::api_context::LoadReader(context);
                let context = mod_api::OwningLoad::new(
                    sunrise_client_render::api_context::load_origin(context.origin()),
                    &reader,
                );
                mod_api::dispatch_loot_table_load(&context, id, table);
            },
            modify_default_components: |context, item, components| {
                if !mod_api::__private::listened(mod_api::HookId::ModifyDefaultComponents) {
                    return;
                }
                let reader = sunrise_client_render::api_context::LoadReader(context);
                let context = reader.context();
                mod_api::dispatch_modify_default_components(&context, item, components);
            },
        },
    )
    .is_err()
    {
        log::warn!("Native data-load hooks already installed; keeping the first callbacks");
    }
    if sunrise_world::native_worldgen_hooks::install(
        sunrise_world::native_worldgen_hooks::NativeWorldgenHooks {
            alter_ground: crate::game::api_values::alter_ground,
            on_tree_grow: crate::game::native_tree_grow::on_tree_grow,
            rotate: crate::game::native_rotate::rotate,
            feature: crate::game::native_worldgen_parts::feature,
        },
    )
    .is_err()
    {
        log::warn!("Native worldgen hooks already installed; keeping the first callbacks");
    }
    if sunrise_items::native_enchantment_hooks::install(
        crate::enchantment_levels::get_enchantment_level,
    )
    .is_err()
    {
        log::warn!("The enchantment level hook is already installed; keeping the first");
    }
    install_client_hooks();
    raise_open_file_limit();
    if let Err(error) = game::run_with(options, native) {
        // A bad command line is the caller's mistake, not a crash: no report.
        let missing_mods = game::dedicated::is_missing_mods_refusal(error.as_ref());
        let usage = error.is::<game::UsageError>() || missing_mods;
        #[cfg(target_os = "android")]
        if !usage {
            crate::diagnostics::fatal_error(&error.to_string());
        }
        if let Some(refusals) = error.downcast_ref::<game::mods::DeclarationFailures>() {
            for line in refusals.lines() {
                log::error!("Sunrise: {line}");
            }
        } else if !missing_mods {
            log::error!("Sunrise: {error}");
        }
        logging::flush();
        // The report's path is the last line on stderr (crash::SAVED_LINE).
        if !usage {
            crash::report_error(&home, side, &*error, || {
                save_failure_diagnostics.suppressed_summary()
            });
        }
        // Started from Explorer, the console window closes with the game:
        // keep the message on screen until it is read.
        #[cfg(windows)]
        if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            eprintln!("Press Enter to close.");
            let _ = std::io::stdin().read_line(&mut String::new());
        }
        // The activity ends first, and the process without the C library's exit handlers.
        #[cfg(target_os = "android")]
        crate::android::finish_and_exit();
        #[cfg(not(target_os = "android"))]
        std::process::exit(if usage {
            game::exit_code::USAGE
        } else {
            game::exit_code::ERROR
        });
    }
    sunrise_hooks::engine::shutdown();
    logging::flush();
}

pub(crate) fn install_client_hooks() {
    game::api_values::install_stack_hooks();
    if !crate::native_client_hooks::install(crate::native_client_hooks::NativeClientHooks {
        menu_still_valid: game::api_values::menu_still_valid,
        menu_still_valid_listens: |menu| {
            mod_api::__private::menu_listens(mod_api::HookId::MenuStillValid, menu)
        },
        living_heal: |context, entity, amount, cancel| {
            if !mod_api::__private::listened(mod_api::HookId::LivingHeal) {
                return;
            }
            let reader = sunrise_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_heal(&context, entity, amount, cancel);
        },
        living_breathe_heard: || mod_api::__private::listened(mod_api::HookId::LivingBreathe),
        has_mods: || !mod_api::__private::mods().is_empty(),
        player_name_format: |context, username, displayname| {
            use game::api_values::TryIntoApi;
            if !mod_api::__private::listened(mod_api::HookId::PlayerNameFormat) {
                return;
            }
            let reader = sunrise_client_render::api_context::LoopReader(context);
            let context = reader.context();
            let Ok(username) = username.try_into_api() else {
                return;
            };
            let Ok(mut converted) = displayname.try_into_api() else {
                return;
            };
            mod_api::dispatch_player_name_format(&context, &username, &mut converted);
            if let Ok(converted) = sunrise_client_lib::api_text::to_serialized_engine(&converted) {
                *displayname = converted;
            }
        },
        player_tab_list_name_format: |context, player, name, display_name| {
            use game::api_values::TryIntoApi;
            if !mod_api::__private::listened(mod_api::HookId::PlayerTabListNameFormat) {
                return;
            }
            let reader = sunrise_client_render::api_context::LoopReader(context);
            let context = reader.context();
            let Ok(mut converted) = display_name.try_into_api() else {
                return;
            };
            mod_api::dispatch_player_tab_list_name_format(&context, player, name, &mut converted);
            if let Ok(converted) = converted
                .as_ref()
                .map(sunrise_client_lib::api_text::to_serialized_engine)
                .transpose()
            {
                *display_name = converted;
            }
        },
        living_drown: |context, entity, kind, is_drowning, damage_amount, bubble_count, cancel| {
            if !mod_api::__private::listened(mod_api::HookId::LivingDrown) {
                return;
            }
            let reader = sunrise_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_drown(
                &context,
                entity,
                kind,
                is_drowning,
                damage_amount,
                bubble_count,
                cancel,
            );
        },
        living_fall: |context, entity, kind, distance, multiplier, cancel| {
            if !mod_api::__private::listened(mod_api::HookId::LivingFall) {
                return;
            }
            let reader = sunrise_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_fall(&context, entity, kind, distance, multiplier, cancel);
        },
        living_jump: |context, entity, kind, velocity| {
            if !mod_api::__private::listened(mod_api::HookId::LivingJump) {
                return;
            }
            let reader = sunrise_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_jump(&context, entity, kind, velocity);
        },
        living_jump_power: |context, entity, kind, power| {
            if !mod_api::__private::listened(mod_api::HookId::LivingJumpPower) {
                return;
            }
            let reader = sunrise_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_jump_power(&context, entity, kind, power);
        },
        living_speed: |context, entity, kind, speed| {
            if !mod_api::__private::listened(mod_api::HookId::LivingSpeed) {
                return;
            }
            let reader = sunrise_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_speed(&context, entity, kind, speed);
        },
        living_flying_speed: |context, entity, kind, speed| {
            if !mod_api::__private::listened(mod_api::HookId::LivingFlyingSpeed) {
                return;
            }
            let reader = sunrise_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_flying_speed(&context, entity, kind, speed);
        },
        block_jump_factor: |context, state, factor| {
            crate::game::block_jump_factor(context, state, factor);
        },
        block_speed_factor: |context, state, factor| {
            crate::game::block_speed_factor(context, state, factor);
        },
        is_ladder: |context, entity, pos, state, ladder| {
            crate::game::is_ladder(context, entity, pos, state, ladder);
        },
        makes_open_trapdoor_above_climbable:
            crate::game::native_trapdoor_ladder::makes_open_trapdoor_above_climbable,
        friction: crate::game::native_surface::friction,
        collision_shape: |context, pos, state, query, shape| {
            crate::game::native_collision_shape::collision_shape(context, pos, state, query, shape);
        },
        collision_shape_refused: crate::game::native_collision_shape::refused,
        player_stays_on_ground_surface: |context, entity, staying| {
            if !mod_api::__private::listened(mod_api::HookId::PlayerStaysOnGroundSurface) {
                return;
            }
            let reader = sunrise_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_player_stays_on_ground_surface(&context, entity, staying);
        },
        ender_entity_teleport: |context, entity, kind, from, target, cancel| {
            if !mod_api::__private::listened(mod_api::HookId::EnderEntityTeleport) {
                return;
            }
            let reader = sunrise_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_ender_entity_teleport(&context, entity, kind, from, target, cancel);
        },
        fluid_flow: |context, fluid, pos, flow| {
            if !mod_api::__private::listened(mod_api::HookId::FluidFlow) {
                return;
            }
            let reader = sunrise_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_fluid_flow(&context, fluid, pos, flow);
        },
        sound_type: |context, pos, state, entity, cause, sound| {
            crate::game::native_block_sounds::sound_type(context, pos, state, entity, cause, sound);
        },
        can_grindstone_repair: crate::game::native_item_menu_parts::can_grindstone_repair,
        is_combine_repairable: crate::game::native_item_menu_parts::is_combine_repairable,
        can_fit_inside_container_items:
            crate::game::native_item_menu_parts::can_fit_inside_container_items,
        item_name: crate::game::native_item_menu_parts::item_name,
    }) {
        log::warn!("Native client hooks already installed; keeping the first callbacks");
    }
}

/// The JVM raises its open-file limit to the hard limit as it starts
/// (`-XX:+MaxFDLimit`, on by default), so a vanilla server may hold as many
/// sockets and region files as the host allows, not the soft default (often
/// 1024 in a container). The same here; macOS caps it at `OPEN_MAX`, as the
/// JVM does there.
fn raise_open_file_limit() {
    #[cfg(unix)]
    {
        use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
        let limit = getrlimit(Resource::Nofile);
        let hard = if cfg!(target_os = "macos") {
            Some(limit.maximum.map_or(10_240, |h| h.min(10_240)))
        } else {
            limit.maximum
        };
        let raised = match (limit.current, hard) {
            (Some(soft), Some(hard)) => hard > soft,
            (Some(_), None) => true,
            (None, _) => false,
        };
        if raised {
            let to = Rlimit {
                current: hard,
                maximum: limit.maximum,
            };
            match setrlimit(Resource::Nofile, to) {
                Ok(()) => log::info!(
                    "Open file limit raised from {:?} to {hard:?}",
                    limit.current
                ),
                Err(e) => log::warn!("Open file limit stays at {:?}: {e}", limit.current),
            }
        }
    }
}

#[cfg(test)]
mod installation_tests {
    #[test]
    fn refused_install_names_all_mods_and_only_failed_targets() {
        assert_eq!(
            super::installation_refusals(&[
                ("game::tick", &["first", "second"], false),
                ("game::other", &["third"], true)
            ]),
            vec![
                "Sunrise: first, second: game::tick: a patched copy was already installed; the game was not started"
            ]
        );
        assert!(super::installation_refusals(&[]).is_empty());
    }
}

#[cfg(test)]
mod name_conversion_tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use sunrise_client_core::text::Text;
    use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};

    // Engine -> API rejects unrepresentable opaque metadata (NestedNull);
    // API -> Engine rejects a component tree deeper than 512 nodes at the serialization boundary.
    fn unrepresentable_engine_text() -> Text {
        Text::empty().with_component(serde_json::json!({"text": "leaf", "test:meta": [null]}))
    }

    fn deep_api_text() -> mod_api::Text {
        let mut value = mod_api::Text::literal("leaf");
        for _ in 0..mod_api::text::MAX_TEXT_DEPTH {
            value = mod_api::Text::literal("parent").append(value);
        }
        value
    }

    struct InvalidNames(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for InvalidNames {
        fn player_name_format(
            &self,
            _: &dyn mod_api::LoopContext,
            event: &mut mod_api::PlayerNameFormatEvent<'_>,
        ) {
            self.0.fetch_add(1, Ordering::Relaxed);
            *event.displayname = deep_api_text();
        }
        fn player_tab_list_name_format(
            &self,
            _: &dyn mod_api::LoopContext,
            event: &mut mod_api::PlayerTabListNameFormatEvent<'_>,
        ) {
            self.0.fetch_add(1, Ordering::Relaxed);
            *event.display_name = Some(deep_api_text());
        }
    }

    fn with_names(
        test: impl FnOnce(&sunrise_world::native_load_context::LoopContext, &AtomicUsize),
    ) {
        let calls = Arc::new(AtomicUsize::new(0));
        let mods = vec![mod_api::ModEntry::new(
            "invalid-names",
            Box::new(InvalidNames(calls.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::install_client_hooks();
            let inputs = Arc::new(LoadInputs::new(Default::default(), Default::default()));
            let context = LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs, [])
                .loop_context(LoopSide::Server, []);
            test(&context, &calls);
        });
    }

    #[test]
    fn player_name_conversion_failure_keeps_the_engine_name() {
        with_names(|context, calls| {
            let username = Text::literal("Steve");
            let original = Text::literal("Original");
            let unrepresentable = unrepresentable_engine_text();
            let mut display = original.clone();
            sunrise_client_lib::lan::hub::native_player_name_format(
                context,
                &unrepresentable,
                &mut display,
            );
            assert_eq!(display.component(), original.component());
            assert_eq!(calls.load(Ordering::Relaxed), 0);

            display = unrepresentable.clone();
            sunrise_client_lib::lan::hub::native_player_name_format(
                context,
                &username,
                &mut display,
            );
            assert_eq!(display.component_view(), unrepresentable.component_view());
            assert_eq!(calls.load(Ordering::Relaxed), 0);

            display = original.clone();
            sunrise_client_lib::lan::hub::native_player_name_format(
                context,
                &username,
                &mut display,
            );
            assert_eq!(calls.load(Ordering::Relaxed), 1);
            assert_eq!(display.component(), original.component());
        });
    }

    #[test]
    fn tab_list_name_conversion_failure_keeps_the_engine_name() {
        with_names(|context, calls| {
            let unrepresentable = unrepresentable_engine_text();
            let mut display = Some(unrepresentable.clone());
            sunrise_client_lib::lan::hub::native_player_tab_list_name_format(
                context,
                1,
                "Steve",
                &mut display,
            );
            assert_eq!(
                display.as_ref().unwrap().component_view(),
                unrepresentable.component_view()
            );
            assert_eq!(calls.load(Ordering::Relaxed), 0);

            for original in [None, Some(Text::literal("Original"))] {
                display = original.clone();
                sunrise_client_lib::lan::hub::native_player_tab_list_name_format(
                    context,
                    1,
                    "Steve",
                    &mut display,
                );
                assert_eq!(
                    display.as_ref().map(Text::component),
                    original.as_ref().map(Text::component)
                );
            }
            assert_eq!(calls.load(Ordering::Relaxed), 2);
        });
    }
}
