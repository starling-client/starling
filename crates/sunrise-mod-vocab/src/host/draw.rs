//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `draw` area, implemented by the game and reached through [`crate::ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait DrawClientHost {
    /// Side: client.
    /// Fluid `fluid`'s name as the lang names it (`fluid_type.<mod>.<name>`, vanilla's by its block),
    /// else its path in words.
    fn fluid_name(&self, fluid: &str) -> String;
    /// Side: client.
    /// Entity model layer `layer` baked into its `ModelPart` tree, each part
    /// at its pose, as `EntityModelSet.bakeLayer` bakes it: one a native mod
    /// registered (`register_layer_definitions`), or a prepared vanilla definition by its
    /// `entity-models.json` name (`minecraft:zombie#main`,
    /// `minecraft:player#main`). Layers are indexed once per client resource
    /// load and reload rather than read from disk on each call. The tree is
    /// the caller's own copy: bake it once, as a renderer does, and reuse it
    /// while setting its parts' poses each frame. None for a layer neither
    /// has. API @API:model_layers@.
    fn bake_layer(&self, layer: &str) -> Option<ModelPart>;
}
/// Drawing surface lent to a HUD, screen or tooltip hook. Coordinates are in logical GUI pixels.
/// Use [`Self::with_scissor`] to clip the full context, including widgets and scenes, or
/// [`Self::draw_gui`] to draw a checked group of primitives with a scoped pose and scissor.
pub trait DrawContext {
    /// The GUI's size in GUI pixels, width then height.
    fn size(&self) -> [f32; 2] {
        unimplemented!("DrawContext::size")
    }
    /// Item `item`'s icon drawn with model `model` (`minecraft:paper` as
    /// `mymod:wheel_icon`, an `item_model` of a content mod's), as [`Self::item`]
    /// draws one; false for an unknown item.
    fn item_model(&mut self, item: ItemKey, model: &str, at: [f32; 2], size: f32) -> bool {
        unimplemented!("DrawContext::item_model")
    }
    /// A quad of corners `points` (in order round it) in GUI pixels, `argb`:
    /// a wedge, a slanted bar, or another shape that [`Self::fill`] can't draw.
    fn quad(&mut self, points: [[f32; 2]; 4], argb: u32) {
        unimplemented!("DrawContext::quad")
    }
    /// A rectangle in GUI pixels, `argb` (0x78000000 is the extras' box).
    fn fill(&mut self, rect: [f32; 4], argb: u32) {
        unimplemented!("DrawContext::fill")
    }
    /// `GuiGraphics.fillGradient`: fill `[x, y, w, h]` in GUI pixels, blending from straight-alpha
    /// ARGB `top` at `y` to `bottom` at `y + h`. [`Self::with_scissor`] clips the ramp without
    /// changing its colours. Zero area, inversion on exactly one axis, or non-finite geometry
    /// draws nothing. With both axes inverted, the bounds are sorted and the colours flipped,
    /// keeping `top` at `y`, now the lower edge. For a posed or checked draw, use
    /// [`GuiCanvas::fill_gradient`] inside [`Self::draw_gui`].
    fn fill_gradient(&mut self, rect: [f32; 4], top: u32, bottom: u32) {
        unimplemented!("DrawContext::fill_gradient")
    }
    /// Part `src` (`[x, y, w, h]` in the sprite's own pixels) of sprite
    /// `name` (a GUI atlas sprite by its id, `container/slot` or a mod's `mymod:panel`, another
    /// texture the atlas holds such as vanilla's `gui/...` files; a client mod's dynamic texture
    /// is not one: draw it with `GuiCanvas::texture`),
    /// drawn stretched into `dst` (`[x, y, w, h]` in GUI pixels), every texel multiplied by
    /// `tint` (`0xAARRGGBB`, a fluid's colour) when there is one. False when there is
    /// no such sprite.
    fn sprite(&mut self, name: &str, src: [f32; 4], dst: [f32; 4], tint: Option<u32>) -> bool {
        unimplemented!("DrawContext::sprite")
    }
    /// Item `item`'s icon (`mymod:machine`) with its top left at `at`,
    /// `size` GUI pixels square (16 in a slot); false for an unknown item.
    fn item(&mut self, item: ItemKey, at: [f32; 2], size: f32) -> bool {
        unimplemented!("DrawContext::item")
    }
    /// Plain text at `[x, y]` in GUI pixels, with straight-alpha `argb` (`0xFFFFFFFF` is opaque
    /// white). Fully transparent text draws nothing. For a styled [`Text`], use
    /// [`GuiCanvas::text`] inside [`Self::draw_gui`].
    fn text(&mut self, text: &str, at: [f32; 2], argb: u32, shadow: bool) {
        unimplemented!("DrawContext::text")
    }
    /// Draw formatted [`Text`] at `(x, y)` in GUI pixels, in `argb` (`0xAARRGGBB`) with optional
    /// drop shadow. `font` applies where the text has no explicit font. An unknown font ID uses
    /// missing glyphs, not `minecraft:default`. Returns the drawn width in GUI pixels, or
    /// [`TextFormError`] if the text's component metadata cannot be represented. Local text
    /// is not subject to serialization limits.
    fn text_with_font(
        &mut self,
        font: &Identifier,
        text: &Text,
        at: [f32; 2],
        argb: u32,
        shadow: bool,
    ) -> Result<f32, TextFormError> {
        unimplemented!("DrawContext::text_with_font")
    }
    /// Borrow the tooltip font `font` for measurement or splitting through an object-safe
    /// callback; callers use [`DrawContext::with_tooltip_font`].
    // hidden: object-safe backing method for `impl dyn DrawContext::with_tooltip_font`.
    #[doc(hidden)]
    fn with_tooltip_font_raw(&self, font: &Identifier, f: &mut dyn FnMut(&dyn TooltipFont)) {
        unimplemented!("DrawContext::with_tooltip_font_raw")
    }
    /// A plain line as in [`Self::text`], scaled about `at`, its top-left corner. For styled
    /// text use [`GuiCanvas::text`] in a [`GuiCanvas::scope`] with translate to `at`, scale,
    /// then translate back, as with vanilla's GUI pose.
    fn text_scaled(&mut self, text: &str, at: [f32; 2], scale: f32, argb: u32, shadow: bool) {
        unimplemented!("DrawContext::text_scaled")
    }
    /// Stack drawing does not enforce resident count or damage maxima. Unknown
    /// item identities or unrepresentable component payloads still refuse drawing.
    /// Draw a stack and its slot decorations, except the count, at `at`,
    /// `size` GUI pixels square. False when nothing was drawn: an unknown item
    /// or an empty stack (air, or a count of 0, as `ItemStack.isEmpty`).
    fn item_stack(&mut self, stack: &ItemStack, at: [f32; 2], size: f32) -> bool {
        unimplemented!("DrawContext::item_stack")
    }
    /// Lend a [`GuiCanvas`] with an identity pose for one drawing group, within the context's
    /// current scissor. Draws commit in order on `Ok(())`; returning a refusal with `?` or
    /// unwinding discards the whole group, including successful nested scopes. Return
    /// [`GuiDrawError::Abort`] to discard it explicitly. The callback's own state changes
    /// and independent host calls are not rolled back.
    fn draw_gui(
        &mut self,
        draw: &mut dyn FnMut(&mut dyn GuiCanvas) -> Result<(), GuiDrawError>,
    ) -> Result<(), GuiDrawError> {
        Err(GuiDrawError::Unavailable)
    }
    /// Vanilla's nine-slice widget rendering. `color` is ARGB, as `fill` uses.
    fn gui_nine_slice(&mut self, sprite: &str, rect: [f32; 4], color: u32) {
        unimplemented!("DrawContext::gui_nine_slice")
    }
    /// A vanilla tooltip at the cursor, in GUI pixels. This local draw has no serialization
    /// depth cap, but unrepresentable component metadata refuses the tooltip before drawing.
    fn gui_tooltip(&mut self, lines: &[Text], at: [f32; 2]) -> Result<(), TextFormError> {
        unimplemented!("DrawContext::gui_tooltip")
    }
    /// The width of `text` in GUI pixels as [`Self::text`] draws it (`Font.width`).
    fn text_width(&self, text: &str) -> f32 {
        unimplemented!("DrawContext::text_width")
    }
    /// Fluid `fluid` (`minecraft:water`, `mymod:oil`) drawn as a square with its top left at `at`, `size`
    /// GUI pixels wide (16 in a slot): the sprite its look names (`data/<mod>/sunrise/fluid/<name>.json`)
    /// tinted with its colour, else a flat square of the colour, which vanilla water and lava have. What a
    /// recipe viewer draws for a fluid stack; the slot and the amount are the mod's.
    fn fluid(&mut self, fluid: &str, at: [f32; 2], size: f32) {
        unimplemented!("DrawContext::fluid")
    }
    /// Draws `blocks`, a scene of block states at positions of its own (not
    /// the world's), into the box `rect` (x0, y0, x1, y1, GUI pixels) of a
    /// screen's `batch`, seen as `view` says: Create's Ponder scenes. Each
    /// block is drawn with its model, full bright and shaded by face, a face
    /// against a solid neighbour in the scene left out; a block with an
    /// `entity` (a mod's) is drawn by `block_entity_submit` as in the
    /// world too, its `spin` parts turning with the game time. What lies
    /// outside the box is cut off, and the scene has a depth of its own, so
    /// it never covers the screen's other drawing or is covered by it but in
    /// the order drawn. Draw it from a client screen callback such as
    /// one installed with [`crate::MenuScreen::with_draw`].
    ///
    /// The scene may live too: entities standing in it (`scene.entities`, the
    /// descriptions [`entity`](Self::entity) takes, prepared and drawn by the same
    /// path, sorted with the blocks) and particles (the blocks' own animate ticks,
    /// `SCENE_ANIMATE_PICKS` picks a tick among its blocks, and the mod's), ticked by
    /// a clock the scene owns, kept per calling mod and clock id. A scene of blocks
    /// alone keeps no state. Refused by name ([`SceneError`]): over a cap (16
    /// entities, 512 live particles, 4 animating scenes a frame), an entity
    /// `entity` would refuse, or input that isn't drawable.
    /// API @API:scenes_in_screens@.
    fn draw_scene(
        &mut self,
        rect: [f32; 4],
        scene: &Scene,
        view: SceneView,
    ) -> Result<(), SceneError> {
        Err(SceneError::Unavailable)
    }
    /// Run `draw` with the full drawing context clipped to `rect` (`[x, y, w, h]` in GUI pixels,
    /// as [`fill`](Self::fill) takes it), like `GuiGraphics.enableScissor` / `disableScissor`.
    /// Draw through the context passed to the callback. Clips nest by intersection; a nonpositive
    /// extent clips everything. The enclosing clip is restored when the callback returns or
    /// unwinds. [`GuiClipRefusal`] means the callback was not called.
    ///
    /// The callback returns `()` and the scope does not roll back draws. For a fallible group
    /// of primitives with a local pose and scissor, use [`GuiCanvas::scope`] inside [`Self::draw_gui`].
    fn with_scissor(
        &mut self,
        rect: [f32; 4],
        draw: &mut dyn FnMut(&mut dyn DrawContext),
    ) -> Result<(), GuiClipRefusal> {
        Err(GuiClipRefusal::Unavailable)
    }
    /// Draws living entity `entity` standing in `view.rect`, `view.size` GUI
    /// pixels a block, turned as `view.look` says: vanilla's
    /// `InventoryScreen.renderEntityInInventoryFollowsMouse` (`GuiEntityLook::At`
    /// the cursor) and `renderEntityInInventory` (`GuiEntityLook::Turned`), as
    /// a mob page, an entity icon or a data model's mob shows it. It is drawn by
    /// its type's renderer as in the world (a mod's `register_entity_renderers`
    /// renderer and `register_entity_layers` layers included, else the vanilla
    /// model), full bright, lit by `Lighting.ENTITY_IN_UI`'s two lights, with a
    /// depth of its own and cut off at the box. A client's call, available
    /// with [`crate::MenuScreen::with_draw`], `render_gui_post`, or `screen_render_post`.
    fn entity(&mut self, entity: &GuiEntity, view: GuiEntityView) -> Result<(), GuiEntityError> {
        Err(GuiEntityError::Unavailable)
    }
    /// Draws an item or a resolved standalone model in 3D at `view.at`, `view.size` GUI pixels a
    /// block, under the 3x4 `view.transform` the caller gives (any rotation, roll included), as
    /// Create's symmetry wand screen draws its wand and its mirror. Orthographic, in the frame
    /// `x` right, `y` up, `z` towards the viewer.
    ///
    /// - The item stack is borrowed for this call. The item is drawn as a slot draws it (its
    ///   model's `gui` display transform, then `view.transform`, lit with the GUI light that
    ///   model asks for: flat items side lit, block-like items front lit); its count, damage bar
    ///   and cooldown are not drawn.
    /// - A model is drawn full bright and shaded by face, as the blocks of a scene are.
    /// - Both subjects have a depth of their own, so a draw never intersects what the screen
    ///   drew before or after it, and later drawing covers it in call order. An item's faces meet
    ///   each other by depth, as a model's do, and its enchantment glint follows them; the
    ///   OptiFine CIT glint layers of a stack are not drawn on this path yet (sunrise#3555). There
    ///   is no z input. Both are cut by the clips opened with [`DrawContext::with_scissor`], as
    ///   `fill` is.
    ///
    /// A client's call, available with [`crate::MenuScreen::with_draw`], `render_gui_post`,
    /// or `screen_render_post`. A `minecraft:air` stack returns `Ok(())` without using the view or
    /// converting components.
    /// A non-air item with an unreadable component override returns [`Gui3dError::InvalidStack`].
    /// Per call it draws the subject's baked quads, the cached
    /// model a slot's item uses, with nothing on the tick path.
    fn model_3d(&mut self, subject: Gui3dSubject<'_>, view: Gui3dView) -> Result<(), Gui3dError> {
        Err(Gui3dError::Unavailable)
    }
    /// `Screen.renderMenuBackground`: draws the tiled menu background (32x32) across `rect`
    /// (`[x, y, w, h]` in GUI pixels). If `in_world` is true, uses the dark translucent
    /// texture (`gui/inworld_menu_background`); otherwise uses the opaque dirt-like
    /// texture (`gui/menu_background`). API @API:screen_transparent_background@.
    fn draw_menu_background(&mut self, rect: [f32; 4], in_world: bool) {
        unimplemented!("DrawContext::draw_menu_background")
    }
    /// Repeats sprite `sprite` across `rect` (`[x, y, w, h]` in GUI pixels) with each tile
    /// sized `size` GUI pixels square, tinted by `argb`. False when the sprite is not in the HUD atlas.
    /// API @API:screen_transparent_background@.
    fn tiled_sprite(&mut self, sprite: &str, rect: [f32; 4], size: f32, argb: u32) -> bool {
        unimplemented!("DrawContext::tiled_sprite")
    }
    /// Draw an item stack with custom count/decoration text (or None) at `at` in GUI units.
    /// Uses the same nonresident stack encoding as `item_stack`, not write validation.
    fn gui_draw_item_with(&mut self, stack: &ItemStack, at: [f32; 2], text: Option<&str>) {
        unimplemented!("DrawContext::gui_draw_item_with")
    }
}
/// What a block entity, mob or player submit hook draws with, in blocks under a pose stack (`PoseStack`): one per hook call.
pub trait BlockEntityDrawContext {
    /// `PoseStack.pushPose`: `pop` comes back to the pose as it is now.
    fn push(&mut self) {
        unimplemented!("BlockEntityDrawContext::draw_push")
    }
    /// `PoseStack.popPose`; the block's own pose is never popped.
    fn pop(&mut self) {
        unimplemented!("BlockEntityDrawContext::draw_pop")
    }
    /// Moves what is drawn next by `by`, in blocks.
    fn translate(&mut self, by: [f32; 3]) {
        unimplemented!("BlockEntityDrawContext::draw_translate")
    }
    /// Turns what is drawn next by `degrees` about `axis` (counterclockwise
    /// seen from the axis' positive end, as `Axis.rotationDegrees`) through
    /// `pivot`, in blocks from the pose's origin: `[0.5; 3]` is the block's centre.
    fn rotate_about(&mut self, axis: [f32; 3], degrees: f32, pivot: [f32; 3]) {
        unimplemented!("BlockEntityDrawContext::draw_rotate_about")
    }
    /// Scales what is drawn next by `by` about the pose's origin.
    fn scale(&mut self, by: [f32; 3]) {
        unimplemented!("BlockEntityDrawContext::draw_scale")
    }
    /// Keeps what this call drew, with neither `block_entity_submit` nor
    /// `block_entity_render_bounding_box` called until its state, block entity,
    /// light or resource packs change. Parts marked with `spin` keep turning on the
    /// animation clock (`animation_time`); other geometry stays as submitted. Do not bake frame-dependent
    /// values into kept vertices: item glint and time-dependent item models stop
    /// where they were, and neighboring blocks do not invalidate the cache.
    fn keep(&mut self) {
        unimplemented!("BlockEntityDrawContext::draw_keep")
    }
    /// Read-only frame state for the world view this draw belongs to.
    ///
    /// [`Self::partial`], [`Self::game_time`] and [`Self::animation_time`] are shorthands
    /// returning the same values as this view. In [`DrawContext::draw_scene`], the view carries
    /// the scene's camera and clock with `camera_entity_eye: None`.
    fn view(&self) -> &WorldRenderView<'_> {
        unimplemented!("BlockEntityDrawContext::view")
    }
    /// The frame's partial tick, 0 to 1 (`BlockEntityRenderState`).
    fn partial(&self) -> f32 {
        unimplemented!("BlockEntityDrawContext::draw_partial")
    }
    /// The level's game time, for animations.
    fn game_time(&self) -> i64 {
        unimplemented!("BlockEntityDrawContext::draw_game_time")
    }
    /// The frame on this client's animation clock, in ticks: what turns `spin` parts.
    /// It counts the ticks this client's level has run plus a phase that holds on pause
    /// or `/tick freeze` and resumes without a jump. `/tick step` advances it by whole
    /// ticks. The server's time updates never set it back, unlike `game_time`. Each
    /// client counts from its own start, so it means nothing on the server or to another client.
    fn animation_time(&self) -> f64 {
        unimplemented!("BlockEntityDrawContext::draw_animation_time")
    }
    /// The fixed light setting, even while [`Self::level_light`] is enabled. It starts
    /// as the block's or entity's light: 16 times sky light in bits 0..8 and 16 times
    /// block light in bits 8..16.
    fn light(&self) -> u32 {
        unimplemented!("BlockEntityDrawContext::draw_light")
    }
    /// Lights what is drawn next with `light` (as `native_draw_light` reads it).
    fn set_light(&mut self, light: u32) {
        unimplemented!("BlockEntityDrawContext::draw_set_light")
    }
    /// Lights what is drawn next by the level's sky and block light. Each element uses
    /// one level-light sample, at its centre under the pose and any applied `spin`:
    /// the unit-cell centre for `block`, `model` and `terrain_model`, the box midpoint
    /// for `cube`, or the mean of the corners for `quad`. Baked emission and other
    /// per-channel light floors still apply. Items, text and other draws keep the fixed
    /// light setting. `set_light` returns to fixed light, which is the default.
    ///
    /// Works in world block-entity and entity renderers. Level-less scenes, previews
    /// and mob model frames use the fixed light, as does an element whose sample centre
    /// lies outside the representable cell coordinates. With `keep`, geometry under
    /// `spin` is sampled again each frame without a new submit; stationary geometry keeps
    /// the light it was drawn with until the draw is invalidated.
    fn level_light(&mut self) {
        unimplemented!("BlockEntityDrawContext::draw_level_light")
    }
    /// While `on`, what is drawn next by `item`, `block`, `cube`, `quad`, `model` and
    /// `model_part` skips directional face shading (brightness 1.0) and draws at full-bright
    /// light (block and sky 15), whatever the light setting or [`Self::level_light`]. The
    /// lightmap is still sampled there, so what changes its top cell (night, `darkness`,
    /// coloured lights) still reaches it. Stands for NeoForge's unlit render types
    /// (`NeoForgeRenderTypes.getItemCutoutUnlit`, `getItemTranslucentUnlit`, their glint
    /// variants, `getEntityUnlitTranslucent`): a hologram, a glowing display, a projected item.
    /// An item's enchantment glint still draws over it; `terrain_model` keeps its own
    /// shading option and text its own `glowing`. Off by default; lasts until `unlit(false)`
    /// or the end of the draw. A client's call.
    fn unlit(&mut self, on: bool) {
        unimplemented!("BlockEntityDrawContext::draw_unlit")
    }
    /// The breaking stage, `0..=9`, being drawn at this block entity's position, or `None`
    /// when nobody is mining it or the draw is not a world block entity's (an entity, a mob
    /// model or a preview). It is `Some` at the cracked block and at each extra position its
    /// `break_progress_positions` answer gave, for the local player's crack and for other
    /// players' (`BlockEntityRenderState.breakProgress`; the highest stage at a position when
    /// several players mine it). Read when the block entity is submitted: geometry kept with
    /// `keep` is not submitted again as the stage changes, so mark it with
    /// [`Self::crumbling`] and let the game apply the stage each frame.
    fn crumbling_stage(&self) -> Option<u8> {
        unimplemented!("BlockEntityDrawContext::draw_crumbling_stage")
    }
    /// While `on`, what is drawn next by `quad`, `cube`, `block`, `model` and `terrain_model`
    /// receives the breaking crack at this position (`ModelFeatureRenderer.CrumblingOverlay`, as
    /// `ChestRenderer` covers the parts it chooses): that geometry is drawn again under the
    /// stage's `destroy_stage_0..9` sprite in the frame's `DESTROY` pass, after the block
    /// entities, wherever [`Self::crumbling_stage`] is `Some` at draw time. Items, text,
    /// `model_part` and `fluid_box` are never covered. Off by default; lasts until
    /// `crumbling(false)` or the end of the draw. With `keep`, the marked geometry shows
    /// the current stage, starting, advancing and clearing as the crack does, with no new
    /// `block_entity_submit`, and under `spin` the crack turns with it. Models drawn
    /// under it are drawn as vertices, not instanced copies, while their block is
    /// cracked. Display only, a client's call.
    fn crumbling(&mut self, on: bool) {
        unimplemented!("BlockEntityDrawContext::draw_crumbling")
    }
    /// Draws a quad with `corners` in blocks under `draw`'s pose, in a model
    /// face's order (top left, bottom left, bottom right, top right as seen
    /// from its front), textured with the `uv` rectangle (u0, v0, u1, v1 in
    /// texels of 16 per side) of `sprite` (`mymod:entity/screen`,
    /// `mymod:block/casing`, `block/stone`) times the 0xRRGGBB `tint`
    /// (0xFFFFFF for none). False for an unknown sprite. Its UVs are texels of 16; those of
    /// [`Self::textured_quad`] are unit.
    fn quad(&mut self, sprite: &str, corners: [[f32; 3]; 4], uv: [f32; 4], tint: u32) -> bool {
        unimplemented!("BlockEntityDrawContext::draw_quad")
    }
    /// Draws a quad of `corners` in blocks under `draw`'s pose, in `quad`'s corner order, showing
    /// the client mod's dynamic texture (`create_dynamic_texture`) that `render_type` was made
    /// for (`RenderAssets::entity_solid`, `entity_cutout` or `entity_translucent`, as vanilla's
    /// `RenderTypes.entitySolid(texture)`, `entityCutout` and `entityTranslucent` are), times the
    /// 0xRRGGBB `tint` (0xFFFFFF for none). `uv` is each corner's texture coordinate, one per
    /// corner in the order of `corners`, in unit coordinates: 0 to 1 is the texture once, and
    /// the texture repeats beyond it. A monitor shows its whole texture with
    /// `[[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]]`. `quad`'s UVs are texels of 16.
    ///
    /// The texture is looked up each frame the quad draws, not when it is submitted:
    /// `update_dynamic_texture` shows on the next frame with no new geometry, and kept geometry
    /// (`keep`) stays valid. A texture released after the quad was submitted draws the missing
    /// texture, as vanilla's `TextureManager.getTexture` answers after `release`. Light follows
    /// `quad`: `set_light`, `level_light`, and `unlit(true)` for full bright and no directional
    /// shading. `cutout` and `translucent` discard texels under alpha 0.1
    /// (`ALPHA_CUTOUT`); `translucent` blends, writes depth and, outside a shader pack, sorts far
    /// to near. `solid` keeps the entity shader's alpha discard too, where vanilla's
    /// `ENTITY_SOLID` has none: give it an opaque texture. Under a shader pack the quad draws through the pack's entity program (the
    /// block entity program in a block entity's draw), with its texture bound as the entity
    /// texture.
    ///
    /// It takes only a dynamic texture id: a render type made for any other id, such as a
    /// texture in the resource packs, draws nothing and returns false.
    ///
    /// Returns false, appending nothing, when `render_type` is not one of the three
    /// texture-taking built-ins or is stale, when no dynamic texture named by it is live, when a
    /// corner or UV is not finite, or in a first-person hand draw. It never panics.
    fn textured_quad(
        &mut self,
        render_type: RenderTypeRef,
        corners: [[f32; 3]; 4],
        uv: [[f32; 2]; 4],
        tint: u32,
    ) -> bool {
        unimplemented!("BlockEntityDrawContext::draw_textured_quad")
    }
    /// Draws a box from `from` to `to` (texels of 16 per block, as a block
    /// model's element) under `draw`'s pose, with the faces given, by
    /// `Direction` (down, up, north, south, west, east), baked as a model
    /// element's are (`FaceBakery`), times the 0xRRGGBB `tint`. A turned box
    /// is drawn under a turned pose (`BlockEntityDraw::rotate_about`). False
    /// when a face's sprite is unknown; that face is left out.
    fn cube(
        &mut self,
        from: [f32; 3],
        to: [f32; 3],
        faces: [Option<CubeFace<'_>>; 6],
        tint: u32,
    ) -> bool {
        unimplemented!("BlockEntityDrawContext::draw_cube")
    }
    /// Draws `state`'s baked block model under the current draw pose with its
    /// default tints, matching vanilla's `SubmitNodeCollector.submitBlockModel`.
    /// Resolve states once through `host.block_states()` (or pass an event's
    /// resolved `BlockState`) rather than parsing properties each frame. A
    /// moving part that is not a block is a standalone model ([`Self::model`]).
    /// The state's selected model parts, atlas coordinates and tints are bound to the active
    /// resources. Returns `false` for an unknown state; a valid empty model draws nothing.
    fn block(&mut self, state: BlockState) -> bool {
        unimplemented!("BlockEntityDrawContext::draw_block")
    }
    /// Draws `stack` under `draw`'s pose as an item is drawn in the world
    /// (`ItemModelResolver.updateForTopItem` and `ItemStackRenderState.submit`), centred on the
    /// pose's origin, in its model's `display` transform: `Fixed` as a
    /// campfire's or an item frame's, `Ground` as a dropped item's. `seed`
    /// picks among random models (a campfire passes its slot plus its
    /// position's hash). `owner_yaw` makes the block entity the item's
    /// owner (`ItemOwner`), at the block's centre and turned that many
    /// degrees (`getVisualRotationYInDegrees`), as a shelf is: a compass on
    /// it points to its target from there. None draws it with no owner, as
    /// a campfire does. False for an empty stack or one with no model. The
    /// client's copy of a block entity has no item slots (`entity.items` is
    /// empty there): send the stacks you draw in
    /// `block_entity_update_tag` and read them from `entity.nbt`.
    fn item(
        &mut self,
        stack: &ItemStack,
        display: ItemDisplayContext,
        owner_yaw: Option<f32>,
        seed: i32,
    ) -> bool {
        unimplemented!("BlockEntityDrawContext::draw_item")
    }
    /// What is drawn next turns by itself: `degrees_per_tick` degrees each tick of the
    /// animation clock (`animation_time`; a shaft at `rpm` turns `rpm * 0.3`), about `axis`
    /// through `pivot`, both in blocks from the block's corner whatever the pose,
    /// counterclockwise seen from the axis' positive end. Its angle is
    /// `animation_time * degrees_per_tick + phase`: parts drawn with the same speed keep
    /// their phases apart (a cog meshing with its neighbour). Draw the part at rest; the
    /// game turns it each frame, and with `keep` without asking again until the block
    /// entity changes. With a new speed, pass `old_angle - animation_time() * new_speed`
    /// as the phase to keep the part where it was. For `quad` and `cube`, directional
    /// shading follows the turn. Light stays fixed unless `level_light` is on. Until
    /// `still` or the next `spin`. Entity renderer and mob model frames draw it at rest.
    fn spin(&mut self, axis: [f32; 3], pivot: [f32; 3], degrees_per_tick: f32, phase: f32) {
        unimplemented!("BlockEntityDrawContext::draw_spin")
    }
    /// What is drawn next stands still again, after `spin`.
    fn still(&mut self) {
        unimplemented!("BlockEntityDrawContext::draw_still")
    }
    /// Draws the resolved standalone model under `draw`'s pose: unturned, from
    /// the block's lower north-west corner, lit by `draw.light`. Faces with a tint
    /// index are multiplied by `tint` (`0xAARRGGBB`, the same as [`crate::WorldQuad::color`];
    /// `0xFFFFFFFF` for none), alpha included. Faces with no tint index stay opaque.
    /// A stale generation or absent slot returns an error without appending geometry.
    fn model(&mut self, model: ModelRef, tint: u32) -> Result<(), ModelError> {
        unimplemented!("BlockEntityDrawContext::draw_model")
    }
    /// Draws the resolved standalone model under `draw`'s pose, as [`Self::model`] places it,
    /// with the terrain's own materials: the block sprites (animated, mipmapped as the options
    /// set, sampled as vanilla's moving blocks are: linear minification, nearest
    /// magnification), back faces culled, colour and depth written as opaque terrain, lit by
    /// `draw.light` raised by each quad's own emission. `options` picks the material
    /// (each quad's `solid`, `cutout` or `cutout_mipped`, or one for all: cutout discards
    /// under alpha 0.1, cutout-mipped under 0.5), the directional
    /// shading (independent of light), one sprite drawn as another with moved UVs, a UV
    /// shrink, and a `0xRRGGBB` tint on quads with a tint index. What a machine's belts,
    /// shafts and casings draw with. A stale generation or absent slot returns [`ModelError`]
    /// without appending geometry; otherwise quads are submitted individually (missing sprites
    /// fall back to `minecraft:missingno`, an unmatched `uv_shift` is a no-op, and invalid
    /// primitives are skipped individually).
    fn terrain_model(
        &mut self,
        model: ModelRef,
        options: TerrainModelOptions<'_>,
    ) -> Result<(), ModelError> {
        unimplemented!("BlockEntityDrawContext::draw_terrain_model")
    }
    /// Draws `text` under `draw`'s pose as vanilla's `Font.drawInBatch` draws a component in
    /// the world (a sign's line, a text display's): in the pose's x-y plane, one unit a font
    /// pixel, y down, its left end at `at`, facing +z. Scale the pose first as a sign does
    /// (`draw.scale([1.0 / 64.0, -1.0 / 64.0, 1.0 / 64.0])` for its letters, 8 pixels high).
    /// Styles are drawn as name tags and text displays draw them: each span's colour (the
    /// rest in the 0xRRGGBB `color`), bold, italic, underline, strikethrough, obfuscated, its
    /// font, inline objects and `§` codes, right-to-left text in visual order. `glowing` text
    /// is full bright (a glow ink sac's), the rest lit by `light`. With `outline`, a 0xRRGGBB
    /// colour (`Some(0)` is black), each glyph is ringed with it as glowing sign text is
    /// (`Font.drawInBatch8xOutline`; nine glyphs' worth of quads each). Returns the width
    /// drawn, in font pixels (`Font.width`): centre a line by starting it at minus half its
    /// width. Obfuscated glyphs change only when the block is drawn again: kept text stands
    /// still. Clamp text read from NBT, and widen `block_entity_render_bounding_box`
    /// for a long line. Unrepresentable component metadata returns `TextFormError` before
    /// drawing; the local component has no serialization depth cap. API @API:draw_text@.
    fn text(
        &mut self,
        text: &Text,
        at: [f32; 2],
        color: u32,
        glowing: bool,
        outline: Option<u32>,
    ) -> Result<f32, TextFormError> {
        unimplemented!("BlockEntityDrawContext::draw_text")
    }
    /// The width of `text` in font pixels, as `text` draws it, styles included
    /// (`Font.width`), to lay a line out before drawing it.
    fn text_width(&self, text: &Text) -> f32 {
        unimplemented!("BlockEntityDrawContext::draw_text_width")
    }
    /// Draws model part `part` and the parts under it under `draw`'s pose, as
    /// vanilla's `ModelPart.render` does: each part moved to its place (in
    /// texels, 16 to a block), turned about z, then y, then x, and scaled,
    /// its faces textured from `sprite` (`mymod:entity/golem`, a content
    /// mod's `assets/mymod/textures/entity/golem.png`, or vanilla's
    /// `entity/zombie/zombie`) times the 0xRRGGBB `tint`. A part that isn't
    /// `visible` draws nothing, one that skips drawing (`skip_draw`) only its
    /// children. A mob's model is built in the model's own frame (+y down,
    /// its front towards -z, its feet 24 texels down): in `render_living_pre`,
    /// `draw.scale([1.0, -1.0, -1.0])` then `draw.translate([0.0, -1.501,
    /// 0.0])` first, as `LivingEntityRenderer` does. False for an unknown
    /// sprite. API @API:model_layers@.
    fn model_part(&mut self, part: &ModelPart, sprite: &str, tint: u32) -> bool {
        unimplemented!("BlockEntityDrawContext::draw_model_part")
    }
    /// Draws a retained [`InstanceGroupRef`] under `draw`'s current pose: each live copy in the
    /// group is transformed by the group transform and then by `draw`'s pose stack. Works in
    /// entity renderers, entity layers, block entities, and screen scenes. Stale, destroyed, or
    /// disabled-owner groups return [`GroupError::Stale`] without appending geometry.
    fn instance_group(&mut self, group: InstanceGroupRef) -> Result<(), GroupError> {
        unimplemented!("BlockEntityDrawContext::instance_group")
    }
    /// Draws a [`PreparedItemRef`] resolved by
    /// [`ClientRenderClientHost::prepare_item`](crate::prelude::ClientRenderClientHost::prepare_item)
    /// under `draw`'s current pose, matching [`Self::item`] without resolving the stack's model
    /// again. A stale or destroyed handle draws nothing and returns [`ModelError::StaleModel`].
    fn prepared_item(&mut self, item: PreparedItemRef) -> Result<(), ModelError> {
        unimplemented!("BlockEntityDrawContext::prepared_item")
    }
    /// Draws `stack` as a still fluid filling `bounds` (`[x0, y0, z0, x1, y1, z1]` in blocks
    /// under `draw`'s pose), as a tank's contents: `minecraft:water`, `minecraft:lava` or any
    /// fluid with a look (`data/<mod>/sunrise/fluid/<name>.json`; a registered fluid without a
    /// look or sprite falls back to `minecraft:missingno`). A stack's components change
    /// nothing: pass `options.argb` to colour a potion-like stack. Each face tiles the fluid's
    /// animated still sprite (`block/water_still`, a look's `sprite`; a look with no sprite draws
    /// its tint flat), one tile per block, cut at whole-block lines and keeping each part's
    /// place in its tile, with no inset of the box. The tint is the fluid's (water `0xFF3F76E4`,
    /// lava white, a look's `tint`, grey `0xFF9A9A9A` when it has none, as `fluid` draws it), not the biome's water
    /// colour, unless `options.argb` replaces it. Sky light stays as `draw.light`; block light
    /// rises to the fluid's `light_level` (15 for lava). No face is shaded by direction. Faces are sorted far to
    /// near, alpha blended, culled from behind and write depth, drawn with the block entities
    /// before the level's translucent terrain. An empty or unresolved stack, or reversed or
    /// non-finite bounds, draws nothing; valid quads are submitted individually up to the
    /// device's vertex and index buffer capacity for the combined entity batch.
    fn fluid_box(&mut self, stack: &FluidStack, bounds: [f32; 6], options: FluidBoxOptions) {
        unimplemented!("BlockEntityDrawContext::draw_fluid_box")
    }
    /// Draws `entity`, a block entity a mod carries (a moving structure's chest, sign, banner or
    /// another mod's machine), under `draw`'s current pose and light, as vanilla's
    /// `BlockEntityRenderDispatcher` extracts and submits one. The pose starts at the block's
    /// lower north-west corner as in a block entity's own submit; the light is `draw.light`, which
    /// the caller samples itself; the overlay is vanilla's `NO_OVERLAY`.
    ///
    /// A block entity of the game goes through the engine's vanilla renderer for its block state
    /// (`entity.state`), read from `entity.fields.nbt`, with no case for any type. A mod's goes
    /// to its registered [`BlockEntityPart`](crate::BlockEntityPart) submit with the entity as a
    /// [`BlockEntityDataView`]. A type with no renderer draws nothing and returns `Ok(())`, as the
    /// dispatcher does when it finds none.
    ///
    /// Nothing ticks. The renderer has no level, so what it reads from one is its default:
    /// a double chest draws as a single closed chest, an enchanting table as its closed book
    /// lying on it, a conduit inactive, an end portal and gateway on every face with no beam,
    /// a banner's flag at rest, a beacon with no beam (its sections are computed in a tick it
    /// never gets), and a spawner or a bed, which have no block entity renderer here, nothing.
    /// Animations driven by tickers (chest lids, the bell's shake, the enchanting book, conduit
    /// rotation) stay at rest: build a new [`SceneBlockEntity`] when the data changes. The dispatcher's `shouldRender` culling is skipped, since the caller's
    /// entity was already culled; the block entity's own position is never used.
    ///
    /// Returns [`BlockEntityDrawRefusal::Nested`], drawing nothing, when called from inside a
    /// mod's submit that `block_entity` itself dispatched. One level is allowed: input a mod
    /// controls must never recurse without end, and a carried block entity is drawn from an
    /// entity renderer, not from inside another block entity's draw.
    fn block_entity(&mut self, entity: &SceneBlockEntity) -> Result<(), BlockEntityDrawRefusal> {
        unimplemented!("BlockEntityDrawContext::block_entity")
    }
}
/// What a [`BlockScreenEffectRenderer`](crate::BlockScreenEffectRenderer) draws with: quads in
/// camera space, in blocks, with `+x` right, `+y` up and the camera at the origin looking down `-z`. It is the
/// space of `ScreenEffectRenderer`'s own quads, whose whole-view quad has corners at `(±1, ±1, -0.5)`,
/// under the first-person hand's projection. Quads draw in the order given, after the level and
/// the hand and before the GUI, alpha-blended with no depth test; give corners in order round the quad.
pub trait CameraDrawContext {
    /// A quad of corners `corners` in solid `argb`. False, with nothing drawn, for a coordinate
    /// that is not finite.
    fn quad(&mut self, corners: [[f32; 3]; 4], argb: u32) -> bool {
        unimplemented!("CameraDrawContext::quad")
    }
    /// A quad of corners `corners` showing sprite `name` of the game's 2D atlas (`block/fire_1`,
    /// `misc/underwater`), with `uv` the texture coordinates of each corner in sprite units (0 to
    /// 1 is the sprite once; beyond that it repeats), every texel multiplied by `argb`. False,
    /// with nothing drawn, when there is no such sprite or a coordinate is not finite.
    fn sprite(&mut self, name: &str, corners: [[f32; 3]; 4], uv: [[f32; 2]; 4], argb: u32) -> bool {
        unimplemented!("CameraDrawContext::sprite")
    }
}
/// Per-view world geometry, cleared even without listeners; backing storage grows dynamically.
pub trait WorldDrawContext {
    /// Submit a retained group's live copies for this view, with per-view culling and the batch
    /// route rules. Each call submits explicitly; repeated submissions of the same group in one
    /// view each draw again. Updates made to the group's records, active length, or group
    /// transform during `Hooks::prepare_render_buffers` take effect on this same frame before
    /// instance uploads. Storage grows dynamically across views.
    /// Shader packs and overlays use the same CPU path as transient model batches.
    /// Retired groups return [`GroupError::Stale`], even if drawn earlier this view.
    fn draw_instance_group(&mut self, group: InstanceGroupRef) -> Result<(), GroupError> {
        unimplemented!("WorldDrawContext::draw_instance_group")
    }
    /// Submit unlit ARGB world lines atomically. Invalid coordinates or widths leave the
    /// submitted geometry unchanged.
    fn lines(&mut self, lines: &[WorldLine]) -> Result<(), DrawError> {
        unimplemented!("WorldDrawContext::world_draw_lines")
    }
    /// Submit unlit, alpha-blended world quads atomically, using the white atlas texel.
    /// Refuses non-finite coordinates ([`DrawError::NonFinite`]); quad storage grows
    /// dynamically into the GPU buffers with no fixed per-view cap.
    fn quads(&mut self, quads: &[WorldQuad]) -> Result<(), DrawError> {
        unimplemented!("WorldDrawContext::world_draw_quads")
    }
    /// Submit world quads drawn by the mod pipeline `pipeline`: shorthand over
    /// [`Self::draw_vertices`] for pipelines using the default vertex format
    /// (`VertexFormat::position_tex_color`) and [`PrimitiveTopology::Quads`]. Checks
    /// `RenderPipelineRef` validity and staleness ([`PipelineDrawError::Pipeline`]), refuses
    /// non-`Quads` pipelines
    /// ([`PipelineDrawError::TopologyMismatch`]) and custom vertex-format pipelines
    /// ([`PipelineDrawError::VertexFormatMismatch`]), skips any quad with non-finite coordinates
    /// or UVs individually without rejecting valid quads in the same slice, and lets quad storage
    /// grow dynamically into the GPU buffers. Within a stage, pipeline draws run after plain
    /// quads and lines, in the order submitted.
    fn pipeline_quads(
        &mut self,
        pipeline: RenderPipelineRef,
        quads: &[PipelineQuad],
    ) -> Result<(), PipelineDrawError> {
        unimplemented!("WorldDrawContext::pipeline_quads")
    }
    /// Draw a retained GPU [`MeshRef`] with `pipeline`, recording a POD draw command with zero
    /// per-frame CPU vertex copying. Validates pipeline liveness, mesh ownership/liveness,
    /// per-binding strides against the pipeline's [`VertexFormat`] bindings, and vertex/index/
    /// instance bounds before queueing (never panics, never triggers a GPU validation error).
    fn draw_mesh(
        &mut self,
        pipeline: RenderPipelineRef,
        mesh: MeshRef,
        draw: MeshDraw,
        bindings: &DrawBindings<'_>,
    ) -> Result<(), PipelineDrawError> {
        let _ = (pipeline, mesh, draw, bindings);
        Err(PipelineDrawError::Pipeline(RenderPipelineError::Invalid))
    }
    /// Draw transient per-frame vertex (and optional index) byte slices with `pipeline`,
    /// uploaded into growing per-view GPU byte pages (bounded only by `max_buffer_size`).
    /// Validates pipeline liveness, binding count and stride alignment, and vertex/index/instance
    /// bounds before queueing.
    fn draw_vertices(
        &mut self,
        pipeline: RenderPipelineRef,
        vertices: &[&[u8]],
        indices: Option<(IndexFormat, &[u8])>,
        draw: MeshDraw,
        bindings: &DrawBindings<'_>,
    ) -> Result<(), PipelineDrawError> {
        let _ = (pipeline, vertices, indices, draw, bindings);
        Err(PipelineDrawError::Pipeline(RenderPipelineError::Invalid))
    }
    /// Draw `draw.max_count` commands a compute shader wrote into a storage buffer
    /// (vanilla's `RenderPass.drawIndexedIndirect(commands, drawCount)` and `drawIndirect`), with
    /// no CPU readback, or with a count buffer as many as the GPU wrote, at most `max_count`. The
    /// same draw on every backend and through a translated shader pack. See [`IndirectDraw`] for
    /// the command layout. Everything is checked before anything is recorded (never panics,
    /// never triggers a GPU validation error): pipeline liveness
    /// ([`PipelineDrawError::Pipeline`]), mesh ownership, liveness and strides
    /// ([`PipelineDrawError::Mesh`]), the command and count buffers' handles
    /// ([`PipelineDrawError::Buffer`]) and their use as indirect arguments
    /// ([`PipelineDrawError::InvalidIndirect`]), a count buffer the adapter cannot read
    /// ([`PipelineDrawError::UnsupportedIndirectCount`]), and the bindings. A `max_count` of
    /// zero is a legal no-op.
    fn draw_indirect(&mut self, draw: &IndirectDraw<'_>) -> Result<(), PipelineDrawError> {
        let _ = draw;
        Err(PipelineDrawError::Pipeline(RenderPipelineError::Invalid))
    }
    /// A label at world position `at`, drawn as vanilla draws a name tag
    /// (`SubmitNodeCollection.submitNameTag`): 0.5 blocks above `at`, facing the camera,
    /// 0.025 blocks per font pixel (so it shrinks with distance; scale by distance yourself
    /// for a constant screen size), centred, in `text`'s styles. Without
    /// [`NameTagOptions::see_through`] it is drawn once, as a sneaking name: depth tested and
    /// translucent, the faded text on the backdrop, in [`NameTagOptions::light`] as given. With
    /// it, twice, as an unsneaking name: opaque in the world (sky and block light each at least
    /// 2, or exactly [`NameTagOptions::light`] with [`NameTagOptions::exact_light`]), and
    /// faded on the backdrop over everything, walls included, unlit. Labels draw with the
    /// world's name tags, far to near, whatever the stage.
    /// Refuses a non-finite `at` ([`DrawError::NonFinite`]); on `Err` nothing is drawn.
    /// Only visible text and styles are used, not click, hover or opaque metadata.
    fn name_tag(
        &mut self,
        at: [f64; 3],
        text: &Text,
        options: NameTagOptions,
    ) -> Result<(), DrawError> {
        unimplemented!("WorldDrawContext::name_tag")
    }
    /// Draw every valid visible copy in `instances` of the resolved `model`.
    ///
    /// One batch per model. The engine uploads only the instance records on the opaque
    /// and cutout route, and bakes the same prepared geometry on the CPU when a shader
    /// pack, a copy with an [`Overlay`], a [`ModelInstance::color`] other than white, a
    /// [`ModelInstance::hidden_faces`] or another route cannot take instances. A
    /// `shaders/` pack shades those copies as block entities, with the program that shades
    /// [`BlockEntityDrawContext::model`], except the translucent copies below and copies of a
    /// block state model, which take the terrain model materials. A copy with a colour alpha below `0xFF`, or of a model
    /// with a translucent face, is drawn in the sorted translucent material vanilla uses for a
    /// translucent block outside the terrain (`translucent_moving_block`: alpha blended, depth
    /// tested, depth written, back faces culled), their quads sorted far to near each frame as
    /// vanilla's `sortOnUpload` does. The special renderer's part of a block state model
    /// (`RenderAssets::block_state_model`) follows its copy's faces in the entity materials,
    /// multiplied by [`ModelInstance::color`], and blends as `entity_translucent` when that
    /// colour's alpha is below `0xFF`; the end portal's and gateway's starfield ignores the
    /// colour, as vanilla's `end_portal` render type does. Quads of that part whose pose throws
    /// them past the finite range are skipped individually.
    /// Tint is `0xAARRGGBB` and light is [`LightCoords`]: both stay in
    /// vanilla's space, and the engine converts them to the frame. A tint's alpha
    /// multiplies only faces that have a tint index, the same as vanilla's model
    /// renderer; [`ModelInstance::color`] multiplies every face. Each copy is culled against
    /// the view frustum first; individual visible copies with non-finite coordinates or
    /// transforms, or light channels outside `0..=15`, are skipped without rolling back
    /// valid copies in the same slice. Storage grows dynamically (falling back to CPU baking
    /// beyond GPU buffer room). Stale or unknown models return [`InstanceError::Model`].
    fn draw_model_instances(
        &mut self,
        model: ModelRef,
        instances: &[ModelInstance],
    ) -> Result<(), InstanceError> {
        let _ = (model, instances);
        unimplemented!("WorldDrawContext::draw_model_instances")
    }
    /// Run `draw` with `modifier` pushed onto the active pipeline modifier stack (NeoForge's
    /// `RenderSystem.renderWithPipelineModifier`). Applies to `lines`, `quads`, `pipeline_quads`,
    /// and `draw_model_instances` drawn inside `draw` (not `name_tag` labels). Only the
    /// modifier's `depth` applies to the built-in `lines`, `quads` and `draw_model_instances`;
    /// the whole `PipelineModifier` applies to a registered pipeline drawn with `pipeline_quads`.
    ///
    /// Scopes nest: the most recently pushed modifier applies first, then older ones, so for a
    /// field set by two modifiers the outermost wins (`PipelineModifierStack.apply`). The
    /// modifier is popped automatically when `draw` returns or unwinds.
    fn with_pipeline_modifier(
        &mut self,
        modifier: PipelineModifierRef,
        draw: &mut dyn FnMut(&mut dyn WorldDrawContext),
    ) {
        let _ = (modifier, draw);
    }
    /// Supply this frame's value for a mod-declared shader-pack uniform ([`PackUniformDecl`]).
    /// Returns `true` when `name` matches a registered mod uniform with the matching
    /// [`PackUniformType`], and `false` otherwise (never panics). When the declaration specifies
    /// a `previous_name`, the engine automatically tracks the prior frame's value under it.
    fn set_pack_uniform(&mut self, name: &str, value: PackUniformValue) -> bool {
        let _ = (name, value);
        false
    }
    /// Whether the active Iris/OptiFine shader pack currently routes `pipeline` through one of
    /// its translated programs ([`PackOverrideState::Bound`]), fell back to the mod's own shader
    /// ([`PackOverrideState::UnboundPack`]), or no shader pack is active
    /// ([`PackOverrideState::NoPack`]).
    fn pack_override_state(
        &self,
        pipeline: RenderPipelineRef,
    ) -> Result<PackOverrideState, RenderPipelineError> {
        let _ = pipeline;
        Ok(PackOverrideState::NoPack)
    }
}
/// CPU-side submission of batches for the current world view. The renderer uploads them after
/// this hook, before it opens any pass that may draw them.
pub trait PrepareRenderBuffersContext {
    /// Prepare an atomic batch of unlit ARGB world lines for `stage`.
    fn lines(
        &mut self,
        stage: RenderLevelStage,
        lines: &[WorldLine],
    ) -> Result<PreparedWorldDraw, DrawError>;

