//! Starling's tests of options_ui.rs, out of Sunrise's file (tools/starling/move_out.py).
#![allow(clippy::wildcard_imports)]

use super::*;

// Parked (27/11): Sunrise's Settings became one tabbed screen (`OptionsUi::sunrise`, 82deda2fda1); these two
// still drive the old `extras` and `graphics` screens and need rewriting against its tabs.
#[cfg(any())]
#[test]
fn a_finger_scrolls_from_a_cycle_button_and_drags_a_slider() {
    let s = Settings::default();
    let mut ui = OptionsUi::default();
    let size = (480.0, 600.0);
    ui.show(Page::Extras);
    // The Extras page is upstream's settings shell: its Sort switch is a row there.
    let on_sort = ui
        .extras
        .point_at(&s, size, ExtraRow::Item(crate::extras::Item::Sort));
    assert!(ui.list_button_at(&s, size, on_sort));
    ui.show(Page::Sound);
    let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
    let master = widgets
        .iter()
        .find(|w| {
            matches!(
                w.kind,
                WKind::Slider {
                    opt: Opt::Volume(0),
                    ..
                }
            )
        })
        .unwrap();
    let on_master = (master.x + 10.0, master.y + 10.0);
    assert!(!ui.list_button_at(&s, size, on_master));
}

// Parked (27/11): Sunrise's Settings became one tabbed screen (`OptionsUi::sunrise`, 82deda2fda1); these two
// still drive the old `extras` and `graphics` screens and need rewriting against its tabs.
#[cfg(any())]
#[test]
fn the_graphics_screens_wheel_scrolls_even_over_a_slider_being_changed() {
    use crate::graphics_ui::{H, Item};
    use winit::keyboard::KeyCode as K;
    let mut s = Settings::default();
    let mut ui = OptionsUi::default();
    ui.show(Page::Graphics);
    ui.graphics.show(crate::graphics_ui::Page::Performance);
    let size = (640.0, 360.0);
    s.sunrise.chunk_updates = 16;
    let chunk_updates = Item::Sunrise(H::ChunkUpdates);
    // Tab onto it: the arrows change it.
    while ui.graphics.focused_row() != Some(chunk_updates) {
        shell::Hosted::key(&mut ui.graphics, &mut s, K::Tab, None, false, size);
    }
    assert!(ui.graphics.changes_value());
    let before = ui.graphics.scroll_amount();
    ui.wheel(&mut s, size, (320.0, 150.0), [1920, 1080], -1.0);
    assert_eq!(s.sunrise.chunk_updates, 16);
    assert!(ui.graphics.scroll_amount() > before);
}

/// The game sets the calibration page's buttons once the page is open: a tap on Calibrate
/// now reaches the game, not the Done that stood there before them (the first phone test
/// went back to the graphics screen instead).
#[test]
fn a_calibration_button_set_after_the_page_opened_is_the_one_tapped() {
    let atlas = atlas();
    let size = (960.0, 540.0);
    let mut s = Settings::default();
    let mut ui = OptionsUi::default();
    ui.show(Page::PerfCalibration);
    // The layout as the page opened, before the game set its rows.
    let _ = ui.built_layout(&s, size.0, size.1);
    ui.set_calibrate(
        vec!["Not calibrated yet.".into()],
        vec!["Calibrate now".into()],
    );
    let layout = ui.built_layout(&s, size.0, size.1);
    let (widgets, ..) = layout.parts();
    let (x, y, w, h) = widgets
        .iter()
        .find(|w| matches!(&w.kind, WKind::Button { act, .. } if *act == Act::Calibrate(0)))
        .map(|w| (w.x, w.y, w.w, w.h))
        .expect("Calibrate now on the page");
    let events = ui.click(
        &mut s,
        &atlas,
        x + w / 2.0,
        y + h / 2.0,
        size,
        [854, 480],
        false,
        false,
    );
    assert!(events.contains(&Event::Calibrate(0)), "{events:?}");
    assert_eq!(ui.page(), Page::PerfCalibration);
}

/// The GUI sizes a landscape phone has, across its GUI scales.
const PHONE_GUI: [(f32, f32); 5] = [
    (427.0, 240.0),
    (533.0, 240.0),
    (600.0, 270.0),
    (800.0, 360.0),
    (1200.0, 540.0),
];

/// A pack as the pack screen lists it.
fn pack_info(id: &str) -> crate::pack_screen::PackInfo {
    crate::pack_screen::PackInfo {
        id: id.into(),
        title: Text::literal(id),
        description: Text::literal(""),
        description_style: crate::text::Style::default(),
        source: crate::pack_screen::Source::Default,
        compat: crate::pack_screen::Compat::Compatible,
        icon: None,
        required: false,
        position: crate::packs::Position::Top,
        fixed: false,
        hidden: false,
    }
}

