use super::*;

impl Game<'_> {
    /// A mod menu's recipe-book click, answered by the part of the open menu's type: the part moves the
    /// recipe's ingredients itself and sets `ghost` when items are missing.
    #[inline(never)]
    pub(in crate::game) fn native_recipe_book_place(
        &mut self,
        call: mod_api::__private::RecipeBookPlaceCall<'_>,
    ) {
        self.fire_recipe_book_place(call);
    }
}
