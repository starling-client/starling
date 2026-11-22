//! The options pages' built layout: every widget of a page, the list's top
//! and bottom and its content height, made once per size and revision
//! (`OptionsUi::layout`) and read by the drawing, the hit tests and the scroll.
use super::*;

/// A page's built layout: its widgets, the list's top and bottom and its
/// content height. `OptionsUi` makes it once per size and revision; every
/// reader of the page's rectangles reads this.
#[derive(Clone, Debug, PartialEq)]
pub struct OptionsLayout {
    widgets: Vec<Widget>,
    top: f32,
    bottom: f32,
    content: f32,
}

impl OptionsLayout {
    /// The layout's parts: widgets, list top, list bottom, content height.
    pub(super) fn parts(&self) -> (&[Widget], f32, f32, f32) {
        (&self.widgets, self.top, self.bottom, self.content)
    }
}
impl OptionsUi {
    /// The preview and the options list share the available width.
    pub(super) fn wardrobe_columns(&self, width: f32) -> Option<(f32, f32, f32)> {
        if self.page() != Page::Skin || !self.wardrobe_enabled {
            return None;
        }
        let total = (width - 26.0).min(480.0);
        let preview = (total * 0.33).floor();
        Some(((width - total) / 2.0, preview, total - preview - 12.0))
    }

    pub fn wardrobe_preview_rect(&self, width: f32, height: f32) -> Option<[f32; 4]> {
        let (left, preview, _) = self.wardrobe_columns(width)?;
        let bottom = height - 59.0;
        (bottom > 33.0).then_some([left, 33.0, left + preview, bottom])
    }

