//! The pages crates add to the options screens: a native mod's settings page (its keys and
//! buttons), and the pages crates above the engine register (`sunrise_hooks::pages`, docs/layers.md
//! "Platform registries") with their entries on Sunrise Settings. Kept out of options_ui.rs.
use super::*;
use sunrise_hooks::pages as added;

/// `selected_key` while a registered page's key waits (`selected_page_key`: page, row).
pub(super) const PAGE_KEY: &str = "sunrise.pageKey";

#[cfg(test)]
thread_local! {
    /// A test's own pages, instead of the game's registered ones: each test runs on its thread.
    static TEST_PAGES: std::cell::Cell<Option<&'static [&'static dyn added::Page]>> =
        const { std::cell::Cell::new(None) };
}

/// The pages crates registered.
fn registered() -> &'static [&'static dyn added::Page] {
    #[cfg(test)]
    if let Some(pages) = TEST_PAGES.get() {
        return pages;
    }
    added::engine::all()
}

/// A registered page's title (a translation key).
pub(super) fn title(page: usize) -> &'static str {
    registered().get(page).map_or("", |p| p.title())
}

/// Row `r` of a registered page, as it is now.
fn row(page: usize, r: usize) -> Option<added::Row> {
    let mut rows = Vec::new();
    registered().get(page)?.rows(&mut rows);
    rows.into_iter().nth(r)
}

/// A registered page's slider ([`Opt::Page`]): its caption (a translation key).
pub(super) fn slider_caption(page: usize, r: usize) -> &'static str {
    match row(page, r) {
        Some(added::Row::Slider { label, .. }) => label,
        _ => "",
    }
}

/// A registered page's slider: its position.
pub(super) fn slider_kind(page: usize, r: usize) -> Kind {
    match row(page, r) {
        Some(added::Row::Slider { value, .. }) => Kind::Slider(f64::from(value.clamp(0.0, 1.0))),
        _ => Kind::Cycle,
    }
}

/// A registered page's slider: the value its message shows after the caption.
pub(super) fn slider_text(page: usize, r: usize) -> Text {
    match row(page, r) {
        Some(added::Row::Slider { text, .. }) => label_text(text),
        _ => Text::empty(),
    }
}

/// A registered page's slider moved.
pub(super) fn slide(page: usize, r: usize, value: f32) {
    if let Some(registered) = registered().get(page) {
        registered.act(r, added::Act::Slide(value));
    }
}

/// What a registered page's row says on hover.
pub(super) fn tooltip(page: usize, r: usize) -> Option<Text> {
    registered().get(page)?.tooltip(r).map(label_text)
}

/// What a button opening one of Sunrise's pages says on hover.
pub(super) fn sunrise_page_tooltip(page: Page) -> Option<&'static str> {
    Some(match page {
        Page::Sunrise => "sunrise.pages.settings.tooltip",
        Page::Studio => "sunrise.pages.studio.tooltip",
        Page::Graphics => "sunrise.pages.graphics.tooltip",
        Page::GraphicsAdvanced => "sunrise.pages.graphics_advanced.tooltip",
        Page::Controller => "sunrise.pages.controller.tooltip",
        Page::Extras => "sunrise.pages.extras.tooltip",
        Page::Pvp => "sunrise.pages.pvp.tooltip",
        Page::Agent => "sunrise.pages.agent.tooltip",
        Page::ShaderPacks => "sunrise.pages.shader_packs.tooltip",
        Page::GpuDrivers => "sunrise.pages.gpu_drivers.tooltip",
        Page::Mods => "sunrise.pages.mods.tooltip",
        Page::Music => "sunrise.pages.music.tooltip",
        _ => return None,
    })
}

fn label_text(label: added::Label) -> Text {
    match label {
        added::Label::Key(key) => tr(key),
        added::Label::Value(value) => Text::literal(value),
    }
}

