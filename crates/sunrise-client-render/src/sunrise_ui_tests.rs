use super::*;
use crate::{
    graphics_ui::{H, Item, Page as G},
    options_ui::Event,
    settings::Opt,
    settings_shell::{Hosted, Shell, Target},
};

const SIZE: (f32, f32) = (854.0, 480.0);

fn sheet_of(s: &Settings, tab: Tab) -> Sheet<Tab, SunriseRow> {
    let sheets = SunriseScreen::default().sheets(s);
    sheets.into_iter().find(|sheet| sheet.id == tab).unwrap()
}

/// Its pages, in order: the graphics screen's, Resource Pack Options after Shaders, then
/// Gameplay, Tools and Extras.
#[test]
fn the_pages_are_graphics_then_pack_options_gameplay_tools_and_extras() {
    let s = Settings::default();
    let ids: Vec<Tab> = SunriseScreen::default()
        .sheets(&s)
        .iter()
        .map(|sheet| sheet.id)
        .collect();
    let want = [
        Tab::Graphics(G::General),
        Tab::Graphics(G::Quality),
        Tab::Graphics(G::Performance),
        Tab::Graphics(G::Shaders),
        Tab::PackOptions,
        Tab::Graphics(G::Advanced),
        Tab::Graphics(G::Hdr),
        Tab::Gameplay,
        Tab::Tools,
        Tab::Extras,
    ];
    assert_eq!(ids, want);
}

/// General has Sunrise's preset first and no vanilla preset, and Brightness is under Display.
#[test]
fn general_keeps_only_sunrises_preset_and_brightness_is_a_display_setting() {
    let s = Settings::default();
    let general = sheet_of(&s, Tab::Graphics(G::General));
    let names: Vec<&str> = general.groups.iter().map(|g| g.name).collect();
    assert_eq!(names, ["Preset", "Distance", "Display", "Interface"]);
    let sunrise_preset = SunriseRow::Graphics(Item::Sunrise(H::Preset));
    assert_eq!(
        general.groups[0]
            .rows
            .iter()
            .map(|r| r.id)
            .collect::<Vec<_>>(),
        [sunrise_preset]
    );
    let rows = |name: &str| -> Vec<SunriseRow> {
        let group = general.groups.iter().find(|g| g.name == name).unwrap();
        group.rows.iter().map(|r| r.id).collect()
    };
    let gamma = SunriseRow::Graphics(Item::Vanilla(Opt::Gamma));
    assert!(rows("Display").contains(&gamma));
    assert!(!rows("Distance").contains(&gamma));
    let all = SunriseScreen::default().sheets(&s);
    let ids = all.iter().flat_map(|sheet| &sheet.groups);
    let ids: Vec<SunriseRow> = ids.flat_map(|g| &g.rows).map(|r| r.id).collect();
    // Vanilla's preset stays on vanilla's Video Settings only.
    assert!(!ids.contains(&SunriseRow::Graphics(Item::Vanilla(Opt::Preset))));
}

/// A page of one group shows no group name: its first row is at the top of the list. A page of
/// more keeps theirs.
#[test]
fn a_page_of_one_group_shows_no_group_name() {
    let s = Settings::default();
    let mut shell = Shell::<SunriseScreen>::default();
    for tab in [Tab::PackOptions, Tab::Tools, Tab::Extras] {
        shell.show(tab);
        let (lines, _) = shell.lines_at(&s, SIZE);
        assert_eq!(lines[0].group.map(|g| g.0), Some(""), "{tab:?}");
        assert_eq!(lines[0].h(), 0.0, "{tab:?}");
        assert_eq!((lines[1].y, lines[1].setting.is_some()), (0.0, true));
    }
    shell.show(Tab::Graphics(G::General));
    let (lines, _) = shell.lines_at(&s, SIZE);
    assert_eq!(lines[0].group.map(|g| g.0), Some("Preset"));
    assert!(lines[1].y > 0.0);
}

/// Tools opens Replay Studio and the AI Agent page, whatever No AI is.
#[test]
fn tools_open_replay_studio_and_the_ai_agent_page_with_no_ai_on_too() {
    let mut s = Settings::default();
    for no_ai in [false, true] {
        s.extras.no_ai = no_ai;
        let mut shell = Shell::<SunriseScreen>::default();
        shell.show(Tab::Tools);
        let rows: Vec<SunriseRow> = (shell.listed(&s, SIZE).into_iter())
            .map(|(_, row)| row)
            .collect();
        let want = [
            SunriseRow::Open(Page::Studio),
            SunriseRow::Open(Page::Agent),
        ];
        assert_eq!(rows, want, "No AI {no_ai}");
        let at = shell.point_at(&s, SIZE, SunriseRow::Open(Page::Agent));
        let outputs = shell.click(&mut s, at, SIZE, false, false, false);
        assert!(outputs.contains(&Output::Push(Page::Agent)), "{outputs:?}");
    }
}

