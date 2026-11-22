//! Read-only keys and views of the engine-owned baked model cache (#861 part 1).
//!
//! Every successful resource load bakes each block state's models and the standalone models
//! once into one immutable store for that resource generation. A mod retains keys into the store;
//! looking up a key borrows the model's geometry and sprites.
use crate::{Direction, ModelRef};
use std::{fmt, iter::FusedIterator, ops::Range};

/// One baked model of the published model store: a block state's variant model or a standalone
/// model, for one resource generation. Get it from
/// [`RenderAssets::block_state_models`](crate::RenderAssets::block_state_models) or from a
/// [`ModelRef`] (`BakedModelKey::from(model_ref)`), and read it with
/// [`RenderAssets::baked_model`](crate::RenderAssets::baked_model). A successful resource reload
/// supersedes it: the old key then answers [`ModelError::StaleModel`](crate::ModelError::StaleModel)
/// and a new one is asked for in `model_baking_completed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: private fields; only the engine issues keys
pub struct BakedModelKey(pub(crate) u64, pub(crate) u32);

impl BakedModelKey {
    pub(crate) fn from_parts(generation: u64, slot: u32) -> Result<Self, &'static str> {
        if generation == 0 {
            return Err("unpublished model generation");
        }
        Ok(Self(generation, slot))
    }
}

impl From<ModelRef> for BakedModelKey {
    /// The baked model a standalone model handle names, in the same generation.
    fn from(model: ModelRef) -> Self {
        Self(model.0, model.1)
    }
}

/// A block state's baked model alternatives, borrowed from the published resource generation.
/// Keep the keys produced by [`Self::iter`], not this view, across host calls.
/// Equality compares the generation and the ordered list of keys.
// no wire: borrows the published resource generation's slot list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BakedModelKeys<'a> {
    generation: u64,
    slots: &'a [u32],
}

impl<'a> BakedModelKeys<'a> {
    /// Number of baked model alternatives in the list.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether this state has no baked model alternatives.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// The keys in bake order, without repeats. No random model selection is performed.
    pub fn iter(self) -> impl ExactSizeIterator<Item = BakedModelKey> + 'a {
        self.into_iter()
    }
}

impl<'a> IntoIterator for BakedModelKeys<'a> {
    type Item = BakedModelKey;
    type IntoIter = BakedModelKeyIter<'a>;

    /// The keys in bake order, without repeats. No random model selection is performed.
    fn into_iter(self) -> Self::IntoIter {
        BakedModelKeyIter {
            generation: self.generation,
            slots: self.slots.iter(),
        }
    }
}

/// An iterator over a block state's baked model keys, yielding keys by value.
// no wire: borrows the published resource generation's slot list.
// exhaustive: private fields; only a baked model key list creates it.
pub struct BakedModelKeyIter<'a> {
    generation: u64,
    slots: std::slice::Iter<'a, u32>,
}

impl Iterator for BakedModelKeyIter<'_> {
    type Item = BakedModelKey;

    fn next(&mut self) -> Option<Self::Item> {
        self.slots
            .next()
            .map(|&slot| BakedModelKey(self.generation, slot))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.slots.size_hint()
    }
}

impl ExactSizeIterator for BakedModelKeyIter<'_> {}
impl FusedIterator for BakedModelKeyIter<'_> {}

/// Immutable geometry and metadata borrowed from one published baked model. Keep its
/// [`BakedModelKey`] to borrow the model again; this view cannot outlive the asset borrow.
// no wire: borrows model geometry and sprite names from the published resource generation.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct BakedModelView<'a> {
    /// Whether the model allows smooth lighting (`ambientocclusion`).
    pub ambient_occlusion: bool,
    /// The block atlas sprite its particles use (`minecraft:block/stone`), when it names one.
    pub particle: Option<&'a str>,
    geometry: &'a dyn BakedQuadSource,
}

impl<'a> BakedModelView<'a> {
    /// Borrow the quad at a zero-based index in bake order, in constant time.
    /// Returns `None` if `index` is out of range.
    pub fn quad(self, index: usize) -> Option<BakedQuad<'a>> {
        self.geometry.quad(index)
    }

    /// Its quads in bake order, already turned and UV-locked as the variant asks. Traversal
    /// borrows geometry and sprite names and does not resolve a block tint or draw material.
    /// `len()` and `count()` give the remaining count in constant time. Traverse in either
    /// direction, or skip straight to a quad with `nth()` or `nth_back()`.
    pub fn quads(
        self,
    ) -> impl ExactSizeIterator<Item = BakedQuad<'a>> + DoubleEndedIterator + FusedIterator {
        BakedQuadIter {
            geometry: self.geometry,
            indices: 0..self.geometry.len(),
        }
    }
}