/// Starling's Mods page has it in its footer; the shader packs page, upstream's settings shell,
/// as a row under Open Folder (`shader_ui::Item::Catalog`).
#[test]
fn the_mods_page_opens_the_catalog_on_its_own_shelf() {
    let atlas = atlas();
    let mut s = Settings::default();
    for size in PHONE_GUI {
        for (page, shelf) in [(Page::NativeGame, CatalogShelf::Mods)] {
            let mut ui = OptionsUi {
                catalog_buttons: true,
                ..OptionsUi::default()
            };
            ui.show(page);
            let (widgets, ..) = ui.layout(&s, size.0, size.1);
            let footer: Vec<&Widget> = widgets
                .iter()
                .filter(|w| !w.list && matches!(w.kind, WKind::Button { .. }))
                .collect();
            assert_eq!(footer.len(), 3, "{page:?} at {size:?}");
            let catalog = footer
                .iter()
                .find(|w| {
                    matches!(&w.kind, WKind::Button { act: Act::OpenCatalog(on), .. } if *on == shelf)
                })
                .expect("Catalog button");
            // On the screen, on the footer's row, and clear of its two buttons.
            assert!(catalog.x >= 0.0, "{page:?} at {size:?}");
            for other in &footer {
                assert_eq!(other.y, size.1 - 27.0);
                if std::ptr::eq(*other, *catalog) {
                    continue;
                }
                // Open Folder is left of it and Done right of it, with room between.
                let clear = if other.x < catalog.x {
                    other.x + other.w < catalog.x
                } else {
                    catalog.x + catalog.w < other.x
                };
                assert!(
                    clear,
                    "{page:?} at {size:?}: Catalog runs into the button at {}",
                    other.x
                );
                if other.x < catalog.x {
                    // Together they fill the 150 Open Folder had.
                    assert_eq!(catalog.x + catalog.w - other.x, 150.0);
                }
            }
            let (x, y) = (catalog.x + catalog.w / 2.0, catalog.y + catalog.h / 2.0);
            let events = ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false);
            assert_eq!(events, vec![Event::ButtonClick, Event::OpenCatalog(shelf)]);
        }
    }
}

#[test]
fn the_resource_packs_page_opens_the_catalog_on_its_shelf() {
    let atlas = atlas();
    let mut s = Settings::default();
    let size = (800.0, 360.0);
    let mut ui = OptionsUi {
        catalog_buttons: true,
        ..OptionsUi::default()
    };
    ui.show(Page::Packs);
    // The right half of Open Folder's slot: the footer starts at (800 - 308) / 2.
    let events = ui.click(&mut s, &atlas, 330.0, 344.0, size, [854, 480], false, false);
    assert_eq!(
        events,
        vec![
            Event::ButtonClick,
            Event::OpenCatalog(CatalogShelf::ResourcePacks)
        ]
    );
    assert_eq!(ui.page(), Page::Packs);
}

#[test]
fn without_the_offer_no_page_has_a_catalog_button() {
    let atlas = atlas();
    let mut s = Settings::default();
    let size = (800.0, 360.0);
    let mut ui = OptionsUi::default();
    for page in [Page::Mods, Page::ShaderPacks] {
        ui.show(page);
        let (widgets, ..) = ui.layout(&s, size.0, size.1);
        assert!(!widgets.iter().any(|w| matches!(
            w.kind,
            WKind::Button {
                act: Act::OpenCatalog(_),
                ..
            }
        )));
    }
    ui.show(Page::Packs);
    // Left of the footer, where the button used to be.
    let events = ui.click(&mut s, &atlas, 200.0, 344.0, size, [854, 480], false, false);
    assert_eq!(events, Vec::new());
}

#[test]
fn only_a_phone_takes_the_offer_of_the_catalog() {
    let mut ui = OptionsUi::default();
    ui.offer_catalog();
    assert_eq!(ui.catalog_buttons, cfg!(target_os = "android"));
}

#[test]
fn create_worlds_data_packs_have_no_catalog_button() {
    let atlas = atlas();
    let mut s = Settings::default();
    let dir = std::env::temp_dir().join(format!("sunrise-catalog-{}", std::process::id()));
    let mut ui = OptionsUi {
        catalog_buttons: true,
        ..OptionsUi::default()
    };
    ui.open_data_packs(dir.clone(), Vec::new(), dir, Vec::new());
    let events = ui.click(
        &mut s,
        &atlas,
        192.0,
        344.0,
        (800.0, 360.0),
        [854, 480],
        false,
        false,
    );
    assert_eq!(events, Vec::new());
}

/// The catalog installs with Options open: from the Mods page there was no reading the folder
/// again, as `pack_infos` is read when Options opens and the watcher runs on this page only.
#[test]
fn a_pack_installed_with_options_open_is_on_the_resource_packs_page() {
    let atlas = atlas();
    let mut ui = OptionsUi {
        pack_infos: vec![pack_info("file/old")],
        ..OptionsUi::default()
    };
    ui.show(Page::Mods);
    ui.update_pack_infos(vec![pack_info("file/old"), pack_info("file/new")]);
    ui.show(Page::Packs);
    let listed = &ui.packs(&atlas, (800.0, 360.0)).model.unselected;
    assert!(listed.iter().any(|p| p.id == "file/new"));
}

