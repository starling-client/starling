//! Starling's part of options_ui.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

use super::*;

/// `text` cut to `chars` characters, `...` marking the cut.
pub(super) fn clipped(text: &str, chars: usize) -> String {
    if text.chars().count() <= chars {
        return text.to_owned();
    }
    let cut: String = text.chars().take(chars - 3).collect();
    format!("{}...", cut.trim_end())
}

/// Where the Shader Packs page's folder is, as a player finds it: on a phone, from the storage's
/// root (`Starling/shaderpacks`), or in the app's own files, which Open Pack Folder sets up a
/// reachable folder for.
pub(crate) fn shader_folder_note() -> String {
    let dir = std::env::current_dir()
        .unwrap_or_default()
        .join("shaderpacks");
    if cfg!(target_os = "android") {
        if crate::logging::game_home() == crate::logging::private_home() {
            return "In the app's own files: Open Pack Folder to reach them".into();
        }
        if let Ok(shared) = dir.strip_prefix("/storage/emulated/0") {
            return format!("Folder: {}", shared.display());
        }
    }
    clipped(&format!("Folder: {}", dir.display()), 52)
}

/// Android's own rows. A constant: horizon-mixins only reads `format!`, `matches!` and `vec!` in
/// a function it hooks.
pub(super) const ON_A_PHONE: bool = cfg!(target_os = "android");

/// The Options page's entries, with Diagnostics on a phone. Apart from `OptionsUi::layout`:
/// horizon-mixins refuses to hook a function with a `#[cfg]` in it (E_UNSUPPORTED_CFG).
pub(super) fn with_diagnostics(
    entries: impl Into<Vec<(&'static str, Option<Page>)>>,
) -> Vec<(&'static str, Option<Page>)> {
    let mut all = entries.into();
    if ON_A_PHONE {
        all.push(("starling.diagnostics.title", Some(Page::Diagnostics)));
    }
    all
}

/// The catalog's shelf for a page with a footer that has a Catalog button. The Resource Packs page
/// draws its own (`pack_screen`).
pub(super) fn catalog_shelf(page: Page) -> Option<CatalogShelf> {
    match page {
        Page::Mods | Page::NativeGame => Some(CatalogShelf::Mods),
        Page::ShaderPacks => Some(CatalogShelf::ShaderPacks),
        _ => None,
    }
}

/// Which long list a custom button's page opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustomPick {
    Control,
    Keyboard,
    Icon,
}

/// What a finger paints with, on a custom button's canvas; the last clears.
pub(super) const PALETTE: [[u8; 4]; 16] = [
    [255, 255, 255, 255],
    [190, 190, 190, 255],
    [120, 120, 120, 255],
    [38, 38, 38, 255],
    [214, 60, 60, 255],
    [240, 140, 40, 255],
    [245, 215, 70, 255],
    [90, 190, 70, 255],
    [40, 120, 60, 255],
    [70, 200, 210, 255],
    [60, 110, 220, 255],
    [130, 70, 200, 255],
    [230, 110, 190, 255],
    [130, 85, 50, 255],
    [245, 210, 170, 255],
    [0, 0, 0, 0],
];

/// The canvas of a custom button's own image.
#[derive(Default)]
pub(super) struct Painter {
    pub(super) pixels: Vec<u8>,
    /// Its place on the page: x, y and how many GUI pixels a pixel is.
    pub(super) at: [f32; 3],
    pub(super) color: usize,
    /// The pixel the finger was last on, to fill in between when it moves fast.
    pub(super) last: Option<(i32, i32)>,
    /// It changed since it opened: Done keeps it.
    pub(super) dirty: bool,
    /// A kept image the atlas's slot has yet to get.
    pub(super) upload: Option<(u32, Vec<u8>)>,
}

impl Painter {
    /// A touch sprite as the atlas has it (Bedrock's look where the phone fetched it), a button's
    /// size; else the game's own.
    pub(super) fn sprite(atlas: &HudAtlas, name: &str) -> Option<Vec<u8>> {
        let rect = atlas
            .sprites
            .get(&format!("starling:gui/sprites/touch/{name}"));
        let cut = rect.and_then(|r| {
            let (x0, y0) = (
                (r.u0 * atlas.width as f32).round() as u32,
                (r.v0 * atlas.height as f32).round() as u32,
            );
            let mut pixels = Vec::with_capacity((r.w * r.h * 4) as usize);
            let all = atlas.pixels();
            for y in y0..y0 + r.h {
                let at = ((y * atlas.width + x0) * 4) as usize;
                pixels.extend_from_slice(all.get(at..at + (r.w * 4) as usize)?);
            }
            Some(own_buttons::fit(r.w, r.h, &pixels))
        });
        cut.or_else(|| crate::hud::touch_sprite(name))
    }

    /// A button's own image, or the icon it wears, else a blank button, to draw on.
    pub(super) fn open(
        &mut self,
        atlas: &HudAtlas,
        button: Option<&crate::settings::CustomButton>,
    ) {
        self.dirty = false;
        self.last = None;
        let side = own_buttons::SIZE as usize;
        let own = button
            .filter(|b| b.image)
            .and_then(|b| own_buttons::load(b.id));
        let icon = button
            .and_then(|b| b.icon.as_deref())
            .and_then(|icon| Self::sprite(atlas, &format!("custom_{icon}")));
        self.pixels = own
            .or(icon)
            .or_else(|| Self::sprite(atlas, "custom_blank"))
            .filter(|p| p.len() == side * side * 4)
            .unwrap_or_else(|| vec![0; side * side * 4]);
    }

