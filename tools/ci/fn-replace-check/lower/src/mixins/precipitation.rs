fn precipitation(value: PrecipitationKind, pos: BlockPos) -> PrecipitationKind {
    static SEEN: std::sync::Once = std::sync::Once::new();
    let _ = pos;
    SEEN.call_once(|| fnr_lower::note("RETURN level::weather::precipitation_at"));
    value
}