#[test]
fn a_pack_installed_over_the_page_keeps_what_was_chosen_on_it() {
    let atlas = atlas();
    let size = (800.0, 360.0);
    let mut ui = OptionsUi {
        pack_infos: vec![pack_info("file/a"), pack_info("file/b")],
        ..OptionsUi::default()
    };
    ui.show(Page::Packs);
    let screen = ui.packs(&atlas, size);
    screen.model.activate(false, 0, true);
    screen.changed = true;
    let chosen = screen.commit();
    assert_eq!(chosen.len(), 1);
    ui.update_pack_infos(vec![
        pack_info("file/a"),
        pack_info("file/b"),
        pack_info("file/new"),
    ]);
    let screen = ui.packs(&atlas, size);
    assert_eq!(screen.commit(), chosen);
    assert!(screen.changed, "Done still applies it");
    assert!(screen.model.unselected.iter().any(|p| p.id == "file/new"));
}

#[test]
fn a_resource_pack_installed_leaves_create_worlds_data_packs_alone() {
    let dir = std::env::temp_dir().join(format!("sunrise-catalog-{}", std::process::id()));
    let mut ui = OptionsUi::default();
    ui.open_data_packs(dir.clone(), Vec::new(), dir, Vec::new());
    let listed = ui.pack_infos.len();
    ui.update_pack_infos(vec![pack_info("file/new")]);
    assert_eq!(ui.pack_infos.len(), listed);
    assert!(!ui.pack_infos.iter().any(|p| p.id == "file/new"));
}

#[test]
fn the_native_game_page_says_which_game_runs_above_the_mods_and_its_buttons_act() {
    use crate::mod_list::{NativeGame, NativeGameAct};
    let atlas = atlas();
    let mut s = Settings::default();
    let mut ui = OptionsUi::default();
    let size = (480.0, 400.0);
    let now = "Now: Starling 0.8.0 from the APK, without native mods.";
    let old = "The build with Wrench was made for an earlier install of Starling, so it no \
               longer loads. Compile again.";
    s.mods.native_game = Some(NativeGame {
        lines: vec![(now.into(), false), (old.into(), true)],
        buttons: vec![
            ("Compile".into(), NativeGameAct::Compile),
            ("Use GitHub (slower)".into(), NativeGameAct::CompileOnGithub),
            (
                "Play without native mods".into(),
                NativeGameAct::PlayWithout,
            ),
        ],
    });
    s.mods.installed.push(crate::mod_list::Installed {
        id: "wrench".into(),
        title: "Wrench 0.1.0".into(),
        description: String::new(),
        problem: Some(crate::mod_list::NOT_BUILT.into()),
        dev: false,
        native: Some(crate::mod_list::NativePage::default()),
    });
    ui.show(Page::NativeGame);
    let (widgets, ..) = ui.layout(&s, size.0, size.1);
    // Each line in pieces that fit a button's width, red for the build that no longer loads.
    let lines = |color: u32| -> Vec<String> {
        widgets
            .iter()
            .filter_map(|w| match &w.kind {
                WKind::Text { text, color: c, .. } if *c == color => Some(text.plain()),
                _ => None,
            })
            .filter(|line| !line.is_empty())
            .collect()
    };
    let grey = lines(0xFFA0A0A0);
    let red = lines(rgba(255, 85, 85, 255));
    assert!(grey.len() > 1 && red.len() > 1, "{grey:?} {red:?}");
    for line in wrap_words(old, NOTE_CHARS) {
        assert!(
            line.chars().count() <= NOTE_CHARS && red.contains(&line),
            "{line}"
        );
    }
    assert!(grey.join(" ").contains(now), "{grey:?}");
    let old_words = old.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(red.join(" ").contains(&old_words), "{red:?}");
    let shown = |act: NativeGameAct| {
        widgets
            .iter()
            .find(|w| matches!(w.kind, WKind::Button { act: Act::NativeGame(a), .. } if a == act))
            .unwrap_or_else(|| panic!("{act:?}'s button"))
    };
    // Two to a row; the one left over takes the row.
    assert_eq!(shown(NativeGameAct::Compile).w, 150.0);
    assert_eq!(shown(NativeGameAct::CompileOnGithub).w, 150.0);
    let without = shown(NativeGameAct::PlayWithout);
    assert_eq!(without.w, 310.0);
    let (x, y) = (without.x + 1.0, without.y + 1.0);
    let events = ui.click(&mut s, &atlas, x, y, size, [480, 400], false, false);
    assert_eq!(
        events.last(),
        Some(&Event::NativeGame(NativeGameAct::PlayWithout))
    );
    // Above the mods. The mod is chosen and not in this game: its row is a ticked box, without
    // "(Not built)" after it, and beside it a warning that presses nothing (Compile is the
    // page's, at the top).
    // The row's toggle (where, how wide, its label) and the button beside it, if any.
    let row = |widgets: &[Widget]| {
        let toggle = widgets
            .iter()
            .find_map(|w| match &w.kind {
                WKind::Button {
                    label,
                    act: Act::ToggleMod(0),
                    ..
                } => Some(([w.x, w.y, w.w], label.plain())),
                _ => None,
            })
            .expect("the mod's row");
        let side = widgets.iter().find_map(|w| match &w.kind {
            WKind::Button {
                act: Act::ModSettings(0),
                ..
            } => Some([w.x, w.y, w.w]),
            _ => None,
        });
        (toggle, side)
    };
    let ((toggle, label), warning) = row(&widgets);
    assert!(toggle[1] > without.y);
    assert!(label.starts_with('\u{2611}'), "{label}");
    assert!(
        !label.contains(&tr("sunrise.mods.not_built").plain()),
        "{label}"
    );
    let warning = warning.expect("the warning beside the row");
    assert_eq!(
        (toggle[2], warning[2], warning[1]),
        (150.0, 150.0, toggle[1])
    );
    let (x, y) = (warning[0] + 1.0, warning[1] + 1.0);
    let events = ui.click(&mut s, &atlas, x, y, size, [480, 400], false, false);
    assert!(!events.contains(&Event::NativeSettings(0)), "{events:?}");
    // Not chosen, it has nothing to compile: the row is the box alone.
    let (x, y) = (toggle[0] + 1.0, toggle[1] + 1.0);
    ui.click(&mut s, &atlas, x, y, size, [480, 400], false, false);
    assert!(!s.mods.is_on(0));
    let (widgets, ..) = ui.layout(&s, size.0, size.1);
    let ((toggle, label), warning) = row(&widgets);
    assert!(label.starts_with('\u{2610}'), "{label}");
    assert!(warning.is_none());
    assert_eq!(toggle[2], 310.0);
}

