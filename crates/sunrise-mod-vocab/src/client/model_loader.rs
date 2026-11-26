//! The model loaders a mod registers (`register_model_loaders`, NeoForge's
//! `ModelEvent.RegisterLoaders`) and the geometry they bake.
use crate::{Direction, Identifier, RegisterModelLoadersEvent};
use std::{collections::BTreeMap, sync::Arc};

/// Which standard model fields a loader delegates to the cuboid decoder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// no wire: selects the engine decoder for an in-process loader registration.
// exhaustive: these are the standard fields the cuboid decoder can supply.
pub enum ModelLoaderSchema {
    /// The loader owns the entire JSON schema (apart from the `loader` declaration).
    Custom,
    /// Use the standard parent, texture slots, display properties and visibility, but replace
    /// all cuboid geometry. The loader can give `elements` its own meaning.
    ModelProperties,
    /// Decode and bake standard cuboid elements too, before the loader's quads.
    Cuboids,
}

/// How a registered model loader names a quad's sprite.
#[derive(Clone, Debug, PartialEq, Eq)]
// no wire: emitted by an in-process model loader as part of its baked geometry.
// exhaustive: a loader either names a sprite directly or refers to a model slot.
pub enum ModelLoaderTexture {
    /// A key in the model's resolved texture slots (`side`, or `#side`).
    Slot(String),
    /// A sprite identifier independent of the cuboid model's texture slots.
    Sprite(Identifier),
}

/// One quad baked by a registered model loader (`register_model_loaders`): turned with the
/// blockstate variant, culled and shaded as element faces are.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: mods construct quads and the game reads every field
// no wire: baked in-process by a registered native model loader.
pub struct ModelLoaderQuad {
    /// Corners in block units (0 to 1 across the block), counter-clockwise seen from the front,
    /// as an obj model's are.
    pub positions: [[f32; 3]; 4],
    /// Each corner's texture coordinates, 0 to 1 across the sprite.
    pub uvs: [[f32; 2]; 4],
    /// The sprite, or a key in the model's standard texture slots.
    pub texture: ModelLoaderTexture,
    /// The face whose solid neighbour hides the quad; `None` never culls.
    pub cull: Option<Direction>,
    /// The tint index a block tint source colours it by; -1 is none.
    pub tint_index: i32,
    /// Whether it takes directional shading.
    pub shade: bool,
}

/// Borrowed input passed to a registered model loader callback when a model is baked.
// no wire: borrowed model loader bake input.
#[non_exhaustive]
pub struct ModelLoaderInput<'a> {
    /// The model's identifier (`mymod:block/pillar`).
    pub model: &'a str,
    /// The model's original JSON object. Deserialize it with your loader's own schema.
    pub config: &'a str,
    /// Resolved cuboid texture slots (`slot -> sprite identifier`), including inherited slots.
    /// [`ModelLoaderSchema::Custom`] does not decode texture slots from its own JSON.
    pub textures: &'a BTreeMap<String, String>,
}

impl<'a> ModelLoaderInput<'a> {
    // hidden: the game supplies this read-only callback input
    #[doc(hidden)]
    pub fn new(model: &'a str, config: &'a str, textures: &'a BTreeMap<String, String>) -> Self {
        Self {
            model,
            config,
            textures,
        }
    }
}

/// A retained model loader bake callback bound at `register_model_loaders`.
// no wire: retained process-local model loader callback.
pub type ModelLoaderFn =
    dyn Fn(&ModelLoaderInput<'_>) -> Result<Vec<ModelLoaderQuad>, String> + Send + Sync;

impl RegisterModelLoadersEvent<'_> {
    /// Register a model loader under `id` (NeoForge's `ModelEvent.RegisterLoaders`).
    ///
    /// Any valid `<namespace>:<path>` identifier is accepted; duplicates and collisions with the
    /// built-in loaders (`neoforge:composite`, `neoforge:obj`, `neoforge:empty`, `fusion:model`) are rejected as
    /// registration errors. `schema` explicitly selects which fields use the cuboid decoder.
    /// The callback runs once per model and resolved slot set per resource load, never per frame.
    pub fn register<F>(&mut self, id: Identifier, schema: ModelLoaderSchema, loader: F)
    where
        F: Fn(&ModelLoaderInput<'_>) -> Result<Vec<ModelLoaderQuad>, String>
            + Send
            + Sync
            + 'static,
    {
        self.loaders.push(ModelLoaderRegistration {
            id,
            schema,
            bake: Arc::new(loader),
        });
    }
}

/// A staged model loader callback; dispatch attributes it to the registering mod.
// no wire: retained process-local model loader callback.
// hidden: retained registration staging belongs to the game
#[doc(hidden)]
#[non_exhaustive]
pub struct ModelLoaderRegistration {
    /// The loader identifier.
    pub id: Identifier,
    /// Which model fields the engine parses before bake.
    pub schema: ModelLoaderSchema,
    /// The bake callback invoked when a model naming this loader is resolved.
    pub bake: Arc<ModelLoaderFn>,
}
