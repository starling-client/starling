use super::*;
use crate::game::host::draw::GameDrawContext;
use mod_api::{__private::game::CallbackOwner, ClientHost, DrawContext, GuiLayerWrapContext};
use sunrise_client::{
    gui_layers::{self, GuiLayerBody, VanillaGuiLayer},
    hud::HudPaintCheckpoint,
};

#[allow(dead_code)]
impl Game<'_> {
    /// Sets a block in the current authoritative level through `Level.setBlock`.
    /// Returns false for another dimension, an invalid state, or no change.
    /// `flags` selects vanilla's neighbour, shape and client updates. A block
    /// entity block gets its block entity.
    pub(in crate::game) fn native_set_block(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> bool {
        if (self.serving.is_none() && self.options.dedicated.is_none())
            || dimension != self.dim.id()
            || self.pack.assets.registry.state(state).is_none()
        {
            return false;
        }
        let changed = self.set_block_live(pos, state, flags, true);
        let _ = self.process_level_events();
        changed
    }
    /// The item entities of the server level `dimension` whose box meets
    /// the box `min`..`max`, in the level's order: vanilla's
    /// `Level.getEntitiesOfClass(ItemEntity.class, box, ENTITY_STILL_ALIVE)`, as a
    /// hopper's `getItemsAtAndAbove` asks for its block and the one above
    /// (`[x, y, z]`..`[x + 1, y + 2, z + 1]`). Empty for a level not in place
    /// (as `native_set_block`) and on a client of a server.
    pub(in crate::game) fn native_item_entities(
        &self,
        dimension: &str,
        min: [f64; 3],
        max: [f64; 3],
    ) -> Vec<NativeItemEntity> {
        if (self.serving.is_none() && self.options.dedicated.is_none())
            || dimension != self.dim.id()
        {
            return Vec::new();
        }
        self.dropped
            .iter()
            .filter(|e| e.stack.count > 0 && box_meets(&e.bounds(), min, max))
            .map(|e| NativeItemEntity {
                id: e.id,
                position: [e.position.x, e.position.y, e.position.z],
                stack: e.stack.clone(),
                // The helper exposes an unsigned delay; keep its projection.
                pickup_delay: e.pickup_delay.clamp(0, i64::from(u32::MAX)) as u32,
            })
            .collect()
    }
    /// Runs `change` on the mod block entity at `pos` (a generator charging
    /// the machine beside it) and returns what it returns; none for any
    /// other block entity. Afterwards, as its `setChanged` would, its chunk is saved next
    /// time and, when its items changed, comparators read it again.
    pub(in crate::game) fn native_with_mod_block_entity<R>(
        &mut self,
        pos: [i32; 3],
        change: impl FnOnce(&mut EngineBlockEntity) -> R,
    ) -> Option<R> {
        let key = (pos[0], pos[1], pos[2]);
        let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&key) else {
            return None;
        };
        let items = m.items.clone();
        let out = change(m);
        let moved = m.items != items;
        self.world.modified.insert((key.0 >> 4, key.2 >> 4));
        if moved {
            self.container_changed(key);
        }
        Some(out)
    }
    /// This `Game` is the window's, not a server's.
    pub(in crate::game) fn native_window(&self) -> bool {
        self.options.execution_role == ExecutionRole::Client
    }
    /// `ParticleEngine.countParticles`: every live group. Sunrise keeps vanilla's groups
    /// apart: ordinary particles, firework starters and elder guardians in the engine,
    /// and ITEM_PICKUP in `pickups`.
    pub(in crate::game) fn client_particle_count(&self) -> usize {
        self.particles.group_count() + self.pickups.live.len()
    }
    /// `ParticleEngine.clearParticles` over every group Sunrise keeps apart: the engine's
    /// particles, starters, queue and emitters, the item pickups and the elder guardian
    /// apparition. Leaving a level and switching levels clear through it too.
    pub(in crate::game) fn clear_client_particles(&mut self) {
        self.mod_clear_particles.set(false);
        self.particles.clear();
        self.pickups.clear();
    }
    /// Applies a client mod's `clear_particles` if one is waiting. Called at the start of
    /// each particle tick, after `RenderFrameEvent.Pre`, and at particle extraction, after
    /// HUD, screen and entity callbacks. Extraction also discards its geometry if a model
    /// callback asks to clear while it is being built, even while paused or tick-frozen.
    pub(in crate::game) fn apply_mod_particle_clear(&mut self) {
        if self.mod_clear_particles.take() {
            self.clear_client_particles();
        }
    }
    /// Whether a box in `corner` shows: F3's debug screen takes both top
    /// corners (its left and right columns), and a bottom corner while one
    /// of its charts is drawn there.
    pub(in crate::game) fn native_corner_shown(&self, corner: Corner) -> bool {
        let charts = self.debug_charts.corners(!self.has_singleplayer_server());
        !debug_takes(corner, self.debug, charts)
    }
    /// A mod's own screen closes (Escape, or `mod_screen_close`).
    pub(in crate::game) fn close_mod_screen(&mut self) {
        if self.screen == Screen::Mod {
            if self.title.is_some() {
                self.close_mod_screen_to_title();
            } else {
                self.set_screen(Screen::Playing);
                self.capture(true);
            }
            if self.screen == Screen::Mod {
                // `screen_opening` kept it open, or opened a mod's own.
                return;
            }
        }
        self.native_mod_screen = None;
    }
    /// `Screen.onClose`'s `setScreen(null)` with no world: the title screen
    /// opens, asked for once as `mod` to `title`, and a mod's answer applies
    /// as on any title screen. Told to the frame's observer, so it doesn't
    /// announce it again.
    fn close_mod_screen_to_title(&mut self) {
        let decision = if self.screen_hooks_heard() {
            let current = self.screen_change_view(Screen::Mod);
            let none = self.screen_change_view(Screen::Playing);
            self.decide_screen(&current, &screen_view_with_id(&none, "title"), true)
        } else {
            ScreenDecision::Open
        };
        match decision {
            ScreenDecision::Keep => return,
            ScreenDecision::ModScreen(mod_id) => self.apply_screen(Screen::Mod, Some(mod_id)),
            ScreenDecision::Vanilla(target) => self.open_screen_target(target),
            ScreenDecision::Open => {
                if let Some(title) = self.title.as_mut() {
                    title.open_by_id("title");
                }
                self.apply_screen(Screen::Playing, None);
            }
        }
        self.screen_state.observed = self.observed_now();
        self.shown_init_post();
    }
}