impl OptionsUi {
    /// Sunrise Settings: its pages, then the pages crates registered for it.
    pub(super) fn sunrise_rows(&self, s: &Settings) -> Vec<Row> {
        // Two to a row, as vanilla's OptionsList.addSmall lays out
        // buttons; each opens one of Sunrise's settings pages. Each page has
        // one way in: graphics and shader packs are Video Settings' (the graphics
        // screen), the controller is Controls', mods are the Options screen's. The
        // registry's mod browser is Sunrise's own and opens from here, not from the
        // two-button Mods footer.
        let open = |key: &str, page: Page| button(0.0, 0.0, 150.0, tr(key), Act::Push(page));
        let mut rows: Vec<Row> = vec![
            // Starling: no PvP page (its widgets and their editor are a mouse's).
            Row::Buttons(
                open("sunrise.pages.open.extras", Page::Extras),
                Some(open("sunrise.pages.open.studio", Page::Studio)),
            ),
            // No AI hides the AI Agent page and everything AI.
            if s.extras.no_ai {
                Row::Buttons(self.extra_button(s, crate::extras::Item::NoAi), None)
            } else {
                Row::Buttons(
                    open("sunrise.pages.open.agent", Page::Agent),
                    Some(self.extra_button(s, crate::extras::Item::NoAi)),
                )
            },
            Row::Buttons(open("sunrise.mods.browse", Page::ModBrowser), None),
        ];
        self.added_entries(&mut rows);
        rows
    }

    /// The entries of the pages registered for Sunrise Settings, two to a row like the engine's.
    fn added_entries(&self, rows: &mut Vec<Row>) {
        let mut entries = registered()
            .iter()
            .enumerate()
            .filter(|(_, page)| page.place() == added::Place::SunriseSettings)
            .map(|(i, page)| button(0.0, 0.0, 150.0, tr(page.title()), Act::Push(Page::Added(i))));
        while let Some(first) = entries.next() {
            rows.push(Row::Buttons(first, entries.next()));
        }
    }

    /// A registered page's rows, asked of it each time the page is drawn.
    pub(super) fn added_rows(&self, page: usize) -> Vec<Row> {
        let Some(registered) = registered().get(page) else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        registered.rows(&mut rows);
        let mut out = Vec::new();
        // The row a half-width button started, waiting for the next to sit beside it.
        let mut half_open = false;
        for (r, row) in rows.into_iter().enumerate() {
            match row {
                // Wrapped as the engine's own notes; space is for pages drawn as a screen.
                added::Row::Text { text, .. } => {
                    out.extend(wrap(&text, 64).into_iter().map(Row::Note))
                }
                added::Row::Space(_) | added::Row::Field { .. } => continue,
                added::Row::Button {
                    label,
                    active,
                    half: true,
                    ..
                } => {
                    let mut half = button(0.0, 0.0, 150.0, tr(label), Act::PageRow(page, r));
                    if !active {
                        half = inactive(half);
                    }
                    match out.last_mut() {
                        Some(Row::Buttons(_, second @ None)) if half_open => *second = Some(half),
                        _ => out.push(Row::Buttons(half, None)),
                    }
                    half_open = !half_open;
                    continue;
                }
                row => out.push(self.added_row(page, r, row)),
            }
            half_open = false;
        }
        out
    }

    fn added_row(&self, page: usize, r: usize, row: added::Row) -> Row {
        let act = Act::PageRow(page, r);
        let wide = |label: Text| Row::Buttons(button(0.0, 0.0, 310.0, label, act.clone()), None);
        match row {
            added::Row::Heading(key) => Row::Heading(key.to_owned()),
            added::Row::Note(text) => Row::Note(text),
            added::Row::Toggle {
                label, on, active, ..
            } => {
                let wide = button(0.0, 0.0, 310.0, generic(label, on_off(on)), act);
                Row::Buttons(if active { wide } else { inactive(wide) }, None)
            }
            added::Row::Cycle { label, value } => wide(generic(label, label_text(value))),
            added::Row::Slider { .. } => Row::Big(Opt::Page(page, r)),
            added::Row::Button { label, active, .. } => {
                let wide = button(0.0, 0.0, 310.0, tr(label), act);
                Row::Buttons(if active { wide } else { inactive(wide) }, None)
            }
            added::Row::Key {
                label, key, active, ..
            } => {
                let mut message = keybinds::display_name(&key);
                if self.selected_key == Some(PAGE_KEY) && self.selected_page_key == Some((page, r))
                {
                    message = Text::literal("> ")
                        .append(message.with_color(text::WHITE))
                        .append(" <")
                        .with_color(text::YELLOW);
                }
                let caption = button(0.0, 0.0, 150.0, tr(label), Act::None);
                let change = button(0.0, 0.0, 150.0, message, act);
                let change = if active { change } else { inactive(change) };
                Row::Buttons(inactive(caption), Some(change))
            }
            // A row kind newer than this engine: shown empty.
            _ => Row::Note(String::new()),
        }
    }

