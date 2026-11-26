fn melee(goal: &MeleeAttack, m: &Mob) -> ::core::ops::ControlFlow<()> {
    static SEEN: std::sync::Once = std::sync::Once::new();
    let _ = (goal, m);
    SEEN.call_once(|| fnr_lower::note("HEAD mobs::goals::MeleeAttack::check_and_perform_attack"));
    ::core::ops::ControlFlow::Continue(())
}
