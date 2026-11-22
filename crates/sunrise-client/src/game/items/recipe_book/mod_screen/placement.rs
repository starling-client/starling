//! Placement state is discarded before a new catalogue or menu can be shown.
use super::*;
/// Ghosts kept from the game and a menu part together, as many as a menu may have slots.
const MAX_GHOSTS: usize = 256;
/// The ghosts to draw: at most `MAX_GHOSTS`, in order, without one in a slot the screen lacks or one
/// that resolves to no stack (`GhostSlots.setSlot` keeps only ghosts with a stack).
fn shown_ghosts(
    base: usize,
    ghosts: Vec<mod_api::GhostSlot>,
    has_slot: impl Fn(u16) -> bool,
    resolve: impl Fn(&mod_api::RecipeSlotDisplay) -> Vec<ItemStack>,
) -> Vec<super::super::Ghost> {
    ghosts
        .into_iter()
        .take(MAX_GHOSTS)
        .filter(|ghost| has_slot(ghost.slot))
        .filter_map(|ghost| {
            let items = resolve(&ghost.display);
            (!items.is_empty()).then(|| super::super::Ghost {
                slot: base + usize::from(ghost.slot),
                items,
                result: ghost.result,
            })
        })
        .collect()
}
pub(super) struct PlacementOverlay {
    pub(super) view: OverlayView,
    pub(super) recipe: String,
    pub(super) sources: Vec<u32>,
}
impl Game<'_> {
    pub(super) fn request_mod_book_recipe(&mut self, recipe: String, source: u32) {
        if !self
            .native_net
            .placement
            .client
            .window()
            .is_some_and(|window| self.native_book_placement_window_matches(window))
        {
            return;
        }
        let Ok(recipe) = recipe.try_into() else {
            return;
        };
        let use_max = self.has_shift_down();
        if let Some(frame) = self.native_net.placement.client.select(
            sunrise_client::recipe_book::placement_sync::DisplayId { recipe, source },
            use_max,
        ) {
            self.send_mod_book_placement(frame);
            if self
                .batch_for_size()
                .is_some_and(|batch| self.mod_book_overlay(batch.width))
                && let Some((_, filtering)) = self.mod_book_flags()
                && let Some(book) = self.native_screen_book().cloned()
            {
                self.request_custom_book_settings(
                    &book,
                    mod_api::RecipeBookSettings {
                        open: false,
                        filtering,
                    },
                );
            }
        }
    }
    pub(super) fn consume_mod_book_overlay(&mut self, right: bool) -> bool {
        let Some(overlay) = self.recipe_ui.custom.overlay.take() else {
            return false;
        };
        if !right {
            let columns = if overlay.view.recipes.len() <= 16 {
                4
            } else {
                5
            };
            for (n, source) in overlay.sources.iter().enumerate() {
                let x = overlay.view.x as f32 + 4.0 + 25.0 * (n % columns) as f32;
                let y = overlay.view.y as f32 + 5.0 + 25.0 * (n / columns) as f32;
                if inside(self.cursor, x, y, 24.0, 24.0) {
                    self.request_mod_book_recipe(overlay.recipe, *source);
                    break;
                }
            }
        }
        true
    }
    pub(in crate::game) fn resolve_mod_book_declaration_slot(
        &self,
        slot: &mod_api::RecipeSlotDisplay,
    ) -> Vec<ItemStack> {
        let stocks = sunrise_client::menu::place::Stacked::default();
        resolve::Resolver::new(self, &stocks, &[]).resolve(slot)
    }
    pub(in crate::game) fn install_mod_book_ghost(
        &mut self,
        selection: &sunrise_client::recipe_book::placement_sync::Selection,
    ) {
        let Some(view) = self
            .native_net
            .placement
            .view
            .as_ref()
            .filter(|view| view.identity == selection.catalog)
        else {
            return;
        };
        let Some(layout) = view.index.layout(selection.catalog.menu.as_str()).cloned() else {
            return;
        };
        let Some(display) = view
            .index
            .categories()
            .flat_map(|category| &category.displays)
            .find(|entry| entry.recipe == selection.display.recipe.as_str())
            .and_then(|entry| entry.displays.get(selection.display.source as usize))
            .cloned()
        else {
            return;
        };
        let result = match layout.declaration() {
            mod_api::RecipeBookLayout::Crafting { result, .. }
            | mod_api::RecipeBookLayout::Furnace { result, .. }
            | mod_api::RecipeBookLayout::Stonecutter { result, .. }
            | mod_api::RecipeBookLayout::Smithing { result, .. } => Some(*result),
            mod_api::RecipeBookLayout::Mod => None,
        };
        let base = self.native_screen_container_slot_base();
        let fuel_empty = match layout.declaration() {
            mod_api::RecipeBookLayout::Furnace { fuel, .. } => {
                self.slot_mut(base + usize::from(*fuel)).is_none()
            }
            _ => true,
        };
        // The game's own ghosts for the layout (none for `Mod`), then the menu part's say.
        let mut ghosts: Vec<mod_api::GhostSlot> = layout
            .map(&display, fuel_empty)
            .unwrap_or_default()
            .into_iter()
            .map(|(slot, display)| mod_api::GhostSlot {
                slot,
                display,
                result: Some(slot) == result,
            })
            .collect();
        let menu = selection.catalog.menu.as_str();
        if mod_api::__private::menu_listens(mod_api::HookId::RecipeBookGhost, menu) {
            self.native_recipe_book_ghost(mod_api::__private::RecipeBookGhostCall {
                menu,
                recipe: selection.display.recipe.as_str(),
                display: &display,
                ghosts: &mut ghosts,
            });
        }
        let shown = shown_ghosts(
            base,
            ghosts,
            |slot| self.native_screen_slot(usize::from(slot)).is_some(),
            |display| self.resolve_mod_book_declaration_slot(display),
        );
        self.recipe_ui.ghost = shown;
    }
    pub(in crate::game) fn reset_mod_book_placement_ui(&mut self) {
        self.recipe_ui.ghost.clear();
        self.recipe_ui.custom.prepared = None;
        self.recipe_ui.custom.presentation = None;
        self.recipe_ui.custom.overlay = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ghost(slot: u16, item: &str, result: bool) -> mod_api::GhostSlot {
        mod_api::GhostSlot {
            slot,
            display: mod_api::RecipeSlotDisplay::Item(item.into()),
            result,
        }
    }

    #[test]
    fn ghosts_without_a_slot_or_a_stack_are_dropped_and_the_rest_keep_their_order() {
        let shown = shown_ghosts(
            300,
            vec![
                ghost(6, "demo:meal", true),
                ghost(40, "demo:a", false),
                ghost(0, "demo:none", false),
                ghost(1, "demo:b", false),
            ],
            |slot| slot < 10,
            |display| match display {
                mod_api::RecipeSlotDisplay::Item(id) if id != "demo:none" => {
                    vec![ItemStack::new(id, 1)]
                }
                _ => Vec::new(),
            },
        );
        let seen: Vec<_> = shown
            .iter()
            .map(|g| (g.slot, g.items[0].item.to_string(), g.result))
            .collect();
        assert_eq!(
            seen,
            [
                (306, "demo:meal".to_owned(), true),
                (301, "demo:b".to_owned(), false)
            ]
        );
    }

    #[test]
    fn no_more_than_the_cap_are_kept() {
        let many = (0..300).map(|n| ghost(n % 10, "demo:a", false)).collect();
        let shown = shown_ghosts(300, many, |_| true, |_| vec![ItemStack::new("demo:a", 1)]);
        assert_eq!(shown.len(), 256);
    }
}
