//! `HoneycombItem.getWaxed` for the player's and the dispenser's honeycomb, and the axe's scrape
//! and wax-off from the world's NeoForge block maps.
use super::{Game, TransformKind, native_block_tables::BlockTables};
use sunrise_world::registry::BlockStateId;

impl Game<'_> {
    /// The waxed twin of `state`; see [`BlockTables::honeycomb_waxed`].
    pub(in crate::game) fn honeycomb_waxed(&self, state: BlockStateId) -> Option<BlockStateId> {
        BlockTables::of(self).honeycomb_waxed(state)
    }

    /// The axe's scrape or wax-off of `block` from the world's NeoForge block maps, asked
    /// before the axe's transform table: the derived `neoforge:oxidizables` inverse
    /// (`DataMapHooks.getPreviousOxidizedStage`), then the `neoforge:waxables` inverse
    /// (`getBlockUnwaxed`). None for another tool or a block the maps leave out.
    pub(in crate::game) fn axe_mapped(
        &self,
        transformer: &str,
        block: BlockStateId,
    ) -> Option<(BlockStateId, mod_api::ToolAbility)> {
        if transformer != "minecraft:axe" {
            return None;
        }
        let behaviours = self.res.behaviours();
        let scraped = behaviours.mapped_scraped(block);
        scraped
            .map(|state| (state, mod_api::ToolAbility::AxeScrape))
            .or_else(|| {
                let unwaxed = behaviours.mapped_unwaxed(block)?;
                Some((unwaxed, mod_api::ToolAbility::AxeWaxOff))
            })
    }

    /// The use a tool ability makes outside the transform table: `new`, the ability's sound
    /// and particle, no loot, one point of damage and no transform type.
    pub(in crate::game) fn tool_ability_result(
        new: BlockStateId,
        ability: mod_api::ToolAbility,
    ) -> (
        BlockStateId,
        String,
        Option<String>,
        u32,
        String,
        TransformKind,
    ) {
        use mod_api::ToolAbility::*;
        let (sound, particle) = match ability {
            AxeStrip => ("minecraft:item.axe.strip", "none"),
            AxeScrape => ("minecraft:item.axe.scrape", "scrape"),
            AxeWaxOff => ("minecraft:item.axe.wax_off", "wax_off"),
            ShovelFlatten => ("minecraft:item.shovel.flatten", "none"),
            HoeTill => ("minecraft:item.hoe.till", "none"),
            ShovelDouse => unreachable!(),
            _ => unreachable!("ToolAbility: unhandled variant"),
        };
        (
            new,
            sound.to_owned(),
            None,
            1,
            particle.to_owned(),
            TransformKind::default(),
        )
    }
}
