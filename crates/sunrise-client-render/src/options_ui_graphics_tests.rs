//! The graphics screen as the options host it: the output of the renderer they are drawn for,
//! and its pages.
use super::tests::atlas;
use super::*;
use crate::gfx::EffectiveOutput;
use crate::sunrise_ui::{SunriseRow, Tab};

/// What a window offering HDR10 shows the options when its system reports `system` HDR, Auto
/// saved.
fn auto_window(system: Option<bool>) -> EffectiveOutput {
    use crate::gfx::output::tests::{snapshot, window};
    let (caps, reading) = window(Some(crate::gfx::HdrOutput::Pq), system);
    let auto = crate::graphics::Graphics {
        hdr: crate::graphics::HdrMode::Auto,
        ..Default::default()
    };
    snapshot(&auto, Default::default(), caps, reading)
}
/// Calibrate on the graphics screen's HDR page, clicked: whether calibration opened (its
/// row is blocked without an offer, or with Auto and no positive report).
fn calibrates(ui: &mut OptionsUi, s: &mut Settings, atlas: &HudAtlas) -> bool {
    use crate::graphics_ui::{H, Item};
    ui.sunrise
        .show(Tab::Graphics(crate::graphics_ui::Page::Hdr));
    let calibrate = SunriseRow::Graphics(Item::Sunrise(H::HdrCalibrate));
    let (x, y) = ui.sunrise.point_at(s, ui.size, calibrate);
    ui.click(s, atlas, x, y, ui.size, ui.framebuffer, false, false);
    ui.page() == Page::HdrCalibration
}

/// Opening the graphics screen keeps the offer and report of the renderer the options are
/// drawn for: the screen opens afresh, the output doesn't go with it.
#[test]
fn opening_the_graphics_screen_keeps_the_offer_and_report() {
    let atlas = atlas();
    let mut s = Settings::default();
    s.sunrise.hdr = crate::graphics::HdrMode::Auto;
    let mut ui = OptionsUi {
        size: (427.0, 240.0),
        ..Default::default()
    };
    let batch = || HudBatch::scratch([427, 240]);
    ui.draw_test_frame(
        &s,
        auto_window(Some(true)),
        &mut batch(),
        &atlas,
        (-1.0, -1.0),
        &mut |_, _| {},
    );
    ui.show(Page::Graphics);
    assert!(calibrates(&mut ui, &mut s, &atlas));
    // And again, from another page and back.
    ui.back(&mut s);
    ui.back(&mut s);
    ui.show(Page::Graphics);
    assert!(calibrates(&mut ui, &mut s, &atlas));
}

/// A report that changes while the graphics screen is open (Auto's system HDR turned on)
/// shows on the next draw, with no setting applied.
#[test]
fn a_changed_report_shows_without_an_apply() {
    let atlas = atlas();
    let mut s = Settings::default();
    s.sunrise.hdr = crate::graphics::HdrMode::Auto;
    let mut ui = OptionsUi {
        size: (427.0, 240.0),
        ..Default::default()
    };
    let batch = || HudBatch::scratch([427, 240]);
    ui.show(Page::Graphics);
    ui.draw_test_frame(
        &s,
        auto_window(Some(false)),
        &mut batch(),
        &atlas,
        (-1.0, -1.0),
        &mut |_, _| {},
    );
    assert!(!calibrates(&mut ui, &mut s, &atlas));
    ui.draw_test_frame(
        &s,
        auto_window(Some(true)),
        &mut batch(),
        &atlas,
        (-1.0, -1.0),
        &mut |_, _| {},
    );
    assert!(calibrates(&mut ui, &mut s, &atlas));
}

