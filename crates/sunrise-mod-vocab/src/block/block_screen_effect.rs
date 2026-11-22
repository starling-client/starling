use crate::{BlockPos, BlockState, CameraDrawContext, ClientHost, ExtractBlockScreenEffectEvent};

/// Which of `ScreenEffectRenderer`'s overlays a block puts over the first-person view
/// (NeoForge's `ExtractBlockScreenEffectEvent.OverlayType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum BlockScreenEffect {
    /// The player is on fire: the fire sprite over the lower view.
    Fire,
    /// The camera is inside a view-blocking block: its particle texture.
    Block,
    /// The eye is under water in first person: the underwater texture.
    Water,
}

impl BlockScreenEffect {
    pub(crate) const COUNT: usize = Self::Water as usize + 1;
}

// no wire: the renderer is an in-process callback.
/// A renderer selected for one block overlay by [`ExtractBlockScreenEffectEvent::set_custom_renderer`].
/// Reuse a static renderer rather than allocating one at every extraction.
pub trait BlockScreenEffectRenderer: Sync {
    /// Submit this overlay with read-only access to the client. Return `true` to suppress
    /// its vanilla quads, or `false` to leave them after yours. For block and water,
    /// cancellation at extraction still suppresses vanilla quads. For fire, only this
    /// return value decides once a renderer was selected. A failed submission contributes
    /// no quads and leaves extraction's vanilla drawing decision unchanged. A panic disables
    /// this mod's submissions for this overlay kind for the session; its extractor and
    /// submissions for other overlays still run. Neither borrow outlives the call.
    fn submit(&self, host: &dyn ClientHost, render: BlockScreenEffectRender<'_>) -> bool;
}

// no wire: the drawing context is a frame-local borrow.
/// The overlay's original extraction inputs and its camera-space drawing scope.
#[non_exhaustive]
pub struct BlockScreenEffectRender<'a> {
    /// The overlay selected at extraction.
    pub effect: BlockScreenEffect,
    /// The block state associated with this overlay.
    pub state: BlockState,
    /// The position associated with this overlay.
    pub pos: BlockPos,
    /// The same world partial tick as at extraction.
    pub partial_tick: f32,
    /// Whether the HUD is hidden.
    pub hide_gui: bool,
    /// Camera-space quads, under the first-person hand's projection and before the GUI.
    pub draw: &'a mut dyn CameraDrawContext,
}

// hidden: only the engine has the original extraction, frame timing, and draw scope.
#[doc(hidden)]
pub fn block_screen_effect_render(
    effect: BlockScreenEffect,
    state: BlockState,
    pos: BlockPos,
    partial_tick: f32,
    hide_gui: bool,
    draw: &mut dyn CameraDrawContext,
) -> BlockScreenEffectRender<'_> {
    BlockScreenEffectRender {
        effect,
        state,
        pos,
        partial_tick,
        hide_gui,
        draw,
    }
}

impl ExtractBlockScreenEffectEvent<'_> {
    /// Select a renderer for this overlay. The last eligible listener to select one wins;
    /// listeners that do not select one leave the previous choice in place. Cancellation
    /// skips subsequent listeners unless they opt into `#[listener(receive_cancelled)]`.
    /// The selected renderer still submits even if the overlay is canceled. In particular,
    /// it replaces the no-op that would otherwise suppress canceled fire. A failed listener
    /// leaves the preceding selection and cancellation in place.
    pub fn set_custom_renderer(&mut self, renderer: &'static dyn BlockScreenEffectRenderer) {
        // The event can only borrow the selection slot inside a subscribed listener.
        let owners = crate::runtime::installed().and_then(|installed| {
            let id = crate::content::crash::current_native_mod()?;
            installed.overlay_owners.get(id)
        });
        if let Some(owners) = owners {
            *self.renderer = Some(SelectedBlockScreenEffectRenderer { owners, renderer });
        } else {
            log::error!("extract_block_screen_effect: renderer selection has no listener owner");
        }
    }
}

// no wire: a selected callback and its attribution belong to this process.
/// Engine-owned selection, carrying the listener's attribution to submission.
// hidden: only the game may submit a selected callback.
#[doc(hidden)]
#[derive(Clone, Copy)]
// exhaustive: only the extraction event can construct this private-field selection.
pub struct SelectedBlockScreenEffectRenderer {
    owners: &'static [crate::callback_owner::CallbackOwner; BlockScreenEffect::COUNT],
    renderer: &'static dyn BlockScreenEffectRenderer,
}

impl SelectedBlockScreenEffectRenderer {
    /// Submit under the selecting listener's mod scope, returning no answer on failure.
    pub fn submit(
        self,
        host: &dyn ClientHost,
        render: BlockScreenEffectRender<'_>,
    ) -> Option<bool> {
        let owner = &self.owners[render.effect as usize];
        // A poisoned submission is skipped on later frames, without building a disabled error.
        if !owner.is_enabled() {
            return None;
        }
        owner
            .call(
                "block screen effect renderer",
                "block_screen_effect_renderer::submit",
                || self.renderer.submit(host, render),
            )
            .ok()
    }
}
