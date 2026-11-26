use super::*;

impl Game<'_> {
    /// Before one line of a name tag is drawn (NeoForge's `RenderNameTagEvent.DoRender`), with a
    /// draw posed where vanilla's `submitNameTag` is. `canceled` skips vanilla's drawing of that line.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game) fn native_render_name_tag_do_render(
        &self,
        entity_id: u32,
        kind: &str,
        line: NameTagLine,
        content: &str,
        attachment: [f64; 3],
        partial_tick: f32,
        draw: &mut BlockEntityDraw<'_>,
        canceled: &mut bool,
    ) {
        self.fire_render_name_tag_do_render(
            entity_id,
            kind,
            line,
            content,
            attachment,
            partial_tick,
            draw,
            canceled,
        );
    }
}
