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
    /// registered (`register_layer_definitions`), or vanilla's, by the name
    /// `entity-models.json` gives it (`minecraft:zombie#main`,
    /// `minecraft:player#main`). The tree is the caller's own copy: bake it
    /// once, as a renderer does, and set its parts' poses each frame. None
    /// for a layer neither has. A vanilla layer no mob has (a player's, a
    /// chest's) is read from the resources each call, so slow. API
    /// @API:model_layers@.
    fn bake_layer(&self, layer: &str) -> Option<ModelPart>;
}
/// What the HUD, screen and tooltip hooks draw with, in GUI pixels: one per hook call.
pub trait DrawContext {
    /// The GUI's size in GUI pixels, width then height.
    fn size(&self) -> [f32; 2] {
        unimplemented!("DrawContext::size")
    }
    /// Item `item`'s icon drawn with model `model` (`minecraft:paper` as
    /// `mymod:wheel_icon`, an `item_model` of a content mod's), as `native_item`
    /// draws one; false for an unknown item.
    fn item_model(&mut self, item: ItemKey, model: &str, at: [f32; 2], size: f32) -> bool {
        unimplemented!("DrawContext::item_model")
    }
    /// A quad of corners `points` (in order round it) in GUI pixels, `argb`:
    /// a wedge, a slanted bar, what `native_fill`'s rectangles can't draw.
    fn quad(&mut self, points: [[f32; 2]; 4], argb: u32) {
        unimplemented!("DrawContext::quad")
    }
    /// A rectangle in GUI pixels, `argb` (0x78000000 is the extras' box).
    fn fill(&mut self, rect: [f32; 4], argb: u32) {
        unimplemented!("DrawContext::fill")
    }
    /// `GuiGraphics.fillGradient`: rectangle `rect` (`[x, y, w, h]` in GUI pixels, as
    /// [`fill`](Self::fill) takes it) blending from ARGB `top` at `y` to `bottom` at `y + h`.
    /// It follows the current pose and clip ([`push_clip`](Self::push_clip)) as `fill` does.
    /// Zero area, inversion on exactly one axis, or a non-finite coordinate draws nothing.
    /// With both `w` and `h` negative, bounds are sorted and colours flipped, keeping
    /// `top` at `y`, now the lower edge.
    fn fill_gradient(&mut self, rect: [f32; 4], top: u32, bottom: u32) {
        unimplemented!("DrawContext::fill_gradient")
    }
    /// Part `src` (`[x, y, w, h]` in the sprite's own pixels) of sprite
    /// `name` (a GUI atlas sprite by its id, `container/slot` or a mod's `mymod:panel`, another
    /// texture the atlas holds such as vanilla's `gui/...` files, or
    /// a client mod's dynamic texture, `ClientRenderClientHost::create_dynamic_texture`),
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
    /// A line of text at `(x, y)` in GUI pixels, in `argb` as vanilla's `GuiGraphics` takes it
    /// (`0xFFFFFFFF` is opaque white). Fully transparent text draws nothing, as vanilla's.
    fn text(&mut self, text: &str, at: [f32; 2], argb: u32, shadow: bool) {
        unimplemented!("DrawContext::text")
    }
    /// `text` scaled by `scale` about `at`, its top-left corner: what a GUI pose of translate,
    /// scale and translate back around `text` draws (`PoseStack.scale` in vanilla's HUD), without
    /// a `gui_paint` buffer. `0.5` is the half-size line under a widget's label.
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
    /// Paint GUI extraction calls through this window's atlas, including player faces, under
    /// their poses: every op, an `Item` as vanilla's `item` then `itemDecorations` (the stack of
    /// the local player's hotbar slot, scaled about its pivot, then its decorations unscaled) and
    /// a `Blit` of every [`Blend`](crate::client::hud_layout::Blend). An op it cannot draw (invalid
    /// text metadata, an unknown slot, texture or sprite) refuses the whole
    /// buffer with a [`GuiPaintError`] naming the op, and nothing of it is drawn.
    fn gui_paint(&mut self, draws: &[GuiDraw]) -> Result<(), GuiPaintError> {
        unimplemented!("DrawContext::gui_paint")
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
    /// The width of `text` in GUI pixels, as `native_text` draws it on `draw` (`Font.width`).
    fn text_width(&self, text: &str) -> f32 {
        unimplemented!("DrawContext::text_width")
    }
    /// Lays `text` out as the lines a label shows, wrapped to `max_width` GUI pixels at `scale` and
    /// placed by `align`: measurement and layout only, nothing is drawn, and no widget, theme or
    /// state is involved. Draw each line where it says, and the label is the mod's own.
    ///
    /// The layout is measured unscaled and answered scaled: the text wraps at `max_width / scale`
    /// GUI pixels, and every dimension and position it answers — each line's `at` and `width`, the
    /// block's `width` and `height` — is in scaled GUI pixels. `scale` of one is the text as it
    /// draws at its own size; `scale` of two is the label at twice the size, in a box of the same
    /// `max_width`. Draw a line within [`GuiDraw::Push`](crate::GuiDraw::Push) of
    /// [`PoseOp::Scale`](crate::client::hud_layout::PoseOp::Scale) at its `at` divided by `scale`
    /// (the answer's positions are already scaled, and the pose scales them again), and its
    /// glyphs cover the scaled box it measured.
    ///
    /// Wrapping is vanilla's (`Font.split`, as [`crate::ClientTextClientHost::text_split`] wraps):
    /// a line breaks at a space that fits, a single word wider than the box is broken at a
    /// character rather than dropped, and an explicit `\n` starts a new line, so a trailing `\n`
    /// leaves an empty last line. A width below one pixel — `max_width` itself, or `max_width /
    /// scale` — still wraps, at one pixel, as `Font.split` does: too narrow a box gives one
    /// character a line rather than none, and a line is still wider than `max_width` when one
    /// character alone is.
    ///
    /// Every style and colour inside the component is kept in each line's `Text`, and each line's
    /// `width` is what `Font.width` answers for it, styles included. The lines carry no `§` codes:
    /// the split keeps them, and each line's codes are then applied to the characters that follow
    /// them and dropped, so a line drawn alone draws exactly what it measured. A `§r` goes back to
    /// the style its run carried, so right after a break it restores the style the tail run
    /// carried out of the break (a bold run stays bold), not no style.
    ///
    /// One line is `Font.lineHeight` times `scale` high, and the last line's `at[1]` plus one line
    /// height is exactly [`TextLines::height`]: a scroll container sizes its content from it. A
    /// centred line sits at `wrapWidth / 2 - floor(lineWidth / 2)`, scaled afterwards, both widths
    /// unscaled, as vanilla's `TextAlignment.CENTER.calculateLeft` floors the half line width.
    /// Empty text has no line at all, and then `width` and `height` are zero.
    ///
    /// Draw each line through [`gui_paint`](Self::gui_paint)'s
    /// [`GuiDraw::Text`](crate::GuiDraw::Text): [`text`](Self::text) draws plain characters, so a
    /// styled label is `GuiDraw::Text`'s.
    ///
    /// Refuses, answering no line at all (a [`TextLinesRefusal`], never a partial layout):
    /// a `max_width` that is not finite or that dividing by `scale` overflows, a `scale` that is
    /// not finite or is zero or less, text that wraps into more than 512 lines, and lines holding
    /// more than 8192 characters together. Component depth is not a layout limit.
    fn text_lines(
        &self,
        text: &Text,
        max_width: f32,
        align: TextAlign,
        scale: f32,
    ) -> Result<TextLines, TextLinesRefusal> {
        unimplemented!("DrawContext::text_lines")
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
    /// `GuiGraphics.enableScissor`: what this hook call draws from now on, until the matching
    /// [`pop_clip`](Self::pop_clip), is cut to `rect` (`[x, y, w, h]` in GUI pixels, as
    /// [`fill`](Self::fill) takes it), the visible part of a sprite, glyph or item kept.
    /// Clips nest: an inner one is cut to the one around it, so it never shows more than that
    /// does. A negative width or height clips everything. What a scrolling list, a scrolling
    /// label or a viewport into a larger page draws through; a scene from
    /// [`draw_scene`](Self::draw_scene) keeps its own box. A clip still open when the hook call
    /// returns is closed then, so it also cuts what later listeners of the same call draw:
    /// close what you open. Refuses a rectangle that is not finite and a 65th open clip,
    /// leaving the clips as they were. A client's call; nothing is clipped on a server.
    fn push_clip(&mut self, rect: [f32; 4]) -> Result<(), GuiClipRefusal> {
        unimplemented!("DrawContext::push_clip")
    }
    /// `GuiGraphics.disableScissor`: closes the clip [`push_clip`](Self::push_clip) opened
    /// last, so what is drawn next is cut only by the clips around it. Refuses with
    /// [`GuiClipRefusal::NoneOpen`] when this hook call has none open.
    fn pop_clip(&mut self) -> Result<(), GuiClipRefusal> {
        unimplemented!("DrawContext::pop_clip")
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
    /// with [`crate::MenuScreen::with_draw`] or `gui_render_post`.
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
    ///   is no z input. Both are cut by the clips opened with [`DrawContext::push_clip`], as
    ///   `fill` is.
    ///
    /// A client's call, available with [`crate::MenuScreen::with_draw`] or
    /// `gui_render_post`. A `minecraft:air` stack returns `Ok(())` without using the view or
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
    /// Draws a button and registers it for narration and controller focus like `AbstractButton`.
    /// `rect` is `[x, y, w, h]` in GUI pixels. The label is centred, scrolls when it is too wide,
    /// and is grey when inactive. [`crate::client::widgets::Widgets`] uses this to draw its buttons.
    ///
    /// `None` uses the vanilla nine-slice for the button's state. With [`ButtonSprites`] the
    /// full custom sprite stretches into `rect`, and the label moves by `label_offset`. Inactive
    /// takes precedence over highlighted. An omitted or unknown state id falls back to the custom
    /// normal sprite, then to the vanilla nine-slice for the current state.
    fn widget_button(
        &mut self,
        rect: [f32; 4],
        label: &Text,
        active: bool,
        highlighted: bool,
        sprites: Option<&ButtonSprites>,
    ) -> Result<(), TextFormError> {
        unimplemented!("DrawContext::widget_button")
    }
    /// A text field as the game's screens draw one (`EditBox`): its text from the first
    /// character `field.scroll` shows, scrolled on to keep the cursor in view, the selection
    /// highlighted, the caret, the suggestion, the `widget/text_field` sprite when bordered, and
    /// a record for the narrator and a controller's focus. Answers the scroll it drew with and
    /// the x of each shown character boundary, for the next press. What
    /// [`crate::client::widgets::Widgets`] draws its fields with.
    fn widget_text_field(&mut self, field: &TextFieldView) -> TextFieldLayout {
        unimplemented!("DrawContext::widget_text_field")
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
    /// Draws a quad with `corners` in blocks under `draw`'s pose, in a model
    /// face's order (top left, bottom left, bottom right, top right as seen
    /// from its front), textured with the `uv` rectangle (u0, v0, u1, v1 in
    /// texels of 16 per side) of `sprite` (`mymod:entity/screen`,
    /// `mymod:block/casing`, `block/stone`) times the 0xRRGGBB `tint`
    /// (0xFFFFFF for none). False for an unknown sprite.
    fn quad(&mut self, sprite: &str, corners: [[f32; 3]; 4], uv: [f32; 4], tint: u32) -> bool {
        unimplemented!("BlockEntityDrawContext::draw_quad")
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
    /// Draws block `block`'s model (`mymod:cog`, any block with a model) in
    /// the state with `properties` (`[("axis", "x")]`, the rest default)
    /// under `draw`'s pose, as vanilla's `submitBlockModel` draws a block in
    /// an entity's place: a block's baked model, with its default tints. A
    /// moving part that is no block is a standalone model
    /// (`native_draw_model`). False for an unknown block or property.
    fn block(&mut self, block: BlockKey, properties: &[(&str, &str)]) -> bool {
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
    /// shafts and casings draw with. A refusal draws nothing.
    fn terrain_model(
        &mut self,
        model: ModelRef,
        options: TerrainModelOptions<'_>,
    ) -> Result<(), TerrainModelError> {
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
    /// Draws `stack` as a still fluid filling `bounds` (`[x0, y0, z0, x1, y1, z1]` in blocks
    /// under `draw`'s pose), as a tank's contents: `minecraft:water`, `minecraft:lava` or any
    /// fluid with a look (`data/<mod>/sunrise/fluid/<name>.json`). A stack's components change
    /// nothing: pass `options.argb` to colour a potion-like stack. Each face tiles the fluid's
    /// animated still sprite (`block/water_still`, a look's `sprite`; a look with no sprite draws
    /// its tint flat), one tile per block, cut at whole-block lines and keeping each part's
    /// place in its tile, with no inset of the box. The tint is the fluid's (water `0xFF3F76E4`,
    /// lava white, a look's `tint`, grey `0xFF9A9A9A` when it has none, as `fluid` draws it), not the biome's water
    /// colour, unless `options.argb` replaces it. Sky light stays as `draw.light`; block light
    /// rises to the fluid's `light_level` (15 for lava). No face is shaded by direction. Faces are sorted far to
    /// near, alpha blended, culled from behind and write depth, drawn with the block entities
    /// before the level's translucent terrain. A stack of 0 draws nothing and is `Ok` without
    /// checking `bounds`; a refusal draws nothing.
    fn fluid_box(
        &mut self,
        stack: &FluidStack,
        bounds: [f32; 6],
        options: FluidBoxOptions,
    ) -> Result<(), FluidBoxError> {
        unimplemented!("BlockEntityDrawContext::draw_fluid_box")
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
/// Per-view untextured geometry, cleared even without listeners; `usage` reports fixed setup-time limits shared across listeners and depth modes.
pub trait WorldDrawContext {
    /// Draw a retained group once per view, with per-view culling and the batch route rules.
    /// Repeated submissions in one view are ignored. Only visible copies consume its budget.
    /// The shared limit is 16,384 visible copies and 16,384 groups per view. Refusal is atomic.
    /// Shader packs and overlays use the same CPU path as transient model batches.
    /// Retired or foreign groups return [`GroupError::Stale`], even if drawn earlier this view.
    fn draw_instance_group(&mut self, group: InstanceGroupRef) -> Result<(), GroupError> {
        unimplemented!("WorldDrawContext::draw_instance_group")
    }
    /// Submit unlit ARGB world lines atomically. Invalid coordinates or widths,
    /// or a batch exceeding the view's fixed capacity, leave the batch unchanged.
    fn lines(&mut self, lines: &[WorldLine]) -> Result<(), DrawError> {
        unimplemented!("WorldDrawContext::world_draw_lines")
    }
    /// Submit unlit, alpha-blended world quads atomically, using the white atlas texel.
    fn quads(&mut self, quads: &[WorldQuad]) -> Result<(), DrawError> {
        unimplemented!("WorldDrawContext::world_draw_quads")
    }
    /// Submit world quads drawn by the mod pipeline `pipeline`, atomically: its shader gets each
    /// quad's texture coordinate and colour, and its blend and depth modes apply. The quads
    /// count against the same per-view capacity as [`quads`](Self::quads), and within a stage
    /// they draw after its plain quads and lines, in the order submitted. On `Err`, nothing
    /// from this call is drawn.
    fn pipeline_quads(
        &mut self,
        pipeline: RenderPipelineRef,
        quads: &[PipelineQuad],
    ) -> Result<(), PipelineDrawError> {
        unimplemented!("WorldDrawContext::pipeline_quads")
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
    /// world's name tags, far to near, whatever the stage. Each glyph and backdrop quad, both
    /// copies counted, takes one of the view's 4,096 quads shared with
    /// [`quads`](Self::quads) ([`usage`](Self::usage)).
    /// Refuses a non-finite `at` ([`DrawError::NonFinite`]), unrepresentable component metadata
    /// ([`DrawError::TextRefused`]),
    /// and a label past the quads left ([`DrawError::CapacityExceeded`]); on `Err` nothing
    /// is drawn.
    fn name_tag(
        &mut self,
        at: [f64; 3],
        text: &Text,
        options: NameTagOptions,
    ) -> Result<(), DrawError> {
        unimplemented!("WorldDrawContext::name_tag")
    }
    /// Queued counts and fixed capacities, shared by both depth modes and all listeners.
    /// Capacity is allocated during setup, never grown during submission.
    fn usage(&self) -> WorldDrawUsage {
        unimplemented!("WorldDrawContext::world_draw_usage")
    }
    /// Draw every copy in `instances` of the resolved `model`.
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
    /// vanilla's `sortOnUpload` does.
    /// Tint is `0xAARRGGBB` and light is [`LightCoords`]: both stay in
    /// vanilla's space, and the engine converts them to the frame. A tint's alpha
    /// multiplies only faces that have a tint index, the same as vanilla's model
    /// renderer; [`ModelInstance::color`] multiplies every face. The view budget is 16,384 copies. Pass at most that many: a longer
    /// list is refused with [`InstanceError::CapacityExceeded`] before any copy is
    /// examined. Within the bound, copies outside the frustum
    /// are omitted and do not count. On `Err`, nothing from this call is drawn.
    fn draw_model_instances(
        &mut self,
        model: ModelRef,
        instances: &[ModelInstance],
    ) -> Result<(), InstanceError> {
        let _ = (model, instances);
        unimplemented!("WorldDrawContext::draw_model_instances")
    }
}
