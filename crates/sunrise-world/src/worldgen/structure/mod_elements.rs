//! Pool element types a mod registers (vanilla's `StructurePoolElementType`
//! with its own `StructurePoolElement`), handed over once at startup and
//! looked up while a template pool decodes, after the vanilla element types.
//! An element of one is [`PoolElement::Mod`](super::jigsaw::PoolElement::Mod):
//! the jigsaw assembly asks it for its jigsaws and box, and a pool piece holding
//! it places it through [`ModPoolElement::place`]. Runs on the worldgen worker
//! threads, so an element keeps no mutable state and its contexts carry no
//! `Host`.
use super::jigsaw::{ElementAt, Jigsaw, Projection};
use super::legacy::{PieceContext, PieceDraws, Result};
use super::{Rotation, StructureResources};
use crate::worldgen::block_feature::FeatureBlockBindings;
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use sunrise_core::bounding_box::BoundingBox;
use sunrise_core::nbt::Compound;

/// A pool element type's engine side: the element codec of vanilla's
/// `StructurePoolElementType`, as the executable's adapter sees it.
pub trait ModPoolElementType: Send + Sync {
    /// Decode one element from its pool entry's `element` value (its
    /// `element_type` included). An error leaves the pool out, as a pool
    /// that fails to decode is.
    fn decode(
        &self,
        config: &serde_json::Value,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> std::result::Result<Arc<dyn ModPoolElement>, String>;
}

/// A decoded element of a mod's type: vanilla's `StructurePoolElement`.
/// `Send + Sync` because neighbouring chunks build at once on worker threads;
/// the same random gives the same answers.
pub trait ModPoolElement: Send + Sync + std::fmt::Debug + std::any::Any {
    /// `getShuffledJigsawBlocks`: the element's jigsaws at `position` turned by
    /// `rotation`, in the order the assembly tries them.
    fn shuffled_jigsaws(
        &self,
        resources: &StructureResources,
        position: [i32; 3],
        rotation: Rotation,
        random: &mut dyn PieceDraws,
    ) -> Vec<Jigsaw<'static>>;
    /// `getBoundingBox`.
    fn bounding_box(
        &self,
        resources: &StructureResources,
        position: [i32; 3],
        rotation: Rotation,
    ) -> BoundingBox;
    /// `place`: the element's blocks into the chunk of `context`, clipped to
    /// its box. False when it placed nothing (`StructurePoolElement.place`'s
    /// result). `projection` is the one in effect: the element's own, or the
    /// list's that holds it.
    fn place(
        &self,
        context: &mut PieceContext<'_>,
        at: ElementAt,
        projection: Projection,
    ) -> Result<bool>;
    /// `getProjection`: the element's own, as decoded.
    fn projection(&self) -> Projection;
    /// `getGroundLevelDelta`.
    fn ground_level_delta(&self) -> i32;
    /// The element's fields as its codec writes them, beside `element_type`.
    fn save(&self) -> Compound;
}

/// The installed types, `modid:corridor` → type. Set once, before any world
/// opens (the executable hands the mods' registrations over), and read while
/// template pools decode.
pub type ModPoolElementTypes = BTreeMap<String, Arc<dyn ModPoolElementType>>;

static MOD_TYPES: OnceLock<ModPoolElementTypes> = OnceLock::new();

#[cfg(test)]
thread_local! {
    /// The types a test installs, in place of the process-wide table.
    static TEST_TYPES: std::cell::Cell<Option<&'static ModPoolElementTypes>> =
        const { std::cell::Cell::new(None) };
}

/// Install the process-wide table once; a second installation keeps the first
/// and hands its own types back.
pub fn install_mod_pool_element_types(
    types: ModPoolElementTypes,
) -> std::result::Result<(), ModPoolElementTypes> {
    MOD_TYPES.set(types)
}

/// The installed element type `id`: none until the executable installs the
/// mods', and a test's own while one is installed.
pub fn mod_pool_element_type(id: &str) -> Option<&'static Arc<dyn ModPoolElementType>> {
    #[cfg(test)]
    if let Some(types) = TEST_TYPES.get() {
        return types.get(id);
    }
    MOD_TYPES.get()?.get(id)
}

#[cfg(test)]
/// Runs `with` over a test's own element types, as decoding reads them.
pub fn with_test_pool_element_types<T>(types: &ModPoolElementTypes, with: impl FnOnce() -> T) -> T {
    let leaked: &'static ModPoolElementTypes = Box::leak(Box::new(types.clone()));
    let prev = TEST_TYPES.replace(Some(leaked));
    let out = with();
    TEST_TYPES.set(prev);
    out
}
