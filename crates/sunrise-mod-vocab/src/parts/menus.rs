use super::*;
use crate::client::menu_screen::OwnedMenuScreen;

/// Built-in menu types from vanilla 26.3 `net.minecraft.world.inventory.MenuType`.
const VANILLA_MENU_TYPES: &[&str] = &[
    "minecraft:generic_9x1",
    "minecraft:generic_9x2",
    "minecraft:generic_9x3",
    "minecraft:generic_9x4",
    "minecraft:generic_9x5",
    "minecraft:generic_9x6",
    "minecraft:generic_3x3",
    "minecraft:crafter_3x3",
    "minecraft:anvil",
    "minecraft:beacon",
    "minecraft:blast_furnace",
    "minecraft:brewing_stand",
    "minecraft:crafting",
    "minecraft:enchantment",
    "minecraft:furnace",
    "minecraft:grindstone",
    "minecraft:hopper",
    "minecraft:lectern",
    "minecraft:loom",
    "minecraft:merchant",
    "minecraft:shulker_box",
    "minecraft:smithing",
    "minecraft:smoker",
    "minecraft:cartography_table",
    "minecraft:stonecutter",
];

/// Whether `menu` names one of vanilla 26.3's built-in `MenuType` entries.
// hidden: engine/registrar menu classification; authors pass menu ids directly.
#[doc(hidden)]
pub fn is_vanilla_menu_type(menu: &str) -> bool {
    VANILLA_MENU_TYPES.contains(&menu)
}

// no wire: frozen registration storage; each boundary value has its own Wire derive.
#[derive(Clone)]
pub(crate) struct MenuBookDeclaration {
    pub(crate) owner: &'static str,
    pub(crate) book: crate::RecipeBookTypeKey,
    pub(crate) recipe_type: String,
    pub(crate) layout: crate::RecipeBookLayout,
}

// no wire: frozen registration and panic state, never a window instance.
/// Engine lookup record for a registered constructor.
#[derive(Clone)]
pub(crate) struct MenuScreenRegistration {
    pub(crate) owner: &'static str,
    pub(crate) index: usize,
    pub(crate) factory: Arc<dyn crate::MenuScreenFactory>,
    pub(crate) panicked: Arc<std::sync::atomic::AtomicBool>,
}

/// Look up the screen factory registered for `menu`.
fn menu_screen_factory(menu: &str) -> Option<&'static MenuScreenRegistration> {
    installed()?.menu_screen_factories.get(menu)
}

/// Whether `menu` has a registered screen factory.
pub fn has_menu_screen_factory(menu: &str) -> bool {
    menu_screen_factory(menu).is_some()
}

/// Build a window's screen. The outer option distinguishes an unregistered factory
/// from one that declined or was refused. The returned instance is not part of registration.
pub fn create_menu_screen(
    host: &dyn crate::ClientHost,
    open: crate::MenuScreenOpen<'_>,
) -> Option<(&'static str, Result<Option<OwnedMenuScreen>, String>)> {
    let registration = menu_screen_factory(open.menu)?;
    let screen = crate::callback_owner::call(
        registration.owner,
        registration.index,
        &registration.panicked,
        "menu screen factory",
        "menu_screen_factory",
        || {
            registration
                .factory
                .create(host, open)
                .map(|screen| OwnedMenuScreen::new(registration.owner, registration.index, screen))
        },
    );
    Some((registration.owner, screen))
}

impl HookParts {
    pub(crate) fn menu<'k>(
        &self,
        id: impl FnOnce() -> Option<&'k str>,
    ) -> &[Attached<dyn MenuPart>] {
        attached(&self.menus, id)
    }
    /// Button validation and actions belong to the admitted menu type or verified stock opener,
    /// independently of the package supplying its client screen or its recipe book.
    pub(crate) fn menu_for_opener(
        &self,
        menu: &str,
        block: Option<&str>,
    ) -> &[Attached<dyn MenuPart>] {
        let owner = if is_vanilla_menu_type(menu) {
            block.and_then(crate::runtime::content_block).flatten()
        } else {
            crate::runtime::content_menu_owner(menu)
        };
        owner
            .and_then(|owner| self.menu_owners.get(menu)?.get(&owner))
            .map_or(&[], std::slice::from_ref)
    }
}

