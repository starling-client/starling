//! Starling's tests of graphics.rs, out of Sunrise's file (tools/starling/move_out.py).
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

#[test]
fn the_old_phone_defaults_go_to_the_window_size_and_other_scales_stay() {
    for (saved, now, changed) in [
        (70, 100, true),
        (85, 100, true),
        (75, 75, false),
        (100, 100, false),
    ] {
        let mut s = crate::settings::Settings::default();
        s.sunrise.resolution_scale = saved;
        assert_eq!(lift_old_mobile_resolution(&mut s), changed, "{saved}");
        assert_eq!(s.sunrise.resolution_scale, now, "{saved}");
    }
}

#[test]
fn a_driver_crash_at_start_leaves_vulkan_for_opengl() {
    assert_eq!(after_driver_crash(GraphicsApi::Auto), GraphicsApi::OpenGl);
    assert_eq!(after_driver_crash(GraphicsApi::Vulkan), GraphicsApi::OpenGl);
    assert_eq!(after_driver_crash(GraphicsApi::OpenGl), GraphicsApi::OpenGl);
    assert_eq!(after_driver_crash(GraphicsApi::Dx12), GraphicsApi::Dx12);
}

#[test]
fn phones_start_lighter_by_tier() {
    use crate::system_stats::Tier;
    let mut low = crate::settings::Settings::default();
    apply_mobile_defaults(Tier::Low, &mut low);
    let mut high = crate::settings::Settings::default();
    apply_mobile_defaults(Tier::High, &mut high);
    assert_eq!((low.max_fps, high.max_fps), (60, 60));
    assert!(low.render_distance < high.render_distance);
    assert_eq!(
        (low.clouds.as_str(), high.clouds.as_str()),
        ("false", "fast")
    );
    // The window's size on every tier: below it the upscale pass costs more than it saves.
    assert_eq!(
        (low.sunrise.resolution_scale, high.sunrise.resolution_scale),
        (100, 100)
    );
    assert!(!low.sunrise.animated_textures && high.sunrise.animated_textures);
    // Auto-Jump on, as on Bedrock's touch screens.
    assert!(low.auto_jump && high.auto_jump);
    // Far terrain as coarse as Mobile on every tier.
    assert_eq!(
        (low.sunrise.far_detail, high.sunrise.far_detail),
        (FarDetail::Mobile, FarDetail::Mobile)
    );
    // The crowns' insides solid, their outer leaves cut out.
    assert_eq!(high.sunrise.leaf_culling, LeafCullingMode::Solid);
    // What the options screens read back.
    let file = high.to_file();
    let back = crate::settings::Settings::from_file(file);
    assert_eq!(back.render_distance, 10);
    assert!(back.auto_jump);
}