/// The `shaders/` packs are listed on Sunrise Settings' Shaders page: a press picks one,
/// and reads its settings; Shader Options... opens them, and Done comes back to that page with
/// them still read. With nothing picked, the row is greyed.
#[test]
fn the_shaders_page_lists_the_packs_and_opens_the_picks_settings() {
    use crate::shader_pack_list::Entry;
    use crate::shader_ui::Item;
    let atlas = atlas();
    let mut s = Settings {
        shader_packs: vec![Entry {
            file: "Pack.zip".into(),
        }],
        ..Settings::default()
    };
    let mut ui = OptionsUi {
        size: (427.0, 240.0),
        ..Default::default()
    };
    ui.show(Page::Sunrise);
    let shaders = Tab::Graphics(crate::graphics_ui::Page::Shaders);
    ui.sunrise.show(shaders);
    let listed = ui.sunrise.listed(&s, ui.size);
    let rows: Vec<SunriseRow> = listed.into_iter().map(|(_, row)| row).collect();
    let key = SunriseRow::Extra(crate::extras_ui::ExtraRow::Item(
        crate::extras::Item::ShaderPackKey,
    ));
    let cache = SunriseRow::Graphics(crate::graphics_ui::Item::Sunrise(
        crate::graphics_ui::H::ShadowCache,
    ));
    assert_eq!(
        rows,
        [
            SunriseRow::Pack(Item::Pack(0)),
            SunriseRow::Pack(Item::Folder),
            SunriseRow::ShaderOptions,
            cache,
            key,
        ]
    );
    let click = |ui: &mut OptionsUi, s: &mut Settings, row| {
        let at = ui.sunrise.point_at(s, ui.size, row);
        ui.click(s, &atlas, at.0, at.1, ui.size, [854, 480], false, false)
    };
    // Nothing picked: the way to the pick's settings is greyed.
    assert!(click(&mut ui, &mut s, SunriseRow::ShaderOptions).is_empty());
    assert_eq!(ui.page(), Page::Sunrise);
    let events = click(&mut ui, &mut s, SunriseRow::Pack(Item::Pack(0)));
    assert_eq!(s.sunrise.shader_pack, "Pack.zip");
    assert!(events.contains(&Event::ShaderPack), "{events:?}");
    // The pick's settings are read with it: the game answers that event.
    assert!(events.contains(&Event::OpenShaderOptions), "{events:?}");
    s.shader_options = Some(crate::shader_options::ShaderOptions::new(
        "Pack.zip".into(),
        Default::default(),
        Default::default(),
        Default::default(),
        Default::default(),
    ));
    click(&mut ui, &mut s, SunriseRow::ShaderOptions);
    assert_eq!(ui.page(), Page::ShaderOptions);
    // Back on Sunrise Settings, unchanged, they stay read for the next time.
    let events = ui.back(&mut s);
    assert_eq!(ui.page(), Page::Sunrise);
    assert_eq!(ui.sunrise.open_page(), Some(shaders));
    assert!(s.shader_options.is_some());
    assert!(!events.contains(&Event::ShaderOptions), "{events:?}");
    // Changed, they are saved, then read again, in that order.
    click(&mut ui, &mut s, SunriseRow::ShaderOptions);
    assert_eq!(ui.page(), Page::ShaderOptions);
    if let Some(o) = s.shader_options.as_mut() {
        o.changed = true;
    }
    let events = ui.back(&mut s);
    assert_eq!(ui.page(), Page::Sunrise);
    let shader_events: Vec<&Event> = (events.iter())
        .filter(|e| matches!(e, Event::ShaderOptions | Event::OpenShaderOptions))
        .collect();
    assert_eq!(
        shader_events,
        [&Event::ShaderOptions, &Event::OpenShaderOptions],
        "{events:?}"
    );
    // Undo puts the pick back, and says so.
    let undo = ui.sunrise.foot_at(&s, ui.size);
    let (_, [x, y, w, h]) = undo
        .into_iter()
        .find(|(target, _)| *target == shell::Target::Undo)
        .expect("Undo shows after a pick");
    let events = ui.click(
        &mut s,
        &atlas,
        x + w / 2.0,
        y + h / 2.0,
        ui.size,
        [854, 480],
        false,
        false,
    );
    assert_eq!(s.sunrise.shader_pack, "");
    assert!(events.contains(&Event::ShaderPack), "{events:?}");
}