    /// Prepare an atomic batch of unlit, alpha-blended world quads for `stage`.
    fn quads(
        &mut self,
        stage: RenderLevelStage,
        quads: &[WorldQuad],
    ) -> Result<PreparedWorldDraw, DrawError>;

    /// Prepare a batch of mod-pipeline quads for `stage` (standard pass stages or
    /// [`RenderLevelStage::OitTranslucent`]). Refuses pipelines whose effective
    /// [`PrimitiveTopology`] is not [`PrimitiveTopology::Quads`]
    /// ([`PipelineDrawError::TopologyMismatch`]). At `OitTranslucent`, a pipeline without
    /// [`RenderPipeline::order_independent`] or with [`PipelineBlend::Opaque`] is refused with
    /// [`PipelineDrawError::Pipeline`] (`RenderPipelineError::Invalid`), logged once per mod and
    /// pipeline. Individual quads with non-finite coordinates or UVs are skipped.
    fn pipeline_quads(
        &mut self,
        stage: RenderLevelStage,
        pipeline: RenderPipelineRef,
        quads: &[PipelineQuad],
    ) -> Result<PreparedWorldDraw, PipelineDrawError> {
        let _ = (stage, pipeline, quads);
        Err(PipelineDrawError::Pipeline(RenderPipelineError::Invalid))
    }