    /// The image a button wears while pressed, else a pressed blank button, to draw on.
    pub(super) fn open_pressed(&mut self, atlas: &HudAtlas, slot: u32) {
        self.dirty = false;
        self.last = None;
        let side = own_buttons::SIZE as usize;
        self.pixels = own_buttons::load(slot)
            .or_else(|| Self::sprite(atlas, "custom_blank_pressed"))
            .or_else(|| Self::sprite(atlas, "custom_blank"))
            .filter(|p| p.len() == side * side * 4)
            .unwrap_or_else(|| vec![0; side * side * 4]);
    }

    pub(super) fn clear(&mut self) {
        self.pixels.fill(0);
        self.dirty = true;
    }

    /// Back to the plate of the game's own buttons.
    pub(super) fn plate(&mut self, atlas: &HudAtlas) {
        if let Some(plate) = Self::sprite(atlas, "custom_blank") {
            self.pixels = plate;
            self.dirty = true;
        }
    }

    /// A picture the system handed over.
    pub(super) fn import(&mut self, width: u32, height: u32, rgba: &[u8]) {
        self.pixels = own_buttons::fit(width, height, rgba);
        self.dirty = true;
    }

    /// The swatch at `i`: x and y on the page, beside the canvas.
    pub(super) fn swatch(&self, i: usize) -> (f32, f32) {
        let [x, y, cell] = self.at;
        let right = x + own_buttons::SIZE as f32 * cell + 8.0;
        (right + (i % 2) as f32 * 16.0, y + (i / 2) as f32 * 16.0)
    }

    pub(super) fn paint(&mut self, x: i32, y: i32) {
        let side = own_buttons::SIZE as i32;
        if (0..side).contains(&x) && (0..side).contains(&y) {
            let at = ((y * side + x) * 4) as usize;
            if let Some(pixel) = self.pixels.get_mut(at..at + 4) {
                pixel.copy_from_slice(&PALETTE[self.color]);
                self.dirty = true;
            }
        }
    }

    /// A press or a drag on the page; true when it landed on the canvas or a swatch.
    pub(super) fn press(&mut self, x: f32, y: f32, drag: bool) -> bool {
        let [left, top, cell] = self.at;
        let side = own_buttons::SIZE as f32;
        if !drag {
            self.last = None;
            for i in 0..PALETTE.len() {
                let (sx, sy) = self.swatch(i);
                if ui::hovered((x, y), sx, sy, 14.0, 14.0) {
                    self.color = i;
                    return true;
                }
            }
        }
        if !ui::hovered((x, y), left, top, side * cell, side * cell) {
            // A finger sliding off lifts the stroke, and comes back without a line across.
            self.last = None;
            return drag;
        }
        let now = (((x - left) / cell) as i32, ((y - top) / cell) as i32);
        // Every pixel between the finger's last place and this.
        let (x0, y0) = self.last.unwrap_or(now);
        let steps = (now.0 - x0).abs().max((now.1 - y0).abs()).max(1);
        for t in 0..=steps {
            self.paint(x0 + (now.0 - x0) * t / steps, y0 + (now.1 - y0) * t / steps);
        }
        self.last = Some(now);
        true
    }

    pub(super) fn draw(&self, batch: &mut HudBatch, atlas: &HudAtlas) {
        let [left, top, cell] = self.at;
        let side = own_buttons::SIZE as usize;
        let span = side as f32 * cell;
        batch.fill(
            atlas,
            left - 1.0,
            top - 1.0,
            span + 2.0,
            span + 2.0,
            rgba(16, 16, 16, 255),
        );
        for y in 0..side {
            for x in 0..side {
                let (px, py) = (left + x as f32 * cell, top + y as f32 * cell);
                // Squares under what is clear, so it shows as clear.
                let g = if (x + y) % 2 == 0 { 96 } else { 72 };
                batch.fill(atlas, px, py, cell, cell, rgba(g, g, g, 255));
                let p = self.pixels.get((y * side + x) * 4..(y * side + x) * 4 + 4);
                if let Some(&[r, g, b, a]) = p
                    && a > 0
                {
                    batch.fill(atlas, px, py, cell, cell, rgba(r, g, b, a));
                }
            }
        }
        for (i, [r, g, b, a]) in PALETTE.iter().copied().enumerate() {
            let (x, y) = self.swatch(i);
            let edge = if i == self.color {
                u32::MAX
            } else {
                rgba(16, 16, 16, 255)
            };
            batch.fill(atlas, x - 1.0, y - 1.0, 16.0, 16.0, edge);
            let face = if a == 0 {
                rgba(84, 84, 84, 255)
            } else {
                rgba(r, g, b, a)
            };
            batch.fill(atlas, x, y, 14.0, 14.0, face);
        }
    }
}

/// The catalog's shelf a page's Catalog button opens. This crate can't depend on
/// `starling-catalog`, so the game maps it to the catalog's own (game/options.rs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogShelf {
    Mods,
    ResourcePacks,
    ShaderPacks,
}

#[derive(Clone, Debug)]
pub struct AccountCape {
    pub id: String,
    pub name: String,
    pub url: String,
    pub active: bool,
}

