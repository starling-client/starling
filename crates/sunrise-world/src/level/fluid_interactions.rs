//! What a liquid block turns into beside another fluid or block (NeoForge's
//! `FluidInteractionRegistry`): vanilla's two lava rules and the entries mods declare, as one
//! resolved table per source fluid. Installed on [`Behaviours`] for the open world, resolved
//! once when the world opens and again on each data reload, so no name or tag is looked up
//! while a liquid updates.
use super::{Behaviours, data::BlockSet, states::BlockId};
use crate::{
    fluid::{FluidKind, FluidState},
    registry::BlockStateId,
};
use std::sync::{
    Arc, RwLock,
    atomic::{AtomicU8, AtomicUsize, Ordering},
};

/// A block, or every block of a tag.
#[derive(Clone, Debug)]
pub enum BlockMatch {
    Block(BlockId),
    Tag(Arc<BlockSet>),
}
impl BlockMatch {
    fn matches(&self, block: BlockId) -> bool {
        match self {
            Self::Block(known) => *known == block,
            Self::Tag(set) => set.contains(block),
        }
    }
}

/// What has to be beside the liquid block. A fluid is read from the neighbour cell's fluid
/// state, so a waterlogged block holds water.
#[derive(Clone, Debug)]
pub enum Neighbour {
    /// One fluid, source and flowing form alike.
    Fluid(FluidKind),
    /// The fluids a tag holds, each form as the tag lists it (`FluidState.is(TagKey)`): a tag of
    /// source water alone does not match flowing water.
    FluidTag(Arc<[FluidKind]>),
    Blocks(BlockMatch),
}

/// One interaction of a source fluid. A `None` result is no interaction for that state of the
/// source cell.
#[derive(Clone, Debug)]
pub struct FluidInteraction {
    pub neighbour: Neighbour,
    /// What the block under the source cell has to be.
    pub below: Option<BlockMatch>,
    /// What a source cell becomes.
    pub source: Option<BlockStateId>,
    /// What a flowing cell becomes.
    pub flowing: Option<BlockStateId>,
}

/// A cell beside the liquid block: its fluid state's fluid and its block.
#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub fluid: FluidKind,
    pub block: BlockId,
}

impl FluidInteraction {
    /// The block this interaction makes of the liquid cell, as its state says.
    fn result(&self, source_cell: bool) -> Option<BlockStateId> {
        if source_cell {
            self.source
        } else {
            self.flowing
        }
    }
    fn applies(&self, cell: Cell, below: BlockId) -> bool {
        let neighbour = match &self.neighbour {
            Neighbour::Fluid(fluid) => fluid.same_family(cell.fluid),
            Neighbour::FluidTag(fluids) => fluids.contains(&cell.fluid),
            Neighbour::Blocks(blocks) => blocks.matches(cell.block),
        };
        neighbour && self.below.as_ref().is_none_or(|b| b.matches(below))
    }
}

/// What the liquid cell becomes: the result of the first interaction that applies and has one
/// for the cell's state, by neighbour (`neighbours`, in the order given: vanilla's is above,
/// north, south, west, east, never below), then by interaction, as
/// `FluidInteractionRegistry.canInteract` walks them.
pub fn first_match(
    interactions: &[FluidInteraction],
    neighbours: impl IntoIterator<Item = Cell>,
    below: BlockId,
    source_cell: bool,
) -> Option<BlockStateId> {
    neighbours.into_iter().find_map(|cell| {
        interactions
            .iter()
            .filter(|it| it.applies(cell, below))
            .find_map(|it| it.result(source_cell))
    })
}

/// Both forms of a fluid, source and flowing; none for the empty fluid.
fn forms(kind: FluidKind) -> Vec<FluidKind> {
    match kind {
        FluidKind::Empty => Vec::new(),
        FluidKind::Water | FluidKind::FlowingWater => {
            vec![FluidKind::Water, FluidKind::FlowingWater]
        }
        FluidKind::Lava | FluidKind::FlowingLava => vec![FluidKind::Lava, FluidKind::FlowingLava],
        FluidKind::Mod(fluid) | FluidKind::ModFlowing(fluid) => {
            vec![FluidKind::Mod(fluid), FluidKind::ModFlowing(fluid)]
        }
    }
}

type Entries = Arc<[FluidInteraction]>;