    /// Prepare a retained GPU [`MeshRef`] draw for `stage` (standard pass stages or
    /// [`RenderLevelStage::OitTranslucent`]).
    fn draw_mesh(
        &mut self,
        stage: RenderLevelStage,
        pipeline: RenderPipelineRef,
        mesh: MeshRef,
        draw: MeshDraw,
        bindings: &DrawBindings<'_>,
    ) -> Result<PreparedWorldDraw, PipelineDrawError> {
        let _ = (stage, pipeline, mesh, draw, bindings);
        Err(PipelineDrawError::Pipeline(RenderPipelineError::Invalid))
    }

    /// Prepare a transient per-frame vertex/index draw for `stage` (standard pass stages or
    /// [`RenderLevelStage::OitTranslucent`]).
    fn draw_vertices(
        &mut self,
        stage: RenderLevelStage,
        pipeline: RenderPipelineRef,
        vertices: &[&[u8]],
        indices: Option<(IndexFormat, &[u8])>,
        draw: MeshDraw,
        bindings: &DrawBindings<'_>,
    ) -> Result<PreparedWorldDraw, PipelineDrawError> {
        let _ = (stage, pipeline, vertices, indices, draw, bindings);
        Err(PipelineDrawError::Pipeline(RenderPipelineError::Invalid))
    }

