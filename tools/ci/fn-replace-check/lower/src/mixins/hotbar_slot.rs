fn hotbar_slot(value: usize, selected: usize) -> usize {
    static SEEN: std::sync::Once = std::sync::Once::new();
    let _ = selected;
    SEEN.call_once(|| fnr_lower::note("RETURN pick_item::suitable_hotbar_slot"));
    value
}