/// The interactions of every fluid, by the exact fluid of the liquid block: vanilla's kinds by
/// their registry index, a mod fluid's two forms by its id. Within a fluid they keep the order
/// they were declared in.
#[derive(Clone, Debug, Default)]
pub struct FluidInteractions {
    vanilla: [Entries; 5],
    mods: Vec<[Entries; 2]>,
}
impl FluidInteractions {
    /// `builtin` entries come first, each for exactly the fluid it names (vanilla's rules follow
    /// the lava tag: one entry per fluid the tag lists). A `declared` entry's source names a
    /// fluid, and the entry applies to its source and its flowing form alike.
    pub fn new(
        builtin: impl IntoIterator<Item = (FluidKind, FluidInteraction)>,
        declared: impl IntoIterator<Item = (FluidKind, FluidInteraction)>,
    ) -> Self {
        let mut vanilla: [Vec<FluidInteraction>; 5] = Default::default();
        let mut mods: Vec<[Vec<FluidInteraction>; 2]> = Vec::new();
        let mut push = |kind: FluidKind, interaction: FluidInteraction| match kind.mod_fluid() {
            Some(fluid) => {
                if mods.len() <= fluid.index() {
                    mods.resize_with(fluid.index() + 1, Default::default);
                }
                let flowing = matches!(kind, FluidKind::ModFlowing(_));
                mods[fluid.index()][usize::from(flowing)].push(interaction);
            }
            None => {
                if let Some(index) = kind.vanilla_index().filter(|_| kind != FluidKind::Empty) {
                    vanilla[usize::from(index)].push(interaction);
                }
            }
        };
        for (kind, interaction) in builtin {
            push(kind, interaction);
        }
        for (kind, interaction) in declared {
            for form in forms(kind) {
                push(form, interaction.clone());
            }
        }
        Self {
            vanilla: vanilla.map(Into::into),
            mods: mods
                .into_iter()
                .map(|forms| forms.map(Into::into))
                .collect(),
        }
    }
    /// The interactions of the liquid block of `kind`; none for a fluid that has none.
    pub fn of(&self, kind: FluidKind) -> Option<&Entries> {
        let entries = match kind.mod_fluid() {
            Some(fluid) => {
                let flowing = matches!(kind, FluidKind::ModFlowing(_));
                &self.mods.get(fluid.index())?[usize::from(flowing)]
            }
            None => &self.vanilla[usize::from(kind.vanilla_index()?)],
        };
        (!entries.is_empty()).then_some(entries)
    }
}

impl Behaviours {
    /// `LiquidBlock.shouldSpreadLiquid`'s two rules as the table's first entries, for each fluid
    /// of the `minecraft:lava` tag (vanilla applies them to a liquid in that tag). `lava` and
    /// `water` are the fluids of the `minecraft:lava` and `minecraft:water` tags, which a data
    /// pack may edit.
    pub fn vanilla_fluid_interactions(
        &self,
        lava: &[FluidKind],
        water: &[FluidKind],
    ) -> Vec<(FluidKind, FluidInteraction)> {
        let n = &self.named;
        let state = |block| Some(self.default_of(block));
        lava.iter()
            .flat_map(|&fluid| {
                [
                    (
                        fluid,
                        FluidInteraction {
                            neighbour: Neighbour::FluidTag(water.into()),
                            below: None,
                            source: state(n.obsidian),
                            flowing: state(n.cobblestone),
                        },
                    ),
                    (
                        fluid,
                        FluidInteraction {
                            neighbour: Neighbour::Blocks(BlockMatch::Block(n.blue_ice)),
                            below: Some(BlockMatch::Block(n.soul_soil)),
                            source: state(n.basalt),
                            flowing: state(n.basalt),
                        },
                    ),
                ]
            })
            .collect()
    }
    /// What a liquid cell of `source` becomes against the one `neighbour` cell, with `below`
    /// under it: the table `should_spread_liquid` walks, for one neighbour. A pure read: it
    /// changes no world, asks no hook and plays nothing. `None` is no interaction. The
    /// neighbour's fluid is read through its block state, as a waterlogged block holds water.
    pub fn fluid_interaction_result(
        &self,
        source: FluidState,
        neighbour: BlockStateId,
        below: BlockStateId,
    ) -> Option<BlockStateId> {
        let interactions = self.fluid_interactions(source.kind())?;
        let cell = Cell {
            fluid: self.fluid(neighbour).kind(),
            block: self.block(neighbour),
        };
        first_match(&interactions, [cell], self.block(below), source.is_source())
    }
    /// The fluids of the `minecraft:water` and `minecraft:lava` fluid tags the prepared data
    /// gives.
    pub fn tag_fluids(&self, mask: u8) -> Vec<FluidKind> {
        FluidKind::ALL
            .into_iter()
            .filter(|kind| kind.mask() & mask != 0)
            .collect()
    }
    /// The table of the prepared data alone, which a world without data packs or mods keeps.
    pub(super) fn install_vanilla_fluid_interactions(&self) {
        let lava = self.tag_fluids(self.fluid_tags.lava);
        let water = self.tag_fluids(self.fluid_tags.water);
        self.set_fluid_interactions(FluidInteractions::new(
            self.vanilla_fluid_interactions(&lava, &water),
            [],
        ));
    }
    /// Install the open world's table, replacing the last load's.
    pub fn set_fluid_interactions(&self, table: FluidInteractions) {
        self.fluid_interactions.install(table);
    }
    /// The interactions of the liquid block of `kind`, taken once per update. A fluid outside
    /// the installed table's atomic bounds answers from atomic loads alone, with no lock. Other
    /// fluids take a read lock and index their own entries, returning immediately when that
    /// slice is empty; the lookup allocates nothing.
    pub(crate) fn fluid_interactions(&self, kind: FluidKind) -> Option<Entries> {
        let slot = &self.fluid_interactions;
        slot.may_have(kind).then(|| slot.of(kind)).flatten()
    }
}

