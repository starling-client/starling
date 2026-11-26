use super::*;

impl Game<'_> {
    /// `EnchantingTableBlock.isValidBookShelf` for the position `offset` from the table at
    /// `table_pos`: `valid` arrives with vanilla's answer and listeners may change it. Both
    /// sides.
    pub(in crate::game) fn native_is_valid_book_shelf(
        &self,
        level: mod_api::LevelRef,
        table_pos: BlockPos,
        offset: BlockPos,
        valid: &mut bool,
    ) {
        self.fire_is_valid_book_shelf(level, table_pos, offset, valid);
    }
}
