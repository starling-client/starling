//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `effects` area, implemented by the game and reached through [`crate::ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait EffectsClientHost {
    /// Muffles the game's sounds for a while, as ears ringing after a close
    /// blast hear them: `amount` (0 to 1) of every sound's volume is taken
    /// away and `lowpass` (0 to 1) dulls it, from untouched to a 300 Hz
    /// low-pass. Both start in full and fade back in a straight line over
    /// `ticks` client ticks, held while the game is paused. The sound events in
    /// `exempt` (`mymod:ear.ringing`) play clearly through it, those already
    /// playing too, so the mod's own ringing stays sharp. Everything else is
    /// muffled, music and the interface's clicks included. A new call replaces
    /// the muffle in place; `ticks` 0 ends it at once. Nothing changes while no
    /// mod calls it. False, with nothing changed, on a server, for an amount or
    /// strength outside 0 to 1, or for an exempt event that isn't
    /// `namespace:path`. API @API:muffle_sounds@.
    fn muffle_sounds(&mut self, amount: f32, lowpass: f32, ticks: u32, exempt: &[&str]) -> bool;
    /// Side: client.
    /// Scales `source`'s volume by `scale` (0 to 1, clamped), the sounds playing now included
    /// (music, a record), as the Options volume sliders update live channels
    /// (`SoundEngine.updateCategoryVolume`). It multiplies the player's slider, which keeps its
    /// own value and is never saved with the scale; the master's scale scales every source.
    /// 1 undoes it. One scale per source for all mods: the last call wins. A mod may lower the
    /// volume while the window is in the background, for example.
    fn set_sound_source_scale(&mut self, source: SoundSource, scale: f32);
    /// Side: client.
    /// A mod's scale over `source`'s volume (`set_sound_source_scale`), 1 when none is set. For
    /// a source other than the master, the master's scale is included.
    fn sound_source_scale(&self, source: SoundSource) -> f32;
    /// Side: client.
    /// Pauses every playing sound, music and the interface's included, until `false` resumes
    /// them (`Channel.pause`, `unpause`); a pause the game holds itself (its pause menu) stays
    /// held. It takes effect on the next client tick, as the game's own pause does. New sounds
    /// still start while paused. One switch for all mods: the last call wins.
    fn set_sounds_paused(&mut self, paused: bool);
    /// Side: client.
    /// Plays `sound` for this player only (`SoundPlay`'s event: `minecraft:ui.button.click`,
    /// or a mod's own from its `assets/<id>/sounds.json`): nothing is sent to the server, and
    /// other players don't hear it. It takes `&self`, so a screen, key or mouse hook can play
    /// one: the sound is queued and starts at the next audio update (once a client tick),
    /// `after` unpaused sound-engine ticks from then (`SoundEngine.playDelayed`).
    /// `SoundPlay::ui` starts at the listener (`SimpleSoundInstance.forUI`); `SoundPlay::at`
    /// starts at a position in the client's level with distance attenuation. The sound engine
    /// computes final volume and pitch; a volume above 1 can carry farther.
    ///
    /// Returns a `SoundRef` for this play, not a promise that it will be audible. Returns `None`
    /// on a server, for a malformed namespaced sound event ID, a position that would not be finite
    /// in the mixer's float coordinates, non-finite volume or pitch, or invalid `SoundEffects`.
    /// An event the resources don't have plays nothing and its handle reads `Stopped` once tried.
    fn play_local_sound(&self, sound: &SoundPlay, after: u32) -> Option<SoundRef>;
    /// Side: client.
    /// The particles alive in the client's level (`ParticleEngine.countParticles`): 0 once
    /// a clear is asked, and on a server.
    fn particle_count(&self) -> u32;
    /// Side: client.
    /// `ParticleEngine.clearParticles`: every particle and emitter goes before the next
    /// frame is drawn or the next particle tick runs, whichever comes first. It takes
    /// `&self`, so a screen or render hook can ask, as with `native_play_local_sound`.
    /// False on a server.
    fn clear_particles(&self) -> bool;
    /// Side: client.
    /// ClientLevel.addParticle: submits one local particle through the ordinary provider, camera-distance, setting and population limits. False without a loaded client world, for non-finite input or for an item crumb whose colour has a component outside 0 to 1. True means submitted, not necessarily visible.
    /// For a registered `ParticleType`, pass `ty.options(value)`; the matching client factory
    /// receives the value and original auxiliary inputs before the particle is queued.
    fn add_particle(&mut self, options: &ParticleOptions, at: [f64; 3], velocity: [f64; 3])
    -> bool;
    /// Side: client.
    /// `ClientLevel.createFireworks`: draws a firework's explosions in this client's world as a
    /// bursting rocket draws them (`FireworkParticles.Starter`): each explosion's shape, colours,
    /// fade colours, trail and twinkle, one every other tick. The blast, and the crackle after a
    /// twinkling explosion, play only when `request.audible`, as for a rocket that isn't silent.
    /// Only the picture and the sound: no entity, no damage, nothing sent to the server, and other
    /// players see nothing (a mod's server half tells each client half to draw it). True means
    /// queued; false on a server, without a client player or world, or for non-finite input.
    /// API @API:client_fireworks@.
    fn create_fireworks(&mut self, request: &FireworksRequest) -> bool;
    /// Every sound event the client's resources have (`SoundManager.getAvailableSounds`).
    fn sound_events(&self) -> Vec<Identifier>;
    /// Side: client.
    /// The output devices sound can play on (`SoundEngine.getAvailableSoundDevices`).
    fn audio_devices(&self) -> Vec<String>;
    /// Side: client.
    /// The output device the options ask for (`Options.soundDevice`).
    fn audio_device(&self) -> String;
}
/// The handles' client reads of the `effects` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EffectsClientBackend {
    /// Side: client.
    /// Where a sound `play_local_sound` started is: waiting to start, playing, or over.
    fn sound_state(&self, sound: SoundRef) -> Option<SoundState>;
    /// Side: client.
    /// Stops the sound for good: a waiting one never starts and a looping one never repeats.
    fn sound_stop(&self, sound: SoundRef) -> bool;
    /// Side: client.
    /// Fades the sound's volume to a finite target over unpaused ticks, or immediately for zero
    /// ticks. A target of zero or less stops the sound at the end. While paused, the fade waits
    /// and interface or music sounds keep playing. False for non-finite volume or a stale sound.
    fn sound_fade(&self, sound: SoundRef, volume: f32, ticks: u32) -> bool;
    /// Side: client.
    /// Moves the playing sound to `position`, in blocks of the client's level, from the next
    /// audio update, without restarting it. For a relative sound, this sets its offset from
    /// the listener. False, changing nothing, if any coordinate would not be finite in the
    /// mixer's float coordinates or if the sound is already over.
    fn sound_move_to(&self, sound: SoundRef, position: [f64; 3]) -> bool;
    /// Side: client.
    /// Sets the playing sound's base pitch from the next audio update, without restarting it.
    /// The sound engine clamps the final pitch after sound-file scaling (`SoundEngine.calculatePitch`).
    /// False, changing nothing, for a non-finite pitch or for a sound that is already over.
    fn sound_set_pitch(&self, sound: SoundRef, pitch: f32) -> bool;
}
