//! The `tick_weather_effects` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Each client tick, before the client runs vanilla's per-tick weather effects
    /// (`ClientLevel.tickWeatherEffects`): the rain splash particles around the camera, the
    /// smoke where rain falls on lava, magma blocks and lit campfires, and the ambient rain
    /// sound. `dimension` is the level the client shows, `game_time` its clock, `camera` the
    /// camera's position and `rain` the rain level (0 when it isn't raining). Set `canceled` to
    /// skip all of it for this tick, the splash scan and the sound timer included, and run your
    /// own weather (NeoForge's `CustomWeatherEffectRenderer.tickRain` returning true). Asked
    /// every tick a level is shown, raining or not, and with the Sunrise Weather option off,
    /// which turns vanilla's effects off on its own. Client-side; the rain columns drawn each
    /// frame are not part of it.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "canceled",
        cancellable = true,
        neoforge = "CustomWeatherEffectRenderer.tickRain"
    )]
    fn tick_weather_effects(
        dimension: &str,
        game_time: i64,
        camera: [f64; 3],
        rain: f32,
        canceled: &mut bool,
    );
}