    /// A registered page's row was pressed: a key row waits for a key, any other row hears it.
    pub(super) fn page_act(&mut self, page: usize, r: usize, shift: bool) -> Vec<Event> {
        let Some(registered) = registered().get(page) else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        registered.rows(&mut rows);
        match rows.get(r) {
            Some(
                added::Row::Toggle { active: false, .. } | added::Row::Key { active: false, .. },
            ) => Vec::new(),
            Some(added::Row::Key { .. }) => {
                self.selected_key = Some(PAGE_KEY);
                self.selected_page_key = Some((page, r));
                Vec::new()
            }
            Some(_) => {
                registered.act(
                    r,
                    if shift {
                        added::Act::PressBack
                    } else {
                        added::Act::Press
                    },
                );
                vec![Event::Changed]
            }
            None => Vec::new(),
        }
    }

    /// The open registered page's rows changed since its layout was made (a row greyed, a value
    /// moved): lay it out again, and end a key's wait when its row is no longer an active key.
    pub(super) fn refresh_added_rows(&mut self) {
        let Page::Added(i) = self.page() else {
            return;
        };
        let Some(page) = registered().get(i) else {
            return;
        };
        let mut rows = Vec::new();
        page.rows(&mut rows);
        if rows == self.added_seen {
            return;
        }
        if self.selected_key == Some(PAGE_KEY)
            && let Some((waiting_page, k)) = self.selected_page_key
            && waiting_page == i
            && !matches!(rows.get(k), Some(added::Row::Key { active: true, .. }))
        {
            self.selected_key = None;
            self.selected_page_key = None;
        }
        self.added_seen = rows;
        self.relayout();
    }