#[test]
fn starlings_mods_page_switches_every_mod_and_keeps_dev_and_invalid_ones_inactive() {
    let atlas = atlas();
    let mut s = Settings::default();
    let mut ui = OptionsUi::default();
    let size = (480.0, 240.0);
    // The title's and the pause menu's Mods open it (a phone has no launcher).
    ui.open_at(false, Page::Mods);
    assert_eq!(ui.page(), Page::NativeGame);
    let empty = ui.layout(&s, size.0, size.1).0;
    assert!(empty.iter().any(|w| {
        matches!(&w.kind, WKind::Text { text, .. } if text.plain() == "sunrise.mods.empty")
    }));
    let open_folder = empty
        .iter()
        .find(|w| {
            matches!(
                &w.kind,
                WKind::Button {
                    act: Act::OpenModFolder,
                    ..
                }
            )
        })
        .expect("the Mods page has an Open Mod Folder button");
    let (x, y) = (open_folder.x + 1.0, open_folder.y + 1.0);
    assert_eq!(
        ui.click(&mut s, &atlas, x, y, size, [480, 240], false, false),
        vec![Event::ButtonClick, Event::OpenModFolder]
    );
    for (id, dev) in [("normal", false), ("dev", true), ("", false)] {
        s.mods.installed.push(crate::mod_list::Installed {
            id: id.into(),
            title: id.into(),
            description: "May show HUD panels.".into(),
            problem: id.is_empty().then(|| "Invalid package".into()),
            dev,
            native: None,
        });
    }
    let (widgets, ..) = ui.layout(&s, size.0, size.1);
    // No Details: upstream's Mods screen has no Compile, so the page doesn't open it.
    assert!(!widgets.iter().any(|w| matches!(
        w.kind,
        WKind::Button {
            act: Act::Push(Page::Mods),
            ..
        }
    )));
    let rows: Vec<(f32, f32)> = widgets
        .iter()
        .filter(|w| {
            matches!(
                w.kind,
                WKind::Button {
                    act: Act::ToggleMod(_),
                    ..
                }
            )
        })
        .map(|w| (w.x + 1.0, w.y + 1.0))
        .collect();
    assert_eq!(rows.len(), 3);
    for (i, &(x, y)) in rows.iter().enumerate() {
        let events = ui.click(&mut s, &atlas, x, y, size, [480, 240], false, false);
        if i == 0 {
            assert!(events.contains(&Event::Changed));
            assert!(events.contains(&Event::ModToggled(0)));
        } else {
            assert!(events.is_empty(), "{events:?}");
        }
    }
    // A mod that is not native starts off: the click turned it on. A dev mod is always on,
    // the invalid one never.
    assert!(s.mods.is_on(0));
    assert!(s.mods.is_on(1));
    assert!(!s.mods.is_on(2));
}

