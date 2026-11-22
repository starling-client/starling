//! The game's side of the `draw` area (`sunrise-mod-api/src/host/draw.rs`).
#[cfg(test)]
use crate::game::api_values::TryIntoApi;
use crate::game::{Game, api_values::TryIntoEngine};
use mod_api::{prelude::*, *};

/// [`DrawContext`] on the game's `crate::hud::HudBatch`: one per hook call.
pub(crate) struct GameDrawContext<'a, 'g> {
    pub(crate) game: &'a Game<'g>,
    pub(crate) surface: &'a mut crate::hud::HudBatch,
    /// The call's open clips (`push_clip`), closed when the call returns.
    clips: crate::game::gui_clips::GuiClips,
}
impl<'a, 'g> GameDrawContext<'a, 'g> {
    /// Lend a fresh clip stack and isolate the drawable tail of one listener.
    pub(crate) fn stage(
        game: &Game<'_>,
        surface: &mut crate::hud::HudBatch,
        listen: &mut dyn FnMut(&mut dyn DrawContext) -> bool,
    ) {
        let counts = (
            surface.vertices.len(),
            surface.pictures.len(),
            surface.blends.len(),
            surface.inline_profiles.len(),
        );
        let last_blend_end = surface.blends.last().map(|(range, _)| range.end);
        let accepted = listen(&mut GameDrawContext::new(game, surface));
        if !accepted {
            surface.vertices.truncate(counts.0);
            surface.pictures.truncate(counts.1);
            surface.blends.truncate(counts.2);
            surface.inline_profiles.truncate(counts.3);
            if let Some((last, end)) = surface.blends.last_mut().zip(last_blend_end) {
                last.0.end = end;
            }
            // Keep glyph uploads: font extraction already cached those atlas locations.
            // They do not draw anything without an accepted vertex referencing them.
        }
    }