    /// Prepare a GPU-driven [`IndirectDraw`] for `stage` (standard pass stages or
    /// [`RenderLevelStage::OitTranslucent`]), checked as
    /// [`WorldDrawContext::draw_indirect`] checks it.
    fn draw_indirect(
        &mut self,
        stage: RenderLevelStage,
        draw: &IndirectDraw<'_>,
    ) -> Result<PreparedWorldDraw, PipelineDrawError> {
        let _ = (stage, draw);
        Err(PipelineDrawError::Pipeline(RenderPipelineError::Invalid))
    }
}

/// Draw batches prepared for the active standard render stage, in call order.
pub trait WorldPassDrawContext {
    /// Draw a batch returned by `PrepareRenderBuffersContext`. A batch for another view or
    /// stage is refused, leaving the current pass unchanged.
    fn draw(&mut self, batch: PreparedWorldDraw) -> Result<(), PreparedWorldDrawError>;
}

/// Draw batches prepared for the active order-independent transparency (OIT) sub-pass.
pub trait WorldOitPassDrawContext {
    /// Which OIT sub-pass is currently active (`DepthBounds`, `Transmittance`, or `Accumulate`).
    fn oit_stage(&self) -> OitStage;

    /// Draw a lines or quads batch prepared for this view and sub-pass, in call order.
    fn draw(&mut self, batch: PreparedWorldDraw) -> Result<(), PreparedWorldDrawError>;
}
/// Stage capability passed to [`Hooks::render_level_stage`](crate::Hooks::render_level_stage).
/// Distinguishes no-pass stages (`AfterSky`, `AfterLevel`), standard world render passes, and
/// OIT translucent sub-passes.
pub trait RenderLevelStageContext {
    /// The active pass capability for this stage, or [`StagePass::None`] for no-pass stages.
    fn pass(&mut self) -> StagePass<'_> {
        StagePass::None
    }
    /// Whether this stage provides an active render pass.
    fn has_render_pass(&self) -> bool {
        false
    }
    /// Borrow the standard world-pass draw context if this is a standard pass stage.
    fn world_pass(&mut self) -> Option<&mut dyn WorldPassDrawContext> {
        match self.pass() {
            StagePass::World(pass) => Some(pass),
            StagePass::None | StagePass::Oit(_, _) => None,
        }
    }
    /// Borrow the active OIT sub-pass stage and draw context if this is an OIT stage.
    fn oit_pass(&mut self) -> Option<(OitStage, &mut dyn WorldOitPassDrawContext)> {
        match self.pass() {
            StagePass::Oit(stage, pass) => Some((stage, pass)),
            StagePass::None | StagePass::World(_) => None,
        }
    }
}

