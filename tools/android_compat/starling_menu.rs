//! Starling's port to Sunrise's API: Horizon asked a block entity's Part about a press of its
//! menu's buttons (`menu_button`, with the block entity's fields to change); Sunrise asks the menu
//! type's `MenuPart`. The Part's old method runs here on Horizon's event, rebuilt around a copy of
//! the block entity that is written back after (starling_tick.rs).
use mod_api::item::containers::ModBlockEntityFields;
use mod_api::item::menu::MenuButtonContext;
use mod_api::{Flow, Host, ItemStack, MenuButtonValidEvent, MenuPart, ServerHost};

/// Horizon's `MenuButtonEvent`.
pub(crate) struct MenuButtonEvent<'a> {
    pub context: &'a MenuButtonContext,
    pub entity: &'a mut ModBlockEntityFields,
    pub carried: &'a mut Option<ItemStack>,
    pub pos: [i32; 3],
}

/// A Part's old menu button method.
pub(crate) trait StarlingMenuButton: Send + Sync + 'static {
    fn menu_button(&self, host: &mut dyn ServerHost, event: &mut MenuButtonEvent<'_>);
}

/// The menu type's Part, running the old method: every button the menu shows is one Horizon
/// delivered.
pub(crate) struct Buttons(pub Box<dyn StarlingMenuButton>);

#[mod_api::sunrise_part]
impl MenuPart for Buttons {
    fn menu_button_valid(&self, _: &dyn Host, _event: &mut MenuButtonValidEvent<'_>) -> Flow {
        Flow::Cancel
    }

    fn menu_button(&self, event: &mut mod_api::MenuButtonEvent<'_>) {
        let context = event.context.clone();
        let mut carried = event.carried.take();
        crate::starling_tick::run_scope(&mut event.subject, |host, entity, pos| {
            self.0.menu_button(
                host,
                &mut MenuButtonEvent {
                    context: &context,
                    entity,
                    carried: &mut carried,
                    pos,
                },
            );
        });
        *event.carried = carried;
    }
}
