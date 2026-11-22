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
    /// `namespace:path` or more than 64 of them. API @API:muffle_sounds@.
    fn muffle_sounds(&mut self, amount: f32, lowpass: f32, ticks: u32, exempt: &[&str]) -> bool;
    /// Side: client.
    /// A small local vanilla particle; caller should rate-limit emission.
    fn flame_particle(&mut self, at: [f64; 3], soul: bool);
    /// Side: client.
    /// Plays `sound` for this player only (`SoundPlay`'s event: `minecraft:ui.button.click`,
    /// or a mod's own from its `assets/<id>/sounds.json`): nothing is sent to the server, and
    /// other players don't hear it. It takes `&self`, so a screen, key or mouse hook can play
    /// one: the sound is queued and starts at the next audio update (once a client tick),
    /// `after` ticks counted from then (0 to 1200, a minute; `SoundEngine.playDelayed`). `SoundPlay::ui` plays it at the listener as an interface sound
    /// (`SimpleSoundInstance.forUI`); `SoundPlay::at` at a position in the client's level,
    /// fading with distance (`volume` past 1 carries farther, 16 blocks times the volume), as
    /// thunder heard after the flash or a blast heard when its shock wave arrives. Its
    /// `SoundRef` follows and controls this play: `state`, `stop` and `fade`. None, with nothing
    /// played, on a server or for a value out of range (`SoundPlay::effects` included, by
    /// `SoundEffects::valid`); an event the resources don't have plays nothing, as vanilla's,
    /// and its handle reads `Stopped` once the update has tried it. Its effects stay as given
    /// unless the sound ticks: a `play_sound` listener can change them on each live refresh.
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
    /// queued; false on a server, without a client player or world, for non-finite input or for
    /// more than 256 explosions. API @API:client_fireworks@.
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
    /// Fades the sound's volume to a target over some unpaused ticks; a fade to 0 stops it at the
    /// end. While the game is paused the fade waits, and an interface or music sound keeps playing.
    fn sound_fade(&self, sound: SoundRef, volume: f32, ticks: u32) -> bool;
    /// Side: client.
    /// Moves the playing sound to `position`, in blocks of the client's level, from the next
    /// audio update: the same instance keeps playing, with no restart, and its distance
    /// fading follows. A sound that plays at the listener (`SoundPlay::ui`) has no position to
    /// move. False, changing nothing, for a position that isn't finite or lies past 30 million
    /// blocks, and for a sound that is already over.
    fn sound_move_to(&self, sound: SoundRef, position: [f64; 3]) -> bool;
    /// Side: client.
    /// Sets the playing sound's pitch (1.0 unchanged) from the next audio update, on the same
    /// instance with no restart. False, changing nothing, for a pitch that isn't a number at
    /// or above 0, as `SoundPlay`'s, and for a sound that is already over.
    fn sound_set_pitch(&self, sound: SoundRef, pitch: f32) -> bool;
}
