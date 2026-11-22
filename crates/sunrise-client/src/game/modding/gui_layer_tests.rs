//! HUD slots in a real client: placement, scoped wrapping and replacement, and layer cancellation.
use super::{
    Game, Screen,
    execution_role::ExecutionRole,
    mods,
    native_entity_refs::tests::{run_fresh_cases, with_client_game},
};
use mod_api::{
    ClientHost, Flow, GuiLayerPlacement, GuiLayerPostEvent, GuiLayerPreEvent, Hooks, ModEntry,
    PlayerHeartTypeEvent, RegisterGuiLayersEvent, RegistrationContext, RenderGuiPostEvent,
    RenderGuiPreEvent, ScreenRenderBackgroundEvent, ScreenRenderPostEvent, ScreenRenderPreEvent,
    client::hud_layout::GuiDraw, content::crash::current_native_mod,
};
use std::sync::{Arc, Mutex};
use sunrise_client::{
    chunk::Chunk,
    gui_layers::VanillaGuiLayer,
    hud::HudBatch,
    motion::{MotionWorld, PlayerMotion, WATER, dry_side},
};
use sunrise_core::geometry::Vec3;

#[path = "gui_layer_tests/hidden_panic.rs"]
mod hidden_panic;

static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn log(entry: impl Into<String>) {
    LOG.lock().unwrap().push(entry.into());
}

fn assert_owner(id: &'static str) {
    assert_eq!(current_native_mod(), Some(id));
}

struct Compass;
#[mod_api::__subscriptions]
impl Hooks for Compass {
    fn register_gui_layers(&self, _: &RegistrationContext, event: &mut RegisterGuiLayersEvent<'_>) {
        use GuiLayerPlacement::*;
        // The layer namespace is deliberately different from the registering mod.
        for (id, placement) in [
            (
                "hud_test:under_crosshair",
                Below("minecraft:crosshair".into()),
            ),
            (
                "hud_test:over_crosshair",
                Above("minecraft:crosshair".into()),
            ),
            ("hud_test:bar", Above("minecraft:hotbar".into())),
            (
                "hud_test:above_health",
                Above("minecraft:player_health".into()),
            ),
            (
                "hud_test:below_contextual",
                Below("minecraft:contextual_info_bar".into()),
            ),
            ("hud_test:top", AboveAll),
        ] {
            assert!(event.layers.register(
                id.into(),
                placement,
                Box::new(move |_, draw| {
                    assert_owner("compass");
                    log(format!("draw {id}"));
                    if id == "hud_test:under_crosshair" {
                        draw.fill([0.0, 0.0, 4.0, 4.0], 0xFF00_FF00);
                    }
                }),
            ));
        }
    }
    fn gui_layer_pre(&self, _: &dyn ClientHost, event: &mut GuiLayerPreEvent<'_>) -> Flow {
        log(format!("pre {}", event.layer));
        Flow::Continue
    }
    fn gui_layer_post(&self, _: &dyn ClientHost, event: &mut GuiLayerPostEvent<'_>) {
        log(format!("post {}", event.layer));
    }
}

struct Inner;
#[mod_api::__subscriptions]
impl Hooks for Inner {
    fn register_gui_layers(&self, _: &RegistrationContext, event: &mut RegisterGuiLayersEvent<'_>) {
        assert!(event.layers.wrap_layer(
            "hud_test:under_crosshair",
            Box::new(|ctx| {
                assert_owner("inner");
                log("inner before");
                ctx.draw_previous();
                assert_owner("inner");
                log("inner after");
            }),
        ));
        assert!(event.layers.wrap_layer(
            "minecraft:hotbar",
            Box::new(|ctx| {
                log("hotbar wrapper before");
                ctx.draw_previous();
                log("hotbar wrapper after");
            }),
        ));
        for (id, label) in [
            ("minecraft:crosshair", "crosshair wrapper"),
            ("minecraft:contextual_info_bar", "contextual wrapper"),
        ] {
            assert!(event.layers.wrap_layer(
                id,
                Box::new(move |ctx| {
                    log(label);
                    ctx.draw_previous();
                })
            ));
        }
    }
}

