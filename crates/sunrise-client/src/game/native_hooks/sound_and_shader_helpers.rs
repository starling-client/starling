use mod_api::SoundPlay;

/// Whether `id` is a sound event's id, `namespace:path` as `Identifier`
/// allows: the namespace of `[a-z0-9_.-]`, the path also with `/`.
pub(in crate::game) fn sound_id(id: &str) -> bool {
    let Some((namespace, path)) = id.split_once(':') else {
        return false;
    };
    let allowed = |c: char, slash: bool| {
        c.is_ascii_lowercase() || c.is_ascii_digit() || "_.-".contains(c) || (slash && c == '/')
    };
    !namespace.is_empty()
        && !path.is_empty()
        && namespace.chars().all(|c| allowed(c, false))
        && path.chars().all(|c| allowed(c, true))
}

/// Whether a sound can be sent: a position inside the world's range and a
/// finite, non-negative volume and pitch.
pub(in crate::game) fn level_sound_playable(at: [f64; 3], volume: f32, pitch: f32) -> bool {
    let position_ok = at
        .into_iter()
        .all(|v| v.is_finite() && v.abs() <= f64::from(i32::MAX) / 8.0);
    let gain_ok = volume.is_finite() && volume >= 0.0 && pitch.is_finite() && pitch >= 0.0;
    position_ok && gain_ok
}

/// Whether a native mod's sound can be sent: a namespaced event, a sound
/// source, a position inside the world's range and a finite, non-negative
/// volume and pitch.
pub(in crate::game) fn mod_sound_ok(
    sound: &str,
    at: [f64; 3],
    source: usize,
    volume: f32,
    pitch: f32,
) -> bool {
    sound_id(sound)
        && source < sunrise_client::settings::SOUND_SOURCES.len()
        && level_sound_playable(at, volume, pitch)
}

/// Whether `play_local_sound` takes this sound: a sound event's id (`sound_id`, as
/// `native_play_sound` takes), a finite position, a volume and pitch at or above 0, a repeat
/// gap that isn't negative, valid effects, and a start a minute away at most.
pub(super) fn local_sound_ok(sound: &SoundPlay, after: u32) -> bool {
    sound_id(&sound.identifier)
        && sound
            .position
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 30_000_000.0)
        && sound.volume.is_finite()
        && sound.volume >= 0.0
        && sound.pitch.is_finite()
        && sound.pitch >= 0.0
        && sound.delay >= 0
        && sound.effects.valid()
        && after <= 1200
}

/// `params` with each value that isn't finite as 0, so a mod's mistake can't
/// put a NaN in every pixel of a pack's picture.
pub(super) fn finite_params(params: [[f32; 4]; 8]) -> [[f32; 4]; 8] {
    params.map(|v| v.map(|x| if x.is_finite() { x } else { 0.0 }))
}

/// `v` within `lo..=hi`; `lo` when it is not a number.
pub(super) fn finite(v: f32, lo: f32, hi: f32) -> f32 {
    if v.is_finite() { v.clamp(lo, hi) } else { lo }
}
