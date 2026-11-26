fn takes(value: bool, layer: &str, part: &str) -> bool {
    static SEEN: std::sync::Once = std::sync::Once::new();
    SEEN.call_once(|| fnr_lower::note("RETURN mobs::render::takes_part"));
    value || (layer == "minecraft:player#main" && part == "right_arm")
}