/// Whether the Parts attached to the menu type `menu` subscribe to `hook`; the game
/// asks once when a menu opens, so a menu with none skips the hook while it is open.
pub fn menu_listens(hook: HookId, menu: &str) -> bool {
    if !has_part_hook(hook) {
        return false;
    }
    parts(hook, "menu routing")
        .for_hook(hook)
        .is_some_and(|table| !table.menu(|| Some(menu)).is_empty())
}

impl dyn Registrar + '_ {
    /// Attach gameplay behavior to `menu`: a mod's, or one of vanilla's that
    /// a mod block opens. Button validation and actions go to the package that registered the
    /// open custom type or the verified stock opener, not to its presentation providers.
    /// `minecraft:inventory` names the player's own `InventoryMenu`, which every player always
    /// has open and vanilla gives no `MenuType`: it has no owner, so every mod's Part on it is
    /// asked, in mod order, by `menu_slot_rules`, `menu_clicked` and `recipe_book_place`, and
    /// a refusal from any of them stands. The creative picker takes no Parts.
    /// Register the client screen separately with
    /// [`MenusRegistrar::register_menu_screen`].
    pub fn menu_part<P: MenuPart>(&mut self, menu: &str, part: P) {
        self.attach_menu_part(menu, type_name::<P>(), Arc::new(part));
    }
}

/// Registrar operations for menus.
pub trait MenusRegistrar {
    /// Register one client constructor for a namespaced menu type, matching NeoForge's
    /// `RegisterMenuScreensEvent.register` and vanilla's `MenuScreens.SCREENS`.
    /// Any mod may register a factory for a custom menu type (including another mod's
    /// or a non-built-in `minecraft:*` menu type). Built-in vanilla menu types that
    /// already have a screen in `MenuScreens.SCREENS` and duplicate registrations for
    /// the same menu type refuse installation, even if the error is ignored.
    /// To customize a stock layout (such as a hopper), declare a distinct mod
    /// menu type with the same slots and open it from your block.
    /// The factory builds a fresh screen on the client for each opening. The
    /// window owns its callbacks and local slot moves until close or replacement.
    /// If a custom menu lacks a factory when it opens, the client warns and leaves its current menu unchanged.
    /// A package with a factory cannot declare `side = "server"`.
    fn register_menu_screen(
        &mut self,
        menu: &str,
        factory: Arc<dyn crate::MenuScreenFactory>,
    ) -> Result<(), String>;

    /// Declare this menu's book, authoritative recipe type and destination roles.
    /// `menu` and `recipe_type` must both pass [`crate::Identifier::parse_namespaced`]:
    /// both the namespace and the path must be explicit and nonempty (slash-separated paths
    /// are accepted; whitespace is never trimmed).
    /// The menu remains content-owned; loading checks these roles against its
    /// actual container slots and crafting declaration. A [`crate::RecipeBookLayout::Mod`]
    /// layout declares no static roles and answers recipe clicks through
    /// `MenuPart::recipe_book_place` by setting [`crate::RecipeBookPlacement`].
    /// To show this menu, register a factory with [`Self::register_menu_screen`]
    /// whose layout sets [`crate::MenuScreenLayout::recipe_book`] to this `book`.
    /// Content can load without the factory. The client checks for it when the
    /// menu opens and validates geometry when the factory constructs the screen. The
    /// declaring mod owns the screen's presentation callbacks and slot controls.
    /// Placement availability depends on the connection's negotiated custom-book
    /// capability and current opening, not client brand.
    /// This does not turn display candidates into ingredient predicates.
    /// Built-in menus, unknown books, duplicate menus or malformed roles
    /// refuse installation, even when the caller ignores the error.
    fn menu_recipe_book(
        &mut self,
        menu: &str,
        book: &crate::RecipeBookTypeKey,
        recipe_type: &str,
        layout: crate::RecipeBookLayout,
    ) -> Result<(), String>;
    /// Attach `part`, of kind `kind`, to the menu type `menu`.
    fn attach_menu_part(&mut self, menu: &str, kind: &'static str, part: Arc<dyn MenuPart>);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanilla_menu_list_matches_stock_menus_not_custom_minecraft_ids() {
        assert!(is_vanilla_menu_type("minecraft:hopper"));
        assert!(is_vanilla_menu_type("minecraft:generic_9x6"));
        assert!(is_vanilla_menu_type("minecraft:stonecutter"));
        assert!(!is_vanilla_menu_type("minecraft:custom_machine"));
        assert!(!is_vanilla_menu_type("demo:hopper"));
    }
}
