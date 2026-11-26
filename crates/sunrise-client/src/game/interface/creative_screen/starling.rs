//! Starling's part of creative_screen.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Game<'_> {
    /// Whether GUI position `p` is on an item tab's grid of items, between its cells or a little
    /// past them (`Creative::on_item_grid`). Bedrock Inventory's item window has it whichever of
    /// its windows is worked with.
    pub(in super::super) fn creative_catalog_at(&self, batch: &HudBatch, p: (f32, f32)) -> bool {
        let (x, y) = batch.centered_origin(Creative::WIDTH, Creative::HEIGHT);
        let x = if CREATIVE_SHIFT.get() != 0.0 {
            x - BEDROCK_SHIFT
        } else if self.creative_tab_open() {
            x
        } else {
            return false;
        };
        Creative::on_item_grid((p.0 - x, p.1 - y), self.creative_screen.shown_rows)
    }

    /// A carried stack put down on an item tab's grid off its cells: the click of an empty cell
    /// (`CreativeModeInventoryScreen.slotClicked`), which deletes it, or one of it with the right
    /// button. Its release clicks nothing more.
    pub(super) fn creative_empty_cell_click(&mut self, right: bool) {
        let catalog = world_items!(self);
        let max = |s: &ItemStack| catalog.get(&s.item).map_or(64, |i| i.max_stack);
        sunrise_client::creative::click_display(
            &mut self.carried,
            None,
            max,
            i32::from(right),
            sunrise_client::menu::Input::Pickup,
        );
        self.inv_drag = None;
        self.inv_clicks.skip_release();
    }

    /// A finger lets go over Bedrock Inventory's item window with a stack it brought from the
    /// inventory window (the one its press held): the stack goes back to the tab, as on Bedrock,
    /// clicked on the cell under it or, between the cells, on an empty one. True when it did.
    pub(in super::super) fn creative_catalog_release(&mut self, right: bool) -> bool {
        if self.screen != Screen::Creative
            || !self.has_touch()
            || self.bedrock_items.is_none()
            || self.carried.is_none()
        {
            return false;
        }
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return false;
        };
        let batch = self.interface_batch(size);
        if !self.creative_catalog_at(&batch, self.cursor) {
            return false;
        }
        // The press's drag ends here, and the item window takes the release.
        self.inv_drag = None;
        self.inv_clicks.cancel_drag();
        self.bedrock_side(self.cursor.0, batch.width);
        match self.slot_at(&batch, self.cursor.0, self.cursor.1) {
            Some(slot) => {
                self.click_open_inventory(
                    Some(slot),
                    i32::from(right),
                    sunrise_client::menu::Input::Pickup,
                );
            }
            None => self.creative_empty_cell_click(right),
        }
        true
    }
}

impl Game<'_> {
    /// Bedrock Inventory: the window of the two the rest of the game works with, the inventory's
    /// (`panel`) or the item tab's, for a GUI `width` wide. Too narrow for both, or with the Survival
    /// Inventory tab chosen, there is one window, in the middle, as without it.
    pub(super) fn bedrock_show(&mut self, panel: bool, width: f32) {
        let screen = &mut self.creative_screen;
        let items = self.bedrock_items.unwrap_or(screen.selected_index());
        let inventory = screen.tabs.iter().position(|t| t.inventory);
        let both = self.settings.bedrock_inventory
            && self.screen == Screen::Creative
            && width >= BEDROCK_SHIFT * 2.0 + Creative::WIDTH
            && screen.tabs.get(items).is_some_and(|t| !t.inventory);
        screen.shown_rows = if both && inventory.is_some() { 6 } else { 5 };
        match inventory.filter(|_| both) {
            Some(inventory) if panel => {
                self.bedrock_items = Some(items);
                screen.show(inventory);
                CREATIVE_SHIFT.set(BEDROCK_SHIFT);
            }
            Some(_) => {
                self.bedrock_items = None;
                screen.show(items);
                CREATIVE_SHIFT.set(-BEDROCK_SHIFT);
            }
            None => {
                self.bedrock_items = None;
                screen.show(items);
                CREATIVE_SHIFT.set(0.0);
            }
        }
        // The switch happens inside the touch or click that hit-tests next (`slot_at`), which
        // reads the screen's build: build it now for the window just chosen, not at the next
        // frame, or the touch finds no slot there and counts as outside the window (a drop).
        self.build_screen();
    }

    /// The window the GUI position `x` is over becomes the one worked with; a stack being shared
    /// out stays in the one it started in.
    pub(in super::super) fn bedrock_side(&mut self, x: f32, width: f32) {
        if self.inv_drag.is_none() {
            self.bedrock_show(x >= width / 2.0, width);
        }
    }

    /// The window that shows `slot` becomes the one worked with.
    pub(in super::super) fn bedrock_for_slot(&mut self, slot: usize) {
        if CREATIVE_SHIFT.get() == 0.0 {
            return;
        }
        let width = self.interface_batch(self.touch_size()).width;
        self.bedrock_show(slot < Creative::DISPLAY, width);
    }
}