/// Draw recorder handed to a pass callback inside [`FrameGraphContext::add_pass`].
///
/// Draws are recorded as data during [`crate::Hooks::frame_graph_setup`] and executed when the
/// pass runs in the compiled frame graph. Self-sampling the pass's own target returns
/// [`PipelineDrawError::TargetReadWriteHazard`].
pub trait OffscreenDrawContext {
    /// Record a retained GPU [`MeshRef`] draw into this offscreen pass.
    fn draw_mesh(
        &mut self,
        pipeline: RenderPipelineRef,
        mesh: MeshRef,
        draw: MeshDraw,
        bindings: &DrawBindings<'_>,
    ) -> Result<(), PipelineDrawError> {
        let _ = (pipeline, mesh, draw, bindings);
        Err(PipelineDrawError::Pipeline(RenderPipelineError::Invalid))
    }

    /// Record a transient per-frame vertex/index draw into this offscreen pass.
    fn draw_vertices(
        &mut self,
        pipeline: RenderPipelineRef,
        vertices: &[&[u8]],
        indices: Option<(IndexFormat, &[u8])>,
        draw: MeshDraw,
        bindings: &DrawBindings<'_>,
    ) -> Result<(), PipelineDrawError> {
        let _ = (pipeline, vertices, indices, draw, bindings);
        Err(PipelineDrawError::Pipeline(RenderPipelineError::Invalid))
    }

