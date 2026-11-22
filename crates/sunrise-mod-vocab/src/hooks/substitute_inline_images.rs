//! Image glyph substitution before laying out displayed text.

pub trait Hooks {
    /// Propose inline images for one nonempty, translated text run in chat history or on either
    /// face of a sign, after legacy formatting and before wrapping, measuring and bidi reordering.
    /// Existing inline objects are not passed to the hook. The `run` is one resolved style span;
    /// ranges cannot cross spans or line breaks. Push UTF-8 byte ranges into `images`, in priority
    /// order. The first valid, nonoverlapping proposal wins in hook dispatch order. Missing sprites
    /// and invalid ranges are ignored, leaving the original characters available to later proposals.
    ///
    /// An accepted range draws one 8×8 `minecraft:gui` atlas glyph with an advance of 8, using
    /// the span's color, or white when the span has none; on a sign, the sign's dye never tints it.
    /// Chat hover and click use the span's style at the glyph's displayed position.
    /// The source component, chat history and signatures, and sign editing are not changed.
    /// Cached layout can reuse answers while the text, resources, sprite geometry and enabled mods
    /// are unchanged. To animate a glyph, update the pixels of the same dynamic texture id.
    #[hook(
        receiver = "read",
        dispatch = "api_carrier",
        side = "client",
        contract = "collect",
        out = "images",
        answered_by = "mod",
        neoforge = "none (vanilla ChatComponent and AbstractSignRenderer.submitSignText)"
    )]
    fn substitute_inline_images(run: &str, images: &mut Vec<Answered<InlineImageReplacement>>);
}
