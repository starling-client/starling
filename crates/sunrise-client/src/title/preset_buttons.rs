use super::*;
use sunrise_hooks::title::Button as AddedButton;

/// The main menu's ids for the buttons crates above the engine add (`sunrise_hooks::title`): one
/// per registration, from `sunrise_hooks::title::FIRST_ID` on, so a press resolves the row it
/// selected without asking again whether it shows.
pub(super) const ADDED_IDS: u8 = sunrise_hooks::title::FIRST_ID;

/// Where the logo and edition banner end (`draw_logo`: the banner's top 67, 14 rows).
const BANNER_BOTTOM: f32 = 81.0;

/// The added rows a main menu of height `h` fits: the block below Minecraft Realms must keep
/// below the logo and edition banner (`main_menu_top` lifts its top for the copyright line,
/// which asks `h - 128 - 24 * added` for the top, so at most
/// `(h - BANNER_BOTTOM - 128) / 24` rows keep that top at or below the banner). The rest stay
/// hidden: rows that push Singleplayer over the banner show none of their own.
pub(super) fn added_rows_that_fit(h: f32) -> usize {
    ((h - BANNER_BOTTOM - 128.0) / 24.0).floor().max(0.0) as usize
}

#[cfg(test)]
thread_local! {
    /// A test's own title buttons, instead of the game's registered ones: each test runs on its
    /// thread.
    pub(super) static TEST_BUTTONS: std::cell::Cell<Option<&'static [&'static dyn AddedButton]>> =
        const { std::cell::Cell::new(None) };
}

/// The buttons crates registered for the title screen.
fn registered() -> &'static [&'static dyn AddedButton] {
    #[cfg(test)]
    if let Some(buttons) = TEST_BUTTONS.get() {
        return buttons;
    }
    sunrise_hooks::title::engine::all()
}

/// The action of the press the main menu id `id` carried: the screen the registration names,
/// whether it shows now or not. The row passed hit testing or took the keyboard focus while it
/// showed; asking `shown` again here would drop the press because the state behind it moved in
/// between (`registered` answers by the id alone). No id left, no action.
pub(super) fn added_action(id: u8) -> Action {
    let i = usize::from(id).checked_sub(usize::from(ADDED_IDS));
    match i.and_then(|i| registered().get(i)).map(|b| b.opens()) {
        Some(sunrise_hooks::title::Opens::Mods) => Action::Mods,
        _ => Action::None,
    }
}