#[test]
fn the_graphics_screens_wheel_scrolls_even_over_a_slider_being_changed() {
    use crate::graphics_ui::{H, Item};
    use winit::keyboard::KeyCode as K;
    let mut s = Settings::default();
    let mut ui = OptionsUi::default();
    ui.show(Page::Graphics);
    ui.sunrise
        .show(Tab::Graphics(crate::graphics_ui::Page::Performance));
    let size = (640.0, 360.0);
    s.sunrise.chunk_updates = 16;
    let chunk_updates = SunriseRow::Graphics(Item::Sunrise(H::ChunkUpdates));
    // Tab onto it: the arrows change it.
    while ui.sunrise.focused_row() != Some(chunk_updates) {
        shell::Hosted::key(&mut ui.sunrise, &mut s, K::Tab, None, false, size);
    }
    assert!(ui.sunrise.changes_value());
    let before = ui.sunrise.scroll_amount();
    ui.wheel(&mut s, size, (320.0, 150.0), [1920, 1080], -1.0);
    assert_eq!(s.sunrise.chunk_updates, 16);
    assert!(ui.sunrise.scroll_amount() > before);
}

/// A window holding HDR output back (a start failed on its driver) shows it on the HDR page
/// as soon as the graphics screen opens, with no setting applied: the row's line, and
/// Calibrate blocked, where the same window not holding it back calibrates.
#[test]
fn a_held_hdr_start_shows_as_the_graphics_screen_opens() {
    use crate::gfx::output::tests::{snapshot, window};
    let atlas = atlas();
    let mut s = Settings::default();
    s.sunrise.hdr = crate::graphics::HdrMode::Auto;
    s.sunrise.hdr_failed_start = Some("Vulkan 10de:2684 GPU; driver; 1.0; os".into());
    let (mut caps, reading) = window(Some(crate::gfx::HdrOutput::Pq), Some(true));
    for held in [true, false] {
        caps.held = held;
        let mut ui = OptionsUi {
            size: (427.0, 240.0),
            ..Default::default()
        };
        let output = snapshot(&s.sunrise, Default::default(), caps, reading);
        ui.draw_test_frame(
            &s,
            output,
            &mut HudBatch::new(
                [427, 240],
                crate::hud::GuiScale {
                    requested: 1,
                    unicode: false,
                },
            ),
            &atlas,
            (-1.0, -1.0),
            &mut |_, _| {},
        );
        ui.show(Page::Graphics);
        assert!(calibrates(&mut ui, &mut s, &atlas) != held, "held {held}");
        assert_eq!(ui.sunrise.screen.graphics.output.held, held);
    }
}

/// The slider position `Opt::ChatDelay`'s kind() gives the stored seconds.
fn delay_pos(seconds: f64) -> f64 {
    match values::kind(seconds) {
        Kind::Slider(p) => p,
        _ => panic!("the chat delay is a slider"),
    }
}
/// `Options.chatDelay`: `IntRange(0, 60)` xmap(v / 10) — a slider in tenths of
/// a second from 0 to 6, not a button with no value behind it.
#[test]
fn the_chat_delay_slider_steps_in_tenths() {
    assert_eq!(delay_pos(0.0), 0.0);
    assert!((delay_pos(3.0) - 0.5).abs() < 1e-6);
    assert_eq!(delay_pos(6.0), 1.0);
    // A value past the option's range clamps to its last step.
    assert_eq!(delay_pos(6.5), 1.0);
    assert_eq!(Opt::ChatDelay.slider_steps(), Some(60));
    let mut s = Settings::default();
    set_slider(Opt::ChatDelay, &mut s, 0.0);
    assert_eq!(s.chat_delay, 0.0);
    set_slider(Opt::ChatDelay, &mut s, 0.5);
    assert_eq!(s.chat_delay, 3.0);
    set_slider(Opt::ChatDelay, &mut s, 1.0);
    assert_eq!(s.chat_delay, 6.0);
    set_slider(Opt::ChatDelay, &mut s, 0.53);
    assert_eq!(s.chat_delay, 3.2);
    assert_eq!(Settings::parse(&s.serialize()).chat_delay, 3.2);
}