/// What Skin Customization asks of the game (`OptionsUi::take_skin_actions`), apart from
/// [`Event`]: the menus before the game, which don't show the page, need not know them.
#[derive(Clone, Debug, PartialEq)]
pub enum SkinAction {
    /// The name, checked (`offline_name::is_valid_choice`): keep it.
    Name(String),
    /// The skin or model changed: keep it and put it on.
    Choice(crate::own_skin::SkinChoice),
    Cape(crate::own_skin::CapeChoice),
    PickCape,
    LoadAccountCapes,
    SaveWardrobe {
        cape: Option<Option<String>>,
        skin: Option<crate::own_skin::SkinChoice>,
    },
    /// The skins folder (on a phone without the Starling folder, its offer).
    OpenFolder,
    OpenWardrobeFolder,
    /// Ask the system for a skin file; it arrives through the import inbox.
    PickFile,
    /// Ely.by Skins turned on or off (`elyby`): keep it.
    ElyBy(bool),
    /// This skin to the playing Microsoft account's profile, for every server to show.
    Upload(crate::own_skin::SkinChoice),
    /// Ely.by's page where the playing Ely.by account changes its skin.
    ElySite,
}

/// The Skin Customization page: the name being typed and the skins to choose from, read from
/// beside options.txt when the page opens.
pub(super) struct SkinPage {
    pub(super) name: ui::EditBox,
    /// The name worlds open with now (`offline_name::for_worlds`).
    pub(super) saved: String,
    /// The signed-in account's name, which servers get.
    pub(super) account: Option<String>,
    /// The PNGs of `skins/` (`own_skin::list`).
    pub(super) files: Vec<String>,
    pub(super) capes: bool,
    pub(super) angle: f32,
    pub(super) drag_x: Option<f32>,
    pub(super) auto_rotate_since: Option<Instant>,
    pub(super) thumbnails:
        std::cell::RefCell<std::collections::HashMap<String, Option<crate::own_skin::Thumbnail>>>,
    pub(super) choice: crate::own_skin::SkinChoice,
    /// Ely.by Skins (`elyby::enabled`).
    pub(super) elyby: bool,
    /// A name refused, or a skin the game could not put on.
    pub(super) warning: Option<String>,
    /// For the game, oldest first.
    pub(super) actions: Vec<SkinAction>,
}

impl Default for SkinPage {
    fn default() -> Self {
        let mut name = ui::EditBox::new(0.0, 0.0, 270.0, 20.0);
        name.max_length = 16;
        name.focused = false;
        Self {
            name,
            saved: String::new(),
            account: None,
            files: Vec::new(),
            capes: false,
            angle: 0.0,
            drag_x: None,
            auto_rotate_since: None,
            thumbnails: Default::default(),
            choice: Default::default(),
            elyby: true,
            warning: None,
            actions: Vec::new(),
        }
    }
}

/// The longest line of a Mods page note, in characters: about a button's width.
pub(super) const NOTE_CHARS: usize = 52;

