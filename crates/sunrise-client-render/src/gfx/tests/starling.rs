//! Starling's part of tests.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;
// The device's Starling checks live in gfx/device/starling.rs since the move out of Sunrise's files.
use crate::gfx::device::{renders_depth, rogue_vulkan_crashes};

#[test]
pub(super) fn a_clamped_shadow_view_keeps_what_lies_toward_the_light() {
    use glam::{Mat4, Vec3};
    // The shadow camera with the sun overhead: 128 blocks up, 256 deep (wgpu depth).
    let view = Mat4::look_to_rh(Vec3::new(0.0, 128.0, 0.0), Vec3::NEG_Y, Vec3::Z);
    let cull = Mat4::orthographic_rh(-64.0, 64.0, -64.0, 64.0, 0.05, 256.0) * view;
    let above = (Vec3::new(-8.0, 200.0, -8.0), Vec3::new(8.0, 216.0, 8.0));
    assert!(!super::super::light_frustum(cull, false).intersects(above.0, above.1));
    assert!(super::super::light_frustum(cull, true).intersects(above.0, above.1));
    // Still bounded across the light's rays.
    let beside = (Vec3::new(100.0, 0.0, 0.0), Vec3::new(116.0, 16.0, 16.0));
    assert!(!super::super::light_frustum(cull, true).intersects(beside.0, beside.1));
}

/// Adreno 506's Vulkan can't draw into Depth32Float (#395): such an adapter is skipped, so
/// the next backend (OpenGL ES) opens instead of a pipeline failing at startup.
#[test]
pub(super) fn an_adapter_that_cannot_draw_into_depth32float_is_skipped() {
    use wgpu::{TextureFormatFeatureFlags, TextureFormatFeatures, TextureUsages};
    let features = |allowed_usages| TextureFormatFeatures {
        allowed_usages,
        flags: TextureFormatFeatureFlags::empty(),
    };
    assert!(renders_depth(features(
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING
    )));
    assert!(!renders_depth(features(
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC
    )));
}

/// PowerVR Rogue starts on OpenGL ES with Auto; the B-Series keeps Vulkan (#381).
#[test]
pub(super) fn auto_leaves_vulkan_on_powervr_rogue_only() {
    assert!(rogue_vulkan_crashes("PowerVR Rogue GE8320"));
    assert!(rogue_vulkan_crashes("PowerVR Rogue GE8322"));
    assert!(
        !rogue_vulkan_crashes("PowerVR BXM-8-256"),
        "the B-Series keeps Vulkan"
    );
    assert!(!rogue_vulkan_crashes("Mali-G57 MC2"));
    assert!(!rogue_vulkan_crashes("Adreno (TM) 610"));
}