/// `Options.chatDelay`'s message: None at zero, else the delay to one decimal.
#[test]
fn the_chat_delay_slider_says_its_delay() {
    let mut s = Settings::default();
    assert_eq!(chat_delay_label(&s).plain(), "options.chat.delay_none");
    s.chat_delay = 2.0;
    assert_eq!(chat_delay_label(&s).plain(), "Chat Delay: 2.0 second(s)");
    s.chat_delay = 6.0;
    assert_eq!(chat_delay_label(&s).plain(), "Chat Delay: 6.0 second(s)");
    s.chat_delay = 0.0;
    assert_eq!(chat_delay_label(&s).plain(), "options.chat.delay_none");
}

#[test]
fn the_graphics_preset_slider_uses_floor_stops() {
    assert_eq!(preset_from_slider(0.0), 0);
    assert_eq!(preset_from_slider(0.24), 0);
    assert_eq!(preset_from_slider(0.25), 1);
    assert_eq!(preset_from_slider(0.49), 1);
    assert_eq!(preset_from_slider(0.5), 2);
    assert_eq!(preset_from_slider(0.74), 2);
    assert_eq!(preset_from_slider(0.75), 3);
    assert_eq!(preset_from_slider(1.0), 3);
    let mut s = Settings::default();
    set_slider(Opt::Preset, &mut s, 0.2);
    assert_eq!(s.graphics_preset, "fast");
    set_slider(Opt::Preset, &mut s, 0.8);
    assert_eq!(s.graphics_preset, "custom");
}

/// Sunrise Settings' Resource Pack Options page opens a selected pack's Polytone options, as
/// the pack screen's button does, with that pack's entries; Done comes back to the page.
#[test]
fn resource_pack_options_open_a_selected_packs_options() {
    let atlas = atlas();
    let mut s = Settings {
        resource_packs: vec!["file/Birds".into()],
        ..Settings::default()
    };
    let mut ui = OptionsUi {
        size: (427.0, 240.0),
        ..Default::default()
    };
    let mut configs = sunrise_polytone::pack::Configs::default();
    configs.add_entry(
        "p:birds".into(),
        &serde_json::json!({"default_value": true}),
    );
    ui.pack_configs.insert("file/Birds".into(), configs);
    // Options, but not selected: not listed.
    ui.pack_configs.insert("file/Other".into(), {
        let mut c = sunrise_polytone::pack::Configs::default();
        c.add_entry("p:fish".into(), &serde_json::json!({"default_value": true}));
        c
    });
    ui.show(Page::Sunrise);
    ui.sunrise.show(Tab::PackOptions);
    let _ = ui.shell_mut();
    let rows = ui.sunrise.row_names(&s);
    assert_eq!(rows, ["file/Birds"], "untitled packs go by their id");
    let at = ui.sunrise.point_at(&s, ui.size, SunriseRow::PackOptions(0));
    ui.click(
        &mut s,
        &atlas,
        at.0,
        at.1,
        ui.size,
        [854, 480],
        false,
        false,
    );
    assert_eq!(ui.page(), Page::PackOptions);
    let entries = ui.pack_options.row_names(&s);
    assert!(
        entries
            .iter()
            .any(|name| name.to_lowercase().contains("birds")),
        "the pack's own entry: {entries:?}"
    );
    ui.back(&mut s);
    assert_eq!(ui.page(), Page::Sunrise);
    assert_eq!(ui.sunrise.open_page(), Some(Tab::PackOptions));
}