    pub(crate) fn new(game: &'a Game<'g>, surface: &'a mut crate::hud::HudBatch) -> Self {
        Self {
            game,
            surface,
            clips: Default::default(),
        }
    }
}
impl Drop for GameDrawContext<'_, '_> {
    fn drop(&mut self) {
        self.clips.close_all(self.surface);
    }
}
impl DrawContext for GameDrawContext<'_, '_> {
    fn size(&self) -> [f32; 2] {
        self.game.native_size(&*self.surface)
    }
    fn item_model(&mut self, item: ItemKey, model: &str, at: [f32; 2], size: f32) -> bool {
        let Some(item) = mod_api::__private::item_key_id(item) else {
            return false;
        };
        self.game
            .native_item_model(&mut *self.surface, item, model, at, size)
    }
    fn quad(&mut self, points: [[f32; 2]; 4], argb: u32) {
        self.game.native_quad(&mut *self.surface, points, argb)
    }
    fn fill(&mut self, rect: [f32; 4], argb: u32) {
        self.game.native_fill(&mut *self.surface, rect, argb)
    }
    fn fill_gradient(&mut self, rect: [f32; 4], top: u32, bottom: u32) {
        self.game
            .native_fill_gradient(&mut *self.surface, rect, top, bottom)
    }
    fn sprite(&mut self, name: &str, src: [f32; 4], dst: [f32; 4], tint: Option<u32>) -> bool {
        self.game
            .native_sprite(&mut *self.surface, name, src, dst, tint)
    }
    fn item(&mut self, item: ItemKey, at: [f32; 2], size: f32) -> bool {
        let Some(item) = mod_api::__private::item_key_id(item) else {
            return false;
        };
        self.game.native_item(&mut *self.surface, item, at, size)
    }
    fn text(&mut self, text: &str, at: [f32; 2], argb: u32, shadow: bool) {
        self.game
            .native_text(&mut *self.surface, text, at, argb, shadow)
    }
    fn text_scaled(&mut self, text: &str, at: [f32; 2], scale: f32, argb: u32, shadow: bool) {
        self.game
            .native_text_scaled(&mut *self.surface, text, at, scale, argb, shadow)
    }
    fn item_stack(&mut self, stack: &ItemStack, at: [f32; 2], size: f32) -> bool {
        let Ok(stack) = crate::game::api_values::stack_value(stack) else {
            return false;
        };
        self.game
            .native_item_stack(&mut *self.surface, &stack, at, size)
    }
    fn gui_paint(&mut self, draws: &[GuiDraw]) -> Result<(), GuiPaintError> {
        let draws = draws.try_into_engine(&())?;
        crate::game::gui_pose::report_refused_poses(&draws);
        self.game.native_gui_paint(&mut *self.surface, &draws)
    }
    fn gui_nine_slice(&mut self, sprite: &str, rect: [f32; 4], color: u32) {
        self.game
            .native_gui_nine_slice(&mut *self.surface, sprite, rect, color)
    }
    fn gui_tooltip(
        &mut self,
        lines: &[Text],
        at: [f32; 2],
    ) -> Result<(), mod_api::text::TextFormError> {
        let lines: Vec<_> = lines
            .iter()
            .map(sunrise_client_lib::api_text::to_engine)
            .collect::<Result<_, _>>()?;
        self.game.native_gui_tooltip(&mut *self.surface, &lines, at);
        Ok(())
    }
    fn text_width(&self, text: &str) -> f32 {
        self.game.native_text_width(&*self.surface, text)
    }
    fn text_lines(
        &self,
        text: &Text,
        max_width: f32,
        align: TextAlign,
        scale: f32,
    ) -> Result<TextLines, TextLinesRefusal> {
        let local = sunrise_client_lib::api_text::to_local_engine(text);
        label_layout_with(
            &self.game.gui.atlas.font,
            local.engine(),
            max_width,
            align,
            scale,
            |piece| Ok(local.text_to_api(piece)),
        )
    }
    fn fluid(&mut self, fluid: &str, at: [f32; 2], size: f32) {
        self.game.native_fluid(&mut *self.surface, fluid, at, size)
    }
    fn draw_scene(
        &mut self,
        rect: [f32; 4],
        scene: &mod_api::Scene,
        view: SceneView,
    ) -> Result<(), mod_api::SceneError> {
        let blocks = crate::game::api_values::aggregates::scene_blocks(&scene.blocks)?;
        self.game
            .native_draw_scene(&mut *self.surface, rect, &blocks, scene, view)
    }
    fn model_3d(
        &mut self,
        subject: mod_api::Gui3dSubject<'_>,
        view: mod_api::Gui3dView,
    ) -> Result<(), mod_api::Gui3dError> {
        self.game.native_model_3d(&mut *self.surface, subject, view)
    }
    fn push_clip(&mut self, rect: [f32; 4]) -> Result<(), GuiClipRefusal> {
        self.clips.push(self.surface, rect)
    }
    fn pop_clip(&mut self) -> Result<(), GuiClipRefusal> {
        self.clips.pop(self.surface)
    }
    fn entity(&mut self, entity: &GuiEntity, view: GuiEntityView) -> Result<(), GuiEntityError> {
        self.game
            .native_draw_entity(&mut *self.surface, entity, view)
    }
    fn draw_menu_background(&mut self, rect: [f32; 4], in_world: bool) {
        self.game
            .native_draw_menu_background(&mut *self.surface, rect, in_world)
    }
    fn tiled_sprite(&mut self, sprite: &str, rect: [f32; 4], size: f32, argb: u32) -> bool {
        self.game
            .native_tiled_sprite(&mut *self.surface, sprite, rect, size, argb)
    }
    fn gui_draw_item_with(&mut self, stack: &ItemStack, at: [f32; 2], text: Option<&str>) {
        let Ok(stack) = crate::game::api_values::stack_value(stack) else {
            return;
        };
        self.game
            .native_gui_draw_item_with(&mut *self.surface, &stack, at, text)
    }
    fn widget_button(
        &mut self,
        rect: [f32; 4],
        label: &Text,
        active: bool,
        highlighted: bool,
        sprites: Option<&ButtonSprites>,
    ) -> Result<(), mod_api::text::TextFormError> {
        let label = sunrise_client_lib::api_text::to_engine_inprocess(label);
        self.game.native_widget_button(
            &mut *self.surface,
            rect,
            &label,
            active,
            highlighted,
            sprites,
        );
        Ok(())
    }
    fn widget_text_field(&mut self, field: &TextFieldView) -> TextFieldLayout {
        self.game
            .native_widget_text_field(&mut *self.surface, field)
    }
}
/// `Font.lineHeight`: nine GUI pixels a line, before `scale`.
const LINE_HEIGHT: f32 = 9.0;
/// The most lines one layout may have; [`DrawContext::text_lines`] refuses more
/// ([`TextLinesRefusal::TooManyLines`]) rather than answer a component of any size. The method's
/// doc names the same number: keep them in step.
const MAX_LINES: usize = 512;
/// The most characters one layout's lines may hold together, before
/// [`TextLinesRefusal::TooManyGlyphs`]. The method's doc names the same number: keep them in step.
const MAX_GLYPHS: usize = 8192;
/// Lays `text` out as a label's lines, as [`DrawContext::text_lines`] documents: the font's own
/// split wraps it at `max_width / scale`, the font measures each line, `align` places them inside
/// that wrapping width, and every dimension and position answered is scaled by `scale`.
#[cfg(test)]
fn label_layout(
    font: &crate::font::Font,
    text: &crate::text::Text,
    max_width: f32,
    align: TextAlign,
    scale: f32,
) -> Result<TextLines, TextLinesRefusal> {
    label_layout_with(font, text, max_width, align, scale, |piece| {
        Ok(piece.try_into_api()?)
    })
}
fn label_layout_with(
    font: &crate::font::Font,
    text: &crate::text::Text,
    max_width: f32,
    align: TextAlign,
    scale: f32,
    mut to_api: impl FnMut(&crate::text::Text) -> Result<Text, TextLinesRefusal>,
) -> Result<TextLines, TextLinesRefusal> {
    // No position and no size to answer: a scale of zero or less, or one that is not a number,
    // refuses rather than answering every line at an undefined place.
    if !scale.is_finite() || scale <= 0.0 {
        return Err(TextLinesRefusal::ScaleInvalid);
    }
    // A width the game cannot measure in refuses too: it is not "no wrapping", which would answer
    // lines of any length in a box of no width.
    if !max_width.is_finite() {
        return Err(TextLinesRefusal::WidthNotFinite);
    }
    // Vanilla's one-pixel minimum (`StringSplitter.splitLines`): a finite box narrower than one
    // pixel wraps at one — and so does one `scale` makes narrower — rather than not wrapping.
    let wrap_width = max_width / scale;
    if !wrap_width.is_finite() {
        return Err(TextLinesRefusal::WidthNotFinite);
    }
    let wrap_width = wrap_width.max(1.0);
    let wrapped = font.split_formatted(text, wrap_width);
    if wrapped.len() > MAX_LINES {
        return Err(TextLinesRefusal::TooManyLines);
    }
    let mut lines = Vec::with_capacity(wrapped.len());
    let mut glyphs = 0usize;
    for (i, piece) in wrapped.into_iter().enumerate() {
        // The split keeps the legacy codes; applying them to each run's own style (a `§r` restores
        // the style the run carried out of the break) lets a line measure and draw the same way.
        let piece = piece.resolve_legacy();
        glyphs += piece.chars().count();
        if glyphs > MAX_GLYPHS {
            return Err(TextLinesRefusal::TooManyGlyphs);
        }
        let width = font.width_text(&piece);
        let x = match align {
            TextAlign::Left => 0.0,
            // `TextAlignment.CENTER.calculateLeft`: the half line width is rounded down.
            TextAlign::Centre => wrap_width / 2.0 - (width / 2.0).floor(),
            TextAlign::Right => wrap_width - width,
        };
        let at = [x * scale, i as f32 * LINE_HEIGHT * scale];
        lines.push(TextLine::new(to_api(&piece)?, at, width * scale));
    }
    let width = lines.iter().map(|line| line.width).fold(0.0, f32::max);
    let height = lines
        .last()
        .map_or(0.0, |line| line.at[1] + LINE_HEIGHT * scale);
    Ok(TextLines::new(lines, width, height))
}
/// [`BlockEntityDrawContext`] on the game's `crate::game::native_block_entity_draw::BlockEntityDraw`.
pub(crate) struct GameBlockEntityDrawContext<'a, 'g> {
    pub(crate) game: &'a Game<'g>,
    pub(crate) surface: &'a mut crate::game::native_block_entity_draw::BlockEntityDraw,
}
impl<'a, 'g> GameBlockEntityDrawContext<'a, 'g> {
    pub(crate) fn new(
        game: &'a Game<'g>,
        surface: &'a mut crate::game::native_block_entity_draw::BlockEntityDraw,
    ) -> Self {
        Self { game, surface }
    }
}
impl BlockEntityDrawContext for GameBlockEntityDrawContext<'_, '_> {
    fn push(&mut self) {
        self.game.native_draw_push(&mut *self.surface)
    }
    fn pop(&mut self) {
        self.game.native_draw_pop(&mut *self.surface)
    }
    fn translate(&mut self, by: [f32; 3]) {
        self.game.native_draw_translate(&mut *self.surface, by)
    }
    fn rotate_about(&mut self, axis: [f32; 3], degrees: f32, pivot: [f32; 3]) {
        self.game
            .native_draw_rotate_about(&mut *self.surface, axis, degrees, pivot)
    }
    fn scale(&mut self, by: [f32; 3]) {
        self.game.native_draw_scale(&mut *self.surface, by)
    }
    fn keep(&mut self) {
        self.game.native_draw_keep(&mut *self.surface)
    }
    fn partial(&self) -> f32 {
        self.game.native_draw_partial(&*self.surface)
    }
    fn game_time(&self) -> i64 {
        self.game.native_draw_game_time(&*self.surface)
    }
    fn animation_time(&self) -> f64 {
        self.game.native_draw_animation_time(&*self.surface)
    }
    fn light(&self) -> u32 {
        self.game.native_draw_light(&*self.surface)
    }
    fn set_light(&mut self, light: u32) {
        self.game.native_draw_set_light(&mut *self.surface, light)
    }
    fn level_light(&mut self) {
        self.game.native_draw_level_light(&mut *self.surface)
    }
    fn quad(&mut self, sprite: &str, corners: [[f32; 3]; 4], uv: [f32; 4], tint: u32) -> bool {
        self.game
            .native_draw_quad(&mut *self.surface, sprite, corners, uv, tint)
    }
    fn cube(
        &mut self,
        from: [f32; 3],
        to: [f32; 3],
        faces: [Option<CubeFace<'_>>; 6],
        tint: u32,
    ) -> bool {
        self.game
            .native_draw_cube(&mut *self.surface, from, to, faces, tint)
    }
    fn block(&mut self, block: BlockKey, properties: &[(&str, &str)]) -> bool {
        let Some(block) = mod_api::__private::block_key_id(block) else {
            return false;
        };
        self.game
            .native_draw_block(&mut *self.surface, block, properties)
    }
    fn item(
        &mut self,
        stack: &ItemStack,
        display: ItemDisplayContext,
        owner_yaw: Option<f32>,
        seed: i32,
    ) -> bool {
        let Ok(stack) = crate::game::api_values::stack_value(stack) else {
            return false;
        };
        let Ok(display) = display.try_into_engine(self.game.items()) else {
            return false;
        };
        self.game
            .native_draw_item(&mut *self.surface, &stack, display, owner_yaw, seed)
    }
    fn spin(&mut self, axis: [f32; 3], pivot: [f32; 3], degrees_per_tick: f32, phase: f32) {
        self.game
            .native_draw_spin(&mut *self.surface, axis, pivot, degrees_per_tick, phase)
    }
    fn still(&mut self) {
        self.game.native_draw_still(&mut *self.surface)
    }
    fn model(&mut self, model: ModelRef, tint: u32) -> Result<(), ModelError> {
        self.game.native_draw_model(&mut *self.surface, model, tint)
    }
    fn text(
        &mut self,
        text: &Text,
        at: [f32; 2],
        color: u32,
        glowing: bool,
        outline: Option<u32>,
    ) -> Result<f32, mod_api::text::TextFormError> {
        let text = sunrise_client_lib::api_text::to_engine(text)?;
        Ok(self
            .game
            .native_draw_text(&mut *self.surface, &text, at, color, glowing, outline))
    }
    fn terrain_model(
        &mut self,
        model: ModelRef,
        options: TerrainModelOptions<'_>,
    ) -> Result<(), TerrainModelError> {
        self.game
            .native_draw_terrain_model(&mut *self.surface, model, options)
    }
    fn text_width(&self, text: &Text) -> f32 {
        let text = sunrise_client_lib::api_text::to_engine_inprocess(text);
        self.game.native_draw_text_width(&*self.surface, &text)
    }
    fn model_part(&mut self, part: &ModelPart, sprite: &str, tint: u32) -> bool {
        self.game
            .native_draw_model_part(&mut *self.surface, part, sprite, tint)
    }
    fn fluid_box(
        &mut self,
        stack: &FluidStack,
        bounds: [f32; 6],
        options: FluidBoxOptions,
    ) -> Result<(), FluidBoxError> {
        self.game
            .native_draw_fluid_box(&mut *self.surface, stack, bounds, options)
    }
}
impl HandDrawContext for GameBlockEntityDrawContext<'_, '_> {
    fn render_arm(&mut self, arm: MainHand) {
        if let Some(normals) = self.surface.normal_pose() {
            self.game.submit_first_person_arm(
                arm == MainHand::Left,
                (self.surface.pose(), normals),
                self.surface.light(),
                self.surface.vertices_mut(),
            );
        }
    }
}

/// [`CameraDrawContext`] on the game's `crate::screen_effects::CameraQuads`.
pub(crate) struct GameCameraDrawContext<'a, 'g> {
    pub(crate) game: &'a Game<'g>,
    pub(crate) surface: &'a mut crate::screen_effects::CameraQuads,
}
impl<'a, 'g> GameCameraDrawContext<'a, 'g> {
    pub(crate) fn new(
        game: &'a Game<'g>,
        surface: &'a mut crate::screen_effects::CameraQuads,
    ) -> Self {
        Self { game, surface }
    }
}
impl CameraDrawContext for GameCameraDrawContext<'_, '_> {
    fn quad(&mut self, corners: [[f32; 3]; 4], argb: u32) -> bool {
        self.game
            .native_camera_quad(&mut *self.surface, corners, argb)
    }
    fn sprite(&mut self, name: &str, corners: [[f32; 3]; 4], uv: [[f32; 2]; 4], argb: u32) -> bool {
        self.game
            .native_camera_sprite(&mut *self.surface, name, corners, uv, argb)
    }
}
/// [`WorldDrawContext`] on the game's `crate::world_geometry::WorldGeometry`.
pub(crate) struct GameWorldDrawContext<'a, 'g> {
    pub(crate) game: &'a Game<'g>,
    pub(crate) surface: &'a mut crate::world_geometry::WorldGeometry,
}
impl<'a, 'g> GameWorldDrawContext<'a, 'g> {
    pub(crate) fn new(
        game: &'a Game<'g>,
        surface: &'a mut crate::world_geometry::WorldGeometry,
    ) -> Self {
        Self { game, surface }
    }
}
impl WorldDrawContext for GameWorldDrawContext<'_, '_> {
    fn draw_instance_group(&mut self, group: InstanceGroupRef) -> Result<(), GroupError> {
        self.game.draw_instance_group(self.surface, group)
    }
    fn lines(&mut self, lines: &[WorldLine]) -> Result<(), DrawError> {
        self.game.native_world_draw_lines(&mut *self.surface, lines)
    }
    fn quads(&mut self, quads: &[WorldQuad]) -> Result<(), DrawError> {
        self.game.native_world_draw_quads(&mut *self.surface, quads)
    }
    fn pipeline_quads(
        &mut self,
        pipeline: RenderPipelineRef,
        quads: &[PipelineQuad],
    ) -> Result<(), PipelineDrawError> {
        self.game
            .native_world_draw_pipeline_quads(&mut *self.surface, pipeline, quads)
    }
    fn name_tag(
        &mut self,
        at: [f64; 3],
        text: &Text,
        options: NameTagOptions,
    ) -> Result<(), DrawError> {
        self.game
            .native_world_name_tag(&mut *self.surface, at, text, options)
    }
    fn usage(&self) -> WorldDrawUsage {
        self.game.native_world_draw_usage(&*self.surface)
    }
    fn draw_model_instances(
        &mut self,
        model: ModelRef,
        instances: &[ModelInstance],
    ) -> Result<(), InstanceError> {
        self.game
            .native_world_draw_model_instances(&mut *self.surface, model, instances)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A font whose printable characters are all `advance` GUI pixels wide: the vanilla provider
    /// list with a `space` provider, so a layout test measures exact advances.
    fn fake_font(advance: f32) -> crate::font::Font {
        let advances: Vec<String> = (32u8..=126)
            .map(char::from)
            // A JSON object key: the quote and the backslash are the two that need escaping.
            .filter(|&c| !matches!(c, '"' | '\\'))
            .map(|c| format!("\"{c}\":{advance}"))
            .collect();
        let json = format!(
            r#"{{"providers":[{{"type":"space","advances":{{{}}}}}]}}"#,
            advances.join(",")
        );
        let source = crate::resource_source::ResourceSource::indexed(BTreeMap::from([(
            "assets/minecraft/font/default.json".into(),
            crate::resource_source::Resource::bytes(json.into_bytes()),
        )]));
        crate::font::Font::metrics(&source)
    }

    /// A line's component as the game draws it.
    fn engine(text: &mod_api::Text) -> crate::text::Text {
        text.try_into_engine(&()).unwrap()
    }

    /// The lines' plain characters, top to bottom.
    fn plain(lines: &TextLines) -> Vec<String> {
        lines
            .lines
            .iter()
            .map(|line| engine(&line.text).plain())
            .collect()
    }

    /// Each character six GUI pixels wide, as the widget tests fake one.
    fn layout(text: &str, max_width: f32, align: TextAlign, scale: f32) -> TextLines {
        label_layout(
            &fake_font(6.0),
            &crate::text::Text::literal(text),
            max_width,
            align,
            scale,
        )
        .unwrap()
    }

    /// The layout `text` gets at `scale`, refusing where it should.
    fn refusal(
        text: &str,
        max_width: f32,
        align: TextAlign,
        scale: f32,
    ) -> Result<TextLines, TextLinesRefusal> {
        label_layout(
            &fake_font(6.0),
            &crate::text::Text::literal(text),
            max_width,
            align,
            scale,
        )
    }

    #[test]
    fn a_label_wraps_at_spaces_and_keeps_its_words() {
        // 48 pixels is eight characters: "a quick" fits, "brown" does not fit after it, and
        // "brown fox" is one character too wide for the second line.
        let lines = layout("a quick brown fox", 48.0, TextAlign::Left, 1.0);
        assert_eq!(plain(&lines), ["a quick", "brown", "fox"]);
        assert_eq!(lines.width, 42.0);
        assert!(
            lines.lines.iter().all(|line| line.width <= 48.0),
            "{:?}",
            lines.lines.iter().map(|l| l.width).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_word_longer_than_the_line_is_broken_at_a_character() {
        let lines = layout("abcdefghij", 24.0, TextAlign::Left, 1.0);
        assert_eq!(plain(&lines), ["abcd", "efgh", "ij"]);
        assert_eq!(plain(&lines).concat(), "abcdefghij");
    }

    #[test]
    fn an_explicit_newline_starts_a_line_and_an_empty_component_has_no_line() {
        assert_eq!(
            plain(&layout("AA\nBB", 100.0, TextAlign::Left, 1.0)),
            ["AA", "BB"]
        );
        let empty = layout("", 100.0, TextAlign::Left, 1.0);
        assert!(empty.lines.is_empty());
        assert_eq!((empty.width, empty.height), (0.0, 0.0));
    }

    #[test]
    fn a_trailing_newline_leaves_empty_last_lines() {
        // One `\n` ends the line it closes and the label still holds the empty line after it;
        // two leave two, the second from the split's own trailing newline.
        let one = layout("AA\n", 100.0, TextAlign::Left, 1.0);
        assert_eq!(plain(&one), ["AA", ""]);
        assert_eq!((one.width, one.height), (12.0, 18.0));
        let two = layout("AA\n\n", 100.0, TextAlign::Left, 1.0);
        assert_eq!(plain(&two), ["AA", "", ""]);
        assert_eq!((two.width, two.height), (12.0, 27.0));
    }

    #[test]
    fn leading_spaces_and_runs_of_spaces_stay_in_the_line() {
        // Nothing trims a label: the spaces at its start are characters like any other.
        let lead = layout("  AA", 100.0, TextAlign::Left, 1.0);
        assert_eq!(plain(&lead), ["  AA"]);
        assert_eq!(lead.width, 24.0);
        // A run between words is kept when it fits, and the space a wrap breaks at is dropped.
        assert_eq!(
            plain(&layout("AA   BB", 100.0, TextAlign::Left, 1.0)),
            ["AA   BB"]
        );
        assert_eq!(
            plain(&layout("  AA BB", 24.0, TextAlign::Left, 1.0)),
            ["  AA", "BB"]
        );
    }

    #[test]
    fn spaces_at_a_break_leave_empty_lines() {
        // A leading space that does not fit is the break: an empty first line, then one character
        // a line. The break at a run of spaces drops only the space it breaks at.
        assert_eq!(
            plain(&layout(" AA", 6.0, TextAlign::Left, 1.0)),
            ["", "A", "A"]
        );
        assert_eq!(
            plain(&layout("AA    BBBB", 24.0, TextAlign::Left, 1.0)),
            ["AA  ", "", "BBBB"]
        );
    }

    #[test]
    fn each_line_is_placed_by_the_alignment() {
        // 24 and 12 pixels wide, in a box of 100: left leaves them at 0, centre at 38 and 44,
        // right at 76 and 88.
        for (align, expected) in [
            (TextAlign::Left, [0.0, 0.0]),
            (TextAlign::Centre, [38.0, 44.0]),
            (TextAlign::Right, [76.0, 88.0]),
        ] {
            let lines = layout("AAAA\nBB", 100.0, align, 1.0);
            let placed: Vec<f32> = lines.lines.iter().map(|line| line.at[0]).collect();
            assert_eq!(placed, expected, "{align:?}");
        }
    }

    #[test]
    fn a_centred_line_rounds_the_half_line_width_down() {
        // A 15-pixel line in a 40-pixel box: `TextAlignment.CENTER.calculateLeft` floors the half
        // line width, so it starts at 13, not at the exact 12.5 an unrounded centre would give.
        let lines = label_layout(
            &fake_font(5.0),
            &crate::text::Text::literal("AAA"),
            40.0,
            TextAlign::Centre,
            1.0,
        )
        .unwrap();
        assert_eq!(lines.lines[0].at[0], 13.0);
    }

    #[test]
    fn the_height_is_the_line_count_times_the_line_height() {
        let lines = layout("A\nB\nC", 100.0, TextAlign::Left, 1.0);
        let down: Vec<f32> = lines.lines.iter().map(|line| line.at[1]).collect();
        assert_eq!(down, [0.0, 9.0, 18.0]);
        assert_eq!(lines.height, 3.0 * 9.0);
        // A viewport sizes itself from the last line rather than the count.
        assert_eq!(lines.height, down.last().unwrap() + 9.0);
    }

    #[test]
    fn styles_survive_wrapping() {
        let mut text = mod_api::Text::literal("AAAA BBBB");
        text.style.bold = Some(true);
        let text: crate::text::Text = text.try_into_engine(&()).unwrap();
        let lines = label_layout(&fake_font(6.0), &text, 30.0, TextAlign::Left, 1.0).unwrap();
        assert_eq!(plain(&lines), ["AAAA", "BBBB"]);
        for line in &lines.lines {
            let line = engine(&line.text);
            assert!(
                line.chars().all(|(c, style)| c == ' ' || style.is_bold()),
                "{:?}",
                line.plain()
            );
        }
    }

    #[test]
    fn laid_out_objects_keep_their_sprites_profiles_and_fallbacks_on_redraw() {
        use mod_api::text::{TextContent, TextObject, TextProfile};
        use sunrise_client::text::InlineObject;

        let atlas = Text::new(TextContent::Object(TextObject::Atlas {
            atlas: "minecraft:gui".into(),
            sprite: "minecraft:heart/full".into(),
            fallback: Some(Box::new(Text::literal("[heart]"))),
        }));
        let mut profile = TextProfile::default();
        profile.name = Some("Alex".into());
        let mut head = Text::new(TextContent::Object(TextObject::Player {
            profile,
            hat: false,
            fallback: Some(Box::new(Text::literal("[Alex]"))),
        }));
        head.style.shadow_color = Some(0xFF00_0011);
        let input = atlas.append(head);
        let font = fake_font(6.0);
        let local = sunrise_client_lib::api_text::to_local_engine(&input);
        let lines = label_layout_with(&font, local.engine(), 150.0, TextAlign::Left, 1.0, |line| {
            Ok(local.text_to_api(line))
        })
        .unwrap();
        let redrawn = engine(&lines.lines[0].text);
        assert_eq!(redrawn.plain(), "[heart][Alex]");
        assert_eq!(font.width_text(&redrawn), lines.lines[0].width);
        let [atlas, head] = redrawn.spans.as_slice() else {
            panic!("expected two object spans")
        };
        assert!(matches!(&atlas.style.inline_object,
            Some(InlineObject::Atlas { atlas, sprite, .. })
            if atlas == "minecraft:gui" && sprite == "minecraft:heart/full"));
        assert!(matches!(&head.style.inline_object,
            Some(InlineObject::Player { profile, hat: false, .. })
            if profile.get("name").and_then(serde_json::Value::as_str) == Some("Alex")));
        assert_eq!(head.style.shadow_color, Some(0xFF00_0011));
    }

    #[test]
    fn layout_does_not_read_style_indices_from_mod_metadata() {
        use mod_api::{Tag, text::TextClick};
        let mut styled = Text::literal("A");
        styled.style.click = Some(TextClick::CopyToClipboard("not for B".into()));
        let mut plain = Text::literal("B");
        plain.metadata = Tag::compound([("__sunrise_local_style_id", Tag::Int(0))]);
        let input = Text::literal("").append(styled).append(plain);
        let local = sunrise_client_lib::api_text::to_local_engine(&input);
        let lines = label_layout_with(
            &fake_font(6.0),
            local.engine(),
            150.0,
            TextAlign::Left,
            1.0,
            |line| Ok(local.text_to_api(line)),
        )
        .unwrap();
        let [styled, plain] = lines.lines[0].text.children.as_slice() else {
            panic!("expected two styles")
        };
        assert_eq!(
            styled.style.click,
            Some(TextClick::CopyToClipboard("not for B".into()))
        );
        assert_eq!(
            plain.content,
            mod_api::text::TextContent::Literal("B".into())
        );
        assert_eq!(plain.style.click, None);
    }

    #[test]
    fn layout_copies_inherited_deep_hover_and_opaque_metadata_without_recursion() {
        use mod_api::{
            Tag,
            text::{TextContent, TextHover},
        };
        const DEPTH: usize = 4096;
        let mut hover = Text::literal("hover leaf");
        let mut metadata = Tag::Int(73);
        for _ in 0..DEPTH {
            let mut parent = Text::literal("");
            parent.style.hover = Some(TextHover::Text(Box::new(hover)));
            hover = parent;
            metadata = Tag::List(vec![metadata]);
        }
        let mut root = Text::literal("");
        root.style.hover = Some(TextHover::Text(Box::new(hover)));
        root.style.metadata = Tag::compound([("deep", metadata), ("override", Tag::Int(1))]);
        let mut child = Text::literal("A");
        child.style.metadata = Tag::compound([("override", Tag::Int(2))]);
        root.children.push(child);
        let local = sunrise_client_lib::api_text::to_local_engine(&root);
        let lines = label_layout_with(
            &fake_font(6.0),
            local.engine(),
            150.0,
            TextAlign::Left,
            1.0,
            |line| Ok(local.text_to_api(line)),
        )
        .unwrap();
        let style = &lines.lines[0].text.style;
        let Some(TextHover::Text(hover)) = &style.hover else {
            panic!("inherited hover")
        };
        let mut hover = hover.as_ref();
        let Tag::Compound(metadata) = &style.metadata else {
            panic!("inherited metadata")
        };
        assert_eq!(
            metadata[&sunrise_core::nbt::JavaString::from("override")],
            Tag::Int(2)
        );
        let mut deep = &metadata[&sunrise_core::nbt::JavaString::from("deep")];
        for _ in 0..DEPTH {
            let Tag::List(list) = deep else {
                panic!("nested metadata")
            };
            deep = &list[0];
        }
        assert_eq!(deep, &Tag::Int(73));
        for _ in 0..DEPTH {
            let Some(TextHover::Text(child)) = &hover.style.hover else {
                panic!("nested hover")
            };
            hover = child;
        }
        assert_eq!(hover.content, TextContent::Literal("hover leaf".into()));
    }

    #[test]
    fn a_reset_after_a_break_restores_the_style_the_tail_carried() {
        // `§lAA §rB` in a 14-pixel box: two bold characters fill the first line, the space is
        // where it breaks, and the second line starts in the bold style the split carried over.
        // Its `§r` goes back to that carried style, so `B` stays bold, and no line keeps a code.
        let lines = layout("§lAA §rB", 14.0, TextAlign::Left, 1.0);
        assert_eq!(plain(&lines), ["AA", "B"]);
        let bold: Vec<bool> = engine(&lines.lines[1].text)
            .chars()
            .map(|(_, style)| style.is_bold())
            .collect();
        assert_eq!(bold, [true], "the reset restores the carried bold style");
        for line in plain(&lines) {
            assert!(!line.contains('§'), "a line carries no code: {line:?}");
        }
    }

    #[test]
    fn scale_wraps_at_the_box_over_it_and_scales_every_answer() {
        // 36 pixels at scale 2 wrap in 24: two characters a line, 12 pixels measured and 24
        // answered, the lines 18 apart and the block 54 high.
        let lines = layout("AAAAAA", 24.0, TextAlign::Left, 2.0);
        assert_eq!(plain(&lines), ["AA", "AA", "AA"]);
        let widths: Vec<f32> = lines.lines.iter().map(|line| line.width).collect();
        assert_eq!(widths, [24.0, 24.0, 24.0]);
        let down: Vec<f32> = lines.lines.iter().map(|line| line.at[1]).collect();
        assert_eq!(down, [0.0, 18.0, 36.0]);
        assert_eq!((lines.width, lines.height), (24.0, 54.0));
        // Positions scale too: a 6-pixel line centred in 12 starts at 3, answered at 6.
        let one = layout("A", 24.0, TextAlign::Centre, 2.0);
        assert_eq!(one.lines[0].at, [6.0, 0.0]);
        assert_eq!(one.lines[0].width, 12.0);
    }

    #[test]
    fn a_width_below_one_still_wraps() {
        // `Font.split`'s one-pixel minimum: a box of no width, or one narrower than a pixel, gives
        // one character a line rather than an unwrapped line of any length.
        for max_width in [0.0, 0.5, -4.0] {
            let lines = layout("AAA", max_width, TextAlign::Left, 1.0);
            assert_eq!(plain(&lines), ["A", "A", "A"], "{max_width}");
            assert_eq!(lines.height, 27.0, "{max_width}");
        }
    }

    #[test]
    fn a_glyph_wider_than_the_box_gets_a_line_of_its_own() {
        // A 20-pixel character in a 6-pixel box: neither fits, and neither is dropped.
        let lines = label_layout(
            &fake_font(20.0),
            &crate::text::Text::literal("AB"),
            6.0,
            TextAlign::Left,
            1.0,
        )
        .unwrap();
        assert_eq!(plain(&lines), ["A", "B"]);
        assert_eq!(lines.lines[0].width, 20.0);
        assert_eq!(lines.width, 20.0);
    }

    #[test]
    fn a_layout_refuses_a_width_or_a_scale_it_cannot_place() {
        for max_width in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(
                refusal("AA", max_width, TextAlign::Left, 1.0).unwrap_err(),
                TextLinesRefusal::WidthNotFinite,
                "{max_width}"
            );
        }
        for scale in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
            assert_eq!(
                refusal("AA", 100.0, TextAlign::Left, scale).unwrap_err(),
                TextLinesRefusal::ScaleInvalid,
                "{scale}"
            );
        }
        // A finite width `scale` makes overflow is no box at all, not a box of no width.
        assert_eq!(
            refusal("AA", f32::MAX, TextAlign::Left, f32::MIN_POSITIVE).unwrap_err(),
            TextLinesRefusal::WidthNotFinite
        );
    }

    #[test]
    fn a_layout_refuses_more_lines_or_characters_than_its_budget() {
        // 512 lines and 8192 characters are the most a layout answers, and both bounds are the last
        // one it accepts.
        assert!(refusal(&"x\n".repeat(511), 1e9, TextAlign::Left, 1.0).is_ok());
        assert_eq!(
            refusal(&"x\n".repeat(600), 1e9, TextAlign::Left, 1.0).unwrap_err(),
            TextLinesRefusal::TooManyLines
        );
        assert!(refusal(&"a".repeat(8192), 1e9, TextAlign::Left, 1.0).is_ok());
        assert_eq!(
            refusal(&"a".repeat(8193), 1e9, TextAlign::Left, 1.0).unwrap_err(),
            TextLinesRefusal::TooManyGlyphs
        );
    }

    #[test]
    fn a_layout_survives_the_wire() {
        use mod_api::wire::{Reader, Wire, Writer};
        let lines = layout("AAAA\nBB", 100.0, TextAlign::Centre, 1.0);
        let mut writer = Writer::default();
        lines.put(&mut writer);
        let back: TextLines = Reader::decode(&writer.into_bytes()).unwrap();
        assert_eq!(plain(&back), ["AAAA", "BB"]);
        assert_eq!((back.width, back.height), (lines.width, lines.height));
        let at: Vec<[f32; 2]> = back.lines.iter().map(|line| line.at).collect();
        assert_eq!(at, [[38.0, 0.0], [44.0, 9.0]]);
    }

    /// The game's own font and GUI atlas, read from the prepared resources alone: a real font with
    /// its real advances and the atlas it draws from, with no game, no world and no process-wide
    /// state registered.
    fn prepared_atlas() -> crate::hud::HudAtlas {
        let root = std::env::var_os("SUNRISE_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/sunrise-client")
                    .join(sunrise_core::VANILLA_VERSION)
            });
        assert!(
            root.join("assets/minecraft/font/default.json").is_file(),
            "the prepared resources are not at {}",
            root.display()
        );
        crate::hud::HudAtlas::build(&crate::resource_source::ResourceSource::directory(root))
            .expect("the prepared resources' font and atlas")
    }

