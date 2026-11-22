//! An item or a standalone model drawn in a screen in 3D, turned as the mod says
//! (`DrawContext::model_3d`): what a symmetry wand's screen shows its wand and its mirror with.

use crate::{ItemStack, ModelError, ModelRef};

/// What [`DrawContext::model_3d`](crate::DrawContext::model_3d) draws.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: an item or a model are the only things a screen can ask to have drawn in 3D; a new kind of subject is a new method, so a mod may match without a wildcard
// no wire: the item stack is borrowed for an in-process draw.
pub enum Gui3dSubject<'a> {
    /// A stack borrowed for the draw. The item is drawn as in a slot, without its count or
    /// decorations: its model's `gui` display transform first, then the view's `transform`.
    Item(&'a ItemStack),
    /// A resolved standalone model (`RenderAssets::model`): its 0 to 1 cube about its centre,
    /// drawn full bright and shaded by face, as the blocks of a scene are.
    Model(ModelRef),
}

/// Where and how a [`Gui3dSubject`] is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct Gui3dView {
    /// Where the subject's centre is drawn, in GUI pixels.
    pub at: [f32; 2],
    /// GUI pixels per block: the model's 0 to 1 cube is `size` pixels across. An item is drawn
    /// at the size a slot draws it at when `size` is 16.
    pub size: f32,
    /// A row-major 3x4 affine transform ([`IDENTITY_TRANSFORM`](crate::IDENTITY_TRANSFORM) is
    /// none), applied about the subject's centre before it is placed: the caller's rotation and
    /// any extra scale. It is in the frame `x` right, `y` up, `z` towards the viewer, so a
    /// rotation can be about any axis and no Euler order has to be agreed.
    pub transform: [f32; 12],
}

impl Gui3dView {
    /// A view in the same units as its public fields.
    pub fn new(at: [f32; 2], size: f32, transform: [f32; 12]) -> Self {
        Self {
            at,
            size,
            transform,
        }
    }
    /// Whether the view can be drawn: a positive size, everything finite.
    pub fn is_drawable(&self) -> bool {
        self.at.iter().all(|v| v.is_finite())
            && self.size.is_finite()
            && self.size > 0.0
            && self.transform.iter().all(|v| v.is_finite())
    }
}

/// Why [`DrawContext::model_3d`](crate::DrawContext::model_3d) drew nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum Gui3dError {
    /// The context has no game to draw in (a server, or a drawing surface with none).
    Unavailable,
    /// `at`, `size` or a transform component is not finite, or `size` is not positive, or the
    /// transform sends a corner of the subject somewhere that is not finite. An air stack
    /// does not use its view.
    NotFinite,
    /// The item key is unknown or its item is absent from the current registry; air is a no-op.
    UnknownItem,
    /// The model handle is stale or names no model.
    Model(ModelError),
    /// The item's component overrides could not be converted for drawing.
    InvalidStack,
}

impl std::fmt::Display for Gui3dError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => f.write_str("no game to draw the model in"),
            Self::NotFinite => f.write_str("the view or transform is not finite"),
            Self::UnknownItem => f.write_str("the item key does not name a registered item"),
            Self::Model(error) => write!(f, "the model cannot be drawn: {error:?}"),
            Self::InvalidStack => f.write_str("the item's component overrides cannot be drawn"),
        }
    }
}

impl std::error::Error for Gui3dError {}
