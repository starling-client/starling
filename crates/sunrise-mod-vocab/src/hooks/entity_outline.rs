//! The `entity_outline` declarations reconciled with the fork API.

pub trait Hooks {
    /// Whether an entity the client draws has the glowing outline, and in which
    /// colour (`Entity.isCurrentlyGlowing` and `getTeamColor`, what Fabric's and
    /// NeoForge's glowing and team colour overrides change). `outline` comes
    /// in as vanilla's answer: `Some(0xRRGGBB)`, the team's colour or white,
    /// when the entity glows, None when it doesn't. Set `Some` to make any
    /// entity glow in that colour, None to take its outline away. It changes
    /// only what this client draws: the entity's `Glowing` flag, what the
    /// server and other players see, stays. Asked each frame for every mob,
    /// player (yourself in third person when you hold the camera), dropped item and projectile
    /// drawn. In the ordered body pass, these queries for mobs, players and dropped items follow
    /// level join order, interleaved with general entity renderer submissions. Projectile queries
    /// run in a later pass. General entities, the dragon and end crystals add no outline queries.
    /// API @API:entity_outline@.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 3,
        contract = "listen",
        neoforge = "none (NeoForge 26.3 has no glowing or team colour override)"
    )]
    fn entity_outline(entity_id: u32, kind: &str, outline: &mut Option<u32>);
}