/// Resource Pack Options lists the selected packs that have options, the topmost first, and
/// a press asks for that pack's options; with none, a note says so.
#[test]
fn pack_options_list_the_selected_packs_with_options() {
    let mut s = Settings::default();
    let mut shell = Shell::<SunriseScreen>::default();
    shell.show(Tab::PackOptions);
    let none = lang::get(NO_PACK_OPTIONS);
    assert_eq!(
        sheet_of_screen(&shell.screen, &s),
        [(SunriseRow::NoPackOptions, none.clone())]
    );
    shell.screen.packs_with_options = vec![
        ("file/A".into(), "Alpha".into()),
        ("file/B".into(), "Beta".into()),
        ("file/C".into(), "Gamma".into()),
    ];
    // Not selected: not listed.
    assert_eq!(
        sheet_of_screen(&shell.screen, &s),
        [(SunriseRow::NoPackOptions, none)]
    );
    // `resourcePacks` lists the bottom pack first; the list shows the top one first.
    s.resource_packs = vec!["file/C".into(), "vanilla".into(), "file/A".into()];
    assert_eq!(
        sheet_of_screen(&shell.screen, &s),
        [
            (SunriseRow::PackOptions(0), "Alpha".to_owned()),
            (SunriseRow::PackOptions(1), "Gamma".to_owned()),
        ]
    );
    let at = shell.point_at(&s, SIZE, SunriseRow::PackOptions(1));
    let outputs = shell.click(&mut s, at, SIZE, false, false, false);
    assert!(
        outputs.contains(&Output::Push(Page::PackOptions)),
        "{outputs:?}"
    );
    assert_eq!(shell.screen.take_opening().as_deref(), Some("file/C"));
    assert_eq!(shell.screen.take_opening(), None);
}

fn sheet_of_screen(screen: &SunriseScreen, s: &Settings) -> Vec<(SunriseRow, String)> {
    let sheets = screen.sheets(s);
    let sheet = sheets
        .into_iter()
        .find(|sheet| sheet.id == Tab::PackOptions);
    let rows = sheet.into_iter().flat_map(|sheet| sheet.groups);
    let rows = rows.flat_map(|g| g.rows);
    rows.map(|r| (r.id, r.name)).collect()
}

/// A graphics setting changed shows Undo, which puts it back; an extra changed on Gameplay
/// leaves none, as the Extras page had none.
#[test]
fn undo_follows_graphics_changes_and_leaves_gameplay_alone() {
    let mut s = Settings::default();
    let mut shell = Shell::<SunriseScreen>::default();
    shell.show(Tab::Gameplay);
    let sort = SunriseRow::Extra(ExtraRow::Item(ExtraItem::Sort));
    let at = shell.point_at(&s, SIZE, sort);
    assert!(
        !shell
            .click(&mut s, at, SIZE, false, false, false)
            .is_empty()
    );
    assert!(s.extras.sort);
    assert!(!shell.changed(), "no Undo for an extra");
    shell.show(Tab::Graphics(G::Quality));
    let sky = SunriseRow::Graphics(Item::Sunrise(H::Sky));
    let at = shell.point_at(&s, SIZE, sky);
    assert!(
        !shell
            .click(&mut s, at, SIZE, false, false, false)
            .is_empty()
    );
    assert!(!s.sunrise.sky);
    assert!(shell.changed());
    let undo = shell.foot_at(&s, SIZE);
    let (_, [x, y, w, h]) = undo.into_iter().find(|(t, _)| *t == Target::Undo).unwrap();
    shell.click(
        &mut s,
        (x + w / 2.0, y + h / 2.0),
        SIZE,
        false,
        false,
        false,
    );
    assert!(s.sunrise.sky, "the graphics change is put back");
    assert!(s.extras.sort, "the extra stays");
}

/// The shader pack key is on Shaders, not Gameplay; its row waits for a key, without Undo.
#[test]
fn the_shader_pack_key_is_on_shaders_and_waits_for_a_key() {
    use crate::settings_shell::Capture;
    let mut s = Settings::default();
    let key = SunriseRow::Extra(ExtraRow::Item(ExtraItem::ShaderPackKey));
    let on = |tab| {
        let sheet = sheet_of(&Settings::default(), tab);
        let mut rows = sheet.groups.into_iter().flat_map(|g| g.rows);
        rows.any(|r| r.id == key)
    };
    assert!(on(Tab::Graphics(G::Shaders)));
    assert!(!on(Tab::Gameplay));
    let mut shell = Shell::<SunriseScreen>::default();
    shell.show(Tab::Graphics(G::Shaders));
    let at = shell.point_at(&s, SIZE, key);
    assert_eq!(
        shell.click(&mut s, at, SIZE, false, false, false),
        vec![
            Output::Event(Event::ButtonClick),
            Output::Bind(Capture::Key(crate::extras::SHADER_PACK_KEY))
        ]
    );
    assert!(!shell.changed());
    // The wheel leaves the binding be.
    let bound = s.extras.shader_pack_key.clone();
    shell.screen.wheel_step(&mut s, key, true);
    assert_eq!(s.extras.shader_pack_key, bound);
}

