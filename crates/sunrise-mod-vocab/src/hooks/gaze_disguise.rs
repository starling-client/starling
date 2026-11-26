//! The `gaze_disguise` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether what `player` wears on their head disguises their gaze from
    /// `observer` (NeoForge's `IItemExtension.isGazeDisguise`, vanilla's
    /// `LivingEntity.PLAYER_NOT_WEARING_DISGUISE_ITEM`). `disguised` comes in
    /// as vanilla's answer: true when the head item is in
    /// `#minecraft:gaze_disguise_equipment`, as a carved pumpkin is. Set it
    /// true and the gaze doesn't count; set it false and it counts even under
    /// a carved pumpkin. Asked by an enderman each time its goals check a
    /// stare (`Enderman.isBeingStaredBy`, before the look is measured and
    /// before `enderman_anger`), and by an awake creaking each time it checks
    /// whether it is looked at (`Creaking.checkCanMove`; a dormant creaking
    /// wakes whatever the player wears, so it doesn't ask). Read the worn
    /// stack with `host.living(player.entity())` and `with_equipment`.
    /// Server-side: the mob AI asks through `MobWorld::mob_hooks`, set only on
    /// an authoritative server while a mod listens, so with no listener the
    /// check is the tag test alone.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        host = "read",
        neoforge = "IItemExtension.isGazeDisguise"
    )]
    fn gaze_disguise(observer: EntityRef, player: PlayerRef, disguised: &mut bool);
}
