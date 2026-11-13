//! The creative inventory screen (`CreativeModeInventoryScreen`): its tabs, pages, tooltips,
//! trash slot, search box and clicks.
use super::*;

impl Game<'_> {
    /// `CreativeModeInventoryScreen.hasPermissions` (`canUseGameMasterBlocks`
    /// and the Operator Items Tab option) given to `tryRefreshInvalidatedTabs`,
    /// as the screen's constructor and `containerTick` do. When the tabs need
    /// building, each item tab asks the mods for their stacks
    /// (`BuildCreativeModeTabContentsEvent`) before the build takes them.
    pub(super) fn refresh_creative_tabs(&mut self) {
        let permissions = self.can_use_game_master_blocks() && self.settings.operator_items_tab;
        let enchantments = Arc::clone(self.item_enchantments());
        self.creative_screen.set_enchantments(enchantments);
        let Some(tabs) = self.creative_screen.tabs_to_build(permissions) else {
            return;
        };
        let added = tabs
            .into_iter()
            .map(|tab| {
                let mut items = Vec::new();
                self.native_build_creative_mode_tab_contents(&tab, permissions, &mut items);
                (tab, items)
            })
            .collect();
        self.creative_screen
            .build(permissions, added, &self.pack.names);
    }
    pub(super) fn creative_inventory_open(&self) -> bool {
        self.screen == Screen::Creative && self.creative_screen.tab().inventory
    }
    /// An item tab of the creative screen (its ItemPickerMenu).
    pub(super) fn creative_tab_open(&self) -> bool {
        self.screen == Screen::Creative && !self.creative_screen.tab().inventory
    }
    pub(super) fn creative_origin(batch: &HudBatch) -> (f32, f32) {
        let (x, y) = batch.centered_origin(Creative::WIDTH, Creative::HEIGHT);
        (x + CREATIVE_SHIFT.get(), y)
    }
    /// Bedrock Inventory: the window of the two the rest of the game works with, the inventory's
    /// (`panel`) or the item tab's, for a GUI `width` wide. Too narrow for both, or with the Survival
    /// Inventory tab chosen, there is one window, in the middle, as without it.
    fn bedrock_show(&mut self, panel: bool, width: f32) {
        let screen = &mut self.creative_screen;
        let items = self.bedrock_items.unwrap_or(screen.selected);
        let inventory = screen.tabs.iter().position(|t| t.inventory);
        let both = self.settings.bedrock_inventory
            && self.screen == Screen::Creative
            && width >= BEDROCK_SHIFT * 2.0 + Creative::WIDTH
            && screen.tabs.get(items).is_some_and(|t| !t.inventory);
        screen.shown_rows = if both && inventory.is_some() { 6 } else { 5 };
        match inventory.filter(|_| both) {
            Some(inventory) if panel => {
                self.bedrock_items = Some(items);
                screen.selected = inventory;
                CREATIVE_SHIFT.set(BEDROCK_SHIFT);
            }
            Some(_) => {
                self.bedrock_items = None;
                screen.selected = items;
                CREATIVE_SHIFT.set(-BEDROCK_SHIFT);
            }
            None => {
                self.bedrock_items = None;
                screen.selected = items;
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
    pub(super) fn bedrock_side(&mut self, x: f32, width: f32) {
        if self.inv_drag.is_none() {
            self.bedrock_show(x >= width / 2.0, width);
        }
    }
    /// The window that shows `slot` becomes the one worked with.
    pub(super) fn bedrock_for_slot(&mut self, slot: usize) {
        if CREATIVE_SHIFT.get() == 0.0 {
            return;
        }
        let width = HudBatch::new(
            self.touch_size(),
            horizon_client::hud::GuiScale {
                requested: 0,
                unicode: false,
            },
        )
        .width;
        self.bedrock_show(slot < Creative::DISPLAY, width);
    }
    /// Where the creative screen's window, or both of Bedrock Inventory's, start and end across.
    pub(super) fn creative_bounds(batch: &HudBatch) -> (f32, f32) {
        let (x, _) = batch.centered_origin(Creative::WIDTH, Creative::HEIGHT);
        let shift = CREATIVE_SHIFT.get().abs();
        (x - shift, x + Creative::WIDTH + shift)
    }
    /// A creative tab's title: `itemGroup.*` for vanilla's; a mod tab's
    /// `itemGroup.<mod>.<name>`, else its `title` as a lang key (a mod's own
    /// `itemGroup.<mod>`), else `title` as English text.
    pub(super) fn creative_tab_title(&self, tab: &horizon_client::creative::Tab) -> String {
        if let Some((name, _)) = &tab.name {
            return name.clone();
        }
        match horizon_client::creative::item_group_key(&tab.id) {
            Some(key) => self.translate_with_fallback(key, &tab.title),
            None => {
                let own = self.translate_with_fallback(&tab.title, &tab.title);
                self.translate_with_fallback(
                    &format!("itemGroup.{}", tab.id.replace(':', ".")),
                    &own,
                )
            }
        }
    }
    /// Vanilla tooltip: dark box with a purple gradient border, laid out and
    /// drawn like an item tooltip (4 px of padding on every side).
    pub(super) fn tooltip(&self, batch: &mut HudBatch, text: &str, x: f32, y: f32) {
        let atlas = &self.gui.atlas;
        let lines = vec![(horizon_client::text::Text::literal(text), u32::MAX)];
        let screen = (batch.width, batch.height);
        let Some(at) = self.tooltip_pre(batch, None, &lines, &[], (x, y), screen) else {
            return;
        };
        let (x, y, w, h) = Self::tooltip_geometry(atlas, &lines, at, screen, [0.0; 2]);
        self.tooltip_box(batch, None, x, y, w, h);
        Self::tooltip_text(batch, atlas, &lines, x, y);
    }
    /// `CreativeModeInventoryScreen.getTooltipFromContainerItem`: the lines
    /// that follow the item's name, none for a category tab's own grid slots
    /// (`grid`): the names of the tabs containing the stack, in blue, then on
    /// the search tab's grid the tags a `#` search shows, in dark purple.
    pub(super) fn creative_tooltip_tabs(
        &self,
        stack: &ItemStack,
        grid: bool,
    ) -> Vec<(horizon_client::text::Text, u32)> {
        let screen = &self.creative_screen;
        let tab = screen.tab();
        let category = !tab.search && !tab.inventory;
        if !horizon_client::creative::tooltip_lists_tabs(category, grid) {
            return Vec::new();
        }
        let blue = rgba(85, 85, 255, 255);
        let dark_purple = rgba(170, 0, 170, 255);
        let mut lines: Vec<(horizon_client::text::Text, u32)> = screen
            .tabs_containing(stack)
            .into_iter()
            .map(|i| {
                let title = self.creative_tab_title(&screen.tabs[i]);
                (horizon_client::text::Text::literal(title), blue)
            })
            .collect();
        if tab.search && grid {
            for tag in screen.search_tag_lines(stack) {
                lines.push((horizon_client::text::Text::literal(tag), dark_purple));
            }
        }
        lines
    }
    /// `CreativeModeInventoryScreen.extractRenderState`: item slots share the
    /// container screen's hover target for their highlight and tooltip.
    pub(super) fn draw_creative(&mut self, batch: &mut HudBatch) {
        // CreativeModeInventoryScreen: `CommonComponents.EMPTY` for a title.
        screen_info::begin_container("creative");
        if !self.bedrock_held {
            self.bedrock_side(self.cursor.0, batch.width);
        }
        if CREATIVE_SHIFT.get() == 0.0 {
            self.render_transparent_background("creative", batch);
            self.draw_creative_window(batch, false);
            self.draw_carried(batch);
            self.draw_creative_tooltips(batch);
            return;
        }
        // Bedrock Inventory: both windows, the one worked with last, so its tooltip is on top.
        self.render_transparent_background("creative", batch);
        let on_panel = self.bedrock_items.is_some();
        self.bedrock_show(!on_panel, batch.width);
        self.draw_creative_window(batch, !on_panel);
        self.bedrock_show(on_panel, batch.width);
        self.draw_creative_window(batch, on_panel);
        self.draw_carried(batch);
        self.draw_creative_tooltips(batch);
    }
    /// One creative window: the item tab's or, with `panel`, Bedrock Inventory's inventory, which
    /// has no tabs of its own.
    fn draw_creative_window(&mut self, batch: &mut HudBatch, panel: bool) {
        let (ox, oy) = Self::creative_origin(batch);
        let selected = self.creative_screen.selected;
        // Only what is drawn of the tabs: their stacks stay where they are.
        let tabs = self.creative_screen.tab_headers();
        let Some(tab) = tabs.get(selected.min(tabs.len().saturating_sub(1))) else {
            return;
        };
        let tab_sprite = |t: &horizon_client::creative::Tab, on: bool| {
            format!(
                "gui/sprites/container/creative_inventory/tab_{}_{}_{}",
                if t.top { "top" } else { "bottom" },
                if on { "selected" } else { "unselected" },
                t.column + 1
            )
        };
        // Unselected tabs behind the panel, then the panel, then the selected tab.
        for (i, t) in tabs.iter().enumerate() {
            if !panel && i != selected && self.creative_screen.on_page(t) {
                let (tx, ty) = Creative::tab_position(t);
                batch.sprite(&self.gui.atlas, &tab_sprite(t, false), ox + tx, oy + ty);
                let mouse = (self.cursor.0 - ox, self.cursor.1 - oy);
                if Creative::tab_wants_hand(t, false, mouse) {
                    horizon_client::widgets::request_cursor(
                        horizon_client::widgets::Cursor::PointingHand,
                    );
                }
            }
        }
        let background = if tab.search {
            "gui/container/creative_inventory/tab_item_search"
        } else if tab.inventory {
            "gui/container/creative_inventory/tab_inventory"
        } else {
            "gui/container/creative_inventory/tab_items"
        };
        if let Some(mut r) = self.gui.atlas.sprites.get(background).copied() {
            let whole = r;
            r.u1 = r.u0 + (r.u1 - r.u0) * Creative::WIDTH / r.w as f32;
            r.v1 = r.v0 + (r.v1 - r.v0) * Creative::HEIGHT / r.h as f32;
            batch.rect_uv(ox, oy, Creative::WIDTH, Creative::HEIGHT, r, u32::MAX);
            // Bedrock Inventory's sixth row of items goes right under the fifth: the texture's
            // row again, then the gap it has above the hotbar's, up to the scrollbar's track.
            if !tab.inventory && CREATIVE_SHIFT.get() != 0.0 {
                const ACROSS: f32 = 172.0;
                for (from, to, height) in [(89.0, 107.0, 18.0), (107.0, 125.0, 4.0)] {
                    let mut band = whole;
                    let (u, v) = (whole.u1 - whole.u0, whole.v1 - whole.v0);
                    band.u1 = whole.u0 + u * ACROSS / whole.w as f32;
                    band.v0 = whole.v0 + v * from / whole.h as f32;
                    band.v1 = whole.v0 + v * (from + height) / whole.h as f32;
                    batch.rect_uv(ox, oy + to, ACROSS, height, band, u32::MAX);
                }
            }
        }
        if panel {
            // The crafting grid the window lacks, and the result's slot, as the game's windows
            // draw a slot.
            for slot in BEDROCK_CRAFT {
                let Some((sx, sy)) = Creative::bedrock_craft_position(slot) else {
                    continue;
                };
                let (x, y) = (ox + sx, oy + sy);
                let atlas = &self.gui.atlas;
                batch.fill(atlas, x - 1.0, y - 1.0, 18.0, 18.0, rgba(55, 55, 55, 255));
                batch.fill(atlas, x, y, 17.0, 17.0, u32::MAX);
                batch.fill(atlas, x, y, 16.0, 16.0, rgba(139, 139, 139, 255));
            }
        } else {
            let (tx, ty) = Creative::tab_position(tab);
            batch.sprite(&self.gui.atlas, &tab_sprite(tab, true), ox + tx, oy + ty);
        }
        for t in tabs
            .iter()
            .filter(|t| !panel && self.creative_screen.on_page(t))
        {
            let (tx, ty) = Creative::tab_position(t);
            let stack = ItemStack {
                item: Arc::from(t.icon.as_str()),
                count: 1,
                damage: 0,
                extra: None,
            };
            self.draw_item(
                batch,
                &stack,
                ox + tx + 5.0,
                oy + ty + if t.top { 9.0 } else { 7.0 },
            );
        }
        let atlas = &self.gui.atlas;
        if !tab.inventory {
            let title = self.creative_tab_title(tab);
            batch.text(
                atlas,
                &title,
                ox + 8.0,
                oy + 6.0,
                tab.name
                    .as_ref()
                    .and_then(|n| n.1)
                    .map_or(rgba(64, 64, 64, 255), |c| {
                        rgba((c >> 16) as u8, (c >> 8) as u8, c as u8, 255)
                    }),
                false,
            );
        }
        if tab.search {
            // CreativeModeInventoryScreen: an unbordered EditBox of 50 at (82, 6).
            let caret = self.text_caret_shown();
            let search = &mut self.creative_screen.search;
            search.x = ox + 82.0;
            search.y = oy + 6.0;
            search.width = 80.0;
            search.bordered = false;
            search.focused = caret;
            search.text_color = u32::MAX;
            search.max_length = clipboard::CREATIVE_SEARCH_CHARS;
            if horizon_client::widgets::narrating() {
                search.message = screen_info::tr("itemGroup.search", &[]);
            }
            search.draw(batch, atlas, None);
        }
        // The hovered stack, and whether it is a grid slot (`CustomCreativeSlot`).
        let mut hovered: Option<(ItemStack, bool)> = None;
        // Use the same padded slot bounds as clicks and the other menus, so
        // highlights and tooltips remain visible between adjacent items.
        let hovered_slot = self.slot_at(batch, self.cursor.0, self.cursor.1);
        if tab.inventory {
            self.draw_player_preview(
                batch,
                ox + 73.0,
                oy + 6.0,
                ox + 105.0,
                oy + 49.0,
                20.0,
                0.0625,
            );
            let trash = Self::creative_trash_hovered(batch, self.cursor);
            let (tx, ty) = Creative::TRASH_POSITION;
            if trash {
                self.draw_slot_highlight(batch, false, ox + tx, oy + ty);
            }
            self.draw_hovered_slot_highlight(batch, false, ox, oy);
            self.draw_inventory_slots(batch, ox, oy);
            if trash {
                self.draw_slot_highlight(batch, true, ox + tx, oy + ty);
            }
            self.draw_hovered_slot_highlight(batch, true, ox, oy);
        } else {
            let items = self.creative_screen.items(&self.pack.names);
            let first = self.creative_screen.first_row(&self.pack.names);
            let six = CREATIVE_SHIFT.get() != 0.0;
            for r in 0..if six { 6 } else { 5 } {
                for c in 0..9 {
                    let Some(stack) = items.get((first + r) * 9 + c).filter(|s| s.count > 0) else {
                        continue;
                    };
                    let (x, y) = (ox + 9.0 + c as f32 * 18.0, oy + 18.0 + r as f32 * 18.0);
                    let over = hovered_slot == Some(Creative::DISPLAY + r * 9 + c);
                    if over {
                        self.draw_slot_highlight(batch, false, x, y);
                    }
                    self.draw_item(batch, stack, x, y);
                    if over {
                        self.draw_slot_highlight(batch, true, x, y);
                        hovered = Some((stack.clone(), true));
                    }
                }
            }
            let drag = self.drag_view();
            for c in 0..if six { 0 } else { 9 } {
                let (x, y) = (ox + 9.0 + c as f32 * 18.0, oy + 112.0);
                let over = hovered_slot == Some(c);
                if over {
                    self.draw_slot_highlight(batch, false, x, y);
                }
                self.draw_slot_item(batch, drag.as_ref(), c, &self.inventory[c], x, y);
                if over {
                    self.draw_slot_highlight(batch, true, x, y);
                    hovered = self.inventory[c].clone().map(|s| (s, false));
                }
            }
            let scrollable =
                self.creative_screen.rows(&self.pack.names) > self.creative_screen.shown_rows;
            let scroller = if scrollable {
                "gui/sprites/container/creative_inventory/scroller"
            } else {
                "gui/sprites/container/creative_inventory/scroller_disabled"
            };
            batch.sprite(
                &self.gui.atlas,
                scroller,
                ox + Creative::SCROLL_X,
                oy + Creative::SCROLL_Y + self.creative_screen.scroller_offset(),
            );
        }
        if !panel {
            // Past both windows, when there are two.
            let width = if CREATIVE_SHIFT.get() == 0.0 {
                Creative::WIDTH
            } else {
                Creative::WIDTH + 2.0 * BEDROCK_SHIFT
            };
            self.creative_effect_tip = self.draw_inventory_effects(batch, (ox, oy), width as i32);
            self.draw_creative_pages(batch, ox, oy);
        }
        self.creative_hovered = hovered;
    }
    /// What the cursor is over in the window worked with, over everything else.
    fn draw_creative_tooltips(&mut self, batch: &mut HudBatch) {
        let (ox, oy) = Self::creative_origin(batch);
        let tabs = self.creative_screen.tab_headers();
        let inventory = self.creative_screen.tab().inventory;
        let hovered = self.creative_hovered.take();
        let effect_tip = self.creative_effect_tip.take();
        for t in tabs
            .iter()
            .filter(|t| self.bedrock_items.is_none() && self.creative_screen.on_page(t))
        {
            let (tx, ty) = Creative::tab_position(t);
            if self.cursor.0 >= ox + tx
                && self.cursor.0 < ox + tx + 26.0
                && self.cursor.1 >= oy + ty
                && self.cursor.1 < oy + ty + 32.0
            {
                self.tooltip(
                    batch,
                    &self.creative_tab_title(t),
                    self.cursor.0,
                    self.cursor.1,
                );
            }
        }
        if inventory {
            if Self::creative_trash_hovered(batch, self.cursor) {
                // `CreativeModeInventoryScreen`: the destroy slot's tooltip is
                // `inventory.binSlot`, not an English literal.
                let destroy = self.translate_with_fallback("inventory.binSlot", "Destroy Item");
                self.tooltip(batch, &destroy, self.cursor.0, self.cursor.1);
            } else if let Some(slot) = self.slot_at(batch, self.cursor.0, self.cursor.1)
                && let Some(stack) = self.slot_view(slot)
                && self.tooltip_with_carried(&stack)
            {
                let tabs = self.creative_tooltip_tabs(&stack, false);
                self.draw_stack_tooltip_with(
                    batch,
                    &stack,
                    false,
                    self.cursor.0,
                    self.cursor.1,
                    tabs,
                );
            }
        } else if let Some((item, grid)) = hovered
            && self.tooltip_with_carried(&item)
        {
            let tabs = self.creative_tooltip_tabs(&item, grid);
            // A `CustomCreativeSlot`: TooltipFlag.asCreative.
            self.draw_stack_tooltip_with(batch, &item, grid, self.cursor.0, self.cursor.1, tabs);
        }
        if let Some(lines) = effect_tip {
            self.draw_tooltip_lines(batch, &lines, self.cursor.0, self.cursor.1);
        }
    }
    pub(super) fn creative_trash_hovered(batch: &HudBatch, cursor: (f32, f32)) -> bool {
        let (ox, oy) = Self::creative_origin(batch);
        let (x, y) = Creative::TRASH_POSITION;
        cursor.0 >= ox + x
            && cursor.0 < ox + x + 16.0
            && cursor.1 >= oy + y
            && cursor.1 < oy + y + 16.0
    }
    /// Whether GUI position `p` is on an item tab's grid of items, between its cells or a little
    /// past them (`Creative::on_item_grid`). Bedrock Inventory's item window has it whichever of
    /// its windows is worked with.
    pub(super) fn creative_catalog_at(&self, batch: &HudBatch, p: (f32, f32)) -> bool {
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
    fn creative_empty_cell_click(&mut self, right: bool) {
        let catalog = world_items!(self);
        let max = |s: &ItemStack| catalog.get(&s.item).map_or(64, |i| i.max_stack);
        horizon_client::creative::click_display(
            &mut self.carried,
            None,
            max,
            i32::from(right),
            horizon_client::menu::Input::Pickup,
        );
        self.inv_drag = None;
        self.inv_clicks.skip_release();
    }
    /// A finger lets go over Bedrock Inventory's item window with a stack it brought from the
    /// inventory window (the one its press held): the stack goes back to the tab, as on Bedrock,
    /// clicked on the cell under it or, between the cells, on an empty one. True when it did.
    pub(super) fn creative_catalog_release(&mut self, right: bool) -> bool {
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
        let batch = HudBatch::new(size, self.settings.gui_scale_inputs());
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
                    horizon_client::menu::Input::Pickup,
                );
            }
            None => self.creative_empty_cell_click(right),
        }
        true
    }
    /// NeoForge's creative page buttons, 20 square, 50 above the screen at
    /// its left and right edges: -1 or 1 for the one at `cursor`, when there
    /// is more than one page.
    pub(super) fn creative_page_button(
        ox: f32,
        oy: f32,
        cursor: (f32, f32),
        pages: u32,
    ) -> Option<i64> {
        if pages <= 1 {
            return None;
        }
        let y = oy - 50.0;
        let inside =
            |x: f32| cursor.0 >= x && cursor.0 < x + 20.0 && cursor.1 >= y && cursor.1 < y + 20.0;
        if inside(ox) {
            Some(-1)
        } else if inside(ox + Creative::WIDTH - 20.0) {
            Some(1)
        } else {
            None
        }
    }
    /// The page buttons and "page / pages" between them.
    pub(super) fn draw_creative_pages(&self, batch: &mut HudBatch, ox: f32, oy: f32) {
        let pages = self.creative_screen.pages();
        if pages <= 1 {
            return;
        }
        let atlas = &self.gui.atlas;
        let y = oy - 50.0;
        for (x, label, step) in [(ox, "<", -1), (ox + Creative::WIDTH - 20.0, ">", 1)] {
            let hover = Self::creative_page_button(ox, oy, self.cursor, pages) == Some(step);
            let page = self.creative_screen.page as i64 + step;
            let active = (0..pages as i64).contains(&page);
            let sprite = match (active, hover) {
                (false, _) => "gui/sprites/widget/button_disabled",
                (true, true) => "gui/sprites/widget/button_highlighted",
                (true, false) => "gui/sprites/widget/button",
            };
            horizon_client::ui::blit_sprite(batch, atlas, sprite, x, y, 20.0, 20.0, u32::MAX);
            let color = if active {
                u32::MAX
            } else {
                rgba(160, 160, 160, 255)
            };
            let w = HudBatch::text_width(atlas, label);
            batch.text(atlas, label, x + (20.0 - w) / 2.0, y + 6.0, color, true);
        }
        let text = format!("{} / {}", self.creative_screen.page + 1, pages);
        let w = HudBatch::text_width(atlas, &text);
        batch.text(
            atlas,
            &text,
            ox + (Creative::WIDTH - w) / 2.0,
            oy - 44.0,
            u32::MAX,
            true,
        );
    }
    pub(super) fn click_creative(&mut self, right: bool) {
        if self.mod_browser_press(right) {
            return;
        }
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return;
        };
        let batch = HudBatch::new(size, self.settings.gui_scale_inputs());
        self.bedrock_side(self.cursor.0, batch.width);
        self.bedrock_held = true;
        let (ox, oy) = Self::creative_origin(&batch);
        let (cx, cy) = self.cursor;
        let panel = self.bedrock_items.is_some();
        // NeoForge's page buttons, when mods add tabs.
        if let Some(step) =
            Self::creative_page_button(ox, oy, (cx, cy), self.creative_screen.pages())
                .filter(|_| !panel)
        {
            // Page buttons are outside the inventory panel; their release must
            // not throw a carried stack, including when the button is disabled.
            self.inv_clicks.skip_release();
            let page = self.creative_screen.page as i64 + step;
            if (0..self.creative_screen.pages() as i64).contains(&page) {
                self.audio.play_button_click();
                self.creative_screen.set_page(page as u32);
                self.refresh_saved_hotbars();
            }
            return;
        }
        let tabs = self.creative_screen.tabs.clone();
        for (i, t) in tabs.iter().enumerate() {
            if panel || !self.creative_screen.on_page(t) {
                continue;
            }
            let (tx, ty) = Creative::tab_position(t);
            if cx >= ox + tx && cx < ox + tx + 26.0 && cy >= oy + ty && cy < oy + ty + 32.0 {
                select_creative_tab(&mut self.creative_screen, &mut self.inv_clicks, i);
                // The tab's slots and page replace the previous tab's: the
                // build reads the new ones from this click on.
                self.build_screen();
                self.refresh_saved_hotbars();
                self.inv_drag = None;
                self.last_inv_click = None;
                return;
            }
        }
        if self.creative_inventory_open() {
            if Self::creative_trash_hovered(&batch, self.cursor) {
                if self.has_shift_down() {
                    let before = self.player_equipment();
                    self.inventory.fill(None);
                    self.armor.fill(None);
                    self.offhand = None;
                    self.equipment = Default::default();
                    self.craft_grid[..4].fill(None);
                    self.player_equipment_changed(&before);
                    self.apply_equipment();
                } else {
                    self.carried = None;
                }
                self.inv_drag = None;
                self.inv_clicks = Default::default();
            } else if self.slot_at(&batch, cx, cy).is_some() || self.clicked_outside(&batch) {
                self.inventory_press(right);
            }
            return;
        }
        if !right && self.creative_search_click(ox, oy) {
            return;
        }
        // Holding the scrollbar scrolls as the mouse moves (mouseDragged).
        if !right
            && self
                .creative_screen
                .press_scrollbar(cx - ox, cy - oy, &self.pack.names)
        {
            return;
        }
        if self.slot_at(&batch, cx, cy).is_some() || self.clicked_outside(&batch) {
            self.inventory_press(right);
        } else if self.carried.is_some() && self.creative_catalog_at(&batch, (cx, cy)) {
            self.creative_empty_cell_click(right);
        }
    }
    /// A click on the search tab's box (`EditBox.onClick`).
    pub(super) fn creative_search_click(&mut self, ox: f32, oy: f32) -> bool {
        if !self.creative_screen.tab().search {
            return false;
        }
        let (x, y, w, h) = (ox + 82.0, oy + 6.0, 80.0, 9.0);
        let (mx, my) = self.cursor;
        if mx < x || my < y || mx >= x + w || my >= y + h {
            return false;
        }
        let shift = self.has_shift_down();
        let search = &mut self.creative_screen.search;
        search.x = x;
        search.y = y;
        search.width = w;
        search.click(&self.gui.atlas, mx, shift, false);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `CreativeModeInventoryScreen.slotClicked` clicks the player's own InventoryMenu on the
    /// Survival Inventory tab, its crafting result too (Bedrock Inventory shows the grid).
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_inventory_tab_takes_the_crafting_result() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.screen = Screen::Creative;
            let inventory = game
                .creative_screen
                .tab_headers()
                .iter()
                .position(|tab| tab.inventory)
                .unwrap();
            game.creative_screen.select(inventory);
            game.craft_size = 2;
            game.craft_grid[0] = Some(ItemStack::new("minecraft:oak_log", 1));
            let pickup = horizon_client::menu::Input::Pickup;
            assert!(game.click_open_inventory(Some(200), 0, pickup));
            assert_eq!(
                game.carried,
                Some(ItemStack::new("minecraft:oak_planks", 4))
            );
            assert_eq!(game.craft_grid[0], None);
        });
    }
}
