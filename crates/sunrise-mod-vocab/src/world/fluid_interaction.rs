//! What a liquid block turns into beside another fluid or block (NeoForge's
//! `FluidInteractionRegistry`), declared by a mod.
use crate::{BlockKey, FluidKey, StateConstraint, TagKey};

/// What has to be beside the liquid block: above it or to a side, never below.
/// A fluid is read from the neighbour cell's fluid state, so a waterlogged block holds water.
// exhaustive: the closed vocabulary of what a fluid interaction tests, a link contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub enum FluidNeighbour {
    /// A fluid, source or flowing.
    Fluid(FluidKey),
    /// Every fluid of a fluid tag, as the world's data packs give it.
    FluidTag(TagKey),
    /// A block.
    Block(BlockKey),
    /// Every block of a block tag.
    BlockTag(TagKey),
}

/// A block, or every block of a block tag.
// exhaustive: the closed vocabulary of what the block under a liquid may be, a link contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub enum BlockSelector {
    /// One block.
    Block(BlockKey),
    /// Every block of a block tag.
    Tag(TagKey),
}

/// A block state a registration can name before the game's block states exist: a block, with
/// the properties that differ from its default state.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct PlacedState {
    /// The block.
    pub block: BlockKey,
    /// Property values over its default state; the game refuses a name or value the block lacks.
    pub properties: Vec<StateConstraint>,
}
impl PlacedState {
    /// `block`'s default state.
    pub fn of(block: BlockKey) -> Self {
        Self {
            block,
            properties: Vec::new(),
        }
    }
    /// With the property `name` at `value`.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.properties.push(StateConstraint::new(name, value));
        self
    }
}

/// What the liquid cell becomes. A state left `None` is no interaction for that state of the cell.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct FluidBecomes {
    /// What a source cell becomes (vanilla's obsidian).
    pub source: Option<PlacedState>,
    /// What a flowing cell becomes (vanilla's cobblestone).
    pub flowing: Option<PlacedState>,
}
impl FluidBecomes {
    /// Both states become `state`.
    pub fn always(state: PlacedState) -> Self {
        Self {
            source: Some(state.clone()),
            flowing: Some(state),
        }
    }
}

/// One interaction of the liquid block of the fluid `source`. At most one interaction runs when a
/// liquid block is placed or a neighbour of it changes: the first match, over the neighbours
/// above, north, south, west and east, then over the interactions, vanilla's two lava rules first.
/// The result goes through the `fluid_place_block` hook, which may veto or replace it, and a
/// level event plays the fizz.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct FluidInteraction {
    /// The fluid whose block interacts, source and flowing form alike.
    pub source: FluidKey,
    /// What has to be beside it.
    pub neighbour: FluidNeighbour,
    /// What the block under the liquid has to be, if it matters (soul soil under lava).
    pub below: Option<BlockSelector>,
    /// What the liquid cell becomes.
    pub becomes: FluidBecomes,
}
impl FluidInteraction {
    /// An interaction of `source` beside `neighbour`, whatever lies below it.
    pub fn new(source: FluidKey, neighbour: FluidNeighbour, becomes: FluidBecomes) -> Self {
        Self {
            source,
            neighbour,
            below: None,
            becomes,
        }
    }
    /// Only where the block under the liquid is `below`.
    #[must_use]
    pub fn below(mut self, below: BlockSelector) -> Self {
        self.below = Some(below);
        self
    }
}