    /// A label of 400 characters in a 120-pixel box, with the game's own font: every line fits,
    /// the lines are as many as the text's own width needs, the drawn glyphs cover exactly the
    /// height the layout answered, and every answer doubles with `scale`.
    #[test]
    #[ignore = "daily"]
    fn scaled_text_is_the_text_scaled_about_its_corner() {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            let at = [10.0, 20.0];
            let mut plain = crate::hud::HudBatch::scratch([320, 240]);
            GameDrawContext::new(game, &mut plain).text("Jump", at, 0xFFFF_FFFF, true);
            let mut scaled = crate::hud::HudBatch::scratch([320, 240]);
            GameDrawContext::new(game, &mut scaled).text_scaled("Jump", at, 0.5, 0xFFFF_FFFF, true);
            assert!(!plain.vertices.is_empty(), "the text draws glyphs");
            assert_eq!(plain.vertices.len(), scaled.vertices.len());
            for (p, s) in plain.vertices.iter().zip(&scaled.vertices) {
                let half = [
                    at[0] + (p.pos[0] - at[0]) * 0.5,
                    at[1] + (p.pos[1] - at[1]) * 0.5,
                ];
                assert_eq!(s.pos, half, "every corner halfway to `at`");
                assert_eq!(
                    (s.uv, s.color),
                    (p.uv, p.color),
                    "the same glyphs and colours"
                );
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_wrapped_label_measures_and_draws_its_own_height() {
        let atlas = prepared_atlas();
        let font = &atlas.font;
        // The prepared font, not the fallback one whose every character is six pixels wide.
        assert!(
            font.width("i") < font.width("m"),
            "an {} wide i and {} wide m",
            font.width("i"),
            font.width("m")
        );
        let text = crate::text::Text::literal("mmmm ".repeat(80));
        let lines = label_layout(font, &text, 120.0, TextAlign::Centre, 1.0).unwrap();
        // The text needs this many 120-pixel lines even with no space wasted.
        let floor = (font.width_text(&text) / 120.0).ceil() as usize;
        assert!(
            lines.lines.len() >= floor,
            "{} lines for a floor of {floor}",
            lines.lines.len()
        );
        assert!(lines.lines.len() >= 20, "{} lines", lines.lines.len());
        for line in &lines.lines {
            assert!(line.width <= 120.0, "{} wide", line.width);
        }
        // What the lines draw: the glyphs' own extent, as the HUD batch records them.
        let mut batch = crate::hud::HudBatch::scratch([1, 1]);
        for line in &lines.lines {
            batch.text_styled(
                &atlas,
                &engine(&line.text),
                line.at[0],
                line.at[1],
                0xFFFF_FFFF,
                false,
            );
        }
        let (top, bottom) = batch.vertices.iter().fold(
            (f32::INFINITY, f32::NEG_INFINITY),
            |(top, bottom), vertex| (top.min(vertex.pos[1]), bottom.max(vertex.pos[1])),
        );
        assert!(
            (bottom - top - lines.height).abs() <= 1.0,
            "drawn {} against a height of {}",
            bottom - top,
            lines.height
        );
        // At scale 2 the same two-line word fits its box at twice every answer: a line's width,
        // the second line's place and the block's height.
        let word = crate::text::Text::literal("mmmm\nmmmm");
        let plain_size = label_layout(font, &word, 200.0, TextAlign::Left, 1.0).unwrap();
        let doubled = label_layout(font, &word, 200.0, TextAlign::Left, 2.0).unwrap();
        assert_eq!(doubled.lines[1].width, plain_size.lines[1].width * 2.0);
        assert_eq!(doubled.lines[1].at[1], plain_size.lines[1].at[1] * 2.0);
        assert_eq!(doubled.height, plain_size.height * 2.0);
    }

    #[test]
    #[ignore = "daily"]
    fn local_tooltip_draws_deep_text_but_refuses_unrepresentable_metadata() {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            let mut deep = mod_api::Text::literal("Tooltip");
            for _ in 0..mod_api::text::MAX_TEXT_DEPTH {
                deep = mod_api::Text::literal("").append(deep);
            }
            let mut invalid = mod_api::Text::literal("Unrepresentable");
            invalid.metadata = Tag::compound([("test:nonfinite", Tag::Double(f64::NAN.to_bits()))]);

            let mut batch = crate::hud::HudBatch::scratch([400, 200]);
            let mut draw = GameDrawContext::new(game, &mut batch);
            assert_eq!(
                draw.gui_tooltip(&[deep.clone(), invalid], [16.0, 16.0]),
                Err(TextFormError::NonFinite)
            );
            assert!(draw.surface.vertices.is_empty());
            draw.gui_tooltip(&[deep], [16.0, 16.0]).unwrap();
            assert!(!draw.surface.vertices.is_empty());
        });
    }

