//! The `render_name_tag` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Name tag visibility and optional replacement text.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "RenderNameTagEvent",
        cancellable = false
    )]
    fn render_name_tag(
        entity_id: u32,
        kind: &str,
        vanilla_visible: bool,
        visible: &mut bool,
        text: &mut Option<String>,
    );
}
