//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
/// `Music`: a sound event and the delay range (ticks) before it starts.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct Music {
    pub sound: String,
    pub min_delay: i32,
    pub max_delay: i32,
    pub replace_current_music: bool,
}
impl Music {
    pub fn new(sound: &str, min_delay: i32, max_delay: i32, replace_current_music: bool) -> Self {
        Self {
            sound: sound.to_owned(),
            min_delay,
            max_delay,
            replace_current_music,
        }
    }
    /// `Musics.MENU`.
    pub fn menu() -> Self {
        Self::new("minecraft:music.menu", 20, 600, true)
    }
    /// `Musics.CREDITS`.
    pub fn credits() -> Self {
        Self::new("minecraft:music.credits", 0, 0, true)
    }
    /// `Musics.END_BOSS`.
    pub fn end_boss() -> Self {
        Self::new("minecraft:music.dragon", 0, 0, true)
    }
    /// `Musics.CREATIVE`.
    pub fn creative() -> Self {
        Self::new("minecraft:music.creative", 12000, 24000, false)
    }
    /// `Musics.END`.
    pub fn end() -> Self {
        Self::new("minecraft:music.end", 6000, 24000, true)
    }
    /// `Musics.UNDER_WATER`.
    pub fn under_water() -> Self {
        Self::new("minecraft:music.under_water", 12000, 24000, false)
    }
    /// `Musics.GAME`.
    pub fn game() -> Self {
        Self::new("minecraft:music.game", 12000, 24000, false)
    }
    /// `Music.CODEC`.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn parse(v: &Value, registered: &impl Fn(&str) -> bool) -> Option<Self> {
        let v = v.as_object()?;
        let (sound, _) = parse_sound_event(v.get("sound")?, registered)?;
        let delay = |name: &str| {
            let value = i32::try_from(v.get(name)?.as_i64()?).ok()?;
            (value >= 0).then_some(value)
        };
        Some(Self {
            sound,
            min_delay: delay("min_delay")?,
            max_delay: delay("max_delay")?,
            replace_current_music: match v.get("replace_current_music") {
                Some(value) => value.as_bool()?,
                None => false,
            },
        })
    }
}

/// `SoundEvent.CODEC`: a registered holder or an inline event with an optional
/// fixed range. Client sound instances play by identifier; the fixed range is
/// used when the server distributes a sound, not to resolve `sounds.json`.
// hidden: engine sound-event parser shared with client/world crates; not a mod-facing API
#[doc(hidden)]
pub fn parse_sound_event(
    v: &Value,
    registered: &impl Fn(&str) -> bool,
) -> Option<(String, Option<f32>)> {
    use sunrise_core::identifier::Identifier;

    if let Some(name) = v.as_str() {
        let sound = Identifier::parse(name).ok()?.to_string();
        return registered(&sound).then_some((sound, None));
    }
    let v = v.as_object()?;
    let sound = Identifier::parse(v.get("sound_id")?.as_str()?)
        .ok()?
        .to_string();
    // The direct codec deliberately ignores a malformed optional range.
    let range = v.get("range").and_then(Value::as_f64).map(|r| r as f32);
    Some((sound, range))
}

/// A track a native mod plays over or instead of the music
/// (`music_layers`): a sound event and its volume, 0 to 1.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct MusicLayer {
    pub sound: String,
    pub volume: f32,
}
