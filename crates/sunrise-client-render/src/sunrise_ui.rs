//! Sunrise Settings: one screen on the settings shell (`settings_shell`) for everything Sunrise
//! adds to the options. Its pages are the graphics screen's (`graphics_ui`), with the shader packs
//! listed on Shaders (`shader_ui`), then the selected resource packs' options, then Gameplay (the
//! extras, `extras_ui`), Tools and Extras. Each page's rows, values and what pressing them does
//! stay in the screen they come from: this one hosts them, adds the rows that open other pages,
//! and keeps Undo to what the graphics screen and the pack list put back.
use crate::{
    extras::Item as ExtraItem,
    extras_ui::{self, ExtraRow, ExtrasScreen},
    graphics_ui::{self, GraphicsScreen},
    lang,
    options_ui::Page,
    settings::Settings,
    settings_shell::{self as shell, Group, Output, Row, Screen, Sheet, Value},
    shader_ui::{self, ShaderScreen},
};
use sunrise_hooks::pages as added;

/// The language key of the screen's name.
const TITLE: &str = "sunrise.pages.settings";
const PACK_OPTIONS: &str = "sunrise.settings.tab.resource_pack_options";
const PACK_OPTIONS_ABOUT: &str = "sunrise.settings.tab.resource_pack_options.about";
/// The Resource Pack Options page while no selected pack has options.
const NO_PACK_OPTIONS: &str = "sunrise.settings.pack_options.none";
/// A selected pack's row: what it opens.
const PACK_OPTIONS_ROW: &str = "sunrise.pack.optionsInfo";
const GAMEPLAY: &str = "sunrise.settings.tab.gameplay";
const TOOLS: &str = "sunrise.settings.tab.tools";
const TOOLS_ABOUT: &str = "sunrise.settings.tab.tools.about";
const EXTRAS: &str = "sunrise.settings.tab.extras";
const EXTRAS_ABOUT: &str = "sunrise.settings.tab.extras.about";
/// The Shaders page's row to the picked pack's own settings (that page's name).
const SHADER_OPTIONS: &str = "sunrise.pages.shader_options";

/// A page of the screen, in [`TABS`]' order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Graphics(graphics_ui::Page),
    /// The selected resource packs' Polytone options (`pack_options_ui`), a pack a row.
    PackOptions,
    /// The extras (`extras_ui`): what changes how the game plays, all off by default.
    Gameplay,
    /// Replay Studio and the AI agent.
    Tools,
    /// PvP & HUD, and the pages crates register for Sunrise Settings.
    Extras,
}

/// The pages, in order.
pub const TABS: [Tab; 10] = {
    use graphics_ui::Page as G;
    [
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
    ]
};

/// A row, by the screen it comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SunriseRow {
    Graphics(graphics_ui::Item),
    /// A shader pack, or the folder's row (`shader_ui`'s pack list).
    Pack(shader_ui::Item),
    /// Opens the picked shader pack's own settings (`Page::ShaderOptions`).
    ShaderOptions,
    /// An extra: Gameplay's rows, and the shader pack key on Shaders.
    Extra(ExtraRow),
    /// Opens the options of the `.0`th selected pack that has some.
    PackOptions(usize),
    /// The note while no selected pack has options.
    NoPackOptions,
    /// Opens another options page.
    Open(Page),
}

/// The screen: the screens whose rows it hosts, and what it needs from the options screen.
#[derive(Default)]
pub struct SunriseScreen {
    pub(crate) graphics: GraphicsScreen,
    shader: ShaderScreen,
    extras: ExtrasScreen,
    /// The resource packs with options, by id with their title, as the options screen knows them
    /// (`OptionsUi::pack_configs`): the selected ones are listed.
    pub(crate) packs_with_options: Vec<(String, String)>,
    /// The pack whose options a row asked to open: the options screen opens them.
    opening: Option<String>,
    /// The last row acted on was the graphics screen's (`Screen::changes`): an extra's change is
    /// none Undo puts back, as on the extras' own page, which had no Undo.
    last_graphics: bool,
    /// A row changed a graphics setting since the screen opened, or since Undo: Undo shows
    /// for it, as on the graphics screen, though the value may have been stepped back. A `Cell`:
    /// `Screen::changes`, which sees the change, reads the screen without changing it.
    graphics_changed: std::cell::Cell<bool>,
}

