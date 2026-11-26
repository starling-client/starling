//! Extra bookshelf scan offsets of the enchanting table, collected across mods in load order.
use super::*;

impl EnchantingTableOffsetsRegistrar for PartRegistrar {
    fn add_enchanting_table_offsets(&mut self, offsets: &[[i32; 3]]) {
        for offset in offsets {
            if !self.enchanting_table_offsets.contains(offset) {
                self.enchanting_table_offsets.push(*offset);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_keep_load_order_and_drop_repeats_across_mods() {
        let mut registrar = PartRegistrar::new("first");
        registrar.add_enchanting_table_offsets(&[[3, 0, 0], [0, 0, 3], [3, 0, 0]]);
        registrar.mod_id = "second";
        registrar.add_enchanting_table_offsets(&[[0, 0, 3], [-3, 0, 0]]);
        assert_eq!(
            registrar.enchanting_table_offsets,
            [[3, 0, 0], [0, 0, 3], [-3, 0, 0]]
        );
        assert!(registrar.refused.is_empty());
    }
}
