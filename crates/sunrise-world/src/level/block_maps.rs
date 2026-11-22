//! NeoForge's built-in block data maps (`NeoForgeDataMaps`): `neoforge:flammable`,
//! `neoforge:oxidizables` and `neoforge:waxables`, as the open world's data packs
//! give them, with the inverse maps `DataMapHooks` builds for scraping and
//! unwaxing. Installed on [`Behaviours`] for the open world, as its live
//! features are; without files nothing is installed and every reader takes
//! vanilla's own answer.
use super::Behaviours;
use crate::registry::BlockStateId;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// The three maps by block id (`minecraft:stone`), values decoded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockMaps {
    flammable: HashMap<String, (i32, i32)>,
    oxidizes_to: HashMap<String, String>,
    scrapes_to: HashMap<String, String>,
    waxes_to: HashMap<String, String>,
    unwaxes_to: HashMap<String, String>,
}

/// A decoded built-in block map value. Target names are resolved against the block registry
/// before loading the layer.
#[derive(Clone)]
pub enum BlockMapValue {
    Flammable { ignite_odds: i32, burn_odds: i32 },
    Oxidizable(String),
    Waxable(String),
}

/// Record that `from` comes back to `to` in an inverse map, keeping the lowest
/// `rank` where several blocks share `from`.
fn inverse(map: &mut HashMap<String, String>, from: String, to: &str, rank: &impl Fn(&str) -> u32) {
    match map.get(&from) {
        Some(kept) if (rank(kept), kept.as_str()) <= (rank(to), to) => {}
        _ => {
            map.insert(from, to.to_owned());
        }
    }
}

/// The loops of a map from each block to one other, each listed once in walk
/// order, sorted for a stable error.
fn cycles(map: &HashMap<String, String>) -> Vec<Vec<String>> {
    let mut starts: Vec<&String> = map.keys().collect();
    starts.sort();
    let mut done = std::collections::HashSet::new();
    let mut found = Vec::new();
    for start in starts {
        let mut path: Vec<&String> = Vec::new();
        let mut at = start;
        while !done.contains(at) {
            if let Some(i) = path.iter().position(|p| *p == at) {
                found.push(path[i..].iter().map(|p| (*p).clone()).collect());
                break;
            }
            path.push(at);
            match map.get(at) {
                Some(next) => at = next,
                None => break,
            }
        }
        done.extend(path);
    }
    found
}

impl BlockMaps {
    /// Build from decoded, merged rows. The inverse maps are derived here, never declared:
    /// where several blocks share a next stage or a waxed twin, the one with the lowest `rank`
    /// (its block registry id) is the one scraping or unwaxing gives back, a Sunrise choice
    /// where NeoForge keeps whichever its map iteration met last. An oxidation chain that loops
    /// (a block its own next stage included) is refused with an error naming the map and the
    /// blocks: its scraping steps are left out, so no walk back can hang.
    pub fn from_values<K: Into<String>>(
        values: impl IntoIterator<Item = (K, BlockMapValue)>,
        rank: impl Fn(&str) -> u32,
    ) -> Self {
        let mut maps = Self::default();
        for (block, value) in values {
            let block = block.into();
            match value {
                BlockMapValue::Flammable {
                    ignite_odds,
                    burn_odds,
                } => {
                    maps.flammable.insert(block, (ignite_odds, burn_odds));
                }
                BlockMapValue::Oxidizable(next) => {
                    inverse(&mut maps.scrapes_to, next.clone(), &block, &rank);
                    maps.oxidizes_to.insert(block, next);
                }
                BlockMapValue::Waxable(waxed) => {
                    inverse(&mut maps.unwaxes_to, waxed.clone(), &block, &rank);
                    maps.waxes_to.insert(block, waxed);
                }
            }
        }
        for cycle in cycles(&maps.scrapes_to) {
            log::error!(
                "Sunrise: neoforge:oxidizables: the oxidation chain loops through {}; scraping is left out for those blocks",
                cycle.join(" -> ")
            );
            for block in &cycle {
                maps.scrapes_to.remove(block);
            }
        }
        maps
    }
    /// Whether no map gives any block a value.
    pub fn is_empty(&self) -> bool {
        self.flammable.is_empty() && self.oxidizes_to.is_empty() && self.waxes_to.is_empty()
    }
    /// `neoforge:flammable`: the block's ignite and burn odds.
    pub fn flammable(&self, block: &str) -> Option<(i32, i32)> {
        self.flammable.get(block).copied()
    }
    /// `DataMapHooks.getNextOxidizedStage`: the block one oxidation step on.
    pub fn next_oxidation(&self, block: &str) -> Option<&str> {
        self.oxidizes_to.get(block).map(String::as_str)
    }
    /// `DataMapHooks.getPreviousOxidizedStage`: the block one step back (scraping).
    pub fn previous_oxidation(&self, block: &str) -> Option<&str> {
        self.scrapes_to.get(block).map(String::as_str)
    }
    /// `DataMapHooks.getBlockWaxed`: the block's waxed twin.
    pub fn waxed(&self, block: &str) -> Option<&str> {
        self.waxes_to.get(block).map(String::as_str)
    }
    /// `DataMapHooks.getBlockUnwaxed`: the block a waxed one unwaxes to.
    pub fn unwaxed(&self, block: &str) -> Option<&str> {
        self.unwaxes_to.get(block).map(String::as_str)
    }
}

/// The open world's maps on [`Behaviours`]: replaced whole by each data load.
#[derive(Default)]
pub(super) struct Slot(RwLock<Option<Arc<BlockMaps>>>);