impl SunriseScreen {
    /// The pack a row asked to open the options of, once.
    pub(crate) fn take_opening(&mut self) -> Option<String> {
        self.opening.take()
    }
    /// The selected packs that have options, topmost first, as the pack screen lists them.
    fn selected_with_options<'a>(&'a self, s: &'a Settings) -> Vec<&'a (String, String)> {
        let selected = s.resource_packs.iter().rev();
        selected
            .filter_map(|id| self.packs_with_options.iter().find(|(p, _)| p == id))
            .collect()
    }
    /// Resource Pack Options' rows: a row opening each selected pack with options, topmost first,
    /// or a note when none has any.
    fn pack_options_rows(&self, s: &Settings) -> Vec<Row<SunriseRow>> {
        let packs = self.selected_with_options(s);
        if packs.is_empty() {
            let note = Row::new(
                SunriseRow::NoPackOptions,
                lang::get(NO_PACK_OPTIONS),
                Value::Note,
            );
            return vec![note.live(false)];
        }
        let about = lang::get(PACK_OPTIONS_ROW);
        let rows = packs.into_iter().enumerate();
        rows.map(|(i, (_, title))| {
            Row::new(SunriseRow::PackOptions(i), title.clone(), Value::Press).about(about.clone())
        })
        .collect()
    }
    /// Shaders: the packs, the way to the picked one's settings, then Sunrise's own settings
    /// for every pack and the key that turns the pick off and on.
    fn shaders(&self, s: &Settings, groups: Vec<Group<SunriseRow>>) -> Vec<Group<SunriseRow>> {
        let mut packs: Vec<_> = (shader_ui::pack_rows(s).into_iter())
            .map(|row| row.map_id(SunriseRow::Pack))
            .collect();
        let why = shader_ui::settings_blocked(s).map(lang::get);
        // A row that opens a page ends in "...", as Tools' rows do.
        let name = format!("{}...", lang::get(SHADER_OPTIONS));
        let options = Row::new(SunriseRow::ShaderOptions, name, Value::Press);
        packs.push(
            options
                .about(lang::get(shader_ui::PACK_SETTINGS))
                .blocked(why),
        );
        let mut all = vec![Group {
            name: shader_ui::INSTALLED,
            rows: packs,
        }];
        all.extend(groups);
        let key = extras_ui::item_row(ExtraItem::ShaderPackKey, s).map_id(SunriseRow::Extra);
        match all.iter_mut().find(|g| g.name == shader_ui::BUILT_IN) {
            Some(group) => group.rows.push(key),
            None => all.push(Group {
                name: shader_ui::BUILT_IN,
                rows: vec![key],
            }),
        }
        all
    }
    /// Tools: a row for each of its pages.
    fn tools() -> Vec<Row<SunriseRow>> {
        [
            (Page::Studio, "sunrise.pages.open.studio"),
            (Page::Agent, "sunrise.pages.open.agent"),
        ]
        .into_iter()
        .map(|(page, key)| open_row(page, lang::get(key)))
        .collect()
    }
    /// Extras: PvP & HUD, then each page registered for Sunrise Settings.
    fn extras() -> Vec<Row<SunriseRow>> {
        let pvp = open_row(Page::Pvp, lang::get("sunrise.pages.open.pvp"));
        let pages = crate::options_ui::registered_pages().iter().enumerate();
        let added = pages
            .filter(|(_, page)| page.place() == added::Place::SunriseSettings)
            .map(|(i, page)| open_row(Page::Added(i), lang::get(page.title())));
        std::iter::once(pvp).chain(added).collect()
    }
}

/// A row that opens `page`, with what the page holds as its description.
fn open_row(page: Page, name: String) -> Row<SunriseRow> {
    let about = crate::options_ui::page_tooltip(page).map_or_else(String::new, lang::get);
    Row::new(SunriseRow::Open(page), name, Value::Press).about(about)
}

/// A page of the screen, with its groups; one group shows no name over it.
fn sheet(
    tab: Tab,
    name: String,
    about: String,
    mut groups: Vec<Group<SunriseRow>>,
) -> Sheet<Tab, SunriseRow> {
    if let [only] = groups.as_mut_slice() {
        only.name = "";
    }
    Sheet::new(tab, name, about, groups)
}

