fn harm(value: bool, attacker: i32, victim: i32) -> bool {
    static SEEN: std::sync::Once = std::sync::Once::new();
    let _ = (attacker, victim);
    SEEN.call_once(|| fnr_lower::note("RETURN health::can_harm_player"));
    value
}
