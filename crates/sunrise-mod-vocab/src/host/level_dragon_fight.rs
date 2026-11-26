//! The `level_dragon_fight` area: the End's dragon fight, with or without a living dragon. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_dragon_fight` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelDragonFightBackend {
    /// Side: server.
    /// The level's dragon fight (`EnderDragonFight`): whether it is won, whether a dragon of
    /// it was killed before, the respawn stage and the crystals on the pillars, whether or
    /// not its dragon lives. Sunrise runs the End's fight only; it reads whether the End is in
    /// place or parked. None for a stale handle, a client, a level other than the End, and
    /// before the fight first ticks.
    fn level_dragon_fight(&self, level: LevelRef) -> Option<DragonFightState>;
}
/// The handles' writes of the `level_dragon_fight` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelDragonFightBackendMut {
    /// Side: server.
    /// `EnderDragonFight.tryRespawn`, as placing an End crystal does: when the fight is won
    /// and no respawn is under way, and a crystal stands on each side of the exit portal, the
    /// portal turns back to end stone and the respawn starts at
    /// [`DragonRespawnStage::Start`] (the `dragon_respawn_stage` hook hears it). With no exit
    /// portal yet, an active one is placed first, as vanilla does. True when a respawn
    /// started. False for a stale handle, a client, a level other than the End or parked,
    /// and when vanilla starts nothing.
    fn level_dragon_fight_try_respawn(&mut self, level: LevelRef) -> bool;
    /// Side: server.
    /// `EnderDragonFight.resetSpikeCrystals`: every crystal on a pillar top loses its
    /// protection and its beam, as an aborted or finished respawn leaves them. False for a
    /// stale handle, a client, and a level other than the End or parked.
    fn level_dragon_fight_reset_spike_crystals(&mut self, level: LevelRef) -> bool;
}