impl Behaviours {
    /// Install the open world's NeoForge block maps, replacing the last
    /// load's; `None` where its packs give none.
    pub fn set_block_maps(&self, maps: Option<Arc<BlockMaps>>) {
        *self
            .block_maps
            .0
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = maps;
    }
    /// The installed maps. Take them once per operation and reuse them.
    pub fn block_maps(&self) -> Option<Arc<BlockMaps>> {
        self.block_maps
            .0
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
    /// `read` over the installed maps without taking a reference count: the
    /// per-block readers' path, one uncontended lock read when none are installed.
    pub(crate) fn read_block_maps<R>(
        &self,
        read: impl FnOnce(&BlockMaps) -> Option<R>,
    ) -> Option<R> {
        self.block_maps
            .0
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_deref()
            .and_then(read)
    }
    /// `HoneycombItem.getWaxed` through `DataMapHooks.getBlockWaxed`: the
    /// `neoforge:waxables` twin of `state`'s block, its properties kept; none
    /// where the installed maps give the block none.
    pub fn mapped_waxed(&self, state: BlockStateId) -> Option<BlockStateId> {
        self.mapped(state, BlockMaps::waxed)
    }
    /// The axe's scrape through `DataMapHooks.getPreviousOxidizedStage`: the
    /// stage before `state`'s block in the derived `neoforge:oxidizables`
    /// inverse, its properties kept.
    pub fn mapped_scraped(&self, state: BlockStateId) -> Option<BlockStateId> {
        self.mapped(state, BlockMaps::previous_oxidation)
    }
    /// The axe's wax-off through `DataMapHooks.getBlockUnwaxed`: the block the
    /// derived `neoforge:waxables` inverse unwaxes `state`'s block to, its
    /// properties kept.
    pub fn mapped_unwaxed(&self, state: BlockStateId) -> Option<BlockStateId> {
        self.mapped(state, BlockMaps::unwaxed)
    }
    pub(super) fn mapped(
        &self,
        state: BlockStateId,
        to: impl for<'m> Fn(&'m BlockMaps, &str) -> Option<&'m str>,
    ) -> Option<BlockStateId> {
        self.read_block_maps(|maps| {
            let block = self
                .table
                .block_by_name(to(maps, self.table.name(state))?)?;
            Some(self.table.with_properties_of(block, state))
        })
    }
    /// Whether `state` is a waxed block: vanilla's `waxed_` blocks, and every
    /// block the installed `neoforge:waxables` map waxes to.
    pub fn is_waxed(&self, state: BlockStateId) -> bool {
        let name = self.table.name(state);
        name.starts_with("minecraft:waxed_")
            || self
                .read_block_maps(|maps| maps.unwaxed(name).map(drop))
                .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use BlockMapValue::{Flammable, Oxidizable, Waxable};

    #[test]
    fn the_inverse_maps_follow_the_forward_rows() {
        let maps = BlockMaps::from_values(
            [
                (
                    "minecraft:stone",
                    Flammable {
                        ignite_odds: 5,
                        burn_odds: 20,
                    },
                ),
                ("test:tin", Oxidizable("test:tin_dull".into())),
                ("test:tin", Waxable("test:waxed_tin".into())),
            ],
            |_| 0,
        );
        assert_eq!(maps.flammable("minecraft:stone"), Some((5, 20)));
        assert_eq!(maps.next_oxidation("test:tin"), Some("test:tin_dull"));
        assert_eq!(maps.previous_oxidation("test:tin_dull"), Some("test:tin"));
        assert_eq!(maps.waxed("test:tin"), Some("test:waxed_tin"));
        assert_eq!(maps.unwaxed("test:waxed_tin"), Some("test:tin"));
        assert!(!maps.is_empty() && BlockMaps::default().is_empty());
    }

    #[test]
    fn a_shared_stage_scrapes_back_to_the_lowest_registry_id() {
        let rank = |block: &str| match block {
            "test:tin" => 7,
            "test:zinc" => 3,
            _ => 100,
        };
        // Whichever order the files list them, zinc (id 3) wins over tin (id 7).
        for (first, second) in [("test:tin", "test:zinc"), ("test:zinc", "test:tin")] {
            let maps = BlockMaps::from_values(
                [first, second].into_iter().flat_map(|block| {
                    [
                        (block, Oxidizable("test:dull".into())),
                        (block, Waxable("test:waxed".into())),
                    ]
                }),
                rank,
            );
            assert_eq!(maps.previous_oxidation("test:dull"), Some("test:zinc"));
            assert_eq!(maps.unwaxed("test:waxed"), Some("test:zinc"));
            assert_eq!(maps.next_oxidation("test:tin"), Some("test:dull"));
        }
    }

    #[test]
    fn a_looping_oxidation_chain_loses_its_scraping_steps_at_load() {
        // test:a -> b -> c -> a loops, test:self is its own next stage, test:d -> a does not.
        let maps = BlockMaps::from_values(
            [
                ("test:a", Oxidizable("test:b".into())),
                ("test:b", Oxidizable("test:c".into())),
                ("test:c", Oxidizable("test:a".into())),
                ("test:self", Oxidizable("test:self".into())),
                ("test:d", Oxidizable("test:a".into())),
            ],
            |_| 0,
        );
        for block in ["test:a", "test:b", "test:c", "test:self"] {
            assert_eq!(maps.previous_oxidation(block), None, "{block}");
        }
        // Oxidizing still moves one step, so the loop only alternates.
        assert_eq!(maps.next_oxidation("test:c"), Some("test:a"));
        assert_eq!(maps.next_oxidation("test:self"), Some("test:self"));
        let (a, b) = ("test:a".to_owned(), "test:b".to_owned());
        let loops = cycles(&HashMap::from([(a.clone(), b.clone()), (b, a)]));
        assert_eq!(loops, [["test:a", "test:b"]]);
    }
}
