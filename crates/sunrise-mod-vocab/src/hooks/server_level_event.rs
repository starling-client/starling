//! The `server_level_event` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A level event is about to go out from the server (`ServerLevel.levelEvent` and
    /// `globalLevelEvent`): event `id` (1000s are sounds, 2000s particles, 3000s other effects)
    /// at `pos` in level `dimension`, with its `data`. `global` is a global event (the wither's
    /// spawn 1023, the dragon's death 1028, an End portal opening 1038), sent to every player
    /// while the `global_sound_events` rule is on; otherwise it goes to the players within 64
    /// blocks in that level, as a plain event. Set `canceled` and it goes to nobody: no
    /// `ClientboundLevelEventPacket` is sent, and the world's own player neither hears nor sees
    /// it. To narrow an event instead, cancel it and play your own sound for the players you
    /// choose (`PlayerRef::play_sound`) or near the position (`LevelRef::play_sound`). Asked
    /// for every level event the server sends, the game's and mods' alike (`LevelRef::level_event`
    /// and `global_level_event`), so check `id` first. Server-side. NeoForge has no event for
    /// this.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "canceled",
        cancellable = true,
        neoforge = "none (vanilla ServerLevel.levelEvent and globalLevelEvent; NeoForge adds no event)"
    )]
    fn server_level_event(
        dimension: &str,
        id: i32,
        pos: [i32; 3],
        data: i32,
        global: bool,
        canceled: &mut bool,
    );
}