    /// Binds the waiting key row, read again first: a row that went inactive or away while it
    /// waited hears nothing.
    pub(super) fn bind_page_key(&mut self, value: String) -> Vec<Event> {
        let Some((i, k)) = self.selected_page_key.take() else {
            return Vec::new();
        };
        let Some(page) = registered().get(i) else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        page.rows(&mut rows);
        if !matches!(rows.get(k), Some(added::Row::Key { active: true, .. })) {
            return Vec::new();
        }
        page.act(k, added::Act::Bind(value));
        vec![Event::Changed]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A page with a heading, a switch, a number that steps either way, a key and a volume.
    struct Sample {
        on: Mutex<bool>,
        step: Mutex<i32>,
        key: Mutex<String>,
        volume: Mutex<f32>,
    }

    impl added::Page for Sample {
        fn id(&self) -> &'static str {
            "test.sample"
        }
        fn title(&self) -> &'static str {
            "test.sample.title"
        }
        fn place(&self) -> added::Place {
            added::Place::SunriseSettings
        }
        fn rows(&self, rows: &mut Vec<added::Row>) {
            rows.push(added::Row::Heading("test.sample.heading"));
            rows.push(added::Row::toggle(
                "test.sample.switch",
                *self.on.lock().unwrap(),
            ));
            rows.push(added::Row::Cycle {
                label: "test.sample.step",
                value: added::Label::Value(self.step.lock().unwrap().to_string()),
            });
            rows.push(added::Row::key(
                "test.sample.key",
                self.key.lock().unwrap().clone(),
            ));
            let volume = *self.volume.lock().unwrap();
            rows.push(added::Row::Slider {
                label: "test.sample.volume",
                value: volume,
                text: added::Label::Value(format!("{}%", (volume * 100.0).round())),
            });
        }
        fn act(&self, row: usize, act: added::Act) {
            match (row, act) {
                (1, added::Act::Press) => {
                    let mut on = self.on.lock().unwrap();
                    *on = !*on;
                }
                (2, added::Act::Press) => *self.step.lock().unwrap() += 1,
                (2, added::Act::PressBack) => *self.step.lock().unwrap() -= 1,
                (3, added::Act::Bind(key)) => *self.key.lock().unwrap() = key,
                (4, added::Act::Slide(v)) => *self.volume.lock().unwrap() = v,
                other => panic!("row and act the page doesn't expect: {other:?}"),
            }
        }
        fn tooltip(&self, row: usize) -> Option<added::Label> {
            (row == 1).then_some(added::Label::Key("test.sample.switch.tooltip"))
        }
    }

    static SAMPLE: Sample = Sample {
        on: Mutex::new(false),
        step: Mutex::new(0),
        key: Mutex::new(String::new()),
        volume: Mutex::new(0.0),
    };
    static PAGES: [&dyn added::Page; 1] = [&SAMPLE];

    /// Clicks the active button that does `act`, with Shift held or not.
    fn press(ui: &mut OptionsUi, s: &mut Settings, act: Act, shift: bool) -> Vec<Event> {
        let size = (480.0, 240.0);
        let (widgets, ..) = ui.layout(s, size.0, size.1);
        let (x, y) = widgets
            .iter()
            .find_map(|w| match &w.kind {
                WKind::Button {
                    act: a,
                    active: true,
                    ..
                } if *a == act => Some((w.x + 1.0, w.y + 1.0)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no button does {act:?}"));
        ui.shift = shift;
        let atlas = super::super::tests::atlas();
        ui.click(s, &atlas, x, y, size, [480, 240], false, false)
    }

    #[test]
    fn a_registered_page_opens_from_sunrise_settings_and_hears_its_rows() {
        let mut s = Settings::default();
        TEST_PAGES.set(Some(&PAGES));
        let mut ui = OptionsUi::default();
        ui.show(Page::Sunrise);
        press(&mut ui, &mut s, Act::Push(Page::Added(0)), false);
        assert_eq!(ui.page(), Page::Added(0));
        // Rows: the heading (0), the switch (1), the step (2), the key (3).
        let events = press(&mut ui, &mut s, Act::PageRow(0, 1), false);
        assert_eq!(
            events.last(),
            Some(&Event::Changed),
            "the settings get saved"
        );
        assert!(*SAMPLE.on.lock().unwrap());
        press(&mut ui, &mut s, Act::PageRow(0, 2), false);
        press(&mut ui, &mut s, Act::PageRow(0, 2), true);
        press(&mut ui, &mut s, Act::PageRow(0, 2), true);
        assert_eq!(*SAMPLE.step.lock().unwrap(), -1, "Shift steps back");
        // A key row waits for the next key, as on Key Binds, and the page gets its name.
        press(&mut ui, &mut s, Act::PageRow(0, 3), false);
        let atlas = super::super::tests::atlas();
        let events = ui.key(&mut s, &atlas, winit::keyboard::KeyCode::KeyZ, None);
        assert_eq!(events, [Event::Changed]);
        assert_eq!(*SAMPLE.key.lock().unwrap(), "key.keyboard.z");
        assert_eq!(ui.selected_key, None);
        // The volume is one of the screen's own sliders: dragged to its middle, then let go.
        let size = (480.0, 240.0);
        let (widgets, ..) = ui.layout(&s, size.0, size.1);
        let slider = widgets
            .iter()
            .find(|w| {
                matches!(
                    w.kind,
                    WKind::Slider {
                        opt: Opt::Page(0, 4),
                        ..
                    }
                )
            })
            .expect("the volume's slider");
        let (x, y) = (slider.x + slider.w / 2.0, slider.y + 1.0);
        ui.click(&mut s, &atlas, x, y, size, [480, 240], false, false);
        let volume = *SAMPLE.volume.lock().unwrap();
        assert!((0.45..0.55).contains(&volume), "{volume}");
        assert!(ui.release().contains(&Event::Changed));
        // The slider shows the page's value once let go.
        let (widgets, ..) = ui.layout(&s, size.0, size.1);
        let shown = widgets.iter().find_map(|w| match w.kind {
            WKind::Slider {
                opt: Opt::Page(0, 4),
                value,
                ..
            } => Some(value),
            _ => None,
        });
        assert_eq!(shown, Some(volume));
        // The page's tooltip for its switch.
        let switch = widgets
            .iter()
            .find(|w| {
                matches!(
                    &w.kind,
                    WKind::Button {
                        act: Act::PageRow(0, 1),
                        ..
                    }
                )
            })
            .expect("the switch");
        assert_eq!(
            tooltip_text(&s, switch).map(|t| t.plain()),
            Some(tr("test.sample.switch.tooltip").plain())
        );
    }

    /// Text, a grey note, space, two half buttons (the second inactive) and a footer button.
    struct Shaped(Mutex<Vec<usize>>);

    impl added::Page for Shaped {
        fn id(&self) -> &'static str {
            "test.shaped"
        }
        fn title(&self) -> &'static str {
            "test.shaped.title"
        }
        fn place(&self) -> added::Place {
            added::Place::SunriseSettings
        }
        fn rows(&self, rows: &mut Vec<added::Row>) {
            rows.push(added::Row::text("ABCDE-FGHIJ").groups());
            rows.push(added::Row::text("grey").quiet());
            rows.push(added::Row::Space(6));
            rows.push(added::Row::button("test.shaped.copy").half());
            rows.push(added::Row::button("test.shaped.stop").half().active(false));
            rows.push(added::Row::button("test.shaped.help").footer());
        }
        fn act(&self, row: usize, _: added::Act) {
            self.0.lock().unwrap().push(row);
        }
    }

    static SHAPED: Shaped = Shaped(Mutex::new(Vec::new()));
    static SHAPED_PAGES: [&dyn added::Page; 1] = [&SHAPED];

    #[test]
    fn half_buttons_share_a_row_and_an_inactive_one_is_never_pressed() {
        let mut s = Settings::default();
        TEST_PAGES.set(Some(&SHAPED_PAGES));
        let ui = OptionsUi::default();
        let rows = ui.added_rows(0);
        // The two texts as notes (space shows nothing here), the halves on one row, the
        // footer button below them (the settings shell has no footer of its own).
        assert!(matches!(&rows[0], Row::Note(t) if t == "ABCDE-FGHIJ"));
        assert!(matches!(&rows[1], Row::Note(t) if t == "grey"));
        let Row::Buttons(copy, Some(stop)) = &rows[2] else {
            panic!("the halves sit side by side: {:?}", rows.len());
        };
        assert_eq!((copy.w, stop.w), (150.0, 150.0));
        assert!(matches!(&rows[3], Row::Buttons(help, None) if help.w == 310.0));
        assert_eq!(rows.len(), 4);
        let mut ui = ui;
        ui.show(Page::Added(0));
        press(&mut ui, &mut s, Act::PageRow(0, 3), false);
        press(&mut ui, &mut s, Act::PageRow(0, 5), false);
        let (widgets, ..) = ui.layout(&s, 480.0, 240.0);
        let stop = widgets
            .iter()
            .find(|w| {
                matches!(
                    &w.kind,
                    WKind::Button {
                        act: Act::PageRow(0, 4),
                        ..
                    }
                )
            })
            .expect("Stop is drawn");
        let atlas = super::super::tests::atlas();
        ui.click(
            &mut s,
            &atlas,
            stop.x + 1.0,
            stop.y + 1.0,
            (480.0, 240.0),
            [480, 240],
            false,
            false,
        );
        assert_eq!(
            *SHAPED.0.lock().unwrap(),
            [3, 5],
            "the inactive half heard nothing"
        );
    }

    /// A greyed switch and a greyed key.
    struct Greyed;
    impl added::Page for Greyed {
        fn id(&self) -> &'static str {
            "test.greyed"
        }
        fn title(&self) -> &'static str {
            "test.greyed.title"
        }
        fn place(&self) -> added::Place {
            added::Place::SunriseSettings
        }
        fn rows(&self, rows: &mut Vec<added::Row>) {
            rows.push(added::Row::toggle("test.greyed.switch", true).active(false));
            rows.push(added::Row::key("test.greyed.key", "key.keyboard.z").active(false));
        }
        fn act(&self, _: usize, _: added::Act) {
            panic!("a greyed row heard an act");
        }
    }

    #[test]
    fn an_inactive_switch_and_key_are_greyed_and_never_pressed() {
        static GREYED: Greyed = Greyed;
        static GREYED_PAGES: [&dyn added::Page; 1] = [&GREYED];
        TEST_PAGES.set(Some(&GREYED_PAGES));
        let ui = OptionsUi::default();
        let rows = ui.added_rows(0);
        let Row::Buttons(switch, None) = &rows[0] else {
            panic!("the switch is one wide button");
        };
        assert!(matches!(&switch.kind, WKind::Button { active: false, .. }));
        let Row::Buttons(_, Some(change)) = &rows[1] else {
            panic!("the key has a change button");
        };
        assert!(matches!(&change.kind, WKind::Button { active: false, .. }));
        let mut ui = ui;
        assert!(ui.page_act(0, 0, false).is_empty());
        assert!(ui.page_act(0, 1, false).is_empty());
        assert_eq!(ui.selected_page_key, None);
    }

    /// A key row whose greying the test controls, recording what it hears.
    struct Fading {
        active: Mutex<bool>,
        heard: Mutex<Vec<String>>,
    }
    impl added::Page for Fading {
        fn id(&self) -> &'static str {
            "test.fading"
        }
        fn title(&self) -> &'static str {
            "test.fading.title"
        }
        fn place(&self) -> added::Place {
            added::Place::SunriseSettings
        }
        fn rows(&self, rows: &mut Vec<added::Row>) {
            let active = *self.active.lock().unwrap();
            rows.push(added::Row::key("test.fading.key", "key.keyboard.z").active(active));
        }
        fn act(&self, _: usize, act: added::Act) {
            self.heard.lock().unwrap().push(format!("{act:?}"));
        }
    }

    #[test]
    fn a_key_row_greyed_while_it_waits_stops_waiting_and_is_never_bound() {
        static FADING: Fading = Fading {
            active: Mutex::new(true),
            heard: Mutex::new(Vec::new()),
        };
        static FADING_PAGES: [&dyn added::Page; 1] = [&FADING];
        TEST_PAGES.set(Some(&FADING_PAGES));
        let mut ui = OptionsUi::default();
        ui.stack.push(Page::Added(0));
        ui.refresh_added_rows();
        assert!(ui.page_act(0, 0, false).is_empty());
        assert_eq!(ui.selected_page_key, Some((0, 0)), "the key waits");
        *FADING.active.lock().unwrap() = false;
        // A bind that lands before the page is laid out again still reaches nothing.
        assert!(ui.bind_page_key("key.keyboard.x".into()).is_empty());
        ui.selected_key = Some(PAGE_KEY);
        ui.selected_page_key = Some((0, 0));
        ui.refresh_added_rows();
        assert_eq!((ui.selected_key, ui.selected_page_key), (None, None));
        let rows = ui.added_rows(0);
        let Row::Buttons(_, Some(change)) = &rows[0] else {
            panic!("the key has a change button");
        };
        assert!(matches!(&change.kind, WKind::Button { active: false, .. }));
        assert!(
            FADING.heard.lock().unwrap().is_empty(),
            "the greyed key heard nothing"
        );
    }
}
