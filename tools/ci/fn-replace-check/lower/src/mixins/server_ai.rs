fn server_ai(m: &Mob) -> ::core::ops::ControlFlow<ControlFlow<AiPause>> {
    static SEEN: std::sync::Once = std::sync::Once::new();
    SEEN.call_once(|| fnr_lower::note("HEAD mobs::ai::server_ai_step"));
    match m.custom_name.as_deref() {
        // The piglin the check summons as `fnr_cancel`: the row cancels its step, as a Java mod's
        // mixin skips `Mob.serverAiStep` for a stunned mob. After 40 cancelled steps it notes
        // whether its brain's sensor wait, its `ANGRY_AT` or its zombification count moved.
        Some("fnr_cancel") => {
            static FIRST: std::sync::Mutex<Option<(i32, i32, i32, u32)>> =
                std::sync::Mutex::new(None);
            let now = (m.extra.counter2, m.extra.piglin.sensor_wait, m.extra.anger);
            let mut first = FIRST.lock().unwrap_or_else(|e| e.into_inner());
            match first.as_mut() {
                None => *first = Some((now.0, now.1, now.2, 1)),
                Some(seen) => {
                    seen.3 += 1;
                    if seen.3 == 40 {
                        fnr_lower::note(&format!(
                            "server_ai cancelled fnr_cancel 40 steps, unchanged: {} \
                             (conversion {}->{}, sensor {}->{}, anger {}->{})",
                            (seen.0, seen.1, seen.2) == now,
                            seen.0,
                            now.0,
                            seen.1,
                            now.1,
                            seen.2,
                            now.2
                        ));
                    }
                }
            }
            ::core::ops::ControlFlow::Break(::core::ops::ControlFlow::Continue(()))
        }
        // Its control, `fnr_control`, runs its step: it converts, and the zombified piglin
        // keeps the name.
        Some("fnr_control") => {
            static CONVERTED: std::sync::Once = std::sync::Once::new();
            if &*m.kind.id == "minecraft:zombified_piglin" {
                CONVERTED.call_once(|| fnr_lower::note("server_ai fnr_control converted"));
            }
            ::core::ops::ControlFlow::Continue(())
        }
        _ => ::core::ops::ControlFlow::Continue(()),
    }
}