/// The open world's table on [`Behaviours`]: replaced whole by each data load. Which fluids
/// have entries is also kept as atomics (a mask of vanilla kinds, and how many mod fluids the
/// table reaches), so a fluid outside those bounds is told apart without taking the lock; a mod
/// fluid inside them without entries still takes the read lock and finds an empty slice.
#[derive(Default)]
pub(super) struct Slot {
    table: RwLock<Arc<FluidInteractions>>,
    vanilla: AtomicU8,
    mods: AtomicUsize,
}
impl Slot {
    fn install(&self, table: FluidInteractions) {
        let vanilla = FluidKind::ALL
            .into_iter()
            .filter(|kind| table.of(*kind).is_some())
            .fold(0, |mask, kind| mask | kind.mask());
        self.vanilla.store(vanilla, Ordering::Release);
        self.mods.store(table.mods.len(), Ordering::Release);
        *self
            .table
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Arc::new(table);
    }
    /// Whether `kind`'s fluid may have entries, from the atomics alone.
    fn may_have(&self, kind: FluidKind) -> bool {
        match kind.mod_fluid() {
            Some(fluid) => fluid.index() < self.mods.load(Ordering::Acquire),
            None => kind.mask() & self.vanilla.load(Ordering::Acquire) != 0,
        }
    }
    fn of(&self, kind: FluidKind) -> Option<Entries> {
        self.table
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .of(kind)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AIR: BlockId = BlockId(0);
    const OBSIDIAN: BlockStateId = BlockStateId(10);
    const COBBLESTONE: BlockStateId = BlockStateId(11);
    const BASALT: BlockStateId = BlockStateId(12);
    const MOD_STONE: BlockStateId = BlockStateId(20);
    const BLUE_ICE: BlockId = BlockId(5);
    const SOUL_SOIL: BlockId = BlockId(6);
    const SLAB: BlockId = BlockId(7);
    const STONE: BlockId = BlockId(8);

    fn cell(fluid: FluidKind, block: BlockId) -> Cell {
        Cell { fluid, block }
    }
    fn around(cells: &[(usize, Cell)]) -> [Cell; 5] {
        let mut all = [cell(FluidKind::Empty, AIR); 5];
        for (at, c) in cells {
            all[*at] = *c;
        }
        all
    }
    fn vanilla() -> Vec<FluidInteraction> {
        vec![
            FluidInteraction {
                neighbour: Neighbour::FluidTag([FluidKind::Water, FluidKind::FlowingWater].into()),
                below: None,
                source: Some(OBSIDIAN),
                flowing: Some(COBBLESTONE),
            },
            FluidInteraction {
                neighbour: Neighbour::Blocks(BlockMatch::Block(BLUE_ICE)),
                below: Some(BlockMatch::Block(SOUL_SOIL)),
                source: Some(BASALT),
                flowing: Some(BASALT),
            },
        ]
    }
    fn made(
        rules: &[FluidInteraction],
        cells: [Cell; 5],
        below: BlockId,
        source: bool,
    ) -> Option<u32> {
        first_match(rules, cells, below, source).map(|state| state.0)
    }

    #[test]
    fn water_makes_obsidian_of_a_source_and_cobblestone_of_flowing_lava() {
        let rules = vanilla();
        let water = around(&[(3, cell(FluidKind::FlowingWater, AIR))]);
        assert_eq!(made(&rules, water, STONE, true), Some(OBSIDIAN.0));
        assert_eq!(made(&rules, water, STONE, false), Some(COBBLESTONE.0));
    }

    #[test]
    fn one_cell_walks_the_same_table_as_the_five_around_a_liquid() {
        let rules = vanilla();
        let one = |cell: Cell, below, source| {
            first_match(&rules, [cell], below, source).map(|state| state.0)
        };
        let flowing_water = cell(FluidKind::FlowingWater, AIR);
        assert_eq!(one(flowing_water, STONE, true), Some(OBSIDIAN.0));
        assert_eq!(one(flowing_water, STONE, false), Some(COBBLESTONE.0));
        assert_eq!(one(cell(FluidKind::Empty, STONE), STONE, true), None);
        assert_eq!(
            one(cell(FluidKind::Empty, BLUE_ICE), SOUL_SOIL, true),
            Some(BASALT.0)
        );
        assert_eq!(one(cell(FluidKind::Empty, BLUE_ICE), STONE, true), None);
        // No neighbour at all is no interaction.
        assert_eq!(first_match(&rules, [], STONE, true), None);
    }

    #[test]
    fn a_waterlogged_neighbour_is_water() {
        let waterlogged = around(&[(1, cell(FluidKind::Water, SLAB))]);
        assert_eq!(made(&vanilla(), waterlogged, STONE, true), Some(OBSIDIAN.0));
    }

    #[test]
    fn blue_ice_beside_lava_over_soul_soil_makes_basalt_and_nothing_else_does() {
        let rules = vanilla();
        let ice = around(&[(4, cell(FluidKind::Empty, BLUE_ICE))]);
        assert_eq!(made(&rules, ice, SOUL_SOIL, true), Some(BASALT.0));
        assert_eq!(made(&rules, ice, SOUL_SOIL, false), Some(BASALT.0));
        assert_eq!(
            made(&rules, ice, STONE, true),
            None,
            "needs soul soil below"
        );
        let stone = around(&[(0, cell(FluidKind::Empty, STONE))]);
        assert_eq!(made(&rules, stone, SOUL_SOIL, true), None);
        // The cell below is never a neighbour: only the five others are passed.
        assert_eq!(made(&rules, around(&[]), SOUL_SOIL, true), None);
    }

    #[test]
    fn the_first_neighbour_then_the_first_entry_wins() {
        let rules = vanilla();
        // Water to the east and blue ice above, over soul soil: above comes first.
        let both = around(&[
            (0, cell(FluidKind::Empty, BLUE_ICE)),
            (4, cell(FluidKind::Water, AIR)),
        ]);
        assert_eq!(made(&rules, both, SOUL_SOIL, true), Some(BASALT.0));
        // The same neighbour matching two entries gives the first entry's result.
        let ice_in_water = around(&[(2, cell(FluidKind::Water, BLUE_ICE))]);
        assert_eq!(
            made(&rules, ice_in_water, SOUL_SOIL, true),
            Some(OBSIDIAN.0)
        );
    }

    #[test]
    fn a_mod_entry_matches_a_block_tag_and_follows_vanillas() {
        let mut stones = BlockSet::with_capacity(16);
        stones.insert(STONE);
        let mut rules = vanilla();
        rules.push(FluidInteraction {
            neighbour: Neighbour::Blocks(BlockMatch::Tag(Arc::new(stones))),
            below: None,
            source: Some(MOD_STONE),
            flowing: None,
        });
        let beside = around(&[(1, cell(FluidKind::Empty, STONE))]);
        assert_eq!(made(&rules, beside, AIR, true), Some(MOD_STONE.0));
        assert_eq!(
            made(&rules, beside, AIR, false),
            None,
            "a None result is no interaction"
        );
        // ... and does not stop a later entry from answering.
        rules.push(FluidInteraction {
            neighbour: Neighbour::Blocks(BlockMatch::Block(STONE)),
            below: None,
            source: None,
            flowing: Some(COBBLESTONE),
        });
        assert_eq!(made(&rules, beside, AIR, false), Some(COBBLESTONE.0));
        let water = around(&[(1, cell(FluidKind::Water, STONE))]);
        assert_eq!(made(&rules, water, AIR, true), Some(OBSIDIAN.0));
    }

    fn stone_entry() -> FluidInteraction {
        FluidInteraction {
            neighbour: Neighbour::Blocks(BlockMatch::Block(STONE)),
            below: None,
            source: Some(MOD_STONE),
            flowing: Some(MOD_STONE),
        }
    }

    #[test]
    fn a_tag_matches_exactly_the_fluids_it_lists_and_a_fluid_both_forms() {
        let rule = |neighbour| FluidInteraction {
            neighbour,
            below: None,
            source: Some(OBSIDIAN),
            flowing: Some(OBSIDIAN),
        };
        let flowing = around(&[(1, cell(FluidKind::FlowingWater, AIR))]);
        let source = around(&[(1, cell(FluidKind::Water, AIR))]);
        let source_tag = [rule(Neighbour::FluidTag([FluidKind::Water].into()))];
        assert_eq!(made(&source_tag, source, AIR, true), Some(OBSIDIAN.0));
        assert_eq!(made(&source_tag, flowing, AIR, true), None);
        let fluid = [rule(Neighbour::Fluid(FluidKind::Water))];
        assert_eq!(made(&fluid, source, AIR, true), Some(OBSIDIAN.0));
        assert_eq!(made(&fluid, flowing, AIR, true), Some(OBSIDIAN.0));
    }

    #[test]
    fn a_declared_entry_applies_to_both_forms_and_a_builtin_to_the_fluid_it_names() {
        let table = FluidInteractions::new(
            // Vanilla's rules for flowing lava alone, as a lava tag holding only it would give.
            vanilla().into_iter().map(|it| (FluidKind::FlowingLava, it)),
            [
                (FluidKind::Lava, stone_entry()),
                (FluidKind::Water, stone_entry()),
            ],
        );
        assert!(table.of(FluidKind::Lava).is_some_and(|l| l.len() == 1));
        assert!(
            table
                .of(FluidKind::FlowingLava)
                .is_some_and(|l| l.len() == 3)
        );
        assert!(table.of(FluidKind::Water).is_some_and(|l| l.len() == 1));
        assert!(
            table
                .of(FluidKind::FlowingWater)
                .is_some_and(|l| l.len() == 1)
        );
        assert!(table.of(FluidKind::Empty).is_none());
        // A built-in comes before a declared entry of the same fluid.
        let rules = table.of(FluidKind::FlowingLava).unwrap();
        let ice = around(&[(0, cell(FluidKind::Empty, STONE))]);
        assert_eq!(made(rules, ice, AIR, true), Some(MOD_STONE.0));
        assert!(FluidInteractions::default().of(FluidKind::Lava).is_none());
    }

    #[test]
    fn the_gate_answers_for_the_fluids_with_entries_and_no_other() {
        let slot = Slot::default();
        for kind in FluidKind::ALL {
            assert!(!slot.may_have(kind), "an empty slot has nothing");
        }
        slot.install(FluidInteractions::new(
            [],
            [(FluidKind::Lava, stone_entry())],
        ));
        assert!(slot.may_have(FluidKind::Lava));
        assert!(slot.may_have(FluidKind::FlowingLava));
        assert!(slot.of(FluidKind::FlowingLava).is_some());
        for other in [FluidKind::Water, FluidKind::FlowingWater, FluidKind::Empty] {
            assert!(!slot.may_have(other));
        }
        slot.install(FluidInteractions::new(
            [(FluidKind::Water, stone_entry())],
            [],
        ));
        assert!(slot.may_have(FluidKind::Water));
        assert!(
            !slot.may_have(FluidKind::FlowingWater),
            "exactly the fluid named"
        );
        assert!(!slot.may_have(FluidKind::FlowingLava));
    }
}
