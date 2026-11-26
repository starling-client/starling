//! Refusals from [`crate::DrawContext::with_scissor`].

/// Why [`crate::DrawContext::with_scissor`] refused to invoke its callback.
/// The enclosing clip is unchanged. For errors during checked canvas drawing, see
/// [`crate::GuiDrawError`] instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: a scissor is local to the draw callback.
pub enum GuiClipRefusal {
    /// A coordinate, size or computed endpoint of the rectangle is NaN or infinite.
    NotFinite,
    /// This context has no scissor drawing surface.
    Unavailable,
}
