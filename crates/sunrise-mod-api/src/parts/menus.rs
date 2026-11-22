//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;
#[cfg(test)]
#[path = "menus/book_tests.rs"]
mod book_tests;
#[cfg(test)]
#[path = "menus/screen_tests.rs"]
mod screen_tests;

/// Vanilla's 25 built-in menu types that already carry screens in `MenuScreens.SCREENS`.
const BUILTIN_MENU_SCREENS: &[&str] = &[
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

impl MenusRegistrar for PartRegistrar {
    fn register_menu_screen(
        &mut self,
        menu: &str,
        factory: Arc<dyn crate::MenuScreenFactory>,
    ) -> Result<(), String> {
        let result = (|| {
            if mod_vocab::__private::parts_installation::parts_installed() {
                return Err("registry is frozen");
            }
            crate::Identifier::parse_namespaced(menu)
                .ok_or("menu must be a namespaced identifier")?;
            if BUILTIN_MENU_SCREENS.contains(&menu) || self.menu_screen_factories.contains_key(menu)
            {
                return Err("duplicate menu screen factory");
            }
            self.menu_screen_factories.insert(
                menu.into(),
                MenuScreenRegistration {
                    owner: self.mod_id,
                    index: self.mod_index,
                    factory,
                    panicked: Default::default(),
                },
            );
            Ok(())
        })();
        result.map_err(|reason| {
            let message = format!("mod {} menu {menu:?}: {reason}", self.mod_id);
            self.refused.push(message.clone());
            message
        })
    }

    fn menu_recipe_book(
        &mut self,
        menu: &str,
        book: &crate::RecipeBookTypeKey,
        recipe_type: &str,
        layout: crate::RecipeBookLayout,
    ) -> Result<(), String> {
        let result: Result<(), String> = (|| {
            if mod_vocab::__private::parts_installation::parts_installed() {
                return Err("registry is frozen".into());
            }
            let menu_id =
                crate::Identifier::parse(menu).map_err(|e| format!("invalid menu: {e:?}"))?;
            if !menu.contains(':') || !owns_namespace(self.mod_id, menu_id.namespace()) {
                return Err(
                    "menu namespace must be explicit and owned by the registering mod".into(),
                );
            }
            if !self.recipe_book_types.contains_key(book.as_str()) {
                return Err(format!("unknown recipe book {}", book.as_str()));
            }
            if self.menu_recipe_books.contains_key(menu) {
                return Err("duplicate menu recipe book".into());
            }
            crate::Identifier::parse(recipe_type)
                .map_err(|e| format!("invalid recipe type: {e:?}"))?;
            if !recipe_type.contains(':') {
                return Err("explicit recipe type namespace required".into());
            }
            layout.validate(i16::MAX as u16 + 1)?;
            self.menu_recipe_books.insert(
                menu.into(),
                MenuBookDeclaration {
                    owner: self.mod_id,
                    book: book.clone(),
                    recipe_type: recipe_type.into(),
                    layout,
                },
            );
            Ok(())
        })();
        result.map_err(|reason| {
            let message = format!("mod {} menu {menu:?}: {reason}", self.mod_id);
            self.refused.push(message.clone());
            message
        })
    }
    fn attach_menu_part(&mut self, menu: &str, kind: &'static str, part: Arc<dyn MenuPart>) {
        self.attach(|p| &mut p.menus, kind, menu, part);
    }
}
