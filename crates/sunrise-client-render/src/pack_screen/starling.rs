//! Starling's part of pack_screen.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// The line under the title: where packs come from. A phone has no window to drop files on;
/// they go in the game's folder in its shared storage (`datapacks` for data packs).
pub(super) fn drop_info(data: bool) -> Text {
    if cfg!(target_os = "android") {
        Text::literal(phone_drop_info(data, phone_game_folder().as_deref()))
    } else {
        tr("pack.dropInfo")
    }
}

/// The game's folder on a phone, unless it is in the app's own files.
pub(super) fn phone_game_folder() -> Option<std::path::PathBuf> {
    let shared = crate::logging::game_home() != crate::logging::private_home();
    shared.then(|| std::env::current_dir().unwrap_or_default())
}

/// `drop_info` on a phone, for the game's folder. It differs between the builds (Starling,
/// StarlingPreview, StarlingDev): shown from the storage's root, as a file manager does.
pub(super) fn phone_drop_info(data: bool, folder: Option<&std::path::Path>) -> String {
    let Some(folder) = folder else {
        return "In the app's own files: Open Pack Folder to reach them".into();
    };
    let shown = folder.strip_prefix("/storage/emulated/0").unwrap_or(folder);
    let shown = shown.display();
    if data {
        format!("Put data packs in {shown}/datapacks, mods in {shown}/sunrise-mods")
    } else {
        format!("Put packs in {shown}/resourcepacks")
    }
}

/// The footer's first button is 150 wide. With a Catalog button the two share that width, a gap
/// between: each is this wide.
pub(super) const HALF_BUTTON: f32 = 73.0;

/// Open Folder's label when it shares the slot with Catalog: the longer ones ("Open Pack Folder")
/// don't fit half a button, and a label that doesn't fit scrolls.
pub(crate) const SHORT_OPEN_FOLDER: &str = "Open Folder";

/// The footer's first slot, at `x`: Open Folder's width and, when the page has a Catalog button,
/// its x and width. Open Folder keeps the left half and Catalog takes the right.
pub(crate) fn open_folder_slot(x: f32, catalog: bool) -> (f32, Option<(f32, f32)>) {
    if !catalog {
        return (150.0, None);
    }
    (HALF_BUTTON, Some((x + 150.0 - HALF_BUTTON, HALF_BUTTON)))
}

impl PackScreen {
    /// The screen for data packs (`CreateWorldScreen.openDataPackSelectionScreen`).
    pub fn for_data(mut self, m: &Measure) -> Self {
        self.data = true;
        self.reposition(m);
        self
    }

    pub(super) fn title(&self) -> Text {
        if self.data {
            // Starling: the world's content mods are chosen here too.
            Text::literal("Select Mods and Data Packs")
        } else {
            self.title.clone()
        }
    }

    /// A footer button's width: Open Folder shares its slot with Catalog when there is one.
    pub(super) fn footer_w(&self, i: usize) -> i32 {
        match i {
            0 => open_folder_slot(0.0, self.catalog).0 as i32,
            _ => 150,
        }
    }

    /// The Catalog button, if it has one: x, y and width.
    pub(super) fn catalog_button(&self) -> Option<(i32, i32, i32)> {
        let (footer_x, y) = self.footer()[0];
        let (x, width) = open_folder_slot(footer_x as f32, self.catalog).1?;
        Some((x as i32, y, width as i32))
    }

    /// Whether `(x, y)` is on the Catalog button.
    pub(super) fn on_catalog(&self, x: f64, y: f64) -> bool {
        self.catalog_button().is_some_and(|(bx, by, width)| {
            x >= bx as f64 && y >= by as f64 && x < (bx + width) as f64 && y < (by + 20) as f64
        })
    }

    /// Whether `(x, y)` (GUI pixels) is on the focused search box.
    pub fn search_at(&self, x: f32, y: f32) -> bool {
        self.search_focused() && self.search.contains(f64::from(x), f64::from(y))
    }

    /// The search box, its text and selection (the on-screen keyboard's editor on Android).
    pub fn search_field(&self) -> &TextField {
        &self.search
    }

    /// The on-screen keyboard's editor's text, already through the box's filter, and selection
    /// in the focused search box (Android); the lists filter again when the text changed.
    pub fn set_search(&mut self, m: &Measure, value: &str, cursor: usize, anchor: usize) {
        if !self.search_focused() {
            return;
        }
        let changed = value != self.search.value;
        if changed {
            self.search.set_value(m, value);
        }
        self.search.move_cursor_to(m, anchor, false);
        self.search.move_cursor_to(m, cursor, true);
        if changed {
            self.filter(None);
        }
    }
}