/// `text` in lines of at most `max` characters, broken between words.
pub(super) fn wrap_words(text: &str, max: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > max {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Android: the Mods page's game with native mods (`ModList::native_game`): its lines, cut to the
/// list's width, its buttons two to a row, and a gap before the mods.
pub(super) fn native_game_rows(s: &Settings) -> Vec<Row> {
    let Some(game) = &s.mods.native_game else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for (text, warning) in &game.lines {
        for line in wrap_words(text, NOTE_CHARS) {
            rows.push(if *warning {
                Row::Warning(line)
            } else {
                Row::Note(line)
            });
        }
    }
    let widget = |(label, act): &(String, crate::mod_list::NativeGameAct), w: f32| {
        button(
            0.0,
            0.0,
            w,
            Text::literal(label.clone()),
            Act::NativeGame(*act),
        )
    };
    for pair in game.buttons.chunks(2) {
        rows.push(match pair {
            [a, b] => Row::Buttons(widget(a, 150.0), Some(widget(b, 150.0))),
            _ => Row::Buttons(widget(&pair[0], 310.0), None),
        });
    }
    rows
}

/// Starling's Mods page (`Page::NativeGame`, which the Mods buttons open: a phone has no launcher
/// to choose the mods): which game runs and its buttons (Android), then every mod's switch. A
/// native mod is a box ticked when it is chosen for the next build (the buttons' Compile builds
/// with those); one chosen and not in this game says so beside it, and one in it with settings
/// has Settings there. Upstream's Mods screen (`Page::Mods`) has no way in from here: it has no
/// Compile, and players who found it saw their mods "not in this game yet" with nothing to do.
pub(super) fn native_game_page_rows(s: &Settings) -> Vec<Row> {
    let mut rows = native_game_rows(s);
    if s.mods.installed.is_empty() {
        rows.push(Row::Header("sunrise.mods.empty"));
        return rows;
    }
    let mods = s.mods.installed.iter().enumerate().map(|(i, installed)| {
        let on = s.mods.is_on(i);
        let not_built = installed.problem.as_deref() == Some(crate::mod_list::NOT_BUILT);
        let compiled_here = not_built && installed.native.is_some();
        let chosen_box = installed.native.is_some() && !installed.id.is_empty();
        let mut label = if chosen_box {
            let tick = if on { '\u{2611}' } else { '\u{2610}' };
            Text::literal(format!("{tick} {}", installed.title))
        } else {
            Text::literal(format!("{}: ", installed.title)).append(on_off(on))
        };
        if not_built {
            if !compiled_here {
                label = label.append(tr("sunrise.mods.not_built"));
            }
        } else if installed.problem.is_some() {
            label = label.append(tr("sunrise.mods.stopped"));
        } else if installed.dev {
            label = label.append(tr("sunrise.mods.dev"));
        }
        let dead = installed.dev || installed.id.is_empty();
        // Beside it: a warning, as Java's, for a native mod chosen and not in this game (nothing
        // to press: Compile is the page's), Settings for one in it that has settings.
        let side = if compiled_here {
            on.then(|| {
                let warning = Text::literal(format!(
                    "\u{a7}e\u{26a0} {}",
                    lang::get("starling.mods.native.not_compiled")
                ));
                inactive(button(0.0, 0.0, 150.0, warning, Act::ModSettings(i)))
            })
        } else {
            (installed.native.as_ref())
                .is_some_and(|page| !page.settings.is_empty())
                .then(|| {
                    let settings = tr("sunrise.mods.settings");
                    button(0.0, 0.0, 150.0, settings, Act::ModSettings(i))
                })
        };
        if let Some(side) = side {
            let toggle = button(0.0, 0.0, 150.0, label, Act::ToggleMod(i));
            Row::Buttons(if dead { inactive(toggle) } else { toggle }, Some(side))
        } else {
            let widget = button(0.0, 0.0, 310.0, label, Act::ToggleMod(i));
            Row::Buttons(if dead { inactive(widget) } else { widget }, None)
        }
    });
    rows.extend(mods);
    rows
}

/// The GPU Drivers page (Android arm64, #470): the system driver or one installed driver,
/// applied at the next start, the notice when a driver was set aside for crashing starts, and
/// Import... for a package.
pub(super) fn gpu_driver_rows() -> Vec<Row> {
    let drivers = crate::gpu_drivers::list();
    let selected = crate::gpu_drivers::selected();
    let active = crate::gpu_drivers::active().map(|(id, _)| id.to_owned());
    let mut rows = vec![Row::Paragraph(
        tr("sunrise.pages.gpu_drivers.warning").plain(),
    )];
    if let Some(notice) = crate::gpu_drivers::take_notice() {
        rows.push(Row::Paragraph(notice));
    }
    if let Some(id) = &active
        && let Some(driver) = drivers.iter().find(|d| &d.id == id)
    {
        rows.push(Row::Note(
            Text::translate(
                "sunrise.pages.gpu_drivers.playing",
                &[Text::literal(driver.label())],
            )
            .plain(),
        ));
    }
    if selected != active {
        rows.push(Row::Note(tr("options.restartRequired").plain()));
        rows.push(Row::Buttons(
            button(
                0.0,
                0.0,
                310.0,
                tr("sunrise.pages.gpu_drivers.restart"),
                Act::RestartGame,
            ),
            None,
        ));
    }
    let marked = |on: bool, name: String| if on { format!("> {name} <") } else { name };
    rows.push(Row::Buttons(
        button(
            0.0,
            0.0,
            310.0,
            Text::literal(marked(
                selected.is_none(),
                tr("sunrise.pages.gpu_drivers.system").plain(),
            )),
            Act::PickGpuDriver(String::new()),
        ),
        None,
    ));
    for driver in &drivers {
        let on = selected.as_deref() == Some(driver.id.as_str());
        rows.push(Row::Buttons(
            button(
                0.0,
                0.0,
                150.0,
                Text::literal(marked(on, driver.label())),
                Act::PickGpuDriver(driver.id.clone()),
            ),
            Some(button(
                0.0,
                0.0,
                150.0,
                tr("sunrise.pages.gpu_drivers.delete"),
                Act::DeleteGpuDriver(driver.id.clone()),
            )),
        ));
    }
    rows.push(Row::Buttons(
        button(
            0.0,
            0.0,
            310.0,
            tr("sunrise.pages.gpu_drivers.import"),
            Act::ImportGpuDriver,
        ),
        None,
    ));
    if let Some(id) = &selected
        && let Some(driver) = drivers.iter().find(|d| &d.id == id)
        && !driver.meta.description.is_empty()
    {
        rows.push(Row::Paragraph(driver.meta.description.clone()));
    }
    rows
}

/// The focused text field as the on-screen keyboard's editor on Android mirrors it
/// (`OptionsUi::focused_text`).
pub struct FocusedText {
    /// Which field: 0 the language search, 1 the settings search, 2 the pack search, 3 Skin
    /// Customization's name, 4 a custom button's text.
    pub index: usize,
    /// It searches a list.
    pub search: bool,
    pub value: String,
    /// The cursor and the selection's other end, in characters.
    pub selection: (usize, usize),
    /// The most characters it takes.
    pub max: usize,
}

impl OptionsUi {
    /// The chosen pack's profiles (`profile.<name>` in its shaders.properties, in
    /// `shaderpacks/` beside `options.txt`), for the Shader Packs page.
    pub(super) fn refresh_shader_profiles(&mut self, s: &Settings) {
        self.shader.screen.refresh_profiles(s);
    }

    /// The finger lifted from a touch screen: the widget it tapped keeps no focus, so neither its
    /// highlight nor its tooltip stays there. A keyboard's or a controller's focus stays.
    pub fn drop_pointer_focus(&mut self) {
        if !self.focus_from_keyboard {
            self.focus = None;
        }
    }

    /// Whether `cursor` is on a button that a finger moving from it should scroll past rather than
    /// press (touch.rs, `prefer_scroll`): any active button or language row, and on Video Settings
    /// any row but a slider's. Sliders keep their sideways drag, and the pages with drags of their
    /// own (packs, the canvas, calibration, split screen) are left as they are.
    pub fn list_button_at(&mut self, s: &Settings, size: (f32, f32), cursor: (f32, f32)) -> bool {
        if self.resource_error.is_some() || self.painting() {
            return false;
        }
        // Upstream's settings shell lays these pages' rows out.
        if let Some(shell) = self.shell_mut() {
            return shell.pressable_at(s, size, cursor);
        }
        if matches!(self.page(), Page::Packs | Page::HdrCalibration) {
            return false;
        }
        let (widgets, top, bottom, _) = self.layout(s, size.0, size.1);
        widgets.iter().any(|wd| {
            matches!(
                wd.kind,
                WKind::Button { active: true, .. } | WKind::LanguageRow { .. }
            ) && ui::hovered(cursor, wd.x, wd.y, wd.w, wd.h)
                && !(wd.list && (cursor.1 < top || cursor.1 >= bottom))
        })
    }

    /// Whether `cursor` is on a slider of the settings shell's pages, which a finger takes only
    /// moving clearly sideways (touch.rs, `prefer_slider`).
    pub fn list_slider_at(&mut self, s: &Settings, size: (f32, f32), cursor: (f32, f32)) -> bool {
        if self.resource_error.is_some() || self.painting() {
            return false;
        }
        self.shell_mut()
            .is_some_and(|shell| shell.slider_at(s, size, cursor))
    }

    /// A lifted finger no longer hovers a button. Text editing retains its focus.
    pub fn release_touch_focus(&mut self) {
        if !self.focus_from_keyboard && self.focus != Some(FocusId::Search) {
            self.focus = None;
        }
    }

    /// A new custom button's page. False with every slot taken.
    pub(super) fn custom_add(&mut self, s: &mut Settings) -> bool {
        // A button's id is the slot of its image: the first one free.
        let free = (0..own_buttons::SLOTS).find(|id| !s.custom_buttons.iter().any(|b| b.id == *id));
        let Some(id) = free else {
            return false;
        };
        s.custom_buttons.push(crate::settings::CustomButton {
            id,
            action: crate::settings::CustomAction::Send(String::new()),
            icon: None,
            image: false,
            pressed: false,
            toggle: false,
        });
        self.custom_send.set_value("");
        self.custom_send.focused = false;
        self.push(Page::CustomButton(id));
        true
    }

    /// The touch editor's Add: a new button's page, over the page of them all; done with it, the
    /// editor again.
    pub fn add_custom_button(&mut self, s: &mut Settings) {
        if self.page() != Page::CustomButtons {
            self.push(Page::CustomButtons);
        }
        self.back_to_editor = self.custom_add(s);
    }

    /// Whether a custom button's canvas shows: a finger on the page drags, it never scrolls.
    pub fn painting(&self) -> bool {
        matches!(self.page(), Page::CustomDraw(_))
    }

    /// A picture for the canvas showing now.
    pub fn import_button_image(&mut self, width: u32, height: u32, rgba: &[u8]) {
        if self.painting() {
            self.painter.import(width, height, rgba);
        }
    }
}

impl OptionsUi {
    /// Whether a text field shown now has the focus: the language and settings searches, the
    /// pack search, or Skin Customization's name.
    pub fn text_focused(&self) -> bool {
        match self.page() {
            Page::Language => self.language_search.focused,
            Page::Packs => self
                .pack_screen
                .as_ref()
                .is_some_and(|p| p.search_focused()),
            Page::Skin => self.profile_name.is_none() && self.skin.name.focused,
            Page::CustomButton(_) => self.custom_send.focused,
            _ => self.settings_search_active(),
        }
    }

    /// Whether `(x, y)` (GUI pixels) is on the focused text field shown now.
    pub fn text_field_at(&self, x: f32, y: f32) -> bool {
        let on = |e: &ui::EditBox| {
            e.focused && x >= e.x && x < e.x + e.width && y >= e.y && y < e.y + e.height
        };
        match self.page() {
            Page::Language => on(&self.language_search),
            Page::Packs => self.pack_screen.as_ref().is_some_and(|p| p.search_at(x, y)),
            Page::Skin => self.profile_name.is_none() && on(&self.skin.name),
            Page::CustomButton(_) => on(&self.custom_send),
            _ => self.settings_search_active() && on(&self.settings_search),
        }
    }

    /// The text field that has the focus (`text_focused`), for the on-screen keyboard's editor
    /// on Android.
    pub fn focused_text(&self) -> Option<FocusedText> {
        if !self.text_focused() {
            return None;
        }
        let edit = |e: &ui::EditBox| (e.value.clone(), (e.cursor, e.highlight), e.max_length);
        let (index, (value, selection, max)) = match self.page() {
            Page::Language => (0, edit(&self.language_search)),
            Page::Packs => {
                let s = self.pack_screen.as_ref()?.search_field();
                (2, (s.value.clone(), (s.cursor, s.highlight), s.max_length))
            }
            Page::Skin => (3, edit(&self.skin.name)),
            Page::CustomButton(_) => (4, edit(&self.custom_send)),
            _ => (1, edit(&self.settings_search)),
        };
        Some(FocusedText {
            index,
            search: index <= 2,
            value,
            selection,
            max,
        })
    }

    /// The on-screen keyboard's editor's text and selection on the focused field (Android),
    /// through the field's filter and limit as typing would leave them: a position after a
    /// character the filter drops moves back with it. A search's list goes back to its top
    /// when its text changed, as after a typed key.
    pub fn set_focused_text(&mut self, atlas: &HudAtlas, text: &str, cursor: usize, anchor: usize) {
        if !self.text_focused() {
            return;
        }
        let kept = |pos: usize| {
            let before: String = text.chars().take(pos).collect();
            crate::clipboard::filter_text(&before, false)
                .chars()
                .count()
        };
        let (cursor, anchor) = (kept(cursor), kept(anchor));
        let value = crate::clipboard::filter_text(text, false);
        let page = self.page();
        let edit = match page {
            Page::Packs => {
                if let Some(p) = &mut self.pack_screen {
                    p.set_search(
                        &crate::screen_draw::Measure(&atlas.font),
                        &value,
                        cursor,
                        anchor,
                    );
                }
                return;
            }
            Page::Language => &mut self.language_search,
            Page::Skin => &mut self.skin.name,
            Page::CustomButton(_) => &mut self.custom_send,
            _ => &mut self.settings_search,
        };
        let changed = value != edit.value;
        if changed {
            edit.set_value(&value);
        }
        edit.set_cursor(anchor, false);
        edit.set_cursor(cursor, true);
        if changed && !matches!(page, Page::Skin | Page::CustomButton(_)) {
            self.scroll = 0.0;
        }
    }

    /// Read the Skin Customization page's name, skins and choice again (it opened, or a skin
    /// came through the import inbox).
    pub fn refresh_skin_page(&mut self) {
        let dir = crate::logging::player_dir();
        let page = &mut self.skin;
        page.saved = self
            .profile_name
            .clone()
            .unwrap_or_else(|| crate::offline_name::for_worlds(&dir));
        if self.profile_name.is_some() {
            page.name.focused = false;
        }
        page.account = crate::auth::current().map(|a| a.name.clone());
        page.files = crate::own_skin::list(&dir.join(crate::own_skin::DIR));
        if !self.wardrobe_skin_titles.is_empty() {
            page.files.sort_by_cached_key(|file| {
                self.wardrobe_skin_titles
                    .get(file)
                    .map_or(file.as_str(), String::as_str)
                    .to_lowercase()
            });
        }
        page.thumbnails.get_mut().clear();
        page.choice = if self.wardrobe_enabled {
            crate::own_skin::SkinChoice::for_game(&dir, self.profile_key.as_deref())
        } else {
            crate::own_skin::SkinChoice::for_profile(&dir, self.profile_key.as_deref())
        };
        page.elyby = crate::elyby::enabled(&dir);
        page.warning = None;
        if !page.name.focused {
            page.name.set_value(&page.saved);
        }
        self.relayout();
    }

    /// A line in red on the Skin Customization page (a skin the game could not put on).
    pub fn skin_warning(&mut self, text: String) {
        self.skin.warning = Some(text);
        self.relayout();
    }

    /// What Skin Customization asked of the game since the last call.
    pub fn take_skin_actions(&mut self) -> Vec<SkinAction> {
        std::mem::take(&mut self.skin.actions)
    }

    pub(super) fn select_skin(&mut self, file: Option<String>) {
        if self.skin.choice.file != file {
            self.pending_skin_upload = false;
            self.skin.choice.file = file;
            self.skin.warning = None;
            self.skin
                .actions
                .push(SkinAction::Choice(self.skin.choice.clone()));
        }
    }

    pub(super) fn select_wardrobe_tab(&mut self, capes: bool) {
        self.skin.capes = capes;
        self.finish_wardrobe_drag();
        self.skin.angle = 0.0;
        self.skin.auto_rotate_since = self.skin.auto_rotate_since.map(|_| Instant::now());
        self.scroll = 0.0;
    }

    pub fn viewing_wardrobe_capes(&self) -> bool {
        self.page() == Page::Skin && self.skin.capes
    }

    pub fn wardrobe_rotation(&self) -> f32 {
        let facing = if self.skin.capes { 180.0 } else { 22.5 };
        facing + self.wardrobe_angle()
    }

    pub(super) fn wardrobe_angle(&self) -> f32 {
        let spin = self
            .skin
            .auto_rotate_since
            .filter(|_| self.skin.drag_x.is_none())
            .map_or(0.0, |since| since.elapsed().as_secs_f32() * 36.0);
        (self.skin.angle + spin).rem_euclid(360.0)
    }

    pub(super) fn toggle_wardrobe_auto_rotate(&mut self) {
        self.skin.angle = self.wardrobe_angle();
        self.skin.auto_rotate_since = self.skin.auto_rotate_since.is_none().then(Instant::now);
    }

    pub fn wardrobe_preview_at(&self, size: (f32, f32), cursor: (f32, f32)) -> bool {
        self.wardrobe_preview_rect(size.0, size.1)
            .is_some_and(|[x0, y0, x1, y1]| ui::hovered(cursor, x0, y0, x1 - x0, y1 - y0))
    }

    pub(super) fn finish_wardrobe_drag(&mut self) {
        if self.skin.drag_x.take().is_some() && self.skin.auto_rotate_since.is_some() {
            self.skin.auto_rotate_since = Some(Instant::now());
        }
    }

    pub fn discard_wardrobe_changes(&mut self) {
        self.wardrobe_sync_started = None;
        self.wardrobe_sync_failed = false;
        self.pending_account_cape = None;
        self.pending_skin_upload = false;
    }

    /// The outer option distinguishes no edit from a request to unequip the cape.
    pub fn pending_account_cape(&self) -> Option<Option<&AccountCape>> {
        match self.pending_account_cape.as_ref()? {
            Some(id) => self
                .wardrobe_account_capes
                .as_ref()?
                .iter()
                .find(|cape| cape.id == *id)
                .map(Some),
            None => Some(None),
        }
    }

    pub(super) fn stage_account_cape(&mut self, id: Option<String>) {
        if let Some(capes) = &self.wardrobe_account_capes {
            let active = capes.iter().find(|cape| cape.active).map(|cape| &cape.id);
            self.pending_account_cape = (id.as_ref() != active).then_some(id);
            self.skin.warning = None;
            self.relayout();
        }
    }

    pub(super) fn queue_skin_upload(&mut self) {
        self.skin.warning = None;
        if self.wardrobe_enabled {
            self.pending_skin_upload = true;
            self.relayout();
        } else {
            self.skin
                .actions
                .push(SkinAction::Upload(self.skin.choice.clone()));
        }
    }

    pub fn wardrobe_saving(&self) -> bool {
        self.wardrobe_sync_started.is_some()
    }

    pub(super) fn wardrobe_sync_stage(&self) -> &'static str {
        if self.pending_account_cape.is_some() {
            "starling.wardrobe.syncing_cape"
        } else {
            "starling.wardrobe.syncing_skin"
        }
    }

    pub(super) fn wardrobe_sync_pending(&self) -> bool {
        self.pending_account_cape.is_some() || self.pending_skin_upload
    }

    pub(super) fn start_wardrobe_sync(&mut self) {
        if self.wardrobe_saving() || !self.wardrobe_sync_pending() {
            return;
        }
        let cape = self.pending_account_cape.clone();
        let skin = self.pending_skin_upload.then(|| self.skin.choice.clone());
        self.wardrobe_sync_started = Some(Instant::now());
        self.wardrobe_sync_failed = false;
        self.skin.warning = None;
        self.skin
            .actions
            .push(SkinAction::SaveWardrobe { cape, skin });
        self.relayout();
    }

    pub(super) fn done(&mut self, s: &mut Settings) -> Vec<Event> {
        if self.page() == Page::Skin && self.wardrobe_enabled {
            if self.wardrobe_saving() {
                return Vec::new();
            }
            if self.wardrobe_sync_pending() && !self.wardrobe_sync_failed {
                self.start_wardrobe_sync();
                return Vec::new();
            }
        }
        self.back(s)
    }

    pub fn wardrobe_cape_saved(&mut self) {
        self.pending_account_cape = None;
    }

    pub fn finish_wardrobe_save(
        &mut self,
        settings: &mut Settings,
        result: Result<(), String>,
    ) -> Vec<Event> {
        if self.wardrobe_sync_started.take().is_none() || self.page() != Page::Skin {
            return Vec::new();
        }
        match result {
            Ok(()) => self.back(settings),
            Err(error) => {
                self.wardrobe_sync_failed = true;
                self.scroll = 0.0;
                self.skin_warning(lang::get_or(&error, &error));
                Vec::new()
            }
        }
    }

    pub(super) fn select_cape(&mut self, choice: crate::own_skin::CapeChoice) {
        self.pending_account_cape = None;
        self.skin.warning = None;
        self.skin.actions.push(SkinAction::Cape(choice));
    }

    pub(super) fn cape_rows(&self) -> Vec<Row> {
        use crate::own_skin::CapeChoice;
        let pick = |choice, label: Text, width| {
            let label = if self.wardrobe_cape == choice {
                label.with_color(0x55_FF_55)
            } else {
                label
            };
            button(0.0, 0.0, width, label, Act::SelectCape(choice))
        };
        let mut rows = vec![
            Row::Buttons(
                pick(
                    CapeChoice::Account,
                    tr("starling.wardrobe.account_cape"),
                    150.0,
                ),
                Some(pick(
                    CapeChoice::Hidden,
                    tr("starling.wardrobe.hide_cape"),
                    150.0,
                )),
            ),
            Row::Buttons(
                button(
                    0.0,
                    0.0,
                    310.0,
                    tr("starling.wardrobe.import_cape"),
                    Act::ImportCape,
                ),
                None,
            ),
        ];
        for pair in self.wardrobe_capes.chunks(2) {
            let card = |(id, title): &(String, String)| {
                pick(
                    CapeChoice::Local(id.clone()),
                    Text::literal(clipped(title, 23)),
                    150.0,
                )
            };
            rows.push(Row::Buttons(card(&pair[0]), pair.get(1).map(card)));
        }
        rows.push(Row::Paragraph(lang::get("starling.wardrobe.local_cape")));
        match &self.skin_cloud {
            crate::own_skin::Cloud::Ely => rows.extend(self.skin_cloud_rows()),
            crate::own_skin::Cloud::Microsoft(_) => {
                if self.wardrobe_account_busy {
                    rows.push(Row::Note(lang::get("starling.wardrobe.loading_capes")));
                } else if let Some(capes) = &self.wardrobe_account_capes {
                    if capes.is_empty() {
                        rows.push(Row::Note(lang::get("starling.wardrobe.no_owned_capes")));
                    }
                    let active = capes.iter().any(|cape| cape.active);
                    if active || self.pending_account_cape.is_some() {
                        let remove = tr("starling.wardrobe.remove_account_cape");
                        let remove = if self.pending_account_cape == Some(None) {
                            remove.with_color(0x55_FF_55)
                        } else {
                            remove
                        };
                        rows.push(Row::Buttons(
                            button(0.0, 0.0, 310.0, remove, Act::ActivateCape(None)),
                            None,
                        ));
                    }
                    for cape in capes {
                        let pending = self.pending_account_cape.as_ref().and_then(Option::as_ref)
                            == Some(&cape.id);
                        let label = Text::translate(
                            if cape.active {
                                "starling.wardrobe.active_cape"
                            } else {
                                "starling.wardrobe.activate_cape"
                            },
                            &[Text::literal(clipped(&cape.name, 24))],
                        );
                        let widget = button(
                            0.0,
                            0.0,
                            310.0,
                            if pending {
                                label.with_color(0x55_FF_55)
                            } else {
                                label
                            },
                            Act::ActivateCape(Some(cape.id.clone())),
                        );
                        rows.push(Row::Buttons(
                            if cape.active && self.pending_account_cape.is_none() {
                                inactive(widget)
                            } else {
                                widget
                            },
                            None,
                        ));
                    }
                    if self.pending_account_cape.is_some() {
                        rows.push(Row::Note(lang::get("starling.wardrobe.sync_on_done")));
                    }
                } else {
                    rows.push(Row::Buttons(
                        button(
                            0.0,
                            0.0,
                            310.0,
                            tr("starling.wardrobe.load_capes"),
                            Act::FetchCapes,
                        ),
                        None,
                    ));
                }
            }
            crate::own_skin::Cloud::Local => {}
        }
        rows
    }

    /// What the skin button calls no file of `skins/`: an account's own skin, else the default.
    pub(super) fn skin_default_name(&self) -> String {
        lang::get(match self.skin_cloud {
            crate::own_skin::Cloud::Local => "starling.skin.default",
            _ => "starling.skin.account",
        })
    }

    /// Skin Customization's rows for the playing profile's account, as Horizon Android's wardrobe
    /// has them: a Microsoft account's upload of the chosen skin to its profile, and how the last
    /// one went; an Ely.by account's page on ely.by, where its skin is changed.
    pub(super) fn skin_cloud_rows(&self) -> Vec<Row> {
        use crate::own_skin::{Cloud, Upload};
        let chosen = self.skin.choice.file.is_some();
        match &self.skin_cloud {
            Cloud::Local => Vec::new(),
            Cloud::Ely => vec![
                Row::Buttons(
                    button(
                        0.0,
                        0.0,
                        310.0,
                        tr("starling.skin.ely_site"),
                        Act::ElySkinSite,
                    ),
                    None,
                ),
                Row::Note(lang::get("starling.skin.ely_hint")),
            ],
            Cloud::Microsoft(upload) => {
                let sending = *upload == Upload::Sending;
                let label = tr(if sending {
                    "starling.skin.uploading"
                } else {
                    "starling.skin.upload"
                });
                let upload_button = button(0.0, 0.0, 310.0, label, Act::UploadSkin);
                let upload_button = if chosen && !sending {
                    upload_button
                } else {
                    inactive(upload_button)
                };
                let note = match upload {
                    Upload::Failed(why) => Row::Warning(lang::get_or(why, why)),
                    _ if self.pending_skin_upload => {
                        Row::Note(lang::get("starling.wardrobe.sync_on_done"))
                    }
                    Upload::Sent => Row::Note(lang::get("starling.skin.uploaded")),
                    _ if !chosen => Row::Note(lang::get("starling.skin.upload_choose")),
                    _ => Row::Note(lang::get("starling.skin.upload_hint")),
                };
                vec![Row::Buttons(upload_button, None), note]
            }
        }
    }

    /// The name box loses the focus: a changed name is kept when it is one, else refused and
    /// the old one back in the box.
    pub(super) fn commit_name(&mut self) {
        if self.profile_name.is_some() {
            self.skin.name.focused = false;
            return;
        }
        let page = &mut self.skin;
        page.name.focused = false;
        let typed = page.name.value.clone();
        if typed == page.saved {
            return;
        }
        if crate::offline_name::is_valid_choice(&typed) {
            page.saved = typed.clone();
            page.warning = None;
            page.actions.push(SkinAction::Name(typed));
            return;
        }
        page.warning = Some(format!(
            "Not a name: 3 to 16 letters, digits or _ ({} kept)",
            page.saved
        ));
        page.name.set_value(&page.saved);
    }

    /// The game is up, so on a phone the pages can open its catalog.
    pub fn offer_catalog(&mut self) {
        self.catalog_buttons = ON_A_PHONE;
    }
}

impl crate::sunrise_ui::Tab {
    /// Starling: a tab of Video Settings..., which opens Sunrise Settings' graphics pages and the
    /// packs' options, as Starling's video screen was.
    pub(crate) fn video(self) -> bool {
        matches!(self, Self::Graphics(_) | Self::PackOptions)
    }
}

/// Starling: whether Sunrise Settings opened as `page` is Video Settings..., with the graphics
/// tabs alone (`SunriseScreen::video`); Sunrise Settings... and Gameplay show all of them.
pub(super) fn video_tabs(page: Page) -> bool {
    !matches!(page, Page::Sunrise | Page::Extras)
}
