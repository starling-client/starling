fn tick_precipitation(value: (), pos: BlockPos) {
    static SEEN: std::sync::Once = std::sync::Once::new();
    let _ = (value, pos);
    SEEN.call_once(|| fnr_lower::note("RETURN level::Behaviours::tick_precipitation"));
}