impl Title {
    /// The added buttons the main menu shows now, with their ids, in registration order: at
    /// most the rows that fit between the logo and the copyright line (`added_rows_that_fit`).
    /// The id is the button's registration, not its place among the ones showing, so a press
    /// resolved from it needs no second look at `shown`.
    fn added_buttons(&self, h: f32) -> Vec<(u8, &'static dyn AddedButton)> {
        let title = sunrise_hooks::title::Title::new(self.mods);
        registered()
            .iter()
            .enumerate()
            .filter(|(_, button)| button.shown(&title))
            .take(added_rows_that_fit(h))
            .map(|(i, button)| (ADDED_IDS + i as u8, *button))
            .collect()
    }
    /// The main menu's first row: vanilla's `height / 4 + 48`. With `added` rows below Minecraft
    /// Realms it lifts on short screens, so Options... and Quit Game stay clear of the copyright
    /// line drawn at `height - 10`, and so the lifted top stays below the logo and edition
    /// banner: the caller shows `added_rows_that_fit` rows at most, the most that keeps both.
    pub(super) fn main_menu_top(h: f32, added: usize) -> f32 {
        let top = (h / 4.0).floor() + 48.0;
        if added == 0 {
            return top;
        }
        top.min(h - 128.0 - 24.0 * added as f32)
    }
    pub(super) fn buttons(&self, w: f32, h: f32) -> Vec<Button> {
        // The game rules draw and press their own.
        if self.rules.borrow().is_some() {
            return Vec::new();
        }
        let (w, h) = (w.ceil(), h.ceil());
        let b = |x: f32, y: f32, w: f32, label: &str, enabled: bool, id: u8| Button {
            x,
            y,
            w,
            label: label.into(),
            enabled,
            id,
            icon: None,
            trailing: None,
            action: None,
        };
        match &self.mode {
            Mode::Main => {
                let cx = (w / 2.0).floor();
                let added = self.added_buttons(h);
                // Each added button takes a row below Minecraft Realms; the rows after move down.
                let below = 24.0 * added.len() as f32;
                let y = Self::main_menu_top(h, added.len());
                let mut buttons = vec![
                    b(
                        cx - 100.0,
                        y,
                        200.0,
                        &lang::get_or("menu.singleplayer", "Singleplayer"),
                        true,
                        1,
                    ),
                    b(
                        cx - 100.0,
                        y + 24.0,
                        200.0,
                        &lang::get_or("menu.multiplayer", "Multiplayer"),
                        self.allows_multiplayer,
                        4,
                    ),
                    b(
                        cx - 100.0,
                        y + 48.0,
                        200.0,
                        &lang::get_or("menu.online", "Minecraft Realms"),
                        // TitleScreen.createNormalMenuOptions: Realms is off when
                        // multiplayer is.
                        self.allows_multiplayer,
                        8,
                    ),
                    b(
                        cx - 100.0,
                        y + 96.0 + below,
                        98.0,
                        &lang::get_or("menu.options", "Options..."),
                        true,
                        3,
                    )
                    .does(crate::widgets::Action::Options),
                    b(
                        cx + 2.0,
                        y + 96.0 + below,
                        98.0,
                        &lang::get_or("menu.quit", "Quit Game"),
                        true,
                        2,
                    )
                    .does(crate::widgets::Action::Exit),
                ];
                // After Minecraft Realms, so they come before the icon row and Options... for
                // the keyboard and the narrator too.
                for (i, (id, button)) in added.into_iter().enumerate() {
                    buttons.insert(
                        3 + i,
                        b(
                            cx - 100.0,
                            y + 72.0 + 24.0 * i as f32,
                            200.0,
                            &lang::get(button.label()),
                            true,
                            id,
                        ),
                    );
                }
                // CommonButtons' icon buttons; Sunrise's Join a Friend (an
                // invite code) sits behind vanilla's Friends button. Starling keeps its Mods in
                // Options > Mods (the switches and Compile), as in 0.8.0: three icons, centred.
                for (i, (key, plain, id, icon)) in [
                    ("gui.friends.open", "Friends", 5, "friends/friends"),
                    ("options.language", "Language...", 6, "icon/language"),
                    (
                        "options.accessibility",
                        "Accessibility Settings...",
                        7,
                        "icon/accessibility",
                    ),
                ]
                .into_iter()
                .enumerate()
                {
                    let mut button = b(
                        cx - 34.0 + i as f32 * 24.0,
                        y + 72.0 + below,
                        20.0,
                        &lang::get_or(key, plain),
                        id != 5 || crate::net::joins_friends(),
                        id,
                    );
                    button.icon = Some((icon, 15.0));
                    buttons.push(button);
                }
                // Starling's account, where the game signs in itself: in the top left corner,
                // clear of the logo, so vanilla's buttons stay as they are.
                if crate::accounts::in_game() {
                    buttons.push(b(4.0, 4.0, 120.0, &account::main_label(), true, 80));
                }
                // Starling on Android: the way back to the launcher is the control hub's "H"
                // (android_control_ui's Launcher), from any screen.
                buttons
            }
            Mode::Select { worlds, selected } => {
                let world = selected.and_then(|i| worlds.get(i));
                // SelectWorldScreen.updateButtonStatus: primaryActionMessage, canEdit, canRecreate.
                let corrupted = world.is_some_and(|w| w.corrupted);
                let editable = world.is_some_and(WorldEntry::can_edit);
                let play = if corrupted {
                    lang::get_or("recover_world.button", "Attempt to Recover")
                } else if world.is_some_and(|w| w.file_fixing) {
                    lang::get_or("selectWorld.upgrade_and_play", "Upgrade and Play")
                } else {
                    lang::get_or("selectWorld.select", "Play Selected World")
                };
                vec![
                    b(
                        (w / 2.0).floor() - 154.0,
                        h - 52.0,
                        150.0,
                        &play,
                        world.is_some_and(WorldEntry::can_play),
                        10,
                    ),
                    b(
                        (w / 2.0).floor() + 4.0,
                        h - 52.0,
                        150.0,
                        &lang::get_or("selectWorld.create", "Create New World"),
                        true,
                        11,
                    ),
                    b(
                        (w / 2.0).floor() - 154.0,
                        h - 28.0,
                        71.0,
                        &lang::get_or("selectWorld.edit", "Edit"),
                        editable,
                        13,
                    ),
                    b(
                        (w / 2.0).floor() - 75.0,
                        h - 28.0,
                        71.0,
                        &lang::get_or("selectWorld.delete", "Delete"),
                        selected.is_some(),
                        14,
                    ),
                    b(
                        (w / 2.0).floor() + 4.0,
                        h - 28.0,
                        71.0,
                        &lang::get_or("selectWorld.recreate", "Re-Create"),
                        editable,
                        15,
                    ),
                    b(
                        (w / 2.0).floor() + 83.0,
                        h - 28.0,
                        71.0,
                        &lang::get_or("gui.back", "Back"),
                        true,
                        12,
                    )
                    .does(crate::widgets::Action::Back),
                ]
            }
            Mode::Create {
                creative, options, ..
            } => create_world::buttons(
                w,
                h,
                *creative,
                options,
                self.preset_data
                    .is_some()
                    .then_some(self.preset_editors.as_ref()),
            ),
            Mode::CreateFlat { flat, .. } => flat::buttons(w, h, flat),
            Mode::CreateBuffet { buffet, .. } => buffet::buttons(w, h, buffet),
            Mode::CreatePreset { .. } => vec![
                b(
                    w / 2.0 - 155.0,
                    h - 28.0,
                    150.0,
                    &lang::get("gui.done"),
                    true,
                    94,
                )
                .does(crate::widgets::Action::Done),
                b(
                    w / 2.0 + 5.0,
                    h - 28.0,
                    150.0,
                    &lang::get("gui.cancel"),
                    true,
                    95,
                )
                .does(crate::widgets::Action::Back),
            ],
            Mode::Edit {
                world,
                name,
                adopt_regions,
                ..
            } => {
                let x = (w / 2.0).floor() - 100.0;
                let top = edit_top(h);
                let mut buttons = vec![
                    b(
                        x,
                        top + 64.0,
                        200.0,
                        &lang::get_or("selectWorld.edit.resetIcon", "Reset Icon"),
                        self.saves.join(&world.folder).join("icon.png").is_file(),
                        30,
                    ),
                    b(
                        x,
                        top + 89.0,
                        200.0,
                        &lang::get_or("selectWorld.edit.openFolder", "Open World Folder"),
                        true,
                        31,
                    ),
                    b(
                        x,
                        top + 114.0,
                        200.0,
                        &lang::get_or("selectWorld.edit.backup", "Make Backup"),
                        true,
                        32,
                    ),
                    b(
                        x,
                        top + 139.0,
                        200.0,
                        &lang::get_or("selectWorld.edit.backupFolder", "Open Backups Folder"),
                        true,
                        33,
                    ),
                    b(
                        x,
                        top + 164.0,
                        200.0,
                        &lang::get_or("selectWorld.edit.optimize", "Optimize World"),
                        true,
                        34,
                    ),
                    b(
                        x,
                        top + 214.0,
                        98.0,
                        &lang::get_or("selectWorld.edit.save", "Save"),
                        !name.trim().is_empty(),
                        35,
                    ),
                    b(
                        x + 102.0,
                        top + 214.0,
                        98.0,
                        &lang::get_or("gui.cancel", "Cancel"),
                        true,
                        36,
                    )
                    .does(crate::widgets::Action::Cancel),
                ];
                // Sunrise's opt-in (#2554), in the row vanilla leaves empty above Save.
                if *adopt_regions {
                    buttons.push(b(
                        x,
                        top + 189.0,
                        200.0,
                        &lang::get("sunrise.biome_regions.adopt.button"),
                        true,
                        98,
                    ));
                }
                buttons
            }
            Mode::AdoptBiomeRegions { .. } => {
                let y = confirm_top(h, self.message_lines.get())
                    + 41.0
                    + self.message_lines.get() as f32 * 9.0;
                vec![
                    b(
                        (w / 2.0).floor() - 152.0,
                        y,
                        150.0,
                        &lang::get("sunrise.biome_regions.adopt.confirm"),
                        true,
                        99,
                    ),
                    b(
                        (w / 2.0).floor() + 2.0,
                        y,
                        150.0,
                        &lang::get_or("gui.cancel", "Cancel"),
                        true,
                        100,
                    )
                    .does(crate::widgets::Action::Back),
                ]
            }
            Mode::Delete { .. } => {
                let y = confirm_top(h, self.message_lines.get())
                    + 41.0
                    + self.message_lines.get() as f32 * 9.0;
                vec![
                    b(
                        (w / 2.0).floor() - 152.0,
                        y,
                        150.0,
                        &lang::get_or("selectWorld.deleteButton", "Delete"),
                        true,
                        37,
                    ),
                    b(
                        (w / 2.0).floor() + 2.0,
                        y,
                        150.0,
                        &lang::get_or("gui.cancel", "Cancel"),
                        true,
                        38,
                    )
                    .does(crate::widgets::Action::Cancel),
                ]
            }
            Mode::LowDiskSpace { .. } => {
                let y = confirm_top(h, self.message_lines.get())
                    + 41.0
                    + self.message_lines.get() as f32 * 9.0;
                vec![
                    b(
                        (w / 2.0).floor() - 152.0,
                        y,
                        150.0,
                        &lang::get_or("gui.continue", "Continue"),
                        true,
                        94,
                    ),
                    b(
                        (w / 2.0).floor() + 2.0,
                        y,
                        150.0,
                        &lang::get_or("gui.back", "Back"),
                        true,
                        95,
                    )
                    .does(crate::widgets::Action::Back),
                ]
            }
            Mode::MissingMods { .. } => {
                let (_, _, narrow) = missing_mods::layout(w, h);
                let button_width = if narrow { (w - 20.0).min(300.0) } else { 150.0 };
                let center = (w - button_width) / 2.0;
                let y = h - if narrow { 76.0 } else { 52.0 };
                vec![
                    // Don't Open is the default: first, and what Escape does.
                    b(
                        center,
                        y + if narrow { 48.0 } else { 24.0 },
                        button_width,
                        &lang::get("sunrise.missing_mods.dont_open"),
                        true,
                        101,
                    )
                    .does(crate::widgets::Action::Cancel),
                    b(
                        if narrow {
                            center
                        } else {
                            (w / 2.0).floor() - 152.0
                        },
                        y,
                        button_width,
                        &lang::get("sunrise.missing_mods.backup_open"),
                        true,
                        102,
                    ),
                    b(
                        if narrow {
                            center
                        } else {
                            (w / 2.0).floor() + 2.0
                        },
                        y + if narrow { 24.0 } else { 0.0 },
                        button_width,
                        &lang::get("sunrise.missing_mods.open"),
                        true,
                        103,
                    ),
                ]
            }
            Mode::MissingBiomeRegions { .. } => {
                let y = confirm_top(h, self.message_lines.get())
                    + 41.0
                    + self.message_lines.get() as f32 * 9.0;
                vec![
                    b(
                        (w / 2.0).floor() - 152.0,
                        y,
                        150.0,
                        &lang::get("sunrise.biome_regions.missing.open_vanilla"),
                        true,
                        96,
                    ),
                    b(
                        (w / 2.0).floor() + 2.0,
                        y,
                        150.0,
                        &lang::get("sunrise.biome_regions.missing.dont_open"),
                        true,
                        97,
                    )
                    .does(crate::widgets::Action::Back),
                ]
            }
            Mode::Recover { issues, .. } => {
                let y = recover_buttons_top(h, self.message_lines.get());
                // The restore button is active only when level.dat is the broken one
                // and level.dat_old reads.
                vec![
                    b(
                        (w / 2.0).floor() - 60.0,
                        y,
                        120.0,
                        &lang::get_or("recover_world.restore", "Attempt to Restore"),
                        issues[0].is_some() && issues[1].is_none(),
                        90,
                    ),
                    b(
                        (w / 2.0).floor() - 60.0,
                        y + 28.0,
                        120.0,
                        &lang::get_or("gui.back", "Back"),
                        true,
                        91,
                    )
                    .does(crate::widgets::Action::Back),
                ]
            }
            Mode::Recovered { success: true, .. } => {
                let y = confirm_top(h, self.message_lines.get())
                    + 41.0
                    + self.message_lines.get() as f32 * 9.0;
                vec![
                    b(
                        (w / 2.0).floor() - 152.0,
                        y,
                        150.0,
                        &lang::get_or("gui.continue", "Continue"),
                        true,
                        92,
                    ),
                    b(
                        (w / 2.0).floor() + 2.0,
                        y,
                        150.0,
                        &lang::get_or("gui.back", "Back"),
                        true,
                        93,
                    )
                    .does(crate::widgets::Action::Back),
                ]
            }
            Mode::Recovered { success: false, .. } => {
                // AlertScreen.init: the button follows the text, kept inside the screen.
                let y = (90.0 + self.message_lines.get() as f32 * 9.0 + 12.0)
                    .max(h / 6.0 + 96.0)
                    .min(h - 24.0);
                vec![
                    b(
                        (w / 2.0).floor() - 75.0,
                        y,
                        150.0,
                        &lang::get_or("gui.back", "Back"),
                        true,
                        93,
                    )
                    .does(crate::widgets::Action::Back),
                ]
            }
            Mode::Error { alert, .. } => {
                let lines = self.message_lines.get();
                let y = if alert.is_some() {
                    // AlertScreen.init: the button follows the text, kept inside the screen.
                    (90.0 + lines as f32 * 9.0 + 12.0)
                        .max(h / 6.0 + 96.0)
                        .min(h - 24.0)
                } else {
                    confirm_top(h, lines) + 41.0 + lines as f32 * 9.0
                };
                vec![
                    b(
                        (w / 2.0).floor() - 75.0,
                        y,
                        150.0,
                        &lang::get_or("gui.back", "Back"),
                        true,
                        39,
                    )
                    .does(crate::widgets::Action::Back),
                ]
            }
            Mode::BackupBeforeUpgrade { .. } => {
                // BackupConfirmScreen.init: the buttons sit under the text at fixed offsets.
                let y = 109.0 + self.message_lines.get() as f32 * 9.0;
                vec![
                    b(
                        (w / 2.0).floor() - 155.0,
                        y,
                        150.0,
                        &lang::get_or(
                            "selectWorld.backupJoinConfirmButton",
                            "Create Backup and Load",
                        ),
                        true,
                        40,
                    ),
                    b(
                        (w / 2.0).floor() + 5.0,
                        y,
                        150.0,
                        &lang::get_or("selectWorld.backupJoinSkipButton", "I Know What I'm Doing!"),
                        true,
                        41,
                    ),
                    b(
                        (w / 2.0).floor() - 75.0,
                        y + 24.0,
                        150.0,
                        &lang::get_or("gui.cancel", "Cancel"),
                        true,
                        42,
                    )
                    .does(crate::widgets::Action::Cancel),
                ]
            }
            Mode::OptimizeConfirm { .. } => {
                // BackupConfirmScreen.init with the Erase Cached Data box: the buttons sit
                // under the text at fixed offsets.
                let y = 100.0 + (self.message_lines.get() as f32 + 1.0) * 9.0;
                vec![
                    b(
                        (w / 2.0).floor() - 155.0,
                        y,
                        150.0,
                        &lang::get_or(
                            "optimizeWorld.confirm.proceed",
                            "Create Backup and Optimize",
                        ),
                        true,
                        43,
                    ),
                    b(
                        (w / 2.0).floor() + 5.0,
                        y,
                        150.0,
                        &lang::get_or("selectWorld.backupJoinSkipButton", "I Know What I'm Doing!"),
                        true,
                        44,
                    ),
                    b(
                        (w / 2.0).floor() - 75.0,
                        y + 24.0,
                        150.0,
                        &lang::get_or("gui.cancel", "Cancel"),
                        true,
                        45,
                    )
                    .does(crate::widgets::Action::Cancel),
                ]
            }
            // No `does`: Escape is inert here on purpose, so B is too. A on the button stops it.
            Mode::Optimize { .. } => vec![b(
                (w / 2.0).floor() - 100.0,
                (h / 4.0).floor() + 150.0,
                200.0,
                &lang::get_or("gui.cancel", "Cancel"),
                true,
                46,
            )],
            Mode::MultiplayerWarning { .. } => {
                let lines = self.message_lines.get();
                let y = warning_top(h, lines) + 82.0 + lines as f32 * 9.0;
                vec![
                    b(
                        (w / 2.0).floor() - 154.0,
                        y,
                        150.0,
                        &lang::get_or("gui.proceed", "Proceed"),
                        true,
                        70,
                    ),
                    b(
                        (w / 2.0).floor() + 4.0,
                        y,
                        150.0,
                        &lang::get_or("gui.back", "Back"),
                        true,
                        71,
                    )
                    .does(crate::widgets::Action::Back),
                ]
            }
            Mode::Onboarding(_) => self.onboarding_buttons(w, h),
            Mode::BanNotice(_) => {
                // ConfirmLinkScreen.addButtons: Open in Browser, Copy to Clipboard, then the
                // notice's Acknowledge, 100 wide each.
                let labels = [
                    lang::get_or("chat.link.open", "Open in Browser"),
                    lang::get_or("chat.copy", "Copy to Clipboard"),
                    lang::get_or("gui.acknowledge", "Acknowledge"),
                ];
                let positions = self.ban_buttons.get();
                (0..3)
                    .map(|i| {
                        b(
                            positions[i].0,
                            positions[i].1,
                            100.0,
                            &labels[i],
                            true,
                            120 + i as u8,
                        )
                    })
                    .collect()
            }
            Mode::Backup { .. } => Vec::new(),
            Mode::Account(screen) => screen
                .buttons(w, h)
                .into_iter()
                .map(|(x, y, bw, label, enabled, id)| b(x, y, bw, &label, enabled, id))
                .collect(),
            Mode::Multiplayer(screen) => screen
                .buttons(w, h)
                .into_iter()
                .map(|(x, y, bw, label, enabled, id)| {
                    let mut button = b(x, y, bw, &label, enabled, id);
                    button.action = crate::multiplayer::button_action(id);
                    button
                })
                .collect(),
            Mode::Realms(screen) => screen
                .buttons(w, h)
                .into_iter()
                .map(|(x, y, bw, label, enabled, id, icon)| {
                    let mut btn = b(x, y, bw, &label, enabled, id);
                    btn.icon = icon;
                    btn
                })
                .collect(),
        }
    }
}