/// On Gameplay, the first extra's row is the plate's topmost target the frame declares (where a
/// controller's focus goes across from the search field); the keys go down from the search field
/// to the page list, and Tab steps past every page to that row.
#[test]
fn the_keys_and_a_controller_go_from_the_search_field_to_gameplays_first_row() {
    use crate::settings_shell::Columns;
    use winit::keyboard::KeyCode as K;
    let atlas = crate::hud::HudAtlas::build(&crate::resource_source::ResourceSource::directory(
        "/nonexistent".into(),
    ))
    .unwrap();
    let mut s = Settings::default();
    let mut shell = Shell::<SunriseScreen>::default();
    shell.show(Tab::Gameplay);
    let mut batch = crate::hud::HudBatch::scratch([1708, 960]);
    shell.draw_test_frame(
        &s,
        &mut batch,
        &atlas,
        (0.0, 0.0),
        false,
        None,
        &mut |_, _| {},
    );
    crate::widgets::end_frame(crate::widgets::generation() + 1);
    let size = (batch.width, batch.height);
    let first = SunriseRow::Extra(ExtraRow::Item(ExtraItem::OwnNameTag));
    let at = shell.point_at(&s, size, first);
    // A controller: the plate's topmost target the frame declared.
    let cols = Columns::of(size.0, size.1);
    let targets = crate::widgets::last_targets();
    let right = targets
        .iter()
        .filter(|t| t.rect[0] >= cols.list_x - 1.0)
        .min_by(|a, b| a.rect[1].total_cmp(&b.rect[1]))
        .unwrap();
    let [x, y, w, h] = right.rect;
    assert!(crate::ui::hovered(at, x, y, w, h), "{:?}", right.rect);
    // Down from the search field is the page list's first page in view.
    let top = shell.page_places()[0].0;
    shell.set_focus(Some(Target::Search));
    shell.key(&mut s, K::ArrowDown, None, false, size);
    assert_eq!(shell.focused(), Some(Target::Page(top)));
    // Tab from the search field steps through the pages from the first in view (the list
    // scrolled to Gameplay), on past the list's end, then to the first row.
    shell.set_focus(Some(Target::Search));
    let mut steps = 0;
    while shell.focused_row().is_none() && steps < TABS.len() + 2 {
        shell.key(&mut s, K::Tab, None, false, size);
        steps += 1;
    }
    assert_eq!(shell.focused_row(), Some(first));
    let from = TABS.iter().position(|&tab| tab == top).unwrap();
    assert_eq!(steps, TABS.len() - from + 1, "every page on, then the row");
}

fn pack(file: &str) -> crate::shader_pack_list::Entry {
    crate::shader_pack_list::Entry { file: file.into() }
}

fn undo_shows(shell: &mut Shell<SunriseScreen>, s: &Settings) -> bool {
    let foot = shell.foot_at(s, SIZE);
    foot.iter().any(|(target, _)| *target == Target::Undo)
}

/// A pack picked shows Undo; picked again (shaders off, the pick remembered, as the screen
/// opened) it goes, as on the pack list.
#[test]
fn a_pack_picked_and_unpicked_leaves_no_undo() {
    let mut s = Settings {
        shader_packs: vec![pack("A.zip")],
        ..Settings::default()
    };
    s.extras.last_shader_pack = "A.zip".into();
    let mut shell = Shell::<SunriseScreen>::default();
    shell.show(Tab::Graphics(G::Shaders));
    let row = SunriseRow::Pack(shader_ui::Item::Pack(0));
    let at = shell.point_at(&s, SIZE, row);
    shell.click(&mut s, at, SIZE, false, false, false);
    assert_eq!(s.sunrise.shader_pack, "A.zip");
    assert!(undo_shows(&mut shell, &s));
    shell.click(&mut s, at, SIZE, false, false, false);
    assert_eq!(s.sunrise.shader_pack, "");
    assert!(!undo_shows(&mut shell, &s));
}