#[test]
fn custom_buttons_take_a_line_or_a_key_and_empty_ones_go() {
    use crate::settings::CustomAction;
    let atlas = atlas();
    let mut s = Settings::default();
    let mut ui = OptionsUi {
        stack: vec![Page::Main, Page::CustomButtons],
        ..Default::default()
    };
    let size = (427.0, 480.0);
    let press = |ui: &mut OptionsUi, s: &mut Settings, wanted: Act| {
        let (widgets, ..) = ui.layout(s, size.0, size.1);
        let at = widgets
            .iter()
            .find_map(|w| match &w.kind {
                WKind::Button { act, .. } if *act == wanted => Some((w.x + 2.0, w.y + 2.0)),
                _ => None,
            })
            .expect("the button shows");
        ui.click(s, &atlas, at.0, at.1, size, [1281, 960], false, false)
    };
    // Added and left empty, it goes.
    press(&mut ui, &mut s, Act::CustomAdd);
    assert_eq!(ui.page(), Page::CustomButton(0));
    ui.layout(&s, size.0, size.1);
    assert!(ui.back(&mut s).contains(&Event::CustomButtons));
    assert!(s.custom_buttons.is_empty());
    // What the box holds, it sends.
    press(&mut ui, &mut s, Act::CustomAdd);
    ui.layout(&s, size.0, size.1);
    let field = (ui.custom_send.x + 10.0, ui.custom_send.y + 4.0);
    ui.click(
        &mut s,
        &atlas,
        field.0,
        field.1,
        size,
        [1281, 960],
        false,
        false,
    );
    assert!(ui.text_focused() && ui.text_field_at(field.0, field.1));
    ui.type_text(&atlas, "/home");
    ui.back(&mut s);
    assert_eq!(
        s.custom_buttons[0].action,
        CustomAction::Send("/home".into())
    );
    assert_eq!(s.custom_buttons[0].label(), "/home");
    // A key takes its place, and stays without a line.
    let id = s.custom_buttons[0].id;
    press(&mut ui, &mut s, Act::CustomEdit(id));
    assert_eq!(ui.custom_send.value, "/home");
    ui.push(Page::CustomPick(id, CustomPick::Control));
    press(&mut ui, &mut s, Act::CustomKey(id, "key.sprint"));
    assert_eq!(ui.page(), Page::CustomButton(id));
    assert_eq!(
        s.custom_buttons[0].action,
        CustomAction::Key("key.sprint".into())
    );
    // Any key of the keyboard, F1 first among them.
    ui.push(Page::CustomPick(id, CustomPick::Keyboard));
    press(&mut ui, &mut s, Act::CustomInput(id, "key.keyboard.f1"));
    assert_eq!(
        s.custom_buttons[0].action,
        CustomAction::Input("key.keyboard.f1".into())
    );
    assert_eq!(
        s.custom_buttons[0].label(),
        keybinds::display_name("key.keyboard.f1").plain()
    );
    // An icon, and back to none.
    ui.push(Page::CustomPick(id, CustomPick::Icon));
    press(&mut ui, &mut s, Act::CustomIcon(id, Some("star")));
    assert_eq!(s.custom_buttons[0].icon.as_deref(), Some("star"));
    ui.push(Page::CustomPick(id, CustomPick::Icon));
    press(&mut ui, &mut s, Act::CustomIcon(id, None));
    assert_eq!(s.custom_buttons[0].icon, None);
    // Its own image: a stroke across the canvas, with the swatch picked first, is kept by
    // Done; a canvas nobody touched changes nothing.
    press(&mut ui, &mut s, Act::CustomDraw(id));
    assert!(ui.painting());
    assert!(
        !ui.back(&mut s)
            .iter()
            .any(|e| matches!(e, Event::CustomImage(..)))
    );
    assert!(!s.custom_buttons[0].image);
    press(&mut ui, &mut s, Act::CustomDraw(id));
    ui.layout(&s, size.0, size.1);
    let [left, top, cell] = ui.painter.at;
    let (sx, sy) = ui.painter.swatch(4);
    let tap = |ui: &mut OptionsUi, s: &mut Settings, x: f32, y: f32, drag: bool| {
        ui.click(s, &atlas, x, y, size, [1281, 960], drag, false)
    };
    tap(&mut ui, &mut s, sx + 2.0, sy + 2.0, false);
    assert_eq!(ui.painter.color, 4);
    let at = |c: f32| c * cell + cell / 2.0;
    tap(&mut ui, &mut s, left + at(1.0), top + at(1.0), false);
    // Fast: the pixels between are painted too.
    tap(&mut ui, &mut s, left + at(9.0), top + at(1.0), true);
    let pixel = |ui: &OptionsUi, x: usize, y: usize| {
        let at = (y * own_buttons::SIZE as usize + x) * 4;
        [ui.painter.pixels[at], ui.painter.pixels[at + 3]]
    };
    assert_eq!(pixel(&ui, 5, 1), [PALETTE[4][0], 255]);
    let kept = ui.back(&mut s);
    assert!(
        kept.iter()
            .any(|e| matches!(e, Event::CustomImage(i, p) if *i == id && !p.is_empty()))
    );
    assert!(s.custom_buttons[0].image);
    // An icon picked after it takes its place.
    ui.push(Page::CustomPick(id, CustomPick::Icon));
    press(&mut ui, &mut s, Act::CustomIcon(id, Some("star")));
    assert!(!s.custom_buttons[0].image);
    ui.back(&mut s);
    assert_eq!(
        s.custom_buttons[0].action,
        CustomAction::Input("key.keyboard.f1".into())
    );
    // And it can be taken away.
    assert!(press(&mut ui, &mut s, Act::CustomDelete(id)).contains(&Event::CustomButtons));
    assert!(s.custom_buttons.is_empty());
}

