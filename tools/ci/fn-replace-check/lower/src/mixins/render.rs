fn world_drawn() {
    static SEEN: std::sync::Once = std::sync::Once::new();
    SEEN.call_once(|| fnr_lower::note("HEAD world_drawn (sunrise_mod_render, library mod_render)"));
}
