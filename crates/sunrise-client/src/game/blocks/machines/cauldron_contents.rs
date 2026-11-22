pub(in crate::game) use sunrise_gameplay::cauldron_contents::{
    CauldronContent, CauldronContents, EMPTY_CAULDRON, current, current_if_built, install,
    vanilla_block_has_fluids,
};

#[cfg(test)]
pub(in crate::game) fn without_built_table(test: impl FnOnce()) {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            sunrise_gameplay::cauldron_contents::swap_built(self.0);
        }
    }
    let _restore = Restore(sunrise_gameplay::cauldron_contents::swap_built(false));
    test();
}