struct Editor;
#[mod_api::__subscriptions]
impl Hooks for Editor {
    fn register_gui_layers(&self, _: &RegistrationContext, event: &mut RegisterGuiLayersEvent<'_>) {
        assert!(
            !event
                .layers
                .wrap_layer("hud_test:missing", Box::new(|_| {}))
        );
        assert!(
            !event
                .layers
                .replace_layer("hud_test:missing", Box::new(|_, _| {}))
        );
        assert!(event.layers.wrap_layer(
            "hud_test:under_crosshair",
            Box::new(|ctx| {
                assert_owner("editor");
                log("outer before");
                ctx.with_draw(&mut |draw| draw.fill([0.0, 0.0, 4.0, 4.0], 0xFFFF_0000));
                ctx.draw_previous();
                assert_owner("editor");
                ctx.with_draw(&mut |draw| draw.fill([0.0, 0.0, 4.0, 4.0], 0xFF00_00FF));
                log("outer after");
            }),
        ));
        assert!(event.layers.replace_layer(
            "minecraft:contextual_info_bar",
            Box::new(|_, _| {
                assert_owner("editor");
                log("contextual replacement");
            }),
        ));
    }
    fn gui_layer_pre(&self, _: &dyn ClientHost, event: &mut GuiLayerPreEvent<'_>) -> Flow {
        if matches!(event.layer, "hud_test:bar" | "minecraft:effects") {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

fn take() -> Vec<String> {
    std::mem::take(&mut *LOG.lock().unwrap())
}

const PLAYER_HEALTH: &str = "minecraft:player_health";
const AIR_LEVEL: &str = "minecraft:air_level";
const BUBBLE_POP: &str = "ui.hud.bubble_pop";
// Equal RGB channels make the marker's packed vertex colour independent of channel order.
const HEALTH_MARKER: u32 = 0xFF2D_2D2D;

#[derive(Default)]
struct HealthTrace {
    cancel_gui: bool,
    cancel_health: bool,
    cancel_air: bool,
    calls: Vec<&'static str>,
}

struct HealthLayer {
    replace: bool,
    trace: Arc<Mutex<HealthTrace>>,
}

#[mod_api::__subscriptions]
impl Hooks for HealthLayer {
    fn register_gui_layers(&self, _: &RegistrationContext, event: &mut RegisterGuiLayersEvent<'_>) {
        if self.replace {
            let trace = self.trace.clone();
            assert!(event.layers.replace_layer(
                PLAYER_HEALTH,
                Box::new(move |_, draw| {
                    trace.lock().unwrap().calls.push("replacement");
                    draw.gui_paint(&[GuiDraw::Fill {
                        x0: 80,
                        y0: 100,
                        x1: 150,
                        y1: 104,
                        color: HEALTH_MARKER,
                    }])
                    .unwrap();
                }),
            ));
        }
        let trace = self.trace.clone();
        assert!(event.layers.wrap_layer(
            AIR_LEVEL,
            Box::new(move |ctx| {
                trace.lock().unwrap().calls.push("air wrapper");
                ctx.draw_previous();
            }),
        ));
    }

    fn render_gui_pre(&self, _: &dyn ClientHost, _: &mut RenderGuiPreEvent) -> Flow {
        if self.trace.lock().unwrap().cancel_gui {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn gui_layer_pre(&self, _: &dyn ClientHost, event: &mut GuiLayerPreEvent<'_>) -> Flow {
        let mut trace = self.trace.lock().unwrap();
        match event.layer {
            PLAYER_HEALTH => {
                trace.calls.push("health pre");
                if trace.cancel_health {
                    return Flow::Cancel;
                }
            }
            "minecraft:armor_level" => trace.calls.push("armor pre"),
            "minecraft:food_level" => trace.calls.push("food pre"),
            "minecraft:vehicle_health" => trace.calls.push("vehicle pre"),
            AIR_LEVEL => {
                trace.calls.push("air pre");
                if trace.cancel_air {
                    return Flow::Cancel;
                }
            }
            _ => {}
        }
        Flow::Continue
    }

    fn player_heart_type(&self, _: &mut dyn ClientHost, _: &mut PlayerHeartTypeEvent) {
        self.trace.lock().unwrap().calls.push("hearts");
    }

    fn gui_layer_post(&self, _: &dyn ClientHost, event: &mut GuiLayerPostEvent<'_>) {
        let call = match event.layer {
            PLAYER_HEALTH => "health post",
            "minecraft:armor_level" => "armor post",
            "minecraft:food_level" => "food post",
            "minecraft:vehicle_health" => "vehicle post",
            AIR_LEVEL => "air post",
            _ => return,
        };
        self.trace.lock().unwrap().calls.push(call);
    }
}

#[test]
#[ignore = "daily"]
fn health_extraction_runs_only_in_the_vanilla_layer_body() {
    const CASE: &str = "SUNRISE_HEALTH_LAYER_FIXTURE";
    if run_fresh_cases(
        "game::gui_layer_tests::health_extraction_runs_only_in_the_vanilla_layer_body",
        CASE,
        &["vanilla", "replace"],
    ) {
        return;
    }
    let replace = std::env::var(CASE).unwrap() == "replace";
    let trace = Arc::new(Mutex::new(HealthTrace::default()));
    with_client_game(|game| {
        mod_platform::__private::install(
            vec![ModEntry::new(
                "health-test",
                Box::new(HealthLayer {
                    replace,
                    trace: trace.clone(),
                }),
            )]
            .into(),
        );
        mods::register_gui_layers(ExecutionRole::Client);
        game.title = None;
        game.screen = super::Screen::Playing;
        game.ui.hide_gui = false;
        game.set_mode_id(0);
        let mut player = PlayerMotion::new(game.motion, Vec3::new(2.0, 64.0, 2.0)).unwrap();
        player.set_health(12.0, false).unwrap();
        player
            .set_modifier(
                "minecraft:armor",
                "health-test:armor",
                2.0,
                "add_value",
                true,
            )
            .unwrap();
        // At 181/300, Hud.extractAirBubbles has six full bubbles and one popping.
        // Above water the same depleted air instead resets the pop latch, with no sound.
        player.air = 181;
        game.player = Some(player);
        for (under_water, creative, hidden) in [
            (false, false, false),
            (true, false, false),
            (true, true, false),
            (true, false, true),
        ] {
            game.set_mode_id(if creative { 1 } else { 0 });
            game.ui.hide_gui = hidden;
            if under_water && !game.player.as_ref().unwrap().is_eye_in_fluid(WATER) {
                let water = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:water")
                    .unwrap();
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                for y in 64..=66 {
                    game.world.set(2, y, 2, water).unwrap();
                }
                let player = game.player.as_mut().unwrap();
                MotionWorld {
                    entities: &[],
                    player,
                    scene: &mut game.world,
                    assets: &game.pack.assets,
                    collision_binding: game.collision,
                    resources: game.motion,
                    health: &game.health_resources,
                    tag_items: None,
                    loop_context: &game.loop_context,
                    side: dry_side(),
                    damage_hooks: &game.damage_hooks,
                    block_registries: None,
                }
                .update_fluids()
                .unwrap();
                assert!(player.is_eye_in_fluid(WATER));
            }
            let vanilla_eligible = !creative && !hidden;
            let vetoes: &[(bool, bool)] = if !vanilla_eligible {
                &[(false, false)]
            } else {
                &[(true, false), (true, true), (false, false), (false, true)]
            };
            for &(cancel_health, cancel_air) in vetoes {
                game.health_hud.last_health = 17;
                game.health_hud.last_bubble_pop_sound = 3;
                game.audio.heard.remove(BUBBLE_POP);
                *trace.lock().unwrap() = HealthTrace {
                    cancel_health,
                    cancel_air,
                    ..HealthTrace::default()
                };
                let batch = game.hud_layers([640, 480]);
                let calls = std::mem::take(&mut trace.lock().unwrap().calls);
                let mut expected = vec!["health pre"];
                if !cancel_health {
                    if replace {
                        expected.push("replacement");
                    } else if vanilla_eligible {
                        expected.push("hearts");
                    }
                    expected.push("health post");
                }
                expected.extend([
                    "armor pre",
                    "armor post",
                    "food pre",
                    "food post",
                    "vehicle pre",
                    "vehicle post",
                    "air pre",
                ]);
                if !cancel_air {
                    expected.extend(["air wrapper", "air post"]);
                }
                assert_eq!(calls, expected);
                let painted_replacement = !cancel_health && replace;
                assert_eq!(
                    batch.vertices.iter().any(|v| v.color == HEALTH_MARKER),
                    painted_replacement,
                );
                if painted_replacement {
                    assert_eq!(game.ui.room.bottom(), 100.0);
                } else if !vanilla_eligible {
                    assert_eq!(
                        game.ui.room.bottom(),
                        batch.height,
                        "no vanilla status paint"
                    );
                } else {
                    assert!(game.ui.room.bottom() > 100.0);
                }
                assert_eq!(
                    game.health_hud.last_health,
                    if cancel_health || replace || !vanilla_eligible {
                        17
                    } else {
                        12
                    }
                );
                let draws_air = !cancel_air && vanilla_eligible;
                let pops = draws_air && under_water;
                assert_eq!(
                    game.health_hud.last_bubble_pop_sound,
                    if !draws_air {
                        3
                    } else if pops {
                        7
                    } else {
                        0
                    }
                );
                // The sound is local-only; `heard` counts it even with the device disabled.
                assert_eq!(game.audio.heard.get(BUBBLE_POP).copied(), pops.then_some(1));
            }
        }
        // The last frame hid the vanilla HUD. The global veto still applies to its slots.
        *trace.lock().unwrap() = HealthTrace {
            cancel_gui: true,
            ..HealthTrace::default()
        };
        let batch = game.hud_layers([640, 480]);
        assert!(trace.lock().unwrap().calls.is_empty());
        assert!(!batch.vertices.iter().any(|v| v.color == HEALTH_MARKER));
    });
}

#[test]
#[ignore = "daily"]
fn mod_layers_draw_in_their_place_and_hide_like_vanilla_layers() {
    // The layer order is kept once per process.
    if run_fresh_cases(
        "game::gui_layer_tests::mod_layers_draw_in_their_place_and_hide_like_vanilla_layers",
        "SUNRISE_GUI_LAYER_FIXTURE",
        &["run"],
    ) {
        return;
    }
    with_client_game(|game| {
        mod_platform::__private::install(
            vec![
                ModEntry::new("compass", Box::new(Compass)),
                ModEntry::new("inner", Box::new(Inner)),
                ModEntry::new("editor", Box::new(Editor)),
            ]
            .into(),
        );
        mods::register_gui_layers(ExecutionRole::Client);
        let mut batch = HudBatch::new([320, 240], game.settings.gui_scale_inputs());

        // Simulate the HUD paint sites with the crosshair skipped (third person).
        // Its wrapper doesn't draw, but both neighboring layers still do.
        game.hud_layer_next = Some(0);
        for (layer, paint) in [
            (VanillaGuiLayer::Hotbar, "hotbar paint"),
            (VanillaGuiLayer::Effects, "effects paint"),
            (VanillaGuiLayer::PlayerHealth, "health paint"),
            (VanillaGuiLayer::ContextualInfoBar, "contextual paint"),
            (VanillaGuiLayer::Chat, "chat paint"),
        ] {
            game.gui_layer(layer, &mut batch, &mut |_, _| log(paint));
        }
        game.draw_mod_gui_layers(None, &mut batch);
        game.hud_layer_next = None;
        assert_eq!(current_native_mod(), None);
        assert_eq!(
            take(),
            [
                "pre hud_test:under_crosshair",
                "outer before",
                "inner before",
                "draw hud_test:under_crosshair",
                "inner after",
                "outer after",
                "post hud_test:under_crosshair",
                "pre hud_test:over_crosshair",
                "draw hud_test:over_crosshair",
                "post hud_test:over_crosshair",
                "pre minecraft:hotbar",
                "hotbar wrapper before",
                "hotbar paint",
                "hotbar wrapper after",
                "post minecraft:hotbar",
                "pre hud_test:bar",
                "pre minecraft:effects",
                "pre minecraft:player_health",
                "health paint",
                "post minecraft:player_health",
                "pre hud_test:above_health",
                "draw hud_test:above_health",
                "post hud_test:above_health",
                "pre hud_test:below_contextual",
                "draw hud_test:below_contextual",
                "post hud_test:below_contextual",
                "pre minecraft:contextual_info_bar",
                "contextual replacement",
                "post minecraft:contextual_info_bar",
                "pre minecraft:chat",
                "chat paint",
                "post minecraft:chat",
                "pre hud_test:top",
                "draw hud_test:top",
                "post hud_test:top",
            ]
        );
        // The outer wrapper and the underlying painter append into the same batch in order.
        let mut colors: Vec<_> = batch
            .vertices
            .iter()
            .map(|v| v.color.to_le_bytes())
            .collect();
        colors.dedup();
        assert_eq!(
            colors,
            [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]]
        );

        // With the HUD hidden (F1), vanilla's sleep overlay and ungated layers still draw.
        game.gui_layer(VanillaGuiLayer::SleepOverlay, &mut batch, &mut |_, _| {
            log("sleep paint")
        });
        game.draw_mod_gui_layers(None, &mut batch);
        assert_eq!(
            take(),
            [
                "pre minecraft:sleep_overlay",
                "sleep paint",
                "post minecraft:sleep_overlay"
            ]
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_gui_wrapper_panic_is_contained_but_delegated_engine_paint_is_fatal() {
    use sunrise_client::hud::{Blend, rgba};

    let test = "game::gui_layer_tests::a_gui_wrapper_panic_is_contained_but_delegated_engine_paint_is_fatal";
    crate::game::in_own_process(test, || {
        struct Wrapper(&'static str, bool);
        #[mod_api::__subscriptions]
        impl Hooks for Wrapper {
            fn register_gui_layers(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterGuiLayersEvent<'_>,
            ) {
                let Self(id, broken) = *self;
                assert!(event.layers.wrap_layer(
                    "minecraft:hotbar",
                    Box::new(move |ctx| {
                        assert_owner(id);
                        log(id);
                        if broken {
                            ctx.with_draw(&mut |draw| draw.fill([0.0, 0.0, 4.0, 4.0], 0xFFFF_0000));
                            ctx.draw_previous();
                            ctx.with_draw(&mut |draw| draw.fill([0.0, 0.0, 4.0, 4.0], 0xFF00_00FF));
                            ctx.draw_previous();
                            ctx.with_draw(&mut |draw| draw.fill([0.0, 0.0, 4.0, 4.0], 0xFFFF_0000));
                            panic!("broken wrapper");
                        }
                        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            ctx.draw_previous();
                        }))
                        .is_err()
                        {
                            log(format!("caught by {id}"));
                        }
                        assert_owner(id);
                    }),
                ));
            }
        }
        with_client_game(|game| {
            mod_platform::__private::install(
                vec![
                    ModEntry::new("inner", Box::new(Wrapper("inner", false))),
                    ModEntry::new("broken", Box::new(Wrapper("broken", true))),
                    ModEntry::new("outer", Box::new(Wrapper("outer", false))),
                ]
                .into(),
            );
            mods::register_gui_layers(ExecutionRole::Client);
            let mut batch = HudBatch::scratch([320, 240]);
            let white = rgba(255, 255, 255, 255);
            let green = rgba(0, 255, 0, 255);
            let yellow = rgba(255, 255, 0, 255);
            let mut paint_calls = 0;
            batch.fill(&game.gui.atlas, 0.0, 0.0, 4.0, 4.0, white);
            game.gui_layer(VanillaGuiLayer::Hotbar, &mut batch, &mut |g, batch| {
                log("vanilla paint");
                let start = batch.vertices.len();
                let color = [green, yellow][paint_calls];
                paint_calls += 1;
                batch.fill(&g.gui.atlas, 0.0, 0.0, 4.0, 4.0, color);
                batch.invert_since(start);
            });
            assert_eq!(
                take(),
                [
                    "outer",
                    "broken",
                    "inner",
                    "vanilla paint",
                    "inner",
                    "vanilla paint"
                ]
            );
            assert_eq!(
                batch.vertices.iter().map(|v| v.color).collect::<Vec<_>>(),
                [[white; 6], [green; 6], [yellow; 6]].concat()
            );
            assert_eq!(
                batch.blends,
                [(6..12, Blend::Invert), (12..18, Blend::Invert)]
            );

            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                game.gui_layer(VanillaGuiLayer::Hotbar, &mut batch, &mut |_, _| {
                    assert_eq!(current_native_mod(), None);
                    assert!(!mod_api::__private::game::retained_callback_active());
                    panic!("vanilla paint failed");
                });
            }))
            .unwrap_err();
            assert_eq!(panic.downcast_ref::<&str>(), Some(&"vanilla paint failed"));
            assert_eq!(
                take(),
                ["outer", "inner", "caught by inner", "caught by outer"]
            );
            assert_eq!(current_native_mod(), None);

            game.gui_layer(VanillaGuiLayer::Hotbar, &mut batch, &mut |_, _| {
                log("vanilla paint");
            });
            assert_eq!(take(), ["outer", "inner", "vanilla paint"]);
        });
    });
}

fn enter_test_world(game: &mut Game<'_>) {
    game.title = None;
    game.set_screen(Screen::Playing);
    game.player = Some(
        sunrise_client::motion::PlayerMotion::new(
            game.motion,
            sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5),
        )
        .unwrap(),
    );
}

#[test]
#[ignore = "daily"]
fn f1_hides_vanilla_paint_without_skipping_the_hud_walk() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use sunrise_client::{
        chat::ChatSource,
        hud::rgba,
        text::{Style, Text},
    };

    let test = "game::gui_layer_tests::f1_hides_vanilla_paint_without_skipping_the_hud_walk";
    crate::game::in_own_process(test, || {
        const MOD_PAINT: u32 = 0xFF13_579B;
        static CANCEL_HUD: AtomicBool = AtomicBool::new(false);

        struct Observer;
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn render_gui_pre(&self, _: &dyn ClientHost, _: &mut RenderGuiPreEvent) -> Flow {
                log("hud pre");
                if CANCEL_HUD.load(Ordering::Relaxed) {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }

            fn register_gui_layers(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterGuiLayersEvent<'_>,
            ) {
                assert!(event.layers.register(
                    "hud_test:indicator".into(),
                    GuiLayerPlacement::AboveAll,
                    Box::new(|_, draw| {
                        log("indicator paint");
                        draw.fill([14.0, 18.0, 4.0, 4.0], MOD_PAINT);
                    }),
                ));
                assert!(event.layers.wrap_layer(
                    "minecraft:chat",
                    Box::new(|ctx| {
                        log("chat wrapper");
                        ctx.draw_previous();
                    }),
                ));
            }

            fn gui_layer_pre(&self, _: &dyn ClientHost, event: &mut GuiLayerPreEvent<'_>) -> Flow {
                log(format!("pre {}", event.layer));
                Flow::Continue
            }

            fn gui_layer_post(&self, _: &dyn ClientHost, event: &mut GuiLayerPostEvent<'_>) {
                log(format!("post {}", event.layer));
            }

            fn render_gui_post(&self, _: &dyn ClientHost, _: &mut RenderGuiPostEvent<'_>) {
                log("hud post");
            }

            fn screen_render_pre(
                &self,
                _: &dyn ClientHost,
                _: &mut ScreenRenderPreEvent<'_>,
            ) -> Flow {
                log("screen pre");
                Flow::Continue
            }

            fn screen_render_background(
                &self,
                _: &dyn ClientHost,
                _: &mut ScreenRenderBackgroundEvent<'_>,
            ) {
                log("screen background");
            }

            fn screen_render_post(&self, _: &dyn ClientHost, _: &mut ScreenRenderPostEvent<'_>) {
                log("screen post");
            }
        }

        with_client_game(|game| {
            mod_platform::__private::install(
                vec![ModEntry::new("observer", Box::new(Observer))].into(),
            );
            mods::register_gui_layers(ExecutionRole::Client);
            let size = [640, 480];
            let screen_events = ["screen pre", "screen background", "screen post"];
            game.title = Some(game.new_title(game.save.root.clone()));
            game.open_options(true);
            game.hud_layers(size);
            assert_eq!(take(), screen_events, "title Options has no HUD walk");

            enter_test_world(game);
            let rect = game.gui.atlas.gui_rect("hud/hotbar").unwrap();
            let hotbar_uv = [rect.u0, rect.v0];
            let has_hotbar = |frame: &HudBatch| {
                frame
                    .vertices
                    .iter()
                    .any(|v| v.uv == hotbar_uv && v.pos[1] == frame.height - 22.0)
            };
            let has_indicator = |frame: &HudBatch| {
                frame
                    .vertices
                    .iter()
                    .any(|v| v.color == sunrise_client::hud_layout::abgr(MOD_PAINT))
            };
            let mut expected = vec!["hud pre".to_owned()];
            // NeoForge visits hidden and visible slots the same way. The literal IDs also
            // cover normally absent vanilla bodies, e.g. vehicle health with no mount.
            for id in [
                "crosshair",
                "hotbar",
                "effects",
                "boss_overlay",
                "player_health",
                "armor_level",
                "food_level",
                "vehicle_health",
                "air_level",
                "contextual_info_bar",
                "selected_item_name",
                "sleep_overlay",
                "overlay_message",
                "title",
                "scoreboard_sidebar",
                "subtitle_overlay",
                "chat",
                "tab_list",
            ] {
                expected.push(format!("pre minecraft:{id}"));
                if id == "chat" {
                    expected.push("chat wrapper".to_owned());
                }
                expected.push(format!("post minecraft:{id}"));
            }
            expected.extend(
                [
                    "pre hud_test:indicator",
                    "indicator paint",
                    "post hud_test:indicator",
                    "hud post",
                ]
                .map(str::to_owned),
            );

            let chat_paint = |frame: &HudBatch| {
                frame
                    .vertices
                    .iter()
                    .position(|v| v.color & 0x00FF_FFFF == rgba(0x27, 0xB4, 0xD9, 0))
                    .map(|first_paint| first_paint >= frame.blur_at as usize)
            };
            for screen in [Screen::Playing, Screen::Options, Screen::Chat] {
                let after_hud: &[&str] = match screen {
                    Screen::Options => {
                        game.set_screen(Screen::Paused);
                        game.open_options(false);
                        &screen_events
                    }
                    Screen::Chat => {
                        game.set_screen(Screen::Playing);
                        game.begin_chat_screen("", false);
                        assert!(game.add_chat_line(
                            Text::styled("Focused chat", Style::color(0x27_B4_D9)),
                            ChatSource::Client
                        ));
                        &screen_events
                    }
                    _ => &[],
                };
                let expected: Vec<_> = expected
                    .iter()
                    .map(String::as_str)
                    .chain(after_hud.iter().copied())
                    .collect();
                for hidden in [false, true] {
                    game.ui.hide_gui = hidden;
                    let frame = game.hud_layers(size);
                    assert_eq!(take(), expected, "{screen:?}, F1 hidden: {hidden}");
                    assert_eq!(has_hotbar(&frame), !hidden);
                    assert!(has_indicator(&frame));
                    assert_eq!(chat_paint(&frame), (screen == Screen::Chat).then_some(true));
                }
            }
            CANCEL_HUD.store(true, Ordering::Relaxed);
            let frame = game.hud_layers(size);
            assert_eq!(
                take(),
                ["hud pre", "screen pre", "screen background", "screen post"]
            );
            assert!(!has_hotbar(&frame));
            assert!(!has_indicator(&frame));
            assert_eq!(
                chat_paint(&frame),
                Some(true),
                "HUD cancellation leaves focused chat on the screen"
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn f1_subtitles_paint_in_the_container_background_without_redispatch() {
    use sunrise_client::{hud::rgba, options_file::V, sound_engine::Notification};

    crate::game::in_own_process(
        "game::gui_layer_tests::f1_subtitles_paint_in_the_container_background_without_redispatch",
        || {
            const MARKER: u32 = 0xFFAE_2818;
            struct Observer;
            #[mod_api::__subscriptions]
            impl Hooks for Observer {
                fn gui_layer_pre(
                    &self,
                    _: &dyn ClientHost,
                    event: &mut GuiLayerPreEvent<'_>,
                ) -> Flow {
                    if event.layer == "minecraft:subtitle_overlay" {
                        log("subtitle pre");
                    }
                    Flow::Continue
                }
                fn gui_layer_post(&self, _: &dyn ClientHost, event: &mut GuiLayerPostEvent<'_>) {
                    if event.layer == "minecraft:subtitle_overlay" {
                        log("subtitle post");
                    }
                }
                fn screen_render_background(
                    &self,
                    _: &dyn ClientHost,
                    event: &mut ScreenRenderBackgroundEvent<'_>,
                ) {
                    event.draw.fill([0.0, 0.0, 1.0, 1.0], MARKER);
                }
            }

            with_client_game(|game| {
                mod_platform::__private::install(
                    vec![ModEntry::new("observer", Box::new(Observer))].into(),
                );
                enter_test_world(game);
                game.settings.show_subtitles = true;
                game.settings
                    .file
                    .set("backgroundForChatOnly", V::Bool(false));
                game.settings.text_background_opacity = 0.4;
                let size = [640, 480];
                game.hud(size); // Start listening before the sound plays.
                assert_eq!(take(), ["subtitle pre", "subtitle post"]);
                game.audio.engine.notifications.push(Notification {
                    identifier: "minecraft:entity.player.levelup".into(),
                    subtitle: Some("subtitles.entity.player.levelup".into()),
                    position: game.view.position,
                    relative: false,
                    looping: false,
                    range: 16.0,
                });
                game.tick_subtitles();
                game.toggle_hide_gui();
                let is_subtitle =
                    |v: &sunrise_client::hud::HudVertex| v.color == rgba(0, 0, 0, 102);

                let frame = game.hud(size);
                assert!(
                    !frame.vertices.iter().any(is_subtitle),
                    "F1 without a screen"
                );
                assert_eq!(take(), ["subtitle pre", "subtitle post"]);

                game.set_screen(Screen::Inventory);
                let frame = game.hud(size);
                let mut subtitle = frame
                    .vertices
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| is_subtitle(v));
                let (first, v) = subtitle
                    .next()
                    .expect("F1 keeps subtitles in the inventory");
                assert_eq!(v.pos[1], frame.height - 40.0);
                assert_eq!(subtitle.count(), 5, "one subtitle backdrop quad");
                let dim = frame
                    .vertices
                    .iter()
                    .position(|v| v.color == rgba(16, 16, 16, 160))
                    .unwrap();
                let screen = frame
                    .vertices
                    .iter()
                    .position(|v| v.color == sunrise_client::hud_layout::abgr(MARKER))
                    .unwrap();
                assert!(
                    dim < first && first < screen,
                    "subtitles draw over the dimming before screen widgets"
                );
                assert_eq!(take(), ["subtitle pre", "subtitle post"]);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn options_controller_notice_paints_after_screen_post_despite_hud_cancel() {
    use sunrise_client::{controller_notice::Notice, hud::rgba, hud_layout::abgr};

    crate::game::in_own_process(
        "game::gui_layer_tests::options_controller_notice_paints_after_screen_post_despite_hud_cancel",
        || {
            const MARKER: u32 = 0xFFAE_2818;
            struct CancelHud;
            #[mod_api::__subscriptions]
            impl Hooks for CancelHud {
                fn render_gui_pre(&self, _: &dyn ClientHost, _: &mut RenderGuiPreEvent) -> Flow {
                    Flow::Cancel
                }
                fn screen_render_post(
                    &self,
                    _: &dyn ClientHost,
                    event: &mut ScreenRenderPostEvent<'_>,
                ) {
                    event.draw.fill([0.0, 0.0, 1.0, 1.0], MARKER);
                }
            }

            with_client_game(|game| {
                mod_platform::__private::install(
                    vec![ModEntry::new("cancel_hud", Box::new(CancelHud))].into(),
                );
                enter_test_world(game);
                game.toggle_hide_gui();
                game.set_screen(Screen::Paused);
                game.open_options(false);
                game.controller_notice
                    .raise(Notice::new("Controller disconnected", "Use the keyboard"));
                let frame = game.hud_layers([640, 480]);
                let post = frame
                    .vertices
                    .iter()
                    .position(|v| v.color == abgr(MARKER))
                    .unwrap();
                // The notice plate's 1px border is emitted even at the start of the slide.
                let notice = frame
                    .vertices
                    .iter()
                    .position(|v| v.color == rgba(255, 255, 255, 0x5A))
                    .unwrap();
                assert!(notice > post);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn panicking_hud_listeners_under_f1_disable_only_themselves_and_report_their_owner() {
    use std::time::{Duration, Instant};
    use sunrise_client::hud::Rect;

    crate::game::in_own_process(
        "game::gui_layer_tests::panicking_hud_listeners_under_f1_disable_only_themselves_and_report_their_owner",
        || {
            const OWNER: &str = "broken_hud_listener";
            struct Broken;
            #[mod_api::__subscriptions]
            impl Hooks for Broken {
                fn render_gui_pre(&self, _: &dyn ClientHost, _: &mut RenderGuiPreEvent) -> Flow {
                    log("broken hud pre");
                    panic!("HUD pre failed");
                }
                fn gui_layer_pre(
                    &self,
                    _: &dyn ClientHost,
                    event: &mut GuiLayerPreEvent<'_>,
                ) -> Flow {
                    if event.layer == "minecraft:crosshair" {
                        log("broken crosshair pre");
                        panic!("HUD layer pre failed");
                    }
                    Flow::Continue
                }
                fn render_gui_post(&self, _: &dyn ClientHost, _: &mut RenderGuiPostEvent<'_>) {
                    log("same mod hud post");
                }
            }
            struct Observer;
            #[mod_api::__subscriptions]
            impl Hooks for Observer {
                fn render_gui_pre(&self, _: &dyn ClientHost, _: &mut RenderGuiPreEvent) -> Flow {
                    log("observer hud pre");
                    Flow::Continue
                }
                fn gui_layer_pre(
                    &self,
                    _: &dyn ClientHost,
                    event: &mut GuiLayerPreEvent<'_>,
                ) -> Flow {
                    if event.layer == "minecraft:crosshair" {
                        log("observer crosshair pre");
                    }
                    Flow::Continue
                }
            }

            with_client_game(|game| {
                mod_platform::__private::install(
                    vec![
                        ModEntry::new(OWNER, Box::new(Broken)),
                        ModEntry::new("observing_hud_listener", Box::new(Observer)),
                    ]
                    .into(),
                );
                enter_test_world(game);
                game.toggle_hide_gui();
                assert!(game.ui.player_hides_gui && game.ui.hide_gui);
                let size = [640, 480];
                let _ = game.hud(size);
                assert_eq!(
                    take(),
                    [
                        "broken hud pre",
                        "observer hud pre",
                        "broken crosshair pre",
                        "observer crosshair pre",
                        "same mod hud post"
                    ]
                );
                let [(Some(whole), _), (Some(layer), _)] =
                    game.server_packs.hook_panics.as_mut_slice()
                else {
                    panic!("one shared error toast per panicking listener");
                };
                let rect = *game
                    .gui
                    .atlas
                    .sprites
                    .entry("toast/system".into())
                    .or_insert(Rect {
                        u0: 0.25,
                        v0: 0.25,
                        u1: 0.5,
                        v1: 0.5,
                        w: 160,
                        h: 32,
                    });
                let y =
                    [(whole, "render_gui_pre"), (layer, "gui_layer_pre")].map(|(toast, hook)| {
                        let message = toast.message.as_ref().unwrap().component();
                        assert_eq!(
                            message,
                            serde_json::json!({
                                "translate": "sunrise.mods.hook_panic.description",
                                "with": [{"text": OWNER}, {"text": hook}],
                            })
                        );
                        toast.slide.origin = Some(Instant::now() - Duration::from_secs(1));
                        32.0 * toast.slot.unwrap() as f32
                    });
                let frame = game.hud(size);
                for y in y {
                    assert!(
                        frame.vertices.iter().any(|v| v.uv == [rect.u0, rect.v0]
                            && v.pos[1] == y
                            && v.pos[0] < frame.width),
                        "F1 must not hide the shared callback error toast"
                    );
                }
                assert_eq!(
                    take(),
                    [
                        "observer hud pre",
                        "observer crosshair pre",
                        "same mod hud post"
                    ]
                );
                assert_eq!(game.server_packs.hook_panics.len(), 2);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_panicking_hud_painter_shows_its_error_toast_under_f1() {
    use std::time::{Duration, Instant};
    use sunrise_client::{text::Text, toasts::SystemToast};

    const OWNER: &str = "broken_hud";
    let test = "game::gui_layer_tests::a_panicking_hud_painter_shows_its_error_toast_under_f1";
    crate::game::in_own_process(test, || {
        struct BrokenHud;
        #[mod_api::__subscriptions]
        impl Hooks for BrokenHud {
            fn register_gui_layers(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterGuiLayersEvent<'_>,
            ) {
                assert!(event.layers.register(
                    "hud_test:broken".into(),
                    GuiLayerPlacement::AboveAll,
                    Box::new(|_, _| {
                        log("broken painter called");
                        panic!("broken HUD painter");
                    }),
                ));
            }
        }

        with_client_game(|game| {
            mod_platform::__private::install(
                vec![ModEntry::new(OWNER, Box::new(BrokenHud))].into(),
            );
            mods::register_gui_layers(ExecutionRole::Client);
            enter_test_world(game);
            game.toggle_hide_gui();
            assert!(game.ui.player_hides_gui && game.ui.hide_gui);
            game.server_packs.load_fail = Some(SystemToast::new(
                Text::tr("resourcePack.load_fail", &[]),
                None,
            ));
            let size = [640, 480];
            let _ = game.hud(size);

            // Backdate the slide to test fully visible toasts without waiting for animation.
            let slide_started = Instant::now() - Duration::from_millis(700);
            let [(Some(error), _)] = game.server_packs.hook_panics.as_mut_slice() else {
                panic!("the shared callback owner must report one panic");
            };
            assert_eq!(
                error.title.translation_key(),
                Some("sunrise.mods.hook_panic.title")
            );
            assert_eq!(
                error.message.as_ref().unwrap().component()["with"][0]["text"],
                OWNER
            );
            let error_y = 32.0 * error.slot.unwrap() as f32;
            error.slide.origin = Some(slide_started);
            let ordinary = game.server_packs.load_fail.as_mut().unwrap();
            let ordinary_y = 32.0 * ordinary.slot.unwrap() as f32;
            assert_ne!(ordinary_y, error_y);
            ordinary.slide.origin = Some(slide_started);
            let rect = *game
                .gui
                .atlas
                .sprites
                .entry("toast/system".into())
                .or_insert(sunrise_client::hud::Rect {
                    u0: 0.25,
                    v0: 0.25,
                    u1: 0.5,
                    v1: 0.5,
                    w: 160,
                    h: 32,
                });

            let frame = game.hud(size);
            let has_toast = |y| {
                frame
                    .vertices
                    .iter()
                    .any(|v| v.uv == [rect.u0, rect.v0] && v.pos[1] == y && v.pos[0] < frame.width)
            };
            assert!(
                has_toast(error_y),
                "F1 must not hide the callback error toast"
            );
            assert!(!has_toast(ordinary_y), "F1 still hides ordinary toasts");
            assert_eq!(
                take(),
                ["broken painter called"],
                "a panic disables that painter"
            );
        });
    });
}
