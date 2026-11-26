//! A data component's own tooltip lines (NeoForge's `RegisterTooltipAppendersEvent`): a mod
//! registers an appender for one component with `Registrar::register_tooltip_appender`, and the
//! game asks it for a stack that carries the component and whose tooltip shows it.

use crate::{ClientHost, ItemStack, Text};

/// The data components whose lines the game itself adds to a tooltip, in the order it adds them:
/// NeoForge's `VanillaDataComponentTooltips.collectVanillaAppenders`. They are the first nodes
/// of the order the appenders are sorted in, each before the next.
pub const VANILLA_TOOLTIP_COMPONENTS: &[&str] = &[
    "minecraft:tropical_fish/pattern",
    "minecraft:instrument",
    "minecraft:map_id",
    "minecraft:bees",
    "minecraft:container_loot",
    "minecraft:container",
    "minecraft:banner_patterns",
    "minecraft:pot_decorations",
    "minecraft:written_book_content",
    "minecraft:charged_projectiles",
    "minecraft:fireworks",
    "minecraft:firework_explosion",
    "minecraft:potion_contents",
    "minecraft:jukebox_playable",
    "minecraft:trim",
    "minecraft:stored_enchantments",
    "minecraft:enchantments",
    "minecraft:dyed_color",
    "minecraft:profile",
    "minecraft:lore",
    "minecraft:sulfur_cube_content",
    "minecraft:attribute_modifiers",
    "minecraft:intangible_projectile",
    "minecraft:unbreakable",
    "minecraft:sign_text_front",
    "minecraft:sign_text_back",
    "minecraft:ominous_bottle_amplifier",
    "minecraft:suspicious_stew_effects",
    "minecraft:block_state",
    "minecraft:entity_data",
    "minecraft:block_entity_data",
    "minecraft:can_break",
    "minecraft:can_place_on",
    "minecraft:damage",
];

/// Where a component's lines go among the other components' lines (NeoForge's
/// `registerComponentAppenderBeforeAll`, `Before`, `After` and `AfterAll`). The game sorts every
/// component that adds lines as `ItemTooltipHandler` does: the
/// [`VANILLA_TOOLTIP_COMPONENTS`] chained in their order, and each appender after or before its
/// anchor, in a topological sort that takes the earliest-added node whenever several are free
/// (FML's `TopologicalSort` over insertion order: the vanilla components, then the appenders in
/// registration order). So `Before` puts the lines just before its anchor, while `After` only
/// keeps them after it: with nothing holding them back, they follow the last vanilla component. `Before` and `After` name a vanilla component or one
/// another appender is registered for; any other anchor, or anchors that form a cycle, refuse
/// the mods at startup. The `registerAppender(TooltipLocation, ...)` overload is not represented.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum TooltipPlacement {
    /// Before the first vanilla component's lines, after the item's own lines.
    BeforeAll,
    /// Just before the lines of the component named.
    Before(String),
    /// After the lines of the component named; see the order above for where exactly.
    After(String),
    /// After the last vanilla component's lines (`minecraft:damage`, the durability line),
    /// before the advanced lines.
    AfterAll,
}

/// Shared context for an item's own tooltip and component appenders: player,
/// flags and vanilla `Item.TooltipContext.tickRate()`.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct TooltipAppendContext {
    /// The local player's UUID, including for nested projectiles where vanilla passes no player.
    pub player: u128,
    /// The local player's entity id.
    pub player_entity_id: i32,
    /// The client level's tick rate, or 20 when no client level is present.
    pub tick_rate: f32,
    /// `TooltipFlag.isAdvanced` (F3+H).
    pub advanced: bool,
    /// `TooltipFlag.isCreative`: a creative tab's own slot.
    pub creative: bool,
    /// A crossbow's loaded projectile, whose details its crossbow lists.
    pub nested: bool,
}

/// The lines a data component adds to its stack's tooltip. Kept by the game from registration on;
/// asked only for a stack that carries the component and whose `tooltip_display` shows it, and
/// only while its mod is enabled. Multiple registrations for the same component all run.
pub trait TooltipAppender: Send + Sync + 'static {
    /// Add lines to `lines` for `stack`, each with a colour as `rgba` packs it (`u32::MAX` keeps
    /// the text's own). `context` is the tooltip's player, flags and client-level tick rate.
    fn append(
        &self,
        host: &dyn ClientHost,
        stack: &ItemStack,
        context: &TooltipAppendContext,
        lines: &mut Vec<(Text, u32)>,
    );
}
