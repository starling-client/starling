use super::super::LINEAR_FRAME;

// The frame O1 resolves (`output::resolve_frame`) holds linear light only for a 0.2 pack that
// asks for it, on the float frame. On a device without the float frame the pack draws anyway,
// in gamma. A blend changed is another frame, so the game makes the renderer again
// (`Renderer::frame_follows`).
#[test]
fn only_a_float_frame_holds_linear_light() {
    use super::super::output::{FrameRequest, resolve_frame, tests::window};
    let settings = crate::graphics::Graphics::default();
    for float_frame in [false, true] {
        for (linear_pack, linear_blend) in [(false, false), (true, false), (true, true)] {
            let (mut caps, reading) = window(None, None);
            caps.float_frame = float_frame;
            let request = FrameRequest::new()
                .with_linear_pack(linear_pack)
                .with_linear_blend(linear_blend);
            let frame = resolve_frame(
                &settings,
                request,
                caps,
                reading,
                wgpu::TextureFormat::Bgra8UnormSrgb,
            );
            assert_eq!(frame.format == LINEAR_FRAME, float_frame && linear_pack);
            assert_eq!(
                frame.linear_light,
                float_frame && linear_blend,
                "float {float_frame}, linear {linear_blend}"
            );
        }
    }
}
