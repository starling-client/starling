//! The `render_name_tag_do_render` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before one line of an entity's name tag is drawn (NeoForge's `RenderNameTagEvent.DoRender`):
    /// once a frame for each line of each tag the frame shows, after `render_name_tag_can_render`
    /// decided the tag shows and what it says. Mobs, players, armour stands, item frames and
    /// display entities ask it. The score line (`NameTagLine::Score`: the below-name score, or a
    /// mannequin's description) comes first, then the name (`NameTagLine::Name`).
    ///
    /// `entity_id` and `kind` name the entity; `content` is the line's plain text
    /// (`Component.getString`), borrowed for this call. `draw` is posed as vanilla's at
    /// `submitNameTag`: at the entity's render origin, not turned and not billboarded (+x east,
    /// +y up, +z south, in blocks), raised one line (`9 * 1.15 * 0.025`) for the name when the
    /// tag has a score line, even when that line was canceled. Vanilla draws the text at
    /// `attachment` (the entity's `NAME_TAG` attachment, relative to that pose) plus 0.5 up,
    /// facing the camera, scaled `0.025`; the camera itself is `host.camera()`.
    /// `partial_tick` is the frame's. What `draw` draws is kept, depth tested in the entity pass.
    /// Set `cancelled` to skip vanilla's drawing of this line alone (its text, background and
    /// see-through copy); the other line still draws where it would have.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "RenderNameTagEvent.DoRender",
        cancellable = true
    )]
    fn render_name_tag_do_render(
        entity_id: u32,
        kind: &str,
        line: NameTagLine,
        content: &str,
        attachment: [f64; 3],
        partial_tick: f32,
        draw: &mut dyn BlockEntityDrawContext,
        canceled: &mut bool,
    );
}