#[test]
fn a_selected_profile_prevents_the_legacy_skin_page_from_changing_its_name() {
    let atlas = atlas();
    let mut settings = Settings::default();
    let mut ui = OptionsUi {
        skin_page: true,
        profile_name: Some("TabletAlpha".into()),
        ..Default::default()
    };
    ui.stack.push(Page::Skin);
    ui.skin.saved = "OldAlias".into();
    ui.skin.name.set_value("OldAlias");
    ui.skin.name.focused = true;
    assert!(!ui.text_focused());
    assert!(!ui.text_field_at(ui.skin.name.x, ui.skin.name.y));
    ui.type_text(&atlas, "X");
    ui.key(
        &mut settings,
        &atlas,
        winit::keyboard::KeyCode::KeyA,
        Some("X"),
    );
    ui.commit_name();
    assert_eq!(ui.skin.name.value, "OldAlias");
    assert!(ui.take_skin_actions().is_empty());
}

#[test]
fn dragging_the_wardrobe_preview_keeps_the_pointer_when_it_crosses_the_list() {
    let atlas = atlas();
    let mut settings = Settings::default();
    let mut ui = OptionsUi {
        skin_page: true,
        wardrobe_enabled: true,
        stack: vec![Page::Skin],
        ..Default::default()
    };
    let size = (490.0, 260.0);
    let [left, top, right, _] = ui.wardrobe_preview_rect(size.0, size.1).unwrap();
    let start = left + 20.0;
    let end = right + 40.0;
    let y = top + 20.0;
    let click = |ui: &mut OptionsUi, s: &mut Settings, x, drag| {
        ui.click(s, &atlas, x, y, size, [1470, 780], drag, false)
    };
    assert_eq!(ui.wardrobe_rotation(), 22.5);
    assert!(click(&mut ui, &mut settings, start, false).is_empty());
    assert!(click(&mut ui, &mut settings, end, true).is_empty());
    let turned = 22.5 + (-(end - start) * 2.5_f32).rem_euclid(360.0);
    assert_eq!(ui.wardrobe_rotation(), turned);
    assert_eq!(ui.scroll, 0.0);
    ui.release();
    click(&mut ui, &mut settings, end + 20.0, true);
    assert_eq!(ui.wardrobe_rotation(), turned);
    assert!(ui.take_skin_actions().is_empty());

    ui.toggle_wardrobe_auto_rotate();
    assert!(ui.skin.auto_rotate_since.is_some());
    ui.toggle_wardrobe_auto_rotate();
    assert!(ui.skin.auto_rotate_since.is_none());
}

#[test]
fn wardrobe_confirms_only_the_last_cloud_choices() {
    let mut s = Settings::default();
    let cape = |id: &str, active| AccountCape {
        id: id.into(),
        name: id.into(),
        url: format!("https://textures.minecraft.net/texture/{id}"),
        active,
    };
    let mut ui = OptionsUi {
        wardrobe_enabled: true,
        skin_page: true,
        skin_cloud: crate::own_skin::Cloud::Microsoft(crate::own_skin::Upload::None),
        wardrobe_account_capes: Some(vec![cape("old", true), cape("new", false)]),
        stack: vec![Page::Skin],
        ..Default::default()
    };
    ui.skin.choice.file = Some("skin.png".into());
    ui.select_wardrobe_tab(true);
    let (widgets, ..) = ui.layout(&s, 490.0, 260.0);
    assert!(widgets.iter().any(|widget| matches!(
        &widget.kind,
        WKind::Button {
            act: Act::OpenWardrobeFolder,
            active: true,
            ..
        }
    )));
    ui.stage_account_cape(Some("new".into()));
    assert_eq!(ui.pending_account_cape().flatten().unwrap().id, "new");
    ui.stage_account_cape(Some("old".into()));
    assert!(ui.pending_account_cape().is_none());
    ui.stage_account_cape(Some("new".into()));
    ui.stage_account_cape(None);
    assert!(matches!(ui.pending_account_cape(), Some(None)));
    ui.queue_skin_upload();
    assert!(ui.take_skin_actions().is_empty());
    ui.done(&mut s);
    let request = SkinAction::SaveWardrobe {
        cape: Some(None),
        skin: Some(ui.skin.choice.clone()),
    };
    assert_eq!(ui.take_skin_actions(), vec![request.clone()]);
    assert_eq!(ui.page(), Page::Skin);
    assert!(ui.wardrobe_saving());
    let (widgets, ..) = ui.layout(&s, 490.0, 260.0);
    let contains = |key| {
        widgets.iter().any(|widget| {
            matches!(&widget.kind,
                WKind::Text { text, .. } if text == &tr(key)
            )
        })
    };
    assert!(contains("starling.wardrobe.syncing"));
    assert!(contains("starling.wardrobe.syncing_cape"));
    assert!(!widgets.iter().any(|widget| matches!(
        &widget.kind,
        WKind::Button {
            act: Act::ActivateCape(_),
            ..
        }
    )));
    ui.done(&mut s);
    ui.back(&mut s);
    assert_eq!(ui.page(), Page::Skin);
    assert!(ui.take_skin_actions().is_empty());

    ui.finish_wardrobe_save(&mut s, Err("Network error".into()));
    assert_eq!(ui.page(), Page::Skin);
    assert_eq!(ui.skin.warning.as_deref(), Some("Network error"));
    let (widgets, ..) = ui.layout(&s, 490.0, 260.0);
    for action in [Act::RetryWardrobeSync, Act::Done] {
        assert!(widgets.iter().any(|widget| matches!(
            &widget.kind,
            WKind::Button { act, active: true, .. } if act == &action
        )));
    }
    ui.start_wardrobe_sync();
    assert_eq!(ui.take_skin_actions(), vec![request]);
    ui.wardrobe_cape_saved();
    ui.finish_wardrobe_save(&mut s, Err("Upload failed".into()));
    ui.start_wardrobe_sync();
    assert_eq!(
        ui.take_skin_actions(),
        vec![SkinAction::SaveWardrobe {
            cape: None,
            skin: Some(ui.skin.choice.clone())
        }]
    );
    ui.finish_wardrobe_save(&mut s, Ok(()));
    assert_ne!(ui.page(), Page::Skin);

    ui.stack.push(Page::Skin);
    ui.stage_account_cape(Some("new".into()));
    ui.queue_skin_upload();
    ui.done(&mut s);
    assert_eq!(ui.take_skin_actions().len(), 1);
    ui.finish_wardrobe_save(&mut s, Err("Network error".into()));
    ui.done(&mut s);
    assert_ne!(ui.page(), Page::Skin);
    assert!(ui.take_skin_actions().is_empty());
    assert!(ui.pending_account_cape().is_none() && !ui.pending_skin_upload);
}

