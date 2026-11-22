//! The `gather_skipped_attribute_tooltips` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `GatherSkippedAttributeTooltipsEvent`: the attribute modifiers a stack's tooltip leaves out
    /// (`ItemStack.addAttributeTooltips`), asked once before its attribute lines are built. Set
    /// `skip_all` to hide every one, push modifier ids (`minecraft:base_attack_damage`, a bare
    /// name is `minecraft:`'s) to `skipped_ids`, or `EquipmentSlotGroup` names (`mainhand`,
    /// `armor`, `any`) to `skipped_groups`; a slot group with nothing left shows no heading.
    /// Display and resource-query builds skip it when `minecraft:tooltip_display` hides the
    /// attribute modifiers or the entire tooltip. `player` and `player_entity_id` are the local player's, or `None` for
    /// recipe book search and a stack nested in another's tooltip (a crossbow's loaded
    /// projectile). `advanced` (F3+H) and `creative` are the tooltip's `TooltipFlag`.
    /// Asked when lines are built for display, recipe book search or `ItemResource::tooltip`,
    /// except during reentrant line queries. Client-side, only while a mod listens.
    #[hook(
        receiver = "read",
        dispatch = "api_carrier",
        side = "client",
        contract = "listen",
        neoforge = "GatherSkippedAttributeTooltipsEvent",
        cancellable = false,
        version = 2
    )]
    fn gather_skipped_attribute_tooltips(
        player: Option<u128>,
        player_entity_id: Option<i32>,
        stack: &ItemStack,
        advanced: bool,
        creative: bool,
        skip_all: &mut bool,
        skipped_ids: &mut Vec<String>,
        skipped_groups: &mut Vec<String>,
    );
}
