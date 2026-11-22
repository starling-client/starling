//! Model loaders supplied by the client. Model decoding selects a provider before reading cuboid
//! fields; the resolved geometry is shared by bakes of the same model and texture slots.
use crate::model::Direction;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock},
};

#[cfg(test)]
#[path = "model_loaders_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoaderSchema {
    Custom,
    ModelProperties,
    Cuboids,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoaderTexture {
    Slot(String),
    Sprite(String),
}

/// One quad a registered loader bakes, checked by the client before it gets here.
#[derive(Clone, Debug, PartialEq)]
pub struct LoaderQuad {
    /// Corners in block units, as an obj model's are.
    pub positions: [[f32; 3]; 4],
    /// Each corner's texture coordinates, 0 to 1 across the sprite.
    pub uvs: [[f32; 2]; 4],
    pub texture: LoaderTexture,
    /// The face whose solid neighbour hides it; `None` never culls.
    pub cull: Option<Direction>,
    pub tint_index: i32,
    pub shade: bool,
}

type Bake =
    dyn Fn(&str, &str, &BTreeMap<String, String>) -> Result<Vec<LoaderQuad>, String> + Send + Sync;

/// Immutable registration selected by a model's loader declaration.
#[derive(Clone)]
pub struct LoaderProvider {
    pub schema: LoaderSchema,
    pub bake: Arc<Bake>,
}

type Baked = Result<Arc<[LoaderQuad]>, String>;
type BakeKey = (String, BTreeMap<String, String>);

/// The provider and original JSON of one declaration. Models parsed for a new resource
/// generation get new declarations; block and item libraries over the same parse share them.
pub struct RegisteredLoader {
    id: String,
    config: String,
    provider: LoaderProvider,
    baked: Mutex<BTreeMap<Arc<BakeKey>, Arc<OnceLock<Baked>>>>,
}

impl RegisteredLoader {
    pub(crate) fn new(id: String, config: String, provider: LoaderProvider) -> Self {
        Self {
            id,
            config,
            provider,
            baked: Mutex::default(),
        }
    }

    pub(crate) fn schema(&self) -> LoaderSchema {
        self.provider.schema
    }

    pub(crate) fn quads(&self, model: &str, textures: BTreeMap<String, String>) -> Baked {
        let key = Arc::new((model.to_owned(), textures));
        let pending = self
            .baked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(Arc::clone(&key))
            .or_default()
            .clone();
        // One bake result (including a refusal) per model and effective slots in this generation.
        // Distinct models or slot maps never hold the registry lock while a mod runs.
        pending
            .get_or_init(|| (self.provider.bake)(model, &self.config, &key.1).map(Arc::from))
            .clone()
    }
}

impl std::fmt::Debug for RegisteredLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisteredLoader")
            .field("id", &self.id)
            .field("schema", &self.schema())
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl PartialEq for RegisteredLoader {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.config == other.config && self.schema() == other.schema()
    }
}
