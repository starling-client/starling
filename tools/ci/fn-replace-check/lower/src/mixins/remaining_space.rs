fn remaining_space(value: Option<usize>, stack: &ItemStack, max: u32) -> Option<usize> {
    static SEEN: std::sync::Once = std::sync::Once::new();
    let _ = (stack.count, max);
    SEEN.call_once(|| fnr_lower::note("RETURN menu::slot_with_remaining_space"));
    value
}
