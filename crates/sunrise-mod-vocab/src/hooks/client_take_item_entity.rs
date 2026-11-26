//! The `client_take_item_entity` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// This client heard that `collector` picked up `collected` (vanilla's
    /// `ClientPacketListener.handleTakeItemEntity`, the `ClientboundTakeItemEntityPacket`): an item,
    /// an experience orb, an arrow or a trident, taken by this player, another player or a mob.
    /// Told before the client plays the pickup sound, starts the pickup particle and shrinks or
    /// removes `collected`, so `host` still reads it (its stack, for an item). `amount` is the
    /// packet's count taken (`getAmount`): the items taken from an item's stack, 1 otherwise.
    ///
    /// Told only when this client tracks `collected`, as vanilla's handler acts only then
    /// (`if (from != null)`). `collector` is None when this client doesn't track the packet's
    /// collector; vanilla then flies the particle to this player instead. Only on a client of a
    /// server, a LAN guest included; a server's own pickups are `item_picked_up`, `pickup_xp` and
    /// `player_pickup_arrow`. Client-side; with no listener a pickup costs one flag check.
    #[notes = r##"## Pickups a client sees

`client_take_item_entity(collected, collector, amount)` runs on a client of a server when the
server tells it a nearby entity picked something up (`ClientboundTakeItemEntityPacket`), before the
client shrinks or removes what was taken. A client-only mod can list recent pickups, or tell the
local player's own (`collector` is the local player's entity) from other players' and mobs', on
any server; the server-side pickup hooks never run on a client of an unmodded server."##]
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "none (vanilla ClientPacketListener.handleTakeItemEntity; NeoForge's pickup events are server-side)"
    )]
    fn client_take_item_entity(collected: EntityRef, collector: Option<EntityRef>, amount: i32);
}
