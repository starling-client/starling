//! HUD layers a native mod registers, replaces or wraps (NeoForge's `RegisterGuiLayersEvent`).

use crate::{ClientHost, DrawContext};

// no wire: the callable and its captures are kept in the client process.
/// Draws one registered HUD layer with the current client and drawing context.
/// The client owns the closure for the session and calls it as the mod that installed it.
/// Capture per-layer data; the host and canvas are borrowed only for this draw.
/// A disabled owner is skipped. On panic, open clips are closed, primitives already submitted
/// stay in the batch, the panic is reported, and only this callback is disabled for the session.
/// Later layers still draw. Drawing errors are returned by the drawing methods without disabling
/// the callback.
pub type GuiLayerDraw = dyn Fn(&dyn ClientHost, &mut dyn DrawContext) + Send + Sync + 'static;

// no wire: the wrapper and its captures are kept in the client process.
/// Draws around the previous body of a layer, or omits that body for this draw.
/// The client owns this callback for its lifetime. A disabled wrapper (including one
/// disabled by panic) leaves the previous layer body to draw directly.
/// On a wrapper panic, open clips are closed and completed primitives (from the wrapper or
/// `draw_previous()`) stay in the batch without replaying earlier bodies. The panic is reported,
/// only this wrapper is disabled for the session, and later layers still draw.
pub type GuiLayerWrapper = dyn Fn(&mut dyn GuiLayerWrapContext) + Send + Sync + 'static;

// no wire: lends the current client and the HUD batch only while a wrapper runs.
/// The per-draw context for a registered HUD layer wrapper.
pub trait GuiLayerWrapContext {
    /// The current client host.
    fn host(&self) -> &dyn ClientHost;

    /// Paint into the current HUD batch with a draw context scoped to this call.
    fn with_draw(&mut self, draw: &mut dyn FnMut(&mut dyn DrawContext));

    /// Paint the previous body at this point in the same batch without firing layer hooks again.
    /// The previous mod body runs as its own mod; a previous vanilla body runs as the engine.
    /// A vanilla paint panic still escapes the layer as an engine fault, even if the wrapper
    /// catches its unwind. Omit this call to skip the previous body for this draw.
    fn draw_previous(&mut self);
}

// no wire: the registrar borrows the live HUD registration for one hook call.
/// The HUD layer registrar handed to `register_gui_layers` once at client startup.
/// Edits are visible in call order to the current listener and publish together when it
/// returns successfully. The registrar is borrowed only for that listener.
pub trait GuiLayerRegistrar {
    /// Stage `draw` at `placement` under a namespaced ID such as `mymod:compass`.
    /// Returns false and logs a warning if the ID is malformed, already registered
    /// (including built-in vanilla layer IDs), or relative to a layer that is not
    /// registered yet. A rejected registration does not reserve the ID; use
    /// `replace_layer` or `wrap_layer` to edit an existing layer.
    fn register(
        &mut self,
        id: String,
        placement: GuiLayerPlacement,
        draw: Box<GuiLayerDraw>,
    ) -> bool;

    /// Replace an already registered layer (including a vanilla layer) with `draw`.
    /// Keeps the layer's ID, order and which frames dispatch the slot. Discards the entire
    /// previous body, including wrappers and eligibility checks. For example, a replaced
    /// status slot still runs under F1, even though its original body would not draw.
    /// Superseded callbacks from this listener are dropped before this call returns. A panic
    /// there rejects this listener's batch. Bodies from earlier listeners retire after publication,
    /// in the order this listener first replaced their layers. A panic in retirement is attributed
    /// to the earlier owner and leaves the replacement installed.
    /// Returns false and logs a warning if `id` is not registered.
    fn replace_layer(&mut self, id: &str, draw: Box<GuiLayerDraw>) -> bool;

    /// Wrap an already registered layer in place, keeping its ID, order and slot dispatch.
    /// The newest wrapper runs first. A wrapper's own paint need not follow the original
    /// body's eligibility checks. Those checks apply only when
    /// [`GuiLayerWrapContext::draw_previous`] reaches the original body.
    /// Returns false and logs a warning if `id` is not registered.
    fn wrap_layer(&mut self, id: &str, wrap: Box<GuiLayerWrapper>) -> bool;
}

/// Where a mod's HUD layer goes, the four orderings of `RegisterGuiLayersEvent`.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: NeoForge's four orderings, matched without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum GuiLayerPlacement {
    /// Below every other layer, the crosshair included (`registerBelowAll`).
    BelowAll,
    /// Right below the layer with this ID (`registerBelow`): a vanilla layer, like
    /// `minecraft:hotbar`, or one registered before this one.
    Below(String),
    /// Right above the layer with this ID (`registerAbove`), with the same rule.
    Above(String),
    /// Above every other layer, the tab list included (`registerAboveAll`).
    AboveAll,
}
