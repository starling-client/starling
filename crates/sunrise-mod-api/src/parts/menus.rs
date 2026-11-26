//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;
#[cfg(test)]
#[path = "menus/book_tests.rs"]
mod book_tests;
#[cfg(test)]
#[path = "menus/screen_tests.rs"]
mod screen_tests;

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
            if mod_vocab::__private::is_vanilla_menu_type(menu)
                || self.menu_screen_factories.contains_key(menu)
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
            crate::Identifier::parse_namespaced(menu)
                .ok_or("menu must be a namespaced identifier")?;
            if mod_vocab::__private::is_vanilla_menu_type(menu) {
                return Err("cannot bind a recipe book to a built-in menu".into());
            }
            if !self.recipe_book_types.contains_key(book.as_str()) {
                return Err(format!("unknown recipe book {}", book.as_str()));
            }
            if self.menu_recipe_books.contains_key(menu) {
                return Err("duplicate menu recipe book".into());
            }
            crate::Identifier::parse_namespaced(recipe_type)
                .ok_or("recipe type must be a namespaced identifier")?;
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