struct BakedQuadIter<'a> {
    geometry: &'a dyn BakedQuadSource,
    indices: Range<usize>,
}

impl<'a> BakedQuadIter<'a> {
    fn at(&self, index: usize) -> BakedQuad<'a> {
        self.geometry
            .quad(index)
            .expect("baked quad index within source length")
    }
}

impl<'a> Iterator for BakedQuadIter<'a> {
    type Item = BakedQuad<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.nth(0)
    }

    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        let index = self.indices.nth(n)?;
        Some(self.at(index))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.indices.size_hint()
    }

    fn count(self) -> usize {
        self.indices.len()
    }
}

impl DoubleEndedIterator for BakedQuadIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.nth_back(0)
    }

    fn nth_back(&mut self, n: usize) -> Option<Self::Item> {
        let index = self.indices.nth_back(n)?;
        Some(self.at(index))
    }
}

impl ExactSizeIterator for BakedQuadIter<'_> {}
impl FusedIterator for BakedQuadIter<'_> {}

impl PartialEq for BakedModelView<'_> {
    fn eq(&self, other: &Self) -> bool {
        if self.ambient_occlusion != other.ambient_occlusion || self.particle != other.particle {
            return false;
        }
        let left = self.quads();
        let right = other.quads();
        left.len() == right.len() && left.eq(right)
    }
}

impl fmt::Debug for BakedModelView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BakedModelView")
            .field("ambient_occlusion", &self.ambient_occlusion)
            .field("particle", &self.particle)
            .finish_non_exhaustive()
    }
}

/// One quad of a baked model ([`BakedModelView::quads`]), not a resolved draw command.
// no wire: borrows a baked quad's geometry and sprite name from the published resource generation.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct BakedQuad<'a> {
    /// Corners in block units (0 to 1 across the block for a unit cube), counter-clockwise seen
    /// from the side the quad faces.
    pub positions: &'a [[f32; 3]; 4],
    /// Each corner's texture coordinates, 0 to 1 across the sprite.
    pub uvs: &'a [[f32; 2]; 4],
    /// The block atlas sprite named by the baked quad (`minecraft:block/stone`).
    pub sprite: &'a str,
    /// The side it faces, which determines its usual directional shading.
    pub facing: Direction,
    /// The face whose solid neighbour hides the quad; `None` never culls.
    pub cull: Option<Direction>,
    /// The tint index a block tint source colours it by; -1 is none. Independent of `baked_argb`.
    pub tint_index: i32,
    /// The quad's own `0xAARRGGBB` colour, if set. This does not include tint-source colours,
    /// lighting, or any other draw-time state.
    pub baked_argb: Option<u32>,
    /// Whether the baked quad asks for directional shading.
    pub shade: bool,
    /// Its `light_emission`, 0 to 15.
    pub light_emission: u8,
}

/// Renderer-side backing for a model's immutable quads. Indices are contiguous in bake order.
#[allow(
    clippy::len_without_is_empty,
    reason = "indexed traversal needs only the length"
)]
pub trait BakedQuadSource: Sync {
    /// Borrow the quad at `index` in constant time, or return `None` past the last quad.
    fn quad(&self, index: usize) -> Option<BakedQuad<'_>>;

    /// Number of quads, in constant time. Every index in `0..len()` must have a quad.
    fn len(&self) -> usize;
}

/// Borrow a state's precomputed slots from a published model store.
pub fn baked_model_keys(generation: u64, slots: &[u32]) -> BakedModelKeys<'_> {
    BakedModelKeys { generation, slots }
}

/// Borrow the renderer's model and its particle sprite without exposing engine types.
pub fn baked_model_view<'a>(
    ambient_occlusion: bool,
    particle: Option<&'a str>,
    geometry: &'a dyn BakedQuadSource,
) -> BakedModelView<'a> {
    BakedModelView {
        ambient_occlusion,
        particle,
        geometry,
    }
}

/// Lend one quad's geometry and sprite name, alongside its own baked metadata.
pub fn baked_quad<'a>(
    positions: &'a [[f32; 3]; 4],
    uvs: &'a [[f32; 2]; 4],
    sprite: &'a str,
    facing: Direction,
    cull: Option<Direction>,
    tint_index: i32,
    baked_argb: Option<u32>,
    shade: bool,
    light_emission: u8,
) -> BakedQuad<'a> {
    BakedQuad {
        positions,
        uvs,
        sprite,
        facing,
        cull,
        tint_index,
        baked_argb,
        shade,
        light_emission,
    }
}
