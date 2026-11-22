//! What a liquid block turns into beside another fluid or block, as a mod's
//! `data/<id>/sunrise/fluid_interaction/<name>.json` says it or its code declares it
//! (`Registrar::fluid_interaction`). Ids are full (`namespace:path`); the game resolves them,
//! and refuses an unknown one by name, when a world opens.
use super::namespaced_id;
use serde::Deserialize;

/// What has to be beside the liquid block (above it or to a side, never below).
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Neighbour {
    /// A fluid, source or flowing.
    Fluid(String),
    /// Every fluid of a fluid tag.
    FluidTag(String),
    /// A block.
    Block(String),
    /// Every block of a block tag.
    BlockTag(String),
}

/// A block, or every block of a block tag.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum BlockMatch {
    /// A block.
    Block(String),
    /// Every block of a block tag.
    BlockTag(String),
}

/// What the liquid cell becomes: a block state as `/setblock` writes it
/// (`ruby:slag`, `minecraft:oak_stairs[facing=east]`). A state left out is no interaction for
/// that state of the cell.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Becomes {
    /// What a source cell becomes.
    pub source: Option<String>,
    /// What a flowing cell becomes.
    pub flowing: Option<String>,
}

/// One interaction of the liquid block of the fluid `source`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FluidInteraction {
    /// The fluid whose block interacts, source and flowing form alike: a fluid of the mod's own
    /// or another's, `minecraft:lava` included.
    pub source: String,
    /// What has to be beside it.
    pub neighbour: Neighbour,
    /// What the block under the liquid has to be, if it matters.
    #[serde(default)]
    pub below: Option<BlockMatch>,
    /// What the liquid cell becomes.
    pub becomes: Becomes,
}

impl FluidInteraction {
    /// Refuses a file that names no full id, or that makes nothing of either state.
    pub fn check(&self) -> Result<(), String> {
        let id = |what: &str, id: &str| {
            namespaced_id(id)
                .then_some(())
                .ok_or_else(|| format!("{what} {id:?} is an id (namespace:path)"))
        };
        id("source", &self.source)?;
        match &self.neighbour {
            Neighbour::Fluid(name)
            | Neighbour::FluidTag(name)
            | Neighbour::Block(name)
            | Neighbour::BlockTag(name) => id("neighbour", name)?,
        }
        if let Some(BlockMatch::Block(name) | BlockMatch::BlockTag(name)) = &self.below {
            id("below", name)?;
        }
        let states = [&self.becomes.source, &self.becomes.flowing];
        if states.iter().all(|state| state.is_none()) {
            return Err("becomes names a source or a flowing state".into());
        }
        for state in states.into_iter().flatten() {
            id("becomes", state.split('[').next().unwrap_or(""))?;
        }
        Ok(())
    }
}