#[test]
fn the_skin_page_takes_a_name_and_selects_saved_skins() {
    use winit::keyboard::KeyCode;
    let atlas = atlas();
    let mut s = Settings::default();
    // The game's own options; on the stack without `push`, which reads the page from beside
    // options.txt.
    let mut ui = OptionsUi {
        skin_page: true,
        ..Default::default()
    };
    ui.stack.push(Page::Skin);
    ui.skin.saved = "Player00042".into();
    ui.skin.name.set_value("Player00042");
    ui.skin.files = vec!["alex.png".into(), "steve.png".into()];
    let size = (427.0, 240.0);
    let click = |ui: &mut OptionsUi, s: &mut Settings, (x, y): (f32, f32)| {
        ui.click(s, &atlas, x, y, size, [1281, 720], false, false)
    };
    let find = |ui: &mut OptionsUi, wanted: Act| {
        let (widgets, ..) = ui.layout(&Settings::default(), size.0, size.1);
        widgets
            .iter()
            .find_map(|w| match &w.kind {
                WKind::Button { act, active, .. } if *act == wanted => {
                    Some(((w.x + 2.0, w.y + 2.0), *active))
                }
                _ => None,
            })
            .unwrap()
    };
    // The default skin's model is the default's.
    assert!(!find(&mut ui, Act::SkinModel).1);
    // At the box's right end: the cursor after the text.
    let field = (ui.skin.name.x + 260.0, ui.skin.name.y + 4.0);
    assert!(click(&mut ui, &mut s, field).is_empty());
    assert!(ui.text_focused() && ui.text_field_at(field.0, field.1));
    let retype = |ui: &mut OptionsUi, s: &mut Settings, text: &str| {
        for _ in 0..16 {
            ui.key(s, &atlas, KeyCode::Backspace, None);
        }
        for c in text.chars() {
            ui.key(s, &atlas, KeyCode::KeyA, Some(&c.to_string()));
        }
        ui.key(s, &atlas, KeyCode::Enter, None)
    };
    // Refused: the old name back, a note.
    retype(&mut ui, &mut s, "no");
    assert!(ui.take_skin_actions().is_empty());
    assert_eq!(ui.skin.name.value, "Player00042");
    assert!(!ui.text_focused());
    assert!(ui.skin.warning.as_deref().unwrap().contains("Player00042"));
    click(&mut ui, &mut s, field);
    retype(&mut ui, &mut s, "Steve_2");
    assert_eq!(ui.take_skin_actions(), [SkinAction::Name("Steve_2".into())]);
    assert_eq!(ui.skin.warning, None);
    // Unchanged, nothing to keep; leaving the box by a press elsewhere keeps a new one.
    click(&mut ui, &mut s, field);
    ui.key(&mut s, &atlas, KeyCode::Enter, None);
    assert!(ui.take_skin_actions().is_empty());
    click(&mut ui, &mut s, field);
    ui.type_text(&atlas, "X");
    click(&mut ui, &mut s, (1.0, 1.0));
    assert_eq!(
        ui.take_skin_actions(),
        [SkinAction::Name("Steve_2X".into())]
    );
    // Any saved skin can be chosen directly, and the account's skin can be restored.
    let steve = find(&mut ui, Act::SelectSkin(Some("steve.png".into()))).0;
    let alex = find(&mut ui, Act::SelectSkin(Some("alex.png".into()))).0;
    let default = find(&mut ui, Act::SelectSkin(None)).0;
    let mut choice_after = |ui: &mut OptionsUi, at: (f32, f32)| {
        assert!(click(ui, &mut s, at).contains(&Event::ButtonClick));
        match ui.take_skin_actions().as_slice() {
            [SkinAction::Choice(choice)] => choice.clone(),
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(
        choice_after(&mut ui, steve).file.as_deref(),
        Some("steve.png")
    );
    let (model, active) = find(&mut ui, Act::SkinModel);
    assert!(active);
    assert!(choice_after(&mut ui, model).slim);
    assert_eq!(
        choice_after(&mut ui, alex).file.as_deref(),
        Some("alex.png")
    );
    assert_eq!(choice_after(&mut ui, default).file, None);
    let (folder, active) = find(&mut ui, Act::OpenSkinFolder);
    assert!(active);
    click(&mut ui, &mut s, folder);
    assert_eq!(ui.take_skin_actions(), [SkinAction::OpenFolder]);
    // The layers and Main Hand follow the skin.
    find(&mut ui, Act::Cycle(Opt::MainHand));
    // The menus before the game open the page too, with only the layers and Main Hand: the
    // name and skin are the game's own options'.
    let mut menu = OptionsUi::default();
    let (main, ..) = menu.layout(&s, size.0, size.1);
    assert!(main.iter().any(|w| matches!(
        &w.kind,
        WKind::Button {
            act: Act::Push(Page::Skin),
            active: true,
            ..
        }
    )));
    menu.stack.push(Page::Skin);
    let (widgets, ..) = menu.layout(&s, size.0, size.1);
    let acts: Vec<&Act> = widgets
        .iter()
        .filter_map(|w| match &w.kind {
            WKind::Button { act, .. } => Some(act),
            _ => None,
        })
        .collect();
    assert!(acts.contains(&&Act::Cycle(Opt::ModelPart(0))));
    assert!(!acts.contains(&&Act::SelectSkin(None)) && !acts.contains(&&Act::OpenSkinFolder));
}

#[test]
fn an_account_profile_sends_its_skin_where_its_account_keeps_it() {
    use crate::own_skin::{Cloud, SkinChoice, Upload};
    let atlas = atlas();
    let mut s = Settings::default();
    let mut ui = OptionsUi {
        skin_page: true,
        profile_name: Some("Notch".into()),
        skin_cloud: Cloud::Microsoft(Upload::None),
        ..Default::default()
    };
    ui.stack.push(Page::Skin);
    ui.skin.files = vec!["alex.png".into()];
    let size = (427.0, 240.0);
    let buttons = |ui: &mut OptionsUi| {
        let (widgets, ..) = ui.layout(&Settings::default(), size.0, size.1);
        widgets
            .iter()
            .filter_map(|w| match &w.kind {
                WKind::Button { act, active, .. } => {
                    Some((act.clone(), (w.x + 2.0, w.y + 2.0), *active))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let find = |ui: &mut OptionsUi, wanted: Act| {
        buttons(ui)
            .into_iter()
            .find(|(act, ..)| *act == wanted)
            .map(|(_, at, active)| (at, active))
    };
    // The account's own skin is nothing to upload.
    assert_eq!(find(&mut ui, Act::UploadSkin).map(|b| b.1), Some(false));
    ui.skin.choice = SkinChoice {
        file: Some("alex.png".into()),
        slim: true,
    };
    let (at, active) = find(&mut ui, Act::UploadSkin).unwrap();
    assert!(active);
    ui.click(&mut s, &atlas, at.0, at.1, size, [1281, 720], false, false);
    assert_eq!(
        ui.take_skin_actions(),
        [SkinAction::Upload(ui.skin.choice.clone())]
    );
    // One upload at a time; a refusal shows in red.
    ui.skin_cloud = Cloud::Microsoft(Upload::Sending);
    assert_eq!(find(&mut ui, Act::UploadSkin).map(|b| b.1), Some(false));
    ui.skin_cloud = Cloud::Microsoft(Upload::Failed("Mojang said no".into()));
    assert!(
        ui.list_rows(&s)
            .iter()
            .any(|r| matches!(r, Row::Warning(w) if w == "Mojang said no"))
    );
    // Ely.by takes no uploads: its page instead.
    ui.skin_cloud = Cloud::Ely;
    assert_eq!(find(&mut ui, Act::UploadSkin), None);
    let (at, active) = find(&mut ui, Act::ElySkinSite).unwrap();
    assert!(active);
    ui.click(&mut s, &atlas, at.0, at.1, size, [1281, 720], false, false);
    assert_eq!(ui.take_skin_actions(), [SkinAction::ElySite]);
    // An offline profile's skin stays in this game.
    ui.skin_cloud = Cloud::Local;
    let acts: Vec<Act> = buttons(&mut ui).into_iter().map(|b| b.0).collect();
    assert!(!acts.contains(&Act::UploadSkin) && !acts.contains(&Act::ElySkinSite));
}
