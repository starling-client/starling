//! Starling's port to Sunrise's API: Horizon asked a block entity's Part for its screen (`screen`,
//! `screen_draw`, `screen_click`, `screen_tooltip`, keyed by the block); Sunrise registers one
//! screen per menu type, with closures. The Part's old methods run here, through Horizon's events
//! rebuilt around Sunrise's frame, so the mod's screen code stays as it was written.
use std::sync::Arc;

use mod_api::{
    ClientHost, DrawContext, Host, MenuScreen, MenuScreenLayout, MenuScreenOpen, Registrar,
    ScreenSlot, Text,
};

/// Horizon's screen layout.
#[derive(Clone, Debug, Default)]
pub(crate) struct Screen {
    pub width: f32,
    pub height: f32,
    pub slots: Vec<Option<[f32; 2]>>,
    pub inventory_y: f32,
    pub inventory_x: f32,
    pub own_labels: bool,
    pub own_background: bool,
    pub recipe_book: Option<mod_api::RecipeBookTypeKey>,
    pub recipe_book_toggle: Option<[f32; 2]>,
}

impl Screen {
    fn layout(self) -> MenuScreenLayout {
        MenuScreenLayout {
            width: self.width,
            height: self.height,
            slots: self
                .slots
                .into_iter()
                .map(|slot| {
                    slot.map(|position| ScreenSlot {
                        position,
                        no_item_icon: None,
                    })
                })
                .collect(),
            inventory_y: self.inventory_y,
            inventory_x: self.inventory_x,
            own_labels: self.own_labels,
            own_background: self.own_background,
            recipe_book: self.recipe_book,
            recipe_book_toggle: self
                .recipe_book_toggle
                .map(|[x, y]| mod_api::RecipeBookToggle {
                    x,
                    y,
                    from: mod_api::ToggleOrigin::Menu,
                }),
            ..MenuScreenLayout::default()
        }
    }
}

/// Horizon's `ScreenEvent`: the block whose menu opens.
pub(crate) struct ScreenEvent<'a> {
    pub block: &'a str,
}

/// Horizon's `ScreenDrawEvent`.
pub(crate) struct ScreenDrawEvent<'a> {
    pub block: &'a str,
    pub origin: [f32; 2],
    pub at: [f32; 2],
    pub data: &'a [i32],
    pub draw: &'a mut dyn DrawContext,
}

/// Horizon's `ScreenClickEvent`.
pub(crate) struct ScreenClickEvent<'a> {
    pub block: &'a str,
    pub origin: [f32; 2],
    pub at: [f32; 2],
    pub data: &'a [i32],
    pub right: bool,
}

/// Horizon's `ScreenTooltipEvent`.
pub(crate) struct ScreenTooltipEvent<'a> {
    pub block: &'a str,
    pub origin: [f32; 2],
    pub at: [f32; 2],
    pub data: &'a [i32],
    pub slot: Option<u32>,
    pub lines: &'a mut Vec<(Text, u32)>,
}

/// A Part's old screen methods.
pub(crate) trait StarlingScreens: Send + Sync + 'static {
    fn screen(&self, _host: &dyn Host, _event: &mut ScreenEvent<'_>) -> Option<Screen> {
        None
    }
    fn screen_draw(&self, _host: &dyn ClientHost, _event: &mut ScreenDrawEvent<'_>) {}
    fn screen_click(&self, _host: &dyn ClientHost, _event: &mut ScreenClickEvent<'_>) -> Option<i32> {
        None
    }
    fn screen_tooltip(&self, _host: &dyn ClientHost, _event: &mut ScreenTooltipEvent<'_>) {}
}

/// `part`'s screen for the menu type `menu`.
pub(crate) fn register(r: &mut dyn Registrar, menu: &str, part: impl StarlingScreens) {
    let part = Arc::new(part);
    let factory = move |host: &dyn ClientHost, open: MenuScreenOpen<'_>| -> Option<MenuScreen> {
        let block = open.block.unwrap_or_default().to_owned();
        let screen = part.screen(host, &mut ScreenEvent { block: &block })?;
        let (draw_part, click_part, tip_part) = (part.clone(), part.clone(), part.clone());
        let (draw_block, click_block, tip_block) = (block.clone(), block.clone(), block);
        Some(
            MenuScreen::new(screen.layout())
                .with_draw(move |host, frame, draw| {
                    draw_part.screen_draw(
                        host,
                        &mut ScreenDrawEvent {
                            block: &draw_block,
                            origin: frame.origin,
                            at: frame.at,
                            data: frame.data,
                            draw,
                        },
                    );
                })
                .with_click(move |host, frame, right| {
                    click_part.screen_click(
                        host,
                        &mut ScreenClickEvent {
                            block: &click_block,
                            origin: frame.origin,
                            at: frame.at,
                            data: frame.data,
                            right,
                        },
                    )
                })
                .with_tooltip(move |host, frame, slot, lines| {
                    tip_part.screen_tooltip(
                        host,
                        &mut ScreenTooltipEvent {
                            block: &tip_block,
                            origin: frame.origin,
                            at: frame.at,
                            data: frame.data,
                            slot,
                            lines,
                        },
                    );
                }),
        )
    };
    if let Err(error) = r.register_menu_screen(menu, Arc::new(factory)) {
        r.refuse(&error);
    }
}