/// Undo puts back the graphics settings and the pick, not the pick's own options, which Shader
/// Options changed (and has its own Undo for).
#[test]
fn undo_leaves_the_picks_own_options_alone() {
    let options = || {
        crate::shader_options::ShaderOptions::new(
            "A.zip".into(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
        )
    };
    let mut s = Settings {
        shader_packs: vec![pack("A.zip")],
        shader_options: Some(options()),
        ..Settings::default()
    };
    s.sunrise.shader_pack = "A.zip".into();
    let mut shell = Shell::<SunriseScreen>::default();
    shell.show(Tab::Graphics(G::Quality));
    let sky = SunriseRow::Graphics(Item::Sunrise(H::Sky));
    let at = shell.point_at(&s, SIZE, sky);
    shell.click(&mut s, at, SIZE, false, false, false);
    assert!(!s.sunrise.sky);
    // Shader Options, over this screen, changed the pick's options.
    let mut changed = options();
    changed.changed = true;
    s.shader_options = Some(changed);
    let foot = shell.foot_at(&s, SIZE);
    let (_, [x, y, w, h]) = foot.into_iter().find(|(t, _)| *t == Target::Undo).unwrap();
    shell.click(
        &mut s,
        (x + w / 2.0, y + h / 2.0),
        SIZE,
        false,
        false,
        false,
    );
    assert!(s.sunrise.sky, "the graphics change is put back");
    assert_eq!(s.sunrise.shader_pack, "A.zip");
    assert!(
        s.shader_options.as_ref().is_some_and(|o| o.changed),
        "the options stay"
    );
}

/// Leaving Sunrise Settings closes the pick's options as leaving the pack list did: unchanged,
/// they are dropped; changed, they are saved.
#[test]
fn leaving_sunrise_settings_saves_or_drops_the_picks_options() {
    use crate::shader_options::ShaderOptions;
    let options = || {
        Some(ShaderOptions::new(
            "A.zip".into(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
        ))
    };
    for changed in [false, true] {
        let mut s = Settings {
            shader_options: options(),
            ..Settings::default()
        };
        if let Some(o) = s.shader_options.as_mut() {
            o.changed = changed;
        }
        let mut ui = crate::options_ui::OptionsUi::default();
        ui.show(Page::Sunrise);
        let events = ui.back(&mut s);
        assert_eq!(
            events.contains(&Event::ShaderOptions),
            changed,
            "{events:?}"
        );
        assert_eq!(s.shader_options.is_some(), changed);
    }
}

/// Gameplay's descriptions get the extras' page's five lines, Shaders' the pack list's three,
/// the other pages the shell's two: the strip grows up over the list by a line's height (10 px)
/// for each line past two.
#[test]
fn each_page_gets_the_description_lines_its_rows_had() {
    let s = Settings::default();
    let mut shell = Shell::<SunriseScreen>::default();
    let mut bottom = |tab| {
        shell.show(tab);
        shell.columns_at(&s, SIZE).bottom
    };
    let general = bottom(Tab::Graphics(G::General));
    assert_eq!(general - bottom(Tab::Gameplay), 3.0 * 10.0);
    assert_eq!(general - bottom(Tab::Graphics(G::Shaders)), 10.0);
    for tab in [Tab::Tools, Tab::Extras, Tab::PackOptions] {
        assert_eq!(bottom(tab), general, "{tab:?}");
    }
}

/// The row to the pick's settings is greyed, saying why, while there is nothing to open: no
/// pick (none, or one the folder doesn't have), or a pick whose options weren't read (its pack
/// refused); a pick whose options were read opens them.
#[test]
fn shader_options_is_greyed_while_the_pick_has_no_settings() {
    let row = |s: &Settings| {
        let sheet = sheet_of(s, Tab::Graphics(G::Shaders));
        let mut rows = sheet.groups.into_iter().flat_map(|g| g.rows);
        rows.find(|r| r.id == SunriseRow::ShaderOptions).unwrap()
    };
    let mut s = Settings {
        shader_packs: vec![pack("A.zip")],
        ..Settings::default()
    };
    let no_pick = row(&s);
    assert!(!no_pick.live);
    assert_eq!(
        no_pick.why,
        Some(lang::get("sunrise.settings.shader_options.no_pack"))
    );
    // A pick the folder no longer has: nothing to open either.
    s.sunrise.shader_pack = "Gone.zip".into();
    assert_eq!(
        row(&s).why,
        Some(lang::get("sunrise.settings.shader_options.no_pack"))
    );
    s.sunrise.shader_pack = "A.zip".into();
    let blocked = row(&s);
    assert!(!blocked.live);
    assert_eq!(
        blocked.why,
        Some(lang::get("sunrise.settings.shader_options.none"))
    );
    s.shader_options = Some(crate::shader_options::ShaderOptions::new(
        "A.zip".into(),
        Default::default(),
        Default::default(),
        Default::default(),
        Default::default(),
    ));
    assert!(row(&s).live);
}