    /// Something the page layout reads changed: make it again at the next
    /// read, once.
    pub(super) fn relayout(&mut self) {
        self.discard_draw();
        self.rev = self.rev.wrapping_add(1);
    }
    /// The game changed state the page layout reads (the mod browser's
    /// listings arriving, say): the next read makes the layout once.
    pub fn layout_changed(&mut self) {
        self.relayout();
    }
    /// The Agent page's status lines and recording flag, from the game's
    /// poll: the page lays out again only when one of them changed.
    pub fn set_agent_status(&mut self, status: Vec<String>, recording: bool) {
        if self.agent_status != status || self.agent_recording != recording {
            self.agent_status = status;
            self.agent_recording = recording;
            self.relayout();
        }
    }
    /// The calibration page's lines and buttons, from the game while it shows: the page lays out
    /// again only when one of them changed, so a tap lands on the buttons it shows.
    pub fn set_calibrate(&mut self, notes: Vec<String>, buttons: Vec<String>) {
        if self.calibrate_notes != notes || self.calibrate != buttons {
            self.calibrate_notes = notes;
            self.calibrate = buttons;
            self.relayout();
        }
    }
    /// The page's layout at `(w, h)`: made on the first read after an open, a
    /// resize or a revision change, then read as is.
    pub(super) fn built_layout(
        &mut self,
        s: &Settings,
        w: f32,
        h: f32,
    ) -> std::sync::Arc<OptionsLayout> {
        // The scroll is an input too: a drag, the wheel or a focus move can
        // change it without a revision bump, and the rows move with it.
        let key = (w, h, self.rev, self.scroll.to_bits());
        if let Some((cached, layout)) = &self.layout_cache
            && *cached == key
        {
            return std::sync::Arc::clone(layout);
        }
        let (widgets, top, bottom, content) = self.layout(s, w, h);
        // `layout` clamps the scroll; key the settled value.
        let key = (w, h, self.rev, self.scroll.to_bits());
        let layout = std::sync::Arc::new(OptionsLayout {
            widgets,
            top,
            bottom,
            content,
        });
        self.layout_cache = Some((key, std::sync::Arc::clone(&layout)));
        #[cfg(test)]
        {
            self.layouts_built += 1;
        }
        layout
    }
    /// The layout built straight from the current state, for the caching
    /// test: the cached one must equal this after every operation.
    #[cfg(test)]
    pub(super) fn layout_now(&mut self, s: &Settings, w: f32, h: f32) -> OptionsLayout {
        let (widgets, top, bottom, content) = self.layout(s, w, h);
        OptionsLayout {
            widgets,
            top,
            bottom,
            content,
        }
    }
    /// Build every widget of the current page.
    pub(super) fn layout(&mut self, s: &Settings, w: f32, h: f32) -> (Vec<Widget>, f32, f32, f32) {
        let mut out = Vec::new();
        let mut rows = ListRows::default();
        let page = self.page();
        let cx = (w / 2.0).floor();
        if matches!(
            page,
            Page::Video | Page::Controls | Page::KeyBinds | Page::ModBrowser
        ) {
            self.settings_search.x = cx - 100.0;
            self.settings_search.y = 17.0;
        }
        let done = |y: f32| button(cx - 100.0, y, 200.0, tr("gui.done"), Act::Done);
        // (list top, list bottom, content height)
        let (mut top, mut bottom, mut content) = (33.0, h - 33.0, 0.0);
        match page {
            Page::Main => {
                out.push(Widget {
                    x: 0.0,
                    y: 12.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr("options.title"),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                out.push(option_widget(s, Opt::Fov, cx - 154.0, 29.0, 150.0));
                out.push(button(
                    cx + 4.0,
                    29.0,
                    150.0,
                    tr("options.online"),
                    Act::Push(Page::Online),
                ));
                let entries = [
                    // Everywhere for the layers and Main Hand; `skin_page` adds the name and skin.
                    ("options.skinCustomisation", Some(Page::Skin)),
                    ("options.sounds", Some(Page::Sound)),
                    // Video Settings... opens Horizon's graphics screen, which holds every
                    // video option, as Sodium's replaces vanilla's.
                    ("options.video", Some(Page::Graphics)),
                    ("options.controls", Some(Page::Controls)),
                    (
                        "options.language",
                        (!self.split_guest).then_some(Page::Language),
                    ),
                    ("options.chat", Some(Page::Chat)),
                    (
                        "options.resourcepack",
                        (!self.split_guest).then_some(Page::Packs),
                    ),
                    ("options.accessibility", Some(Page::Accessibility)),
                    // Vanilla's Telemetry Data and Credits slots: Horizon's
                    // settings and its mods, as a mod adds its button here.
                    // Credits & Attribution (WinScreen's credits) follows them.
                    ("horizon.pages.open.horizon_settings", Some(Page::Horizon)),
                    // Starling's Mods page (`open_at`).
                    ("horizon.pages.open.mods", Some(Page::NativeGame)),
                    ("options.credits_and_attribution", Some(Page::Credits)),
                ];
                // horizon-extras: without it, vanilla's last row instead of Horizon Settings and
                // Mods: Telemetry Data (greyed: nothing is sent) and Credits & Attribution.
                let plain;
                let entries: &[_] = if horizon_extras() {
                    &entries
                } else {
                    plain = [
                        &entries[..entries.len() - 3],
                        &[
                            ("options.telemetry", None),
                            ("options.credits_and_attribution", Some(Page::Credits)),
                        ],
                    ]
                    .concat();
                    &plain
                };
                let entries = with_diagnostics(entries);
                let rows = entries.len().div_ceil(2) as f32;
                let gy = (61.0 + 30.0f32).min(h - 33.0 - rows * 24.0);
                for (i, (key, page)) in entries.iter().enumerate() {
                    let x = cx - 158.0 + 4.0 + (i % 2) as f32 * 158.0;
                    let y = gy + (i / 2) as f32 * 24.0;
                    let b = button(x, y, 150.0, tr(key), page.map_or(Act::None, Act::Push));
                    out.push(if page.is_some() { b } else { inactive(b) });
                }
                out.push(done(h - 27.0));
                top = 0.0;
                bottom = 0.0;
            }
            Page::Credits => {
                // CreditsAndAttributionScreen: the title header, three 210-wide
                // buttons 8 apart (HeaderAndFooterLayout: 30 below the header
                // unless the footer is closer), Done.
                out.push(Widget {
                    x: 0.0,
                    y: 12.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr(Self::title(page)),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                let y = (33.0 + 30.0f32).min(h - 33.0 - 76.0);
                let x = cx - 105.0;
                if ON_A_PHONE {
                    // Starling's founders, then the game it is built on (THIRD_PARTY_NOTICES.md).
                    for (i, key) in ["starling.credits.founders", "starling.credits.horizon"]
                        .into_iter()
                        .enumerate()
                    {
                        out.push(Widget {
                            x: 0.0,
                            y: y - 26.0 + 12.0 * i as f32,
                            w,
                            h: 9.0,
                            kind: WKind::Text {
                                text: tr(key),
                                color: u32::MAX,
                                centered: true,
                            },
                            list: false,
                            row: 0,
                        });
                    }
                }
                out.push(button(
                    x,
                    y,
                    210.0,
                    tr("credits_and_attribution.button.credits"),
                    Act::Credits,
                ));
                // ConfirmLinkScreen to minecraft.net: Horizon opens no links.
                for (i, key) in [
                    "credits_and_attribution.button.attribution",
                    "credits_and_attribution.button.licenses",
                ]
                .into_iter()
                .enumerate()
                {
                    let y = y + 28.0 * (i + 1) as f32;
                    out.push(inactive(button(x, y, 210.0, tr(key), Act::None)));
                }
                // horizon-extras: a build without them says where the official one is.
                if !horizon_extras() {
                    out.push(Widget {
                        x: 0.0,
                        y: (y + 92.0).min(h - 42.0),
                        w,
                        h: 9.0,
                        kind: WKind::Text {
                            text: tr("horizon.plain.official"),
                            color: 0xFFA0A0A0,
                            centered: true,
                        },
                        list: false,
                        row: 0,
                    });
                }
                out.push(done(h - 27.0));
                top = 0.0;
                bottom = 0.0;
            }
            Page::KeyBinds => {
                top = 38.0;
                out.push(Widget {
                    x: 0.0,
                    y: 4.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr(Self::title(page)),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                let keypad = s.keypad.enabled;
                if ON_A_PHONE || keypad {
                    top = 64.0;
                    out.push(button(
                        cx - 150.0,
                        39.0,
                        300.0,
                        Text::literal(if keypad {
                            "Input profile: D-pad / keypad"
                        } else {
                            "Input profile: Keyboard"
                        }),
                        Act::KeyboardProfile,
                    ));
                }
                let label = |name: &str| {
                    crate::keypad::ACTIONS
                        .iter()
                        .find(|a| a.0 == name)
                        .map_or_else(|| lang::get(name), |a| a.1.to_owned())
                };
                let mut mappings: Vec<(&str, &str)> = if keypad {
                    crate::keypad::ACTIONS
                        .iter()
                        .map(|a| (a.0, "movement"))
                        .collect()
                } else {
                    MAPPINGS.iter().map(|(n, _, c)| (*n, *c)).collect()
                };
                let query = self.settings_search.value.to_lowercase();
                mappings.retain(|(name, _)| {
                    query.is_empty() || label(name).to_lowercase().contains(&query)
                });
                mappings.sort_by(|a, b| {
                    let ca = CATEGORIES.iter().position(|c| *c == a.1).unwrap_or(99);
                    let cb = CATEGORIES.iter().position(|c| *c == b.1).unwrap_or(99);
                    ca.cmp(&cb).then_with(|| label(a.0).cmp(&label(b.0)))
                });
                let row_left = cx - 170.0;
                let scrollbar_x = row_left + 340.0 + 6.0 + 2.0;
                // VideoSettingsScreen.addTitle: a changed graphics API needs a restart.
                if page == Page::Video && s.horizon.api != s.startup_api {
                    out.push(Widget {
                        x: 0.0,
                        y: 15.0,
                        w,
                        h: 9.0,
                        kind: WKind::Text {
                            text: tr("options.restartRequired"),
                            color: rgba(223, 80, 80, 255),
                            centered: true,
                        },
                        list: false,
                        row: 0,
                    });
                }
                let list_start = out.len();
                let mut y = top + 2.0 - self.scroll;
                let mut last_category = "";
                for (name, category) in mappings {
                    if category != last_category {
                        last_category = category;
                        rows.begin(&out, y);
                        let label =
                            Text::translate(&format!("key.category.minecraft.{category}"), &[])
                                .with_style(&Style {
                                    bold: Some(true),
                                    underlined: Some(true),
                                    ..Style::default()
                                });
                        out.push(Widget {
                            x: 0.0,
                            y: y + 7.0,
                            w,
                            h: 9.0,
                            kind: WKind::Text {
                                text: label,
                                color: u32::MAX,
                                centered: true,
                            },
                            list: false,
                            row: 0,
                        });
                        y += 20.0;
                        rows.end(&mut out, y);
                    }
                    rows.begin(&out, y);
                    out.push(Widget {
                        x: row_left + 2.0,
                        y: y + 6.0,
                        w: 200.0,
                        h: 9.0,
                        kind: WKind::Text {
                            text: if keypad {
                                Text::literal(label(name))
                            } else {
                                tr(name)
                            },
                            color: u32::MAX,
                            centered: false,
                        },
                        list: false,
                        row: 0,
                    });
                    let key = if keypad {
                        s.keypad
                            .keys(name)
                            .first()
                            .cloned()
                            .unwrap_or_else(|| "key.keyboard.unknown".into())
                    } else {
                        s.keys.get(name).to_owned()
                    };
                    let mut message = keybinds::display_name(&key);
                    let conflicts: Vec<_> = if keypad {
                        crate::keypad::ACTIONS
                            .iter()
                            .filter(|a| {
                                a.0 != name
                                    && s.keypad.keys(a.0).contains(&key)
                                    && key != "key.keyboard.unknown"
                            })
                            .map(|a| a.0.to_owned())
                            .collect()
                    } else {
                        key_conflicts(
                            s,
                            "",
                            name,
                            s.keys.get(name),
                            0,
                            crate::mod_list::KeyConflictContext::Universal,
                            s.keys.is_default(name),
                        )
                    };
                    if !conflicts.is_empty() {
                        message = Text::literal("[ ")
                            .append(message.with_color(text::WHITE))
                            .append(" ]")
                            .with_color(text::YELLOW);
                    }
                    if self.selected_key == Some(name) {
                        message = Text::literal("> ")
                            .append(message.with_style(&Style {
                                color: Some(text::WHITE),
                                underlined: Some(true),
                                ..Style::default()
                            }))
                            .append(" <")
                            .with_color(text::YELLOW);
                    }
                    let reset_x = scrollbar_x - 50.0 - 10.0;
                    let change_x = reset_x - 5.0 - 75.0;
                    let name_static = if keypad {
                        crate::keypad::ACTIONS
                            .iter()
                            .find(|a| a.0 == name)
                            .map_or("", |a| a.0)
                    } else {
                        MAPPINGS.iter().find(|m| m.0 == name).map_or("", |m| m.0)
                    };
                    out.push(button(change_x, y, 75.0, message, Act::Key(name_static)));
                    if !conflicts.is_empty() {
                        out.push(Widget {
                            x: change_x - 6.0,
                            y: y + 1.0,
                            w: 3.0,
                            h: 18.0,
                            kind: WKind::Text {
                                text: Text::empty(),
                                color: rgba(255, 255, 0, 255),
                                centered: false,
                            },
                            list: false,
                            row: 0,
                        });
                    }
                    let reset = button(
                        reset_x,
                        y,
                        50.0,
                        tr("controls.reset"),
                        Act::ResetKey(name_static),
                    );
                    out.push(
                        if (keypad && !s.keypad.bindings.contains_key(name))
                            || (!keypad && s.keys.is_default(name))
                        {
                            inactive(reset)
                        } else {
                            reset
                        },
                    );
                    y += 20.0;
                    rows.end(&mut out, y);
                }
                // Each native mod's registered keys, under the mod's title and their categories.
                for (i, m) in s.mods.installed.iter().enumerate() {
                    let Some(page) = m.native.as_ref().filter(|p| !p.mappings.is_empty()) else {
                        continue;
                    };
                    rows.begin(&out, y);
                    out.push(section_title(Text::literal(m.title.clone()), y, w));
                    y += 20.0;
                    rows.end(&mut out, y);
                    let mut last_category = "";
                    for (k, mapping) in page.mappings.iter().enumerate() {
                        if !query.is_empty()
                            && !lang::get(&mapping.label).to_lowercase().contains(&query)
                            && !lang::get(&mapping.category).to_lowercase().contains(&query)
                        {
                            continue;
                        }
                        let category = mapping.category.as_str();
                        if !category.is_empty() && category != last_category {
                            last_category = category;
                            rows.begin(&out, y);
                            out.push(section_title(tr(&mapping.category), y, w));
                            y += 20.0;
                            rows.end(&mut out, y);
                        }
                        let (key, modifiers) = s.mods.binding(
                            &m.id,
                            &mapping.id,
                            (mapping.key.as_str(), mapping.modifiers),
                        );
                        let at_default = key == mapping.key && modifiers == mapping.modifiers;
                        let mut message = binding_text(&key, modifiers);
                        if !key_conflicts(
                            s,
                            &m.id,
                            &mapping.id,
                            &key,
                            modifiers,
                            mapping.context,
                            at_default,
                        )
                        .is_empty()
                        {
                            message = Text::literal("[ ")
                                .append(message.with_color(text::WHITE))
                                .append(" ]")
                                .with_color(text::YELLOW);
                        }
                        if self.selected_mod_key == Some((i, k)) {
                            message = Text::literal("> ")
                                .append(message.with_style(&Style {
                                    color: Some(text::WHITE),
                                    underlined: Some(true),
                                    ..Style::default()
                                }))
                                .append(" <")
                                .with_color(text::YELLOW);
                        }
                        rows.begin(&out, y);
                        out.push(Widget {
                            x: row_left + 2.0,
                            y: y + 6.0,
                            w: 200.0,
                            h: 9.0,
                            kind: WKind::Text {
                                text: tr(&mapping.label),
                                color: u32::MAX,
                                centered: false,
                            },
                            list: false,
                            row: 0,
                        });
                        let reset_x = scrollbar_x - 50.0 - 10.0;
                        let change_x = reset_x - 5.0 - 75.0;
                        out.push(button(change_x, y, 75.0, message, Act::ModKey(i, k)));
                        let reset = button(
                            reset_x,
                            y,
                            50.0,
                            tr("controls.reset"),
                            Act::ResetModKey(i, k),
                        );
                        out.push(if at_default { inactive(reset) } else { reset });
                        y += 20.0;
                        rows.end(&mut out, y);
                    }
                }
                content = y + self.scroll - top + 2.0;
                out[list_start..].iter_mut().for_each(|w| w.list = true);
                out.push(button(
                    cx - 154.0,
                    h - 27.0,
                    150.0,
                    tr("controls.resetAll"),
                    Act::ResetKeys,
                ));
                out.push(button(cx + 4.0, h - 27.0, 150.0, tr("gui.done"), Act::Done));
            }
            Page::Language => {
                top = 36.0;
                bottom = h - 53.0;
                out.push(Widget {
                    x: 0.0,
                    y: 4.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr(Self::title(page)),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                self.language_search.x = cx - 100.0;
                self.language_search.y = 17.0;
                let query = self.language_search.value.to_lowercase();
                let current = self
                    .language_choice
                    .clone()
                    .unwrap_or_else(|| s.language.clone());
                if std::mem::take(&mut self.center_language)
                    && let Some(i) = self.languages.iter().position(|l| l.code == current)
                {
                    // AbstractSelectionList.centerScrollOn.
                    self.scroll = (i as f32 * 18.0 + 9.0 - (bottom - top) / 2.0).max(0.0);
                }
                let list_start = out.len();
                let mut y = top + 2.0 - self.scroll;
                for l in &self.languages {
                    if !query.is_empty()
                        && !l.label().to_lowercase().contains(&query)
                        && !l.code.contains(&query)
                    {
                        continue;
                    }
                    rows.begin(&out, y);
                    out.push(Widget {
                        x: cx - 110.0,
                        y,
                        w: 220.0,
                        h: 18.0,
                        kind: WKind::LanguageRow {
                            label: Text::literal(l.label()),
                            selected: l.code == current,
                            code: l.code.clone(),
                        },
                        list: false,
                        row: 0,
                    });
                    y += 18.0;
                    rows.end(&mut out, y);
                }
                content = y + self.scroll - top + 2.0;
                out[list_start..].iter_mut().for_each(|w| w.list = true);
                out.push(Widget {
                    x: 0.0,
                    y: h - 45.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr("options.languageAccuracyWarning"),
                        color: rgba(0xBA, 0xBA, 0xBA, 255),
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                out.push(button(
                    cx - 154.0,
                    h - 28.0,
                    150.0,
                    tr("options.font"),
                    Act::Push(Page::Font),
                ));
                out.push(button(cx + 4.0, h - 28.0, 150.0, tr("gui.done"), Act::Done));
            }
            // Drawn by `pack_screen` and `mods_screen`.
            Page::Packs | Page::Mods => {}
            _ => {
                let wardrobe = self.wardrobe_columns(w);
                let (list_left, list_w) = wardrobe
                    .map_or((cx - 155.0, 310.0), |(x, preview, list)| {
                        (x + preview + 12.0, list)
                    });
                let pair_w = (list_w - 10.0) / 2.0;
                if matches!(page, Page::Video | Page::Controls | Page::ModBrowser) {
                    top = 38.0;
                }
                out.push(Widget {
                    x: 0.0,
                    y: if matches!(page, Page::Video | Page::Controls | Page::ModBrowser) {
                        4.0
                    } else {
                        12.0
                    },
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr(Self::title(page)),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                if wardrobe.is_some() && self.wardrobe_saving() {
                    top = 33.0;
                    let center = (top + bottom) * 0.5;
                    for (key, y) in [
                        ("starling.wardrobe.syncing", center - 23.0),
                        (self.wardrobe_sync_stage(), center - 8.0),
                    ] {
                        out.push(Widget {
                            x: list_left,
                            y,
                            w: list_w,
                            h: 9.0,
                            kind: WKind::Text {
                                text: tr(key),
                                color: u32::MAX,
                                centered: true,
                            },
                            list: false,
                            row: 0,
                        });
                    }
                } else if let Some((left, preview, _)) = wardrobe {
                    top = 58.0;
                    for (i, key) in ["starling.wardrobe.skins_tab", "starling.wardrobe.capes_tab"]
                        .into_iter()
                        .enumerate()
                    {
                        let capes = i == 1;
                        let label = if self.skin.capes == capes {
                            tr(key).with_color(0x55_FF_55)
                        } else {
                            tr(key)
                        };
                        out.push(button(
                            list_left + i as f32 * (pair_w + 10.0),
                            33.0,
                            pair_w,
                            label,
                            Act::WardrobeTab(capes),
                        ));
                    }
                    out.push(button(
                        left,
                        h - 55.0,
                        preview,
                        generic(
                            "starling.wardrobe.rotate",
                            on_off(self.skin.auto_rotate_since.is_some()),
                        ),
                        Act::AutoRotateWardrobe,
                    ));
                }
                let (text_left, text_w) = if wardrobe.is_some() {
                    (list_left, list_w)
                } else {
                    (0.0, w)
                };
                let list_start = out.len();
                let mut y = top + 2.0 - self.scroll;
                for row in self.list_rows(s) {
                    rows.begin(&out, y);
                    match row {
                        Row::Header(key) => {
                            let label = tr(key);
                            self.heading_row(&mut out, label, &mut y, w, top);
                        }
                        Row::Heading(key) => {
                            let label = tr(&key);
                            self.heading_row(&mut out, label, &mut y, w, top);
                        }
                        Row::Note(text) => {
                            out.push(Widget {
                                x: text_left,
                                y: y + 4.0,
                                w: text_w,
                                h: 9.0,
                                kind: WKind::Text {
                                    text: Text::literal(text),
                                    color: 0xFFA0A0A0,
                                    centered: true,
                                },
                                list: false,
                                row: 0,
                            });
                            y += 16.0;
                        }
                        Row::Paragraph(text) => {
                            // Minecraft's font draws nearly every glyph at most 6 pixels wide.
                            let chars = (((text_w - 20.0) / 6.0).floor() as usize).max(16);
                            for line in wrap_words(&text, chars) {
                                out.push(Widget {
                                    x: text_left,
                                    y: y + 4.0,
                                    w: text_w,
                                    h: 9.0,
                                    kind: WKind::Text {
                                        text: Text::literal(line),
                                        color: 0xFFA0A0A0,
                                        centered: true,
                                    },
                                    list: false,
                                    row: 0,
                                });
                                y += 11.0;
                            }
                            y += 5.0;
                        }
                        Row::Big(opt) => {
                            out.push(option_widget(s, opt, list_left, y + 2.0, list_w));
                            y += 25.0;
                        }
                        Row::Pair(a, b) => {
                            out.push(option_widget(s, a, list_left, y + 2.0, pair_w));
                            if let Some(b) = b {
                                out.push(option_widget(
                                    s,
                                    b,
                                    list_left + pair_w + 10.0,
                                    y + 2.0,
                                    pair_w,
                                ));
                            }
                            y += 25.0;
                        }
                        Row::Buttons(mut a, b) => {
                            a.x = list_left;
                            a.y = y + 2.0;
                            if wardrobe.is_some() {
                                a.w = if b.is_none() && a.w > 150.0 {
                                    list_w
                                } else {
                                    pair_w
                                };
                            }
                            let after = a.x + a.w + 10.0;
                            out.push(a);
                            if let Some(mut b) = b {
                                b.x = after;
                                b.y = y + 2.0;
                                if wardrobe.is_some() {
                                    b.w = pair_w;
                                }
                                out.push(b);
                            }
                            y += 25.0;
                        }
                        Row::Field => {
                            out.push(Widget {
                                x: list_left,
                                y: y + 8.0,
                                w: 40.0,
                                h: 9.0,
                                kind: WKind::Text {
                                    text: Text::literal("Name"),
                                    color: 0xFFA0_A0A0,
                                    centered: false,
                                },
                                list: false,
                                row: 0,
                            });
                            // Drawn and edited by the page itself (`skin.name`).
                            let field = &mut self.skin.name;
                            (field.x, field.y) = (list_left + 40.0, y + 2.0);
                            (field.width, field.height) = (list_w - 40.0, 20.0);
                            y += 25.0;
                        }
                        Row::SendField => {
                            // Drawn and edited by the page itself.
                            let field = &mut self.custom_send;
                            (field.x, field.y) = (cx - 155.0, y + 2.0);
                            (field.width, field.height) = (310.0, 20.0);
                            y += 25.0;
                        }
                        Row::Icons(icons) => {
                            let mut x = cx - 155.0;
                            for mut icon in icons {
                                (icon.x, icon.y) = (x, y + 2.0);
                                icon.list = true;
                                x += 44.0;
                                out.push(icon);
                            }
                            y += 42.0;
                        }
                        Row::Canvas(id) => {
                            // As big as the page has room for, its tools beside it.
                            let side = own_buttons::SIZE as f32;
                            let cell = ((h - 72.0) / side).floor().clamp(3.0, 8.0);
                            let x = cx - 155.0;
                            self.painter.at = [x, y + 2.0, cell];
                            let tools = x + side * cell + 46.0;
                            for (i, (label, act)) in [
                                ("Clear", Act::CanvasClear),
                                ("Plate", Act::CanvasPlate),
                                ("Import PNG", Act::CanvasImport(id)),
                            ]
                            .into_iter()
                            .enumerate()
                            {
                                let mut tool = button(
                                    tools,
                                    y + 2.0 + i as f32 * 24.0,
                                    80.0,
                                    Text::literal(label),
                                    act,
                                );
                                tool.list = true;
                                out.push(tool);
                            }
                            y += side * cell + 6.0;
                        }
                        Row::Warning(text) => {
                            for line in wrap_words(&text, ((text_w / 6.0) as usize).max(16)) {
                                out.push(Widget {
                                    x: text_left,
                                    y: y + 4.0,
                                    w: text_w,
                                    h: 9.0,
                                    kind: WKind::Text {
                                        text: Text::literal(line),
                                        color: rgba(255, 85, 85, 255),
                                        centered: true,
                                    },
                                    list: false,
                                    row: 0,
                                });
                                y += 16.0;
                            }
                        }
                    }
                    rows.end(&mut out, y);
                }
                content = y + self.scroll - top + 2.0;
                out[list_start..].iter_mut().for_each(|w| w.list = true);
                let folder = match page {
                    Page::Accessibility => {
                        Some(("options.accessibility.link", Act::AccessibilityHelp))
                    }
                    // Starling's Mods page.
                    Page::NativeGame => Some(("horizon.mods.open_folder", Act::OpenModFolder)),
                    Page::Skin if self.viewing_wardrobe_capes() => {
                        Some(("starling.wardrobe.open_folder", Act::OpenWardrobeFolder))
                    }
                    Page::Skin if self.skin_page => Some(("Open Skin Folder", Act::OpenSkinFolder)),
                    _ => None,
                };
                let wardrobe_pending = wardrobe.is_some() && self.wardrobe_sync_pending();
                let done_label = if wardrobe_pending && !self.wardrobe_sync_failed {
                    "starling.wardrobe.sync_and_done"
                } else {
                    "gui.done"
                };
                if page == Page::Skin && self.wardrobe_saving() {
                    out.push(inactive(done(h - 27.0)));
                } else if wardrobe_pending && self.wardrobe_sync_failed {
                    out.push(button(
                        cx - 154.0,
                        h - 27.0,
                        150.0,
                        tr("starling.wardrobe.retry_sync"),
                        Act::RetryWardrobeSync,
                    ));
                    out.push(button(cx + 4.0, h - 27.0, 150.0, tr("gui.done"), Act::Done));
                } else if let Some((label, act)) = folder {
                    // Android's way to find mods, resource packs and shader packs.
                    let shelf = catalog_shelf(page).filter(|_| self.catalog_buttons);
                    let (open_w, catalog) =
                        crate::pack_screen::open_folder_slot(cx - 154.0, shelf.is_some());
                    if let (Some(shelf), Some((x, width))) = (shelf, catalog) {
                        out.push(button(
                            x,
                            h - 27.0,
                            width,
                            tr("Catalog"),
                            Act::OpenCatalog(shelf),
                        ));
                    }
                    let label = if catalog.is_some() {
                        crate::pack_screen::SHORT_OPEN_FOLDER
                    } else {
                        label
                    };
                    out.push(button(cx - 154.0, h - 27.0, open_w, tr(label), act));
                    out.push(button(cx + 4.0, h - 27.0, 150.0, tr(done_label), Act::Done));
                } else {
                    out.push(done(h - 27.0));
                }
            }
        }
        // Layout may change the offset (language centering), or the list may
        // shrink after a resize/search. Correct the rows in this same frame.
        let previous_scroll = self.scroll;
        self.clamp_scroll(top, bottom, content);
        for widget in &mut out {
            if widget.list {
                widget.y += previous_scroll - self.scroll;
            }
            if let WKind::Button {
                act: Act::Cycle(Opt::RotateWithMinecarts),
                active,
                ..
            } = &mut widget.kind
            {
                *active = self.in_world && self.minecart_improvements;
            }
        }
        if page == Page::Skin && self.skin_page {
            self.skin.name.y += previous_scroll - self.scroll;
        }
        self.row_bands = rows
            .bands
            .into_iter()
            .map(|band| band.map(|y| y + previous_scroll - self.scroll))
            .collect();
        (out, top, bottom, content)
    }
}
