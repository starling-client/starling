fn free_slot(value: Option<usize>, items: &[Option<ItemStack>]) -> Option<usize> {
    static SEEN: std::sync::Once = std::sync::Once::new();
    let _ = items.len();
    SEEN.call_once(|| fnr_lower::note("RETURN menu::free_slot"));
    value
}
