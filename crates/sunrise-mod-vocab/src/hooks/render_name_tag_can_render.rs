//! The `render_name_tag_can_render` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Name tag visibility and optional replacement text.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "RenderNameTagEvent.CanRender",
        cancellable = false
    )]
    fn render_name_tag_can_render(
        entity_id: u32,
        kind: &str,
        vanilla_visible: bool,
        visible: &mut bool,
        text: &mut Option<String>,
    );
}