// The vanilla paint procedure belongs to the caller's frame, not the frozen layer order.
type VanillaGuiDraw<'p, 'g> = dyn FnMut(&mut Game<'g>, &mut HudBatch) + 'p;

struct GuiLayerWrapFrame<'a, 'p, 'g> {
    game: &'a mut Game<'g>,
    batch: &'a mut HudBatch,
    previous: &'a GuiLayerBody,
    vanilla: Option<&'a mut VanillaGuiDraw<'p, 'g>>,
    paint: &'a mut HudPaintCheckpoint,
}

impl GuiLayerWrapContext for GuiLayerWrapFrame<'_, '_, '_> {
    fn host(&self) -> &dyn ClientHost {
        &*self.game
    }

    fn with_draw(&mut self, draw: &mut dyn FnMut(&mut dyn DrawContext)) {
        draw(&mut GameDrawContext::new(self.game, self.batch));
    }

    fn draw_previous(&mut self) {
        let start = self.batch.vertices.len();
        self.game
            .draw_gui_layer_body(self.batch, self.previous, self.vanilla.as_deref_mut());
        self.paint
            .preserve_delegated(start..self.batch.vertices.len());
    }
}

impl<'g> Game<'g> {
    /// Dispatches a vanilla slot after flushing inserted layers below it. Replacing or wrapping
    /// a body leaves the slot's ID, order and hooks unchanged. The original body's eligibility
    /// applies only to the original paint, including when a wrapper delegates to it.
    pub(in crate::game) fn gui_layer(
        &mut self,
        layer: VanillaGuiLayer,
        batch: &mut HudBatch,
        draw: &mut dyn FnMut(&mut Game<'g>, &mut HudBatch),
    ) {
        self.draw_mod_gui_layers(Some(layer), batch);
        self.run_gui_layer(
            layer.id(),
            batch,
            gui_layers::order().map_or(&GuiLayerBody::Vanilla, |order| {
                &order.slots()[order.position(layer)].body
            }),
            Some(draw),
            matches!(
                layer,
                VanillaGuiLayer::PlayerHealth
                    | VanillaGuiLayer::ArmorLevel
                    | VanillaGuiLayer::FoodLevel
                    | VanillaGuiLayer::VehicleHealth
                    | VanillaGuiLayer::AirLevel
            ),
        );
    }

    /// Draws inserted layers not drawn yet this frame, below `until` or to the end with `None`.
    /// A skipped vanilla layer keeps its place; its neighbours draw at the next paint site.
    /// Inserted layers don't draw while the HUD is hidden.
    pub(in crate::game) fn draw_mod_gui_layers(
        &mut self,
        until: Option<VanillaGuiLayer>,
        batch: &mut HudBatch,
    ) {
        let Some(next) = self.hud_layer_next else {
            return;
        };
        let Some(order) = gui_layers::order() else {
            return;
        };
        let slots = order.slots();
        let end = until.map_or(slots.len(), |layer| order.position(layer));
        // An earlier anchor can still paint later (e.g. deferred subtitles). Don't redraw slots.
        for slot in &slots[next.min(end)..end] {
            if slot.vanilla_anchor().is_none() {
                self.run_gui_layer(slot.id(), batch, &slot.body, None, false);
            }
        }
        // Past `until` itself: its paint site draws it, not the cursor.
        self.hud_layer_next = Some(next.max(end + usize::from(until.is_some())));
    }

    fn run_gui_layer(
        &mut self,
        id: &str,
        batch: &mut HudBatch,
        body: &GuiLayerBody,
        vanilla: Option<&mut VanillaGuiDraw<'_, 'g>>,
        status_row: bool,
    ) {
        let mut canceled = false;
        self.native_gui_layer_pre(id, batch, &mut canceled);
        if canceled {
            return;
        }
        let start = batch.vertices.len();
        self.draw_gui_layer_body(batch, body, vanilla);
        if status_row {
            self.keep_status_rows(&batch.vertices[start..], batch.width, batch.height);
        }
        self.native_gui_layer_post(id, batch);
    }

    fn draw_gui_layer_body(
        &mut self,
        batch: &mut HudBatch,
        body: &GuiLayerBody,
        vanilla: Option<&mut VanillaGuiDraw<'_, 'g>>,
    ) {
        match body {
            GuiLayerBody::Vanilla => {
                // Only a vanilla slot can contain this node; its paint site lends the closure.
                if let Some(draw) = vanilla {
                    CallbackOwner::call_engine(|| draw(self, batch));
                }
            }
            GuiLayerBody::Native { owner, draw } => {
                if let Some(owner) = owner.as_ref().filter(|owner| owner.is_enabled()) {
                    let paint = batch.paint_checkpoint();
                    if owner
                        .call("HUD layer", "gui_layer_draw", || {
                            draw(self, &mut GameDrawContext::new(self, batch));
                        })
                        .is_err()
                    {
                        paint.discard(batch);
                    }
                }
            }
            GuiLayerBody::Wrapped {
                owner,
                wrap,
                previous,
            } => {
                let Some(owner) = owner.as_ref().filter(|owner| owner.is_enabled()) else {
                    self.draw_gui_layer_body(batch, previous, vanilla);
                    return;
                };
                let mut paint = batch.paint_checkpoint();
                if owner
                    .call("HUD layer", "gui_layer_wrapper", || {
                        wrap(&mut GuiLayerWrapFrame {
                            game: self,
                            batch,
                            previous,
                            vanilla,
                            paint: &mut paint,
                        });
                    })
                    .is_err()
                {
                    paint.discard(batch);
                }
            }
        }
    }
}
