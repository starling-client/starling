//! The atlas sprite source types a mod registers (`register_sprite_sources`, NeoForge's
//! `RegisterSpriteSourcesEvent`) and the sprites their providers output.
use crate::{Identifier, RegisterSpriteSourcesEvent, Tag};
use std::sync::Arc;

/// The resources of the load that is listing an atlas, read-only (the `ResourceManager`
/// vanilla's `SpriteSource.run` receives): every selected pack of this load in order, the game's
/// own, the mods' and the player's. Names are full resource paths
/// (`assets/<namespace>/textures/<path>.png`). It is borrowed by a registered sprite source
/// provider (`register_sprite_sources`) for the duration of that call only.
// no wire: borrowed read-only resource pack reader for one sprite source call.
pub trait ResourceReader: Send + Sync + 'static {
    /// The winning copy of `path`, the topmost pack's (`ResourceManager.getResource`); `None`
    /// where no pack provides it.
    fn read(&self, path: &str) -> Option<Vec<u8>>;
    /// Every pack's copy of `path`, lowest first, the winning one last
    /// (`ResourceManager.getResourceStack`); empty where no pack provides it.
    fn read_stack(&self, path: &str) -> Vec<Vec<u8>>;
    /// The resource paths that start with `prefix`, one each, in name order
    /// (`ResourceManager.listResources`).
    fn list(&self, prefix: &str) -> Vec<String>;
}

/// One step a mod's sprite source provider takes on the atlas it is listed in, as vanilla's
/// `SpriteSource.Output` takes them. Providers return them in order, and they apply under the
/// built-in sources' rule: a sprite of an id listed already replaces the earlier one.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: mods construct outputs and the game matches every kind
// no wire: produced in-process by a registered native sprite source provider.
pub enum SpriteSourceOutput {
    /// `add(Identifier, Resource)`: sprite `sprite` is the texture `texture`
    /// (`assets/<namespace>/textures/<path>.png`), with its `.mcmeta`. A texture the resources
    /// lack is logged and skipped.
    Texture {
        /// The sprite's id in the atlas (`mymod:gem`).
        sprite: Identifier,
        /// The texture it is (`mymod:item/gem`).
        texture: Identifier,
    },
    /// `add(Identifier, DiscardableLoader)`: sprite `sprite` is these pixels, `width` by `height`,
    /// rows top first, four bytes (red, green, blue, alpha) a pixel. Pixels whose length isn't
    /// `width * height * 4`, or a side over the largest a sprite can have, are logged and skipped.
    Pixels {
        /// The sprite's id in the atlas.
        sprite: Identifier,
        /// Its width in pixels.
        width: u32,
        /// Its height in pixels.
        height: u32,
        /// Its RGBA bytes.
        rgba: Vec<u8>,
    },
    /// `removeAll`: drops sprite `sprite` from what the sources before listed. An id not listed
    /// changes nothing. A source never reads what was listed: it names the ids it wants gone.
    Remove {
        /// The sprite to drop.
        sprite: Identifier,
    },
}

/// Borrowed input passed to a registered atlas sprite source provider when an atlas definition is
/// listed (`SpriteSource.run`).
// no wire: borrowed sprite source decode input.
#[non_exhaustive]
pub struct SpriteSourceInput<'a> {
    /// The atlas being listed (`minecraft:blocks`, `minecraft:items`, `minecraft:gui`, or an atlas
    /// registered with `register_texture_atlases`).
    pub atlas: &'a Identifier,
    /// The entry's whole JSON object converted to a `Tag`.
    pub config: &'a Tag,
    /// Read-only access to the load's selected resource packs (`ResourceManager`), valid for this
    /// call only.
    pub resources: &'a dyn ResourceReader,
}

impl<'a> SpriteSourceInput<'a> {
    // hidden: the game supplies this read-only callback input
    #[doc(hidden)]
    pub fn new(atlas: &'a Identifier, config: &'a Tag, resources: &'a dyn ResourceReader) -> Self {
        Self {
            atlas,
            config,
            resources,
        }
    }
}

/// A retained atlas sprite source provider callback bound at `register_sprite_sources`.
// no wire: retained process-local sprite source provider callback.
pub type SpriteSourceFn =
    dyn Fn(&SpriteSourceInput<'_>) -> Result<Vec<SpriteSourceOutput>, String> + Send + Sync;

impl RegisterSpriteSourcesEvent<'_> {
    /// Register an atlas sprite source provider under `id` (NeoForge's
    /// `RegisterSpriteSourcesEvent::register`).
    ///
    /// Any valid `<namespace>:<path>` identifier (including `minecraft:`) is accepted; a later
    /// registration for the same id replaces the earlier one (last-writer-wins, matching
    /// NeoForge's `LateBoundIdMapper`), and registering a built-in vanilla source id replaces that
    /// built-in when decoding atlas definitions. The provider runs once per matching entry per
    /// atlas on each resource load and reload, never per frame.
    pub fn register<F>(&mut self, id: Identifier, provider: F)
    where
        F: Fn(&SpriteSourceInput<'_>) -> Result<Vec<SpriteSourceOutput>, String>
            + Send
            + Sync
            + 'static,
    {
        self.sources.push(SpriteSourceRegistration {
            id,
            list: Arc::new(provider),
        });
    }
}

/// A staged atlas sprite source provider; dispatch attributes it to the registering mod.
// no wire: retained process-local sprite source provider callback.
// hidden: retained registration staging belongs to the game
#[doc(hidden)]
#[non_exhaustive]
pub struct SpriteSourceRegistration {
    /// The sprite source type identifier.
    pub id: Identifier,
    /// The provider callback invoked when an atlas definition names this source type.
    pub list: Arc<SpriteSourceFn>,
}