    #[test]
    fn local_label_layout_handles_deep_trees_and_preserves_opaque_styles() {
        let mut opaque_dialog = Tag::ByteArray(vec![7, 8]);
        for _ in 0..600 {
            let mut map = std::collections::BTreeMap::new();
            map.insert("n".into(), opaque_dialog);
            opaque_dialog = Tag::Compound(map.into_iter().collect());
        }
        let mut style = mod_api::text::TextStyle::default();
        style.bold = Some(true);
        style.click = Some(mod_api::text::TextClick::ShowDialog(opaque_dialog.clone()));
        let mut deep = mod_api::Text::literal("AAAA BBBB").with_style(style);
        for _ in 0..mod_api::text::MAX_TEXT_DEPTH {
            let mut parent = mod_api::Text::literal("");
            parent.children.push(deep);
            deep = parent;
        }
        let local = sunrise_client_lib::api_text::to_local_engine(&deep);
        let lines = label_layout_with(
            &fake_font(6.0),
            local.engine(),
            30.0,
            TextAlign::Left,
            1.0,
            |piece| Ok(local.text_to_api(piece)),
        )
        .unwrap();
        assert_eq!(lines.lines.len(), 2);
        for line in &lines.lines {
            assert_eq!(line.text.style.bold, Some(true));
            assert_eq!(
                line.text.style.click,
                Some(mod_api::text::TextClick::ShowDialog(opaque_dialog.clone()))
            );
        }
    }
}