    /// Record a GPU-driven [`IndirectDraw`] into this offscreen pass, checked as
    /// [`WorldDrawContext::draw_indirect`] checks it. A compute node earlier in the graph can
    /// write the commands this pass reads.
    fn draw_indirect(&mut self, draw: &IndirectDraw<'_>) -> Result<(), PipelineDrawError> {
        let _ = draw;
        Err(PipelineDrawError::Pipeline(RenderPipelineError::Invalid))
    }

    /// Shorthand for drawing [`PipelineQuad`]s into this offscreen pass with a quad pipeline of the
    /// default vertex format (`VertexFormat::position_tex_color`).
    fn pipeline_quads(
        &mut self,
        pipeline: RenderPipelineRef,
        quads: &[PipelineQuad],
    ) -> Result<(), PipelineDrawError> {
        let _ = (pipeline, quads);
        Err(PipelineDrawError::Pipeline(RenderPipelineError::Invalid))
    }
}

/// Per-frame graph setup surface passed to
/// [`crate::Hooks::frame_graph_setup`] (NeoForge 26.3's `FrameGraphSetupEvent`, vanilla
/// `FrameGraphBuilder`).
pub trait FrameGraphContext {
    /// Allocate a transient per-frame offscreen render target (`FrameGraphBuilder.createInternal`).
    ///
    /// Pooled across frames and released after the last pass in the compiled graph that reads it.
    /// Passes that write only to transient targets with no downstream reader reaching an external
    /// target (`FrameTarget::Mod` or a built-in `LevelTargetBundle` target) are culled unless
    /// `disable_culling` is set.
    fn create_transient(
        &mut self,
        desc: &RenderTargetDescriptor,
    ) -> Result<TransientTargetId, RenderTargetError> {
        let _ = desc;
        Err(RenderTargetError::Unavailable)
    }

    /// Add a data-driven offscreen render pass to this frame's graph
    /// (`FrameGraphBuilder.addPass`).
    ///
    /// `record` records draw commands synchronously during setup. Kept passes are executed in
    /// deterministic topological order (registration order breaks ties); cycles are refused with
    /// [`FrameGraphError::Cycle`] without panicking or breaking other passes in the frame.
    fn add_pass(
        &mut self,
        desc: FramePassDescriptor,
        record: &mut dyn FnMut(&mut dyn OffscreenDrawContext),
    ) -> Result<FramePassId, FrameGraphError> {
        let _ = (desc, record);
        Err(FrameGraphError::Unavailable)
    }

    /// Add a compute node to this frame's graph: `record` issues dispatches synchronously during
    /// setup, and the graph runs them outside any render pass at the node's place in the
    /// topological order. Never culled. A dispatch the device cannot run is refused by
    /// [`ComputeDispatchContext`] with an `Err`, never a panic.
    fn add_compute_pass(
        &mut self,
        desc: ComputePassDescriptor,
        record: &mut dyn FnMut(&mut dyn ComputeDispatchContext),
    ) -> Result<FramePassId, FrameGraphError> {
        let _ = (desc, record);
        Err(FrameGraphError::Unavailable)
    }
}