impl Screen for SunriseScreen {
    type Page = Tab;
    type Row = SunriseRow;
    fn title(&self) -> &'static str {
        TITLE
    }
    /// Gameplay's descriptions run long, as the Extras page's did, and the packs' on Shaders as
    /// the pack list's did.
    fn note_lines_on(&self, tab: Tab) -> usize {
        match tab {
            Tab::Gameplay => extras_ui::NOTE_LINES,
            Tab::Graphics(graphics_ui::Page::Shaders) => shader_ui::NOTE_LINES,
            _ => self.note_lines(),
        }
    }
    /// The rows read what the graphics screen's, the pack list's and the extras' read, and the
    /// selected packs with options.
    fn stamp(&self, s: &Settings) -> u64 {
        let packs = self.selected_with_options(s);
        let parts = (
            self.graphics.stamp(s),
            self.shader.stamp(s),
            self.extras.stamp(s),
            packs,
        );
        shell::stamp_of(&parts)
    }
    fn sheets(&self, s: &Settings) -> Vec<Sheet<Tab, SunriseRow>> {
        let mut graphics = self.graphics.sheets(s).into_iter();
        let mut sheets = Vec::new();
        for tab in TABS {
            let made = match tab {
                Tab::Graphics(page) => {
                    let Some(g) = graphics.find(|g| g.id == page) else {
                        continue;
                    };
                    let groups = g.groups.into_iter();
                    let groups: Vec<_> = groups.map(|g| g.map_id(SunriseRow::Graphics)).collect();
                    // Shaders says what the pack list said: how to get packs, or how to pick.
                    let (groups, about) = match page {
                        graphics_ui::Page::Shaders => {
                            (self.shaders(s, groups), shader_ui::packs_about(s))
                        }
                        _ => (groups, g.about),
                    };
                    sheet(tab, g.name, about, groups)
                }
                Tab::PackOptions => {
                    let rows = self.pack_options_rows(s);
                    let about = lang::get(PACK_OPTIONS_ABOUT);
                    sheet(tab, lang::get(PACK_OPTIONS), about, vec![group(rows)])
                }
                // The extras' page, under its own description.
                Tab::Gameplay => {
                    let Some(extras) = self.extras.sheets(s).into_iter().next() else {
                        continue;
                    };
                    let groups = extras.groups.into_iter();
                    let groups = groups.map(|g| g.map_id(SunriseRow::Extra)).collect();
                    sheet(tab, lang::get(GAMEPLAY), extras.about, groups)
                }
                Tab::Tools => {
                    let about = lang::get(TOOLS_ABOUT);
                    sheet(tab, lang::get(TOOLS), about, vec![group(Self::tools())])
                }
                Tab::Extras => {
                    let about = lang::get(EXTRAS_ABOUT);
                    sheet(tab, lang::get(EXTRAS), about, vec![group(Self::extras())])
                }
            };
            sheets.push(made);
        }
        sheets
    }
    fn press(&mut self, s: &mut Settings, row: SunriseRow, back: bool) -> Vec<Output> {
        self.last_graphics = false;
        match row {
            SunriseRow::Graphics(item) => {
                self.last_graphics = true;
                self.graphics.press(s, item, back)
            }
            // A pick shows Undo by `differs`.
            SunriseRow::Pack(item) => self.shader.press(s, item, back),
            // The pick's options, read as the screen opened, as a pick read them, or as coming
            // back from them read them again.
            SunriseRow::ShaderOptions if shader_ui::settings_blocked(s).is_none() => {
                vec![Output::Push(Page::ShaderOptions)]
            }
            SunriseRow::ShaderOptions | SunriseRow::NoPackOptions => Vec::new(),
            SunriseRow::Extra(row) => self.extras.press(s, row, back),
            SunriseRow::PackOptions(i) => {
                let pack = self.selected_with_options(s).get(i).map(|p| p.0.clone());
                match pack {
                    Some(id) => {
                        self.opening = Some(id);
                        vec![Output::Push(Page::PackOptions)]
                    }
                    None => Vec::new(),
                }
            }
            SunriseRow::Open(page) => vec![Output::Push(page)],
        }
    }
    fn slide(&mut self, s: &mut Settings, row: SunriseRow, position: f64) {
        self.last_graphics = false;
        match row {
            SunriseRow::Graphics(item) => {
                self.last_graphics = true;
                self.graphics.slide(s, item, position);
            }
            SunriseRow::Extra(row) => self.extras.slide(s, row, position),
            _ => {}
        }
    }
    fn wheel_step(&mut self, s: &mut Settings, row: SunriseRow, back: bool) -> Option<Vec<Output>> {
        self.last_graphics = false;
        match row {
            SunriseRow::Graphics(item) => {
                self.last_graphics = true;
                self.graphics.wheel_step(s, item, back)
            }
            SunriseRow::Pack(item) => self.shader.wheel_step(s, item, back),
            SunriseRow::Extra(row) => self.extras.wheel_step(s, row, back),
            _ => None,
        }
    }
    fn undoes(&self) -> bool {
        true
    }
    /// The graphics screen's settings and the shader pick, as the screen opened with them: not
    /// a pack's own options, which its settings page keeps.
    fn undo(&mut self, s: &mut Settings, before: &Settings) -> Vec<Output> {
        self.graphics_changed.set(false);
        let mut outputs = self.shader.undo_pick(s, before);
        outputs.extend(self.graphics.undo(s, before));
        outputs
    }
    /// Undo shows while a graphics setting was changed (as on the graphics screen) or the pick
    /// differs (as on the pack list: a pack picked and unpicked again isn't a change).
    fn differs(&self, s: &Settings, before: &Settings) -> Option<bool> {
        let pick = ShaderScreen::pick_differs(s, before);
        Some(self.graphics_changed.get() || pick)
    }
    /// A change of the graphics screen's, as it counts them: an extra changed here leaves no
    /// Undo, as on the extras' own page, nor does a key being bound (the shell starts the wait
    /// without a press). A pick shows Undo by `differs`.
    fn changes(&self, outputs: &[Output]) -> bool {
        let bound = outputs.iter().any(|o| matches!(o, Output::Bind(_)));
        let changed = self.last_graphics && !bound && self.graphics.changes(outputs);
        if changed {
            self.graphics_changed.set(true);
        }
        changed
    }
}

/// One group of rows, untitled.
fn group(rows: Vec<Row<SunriseRow>>) -> Group<SunriseRow> {
    Group { name: "", rows }
}

#[cfg(test)]
#[path = "sunrise_ui_tests.rs"]
mod tests;
