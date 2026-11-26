//! Starling's part of flat.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// PowerVR Rogue's Vulkan driver (the GE8320's 1.13) draws terrain in pieces with per-section
/// draws, without an error: flat frames need neither `first_instance` nor a vertex offset.
pub fn is_powervr_vulkan(info: &wgpu::AdapterInfo) -> bool {
    info.backend == wgpu::Backend::Vulkan
        && (info.vendor == 0x1010 || info.name.contains("PowerVR"))
}

/// The same on Adreno's Vulkan, for frames without a shader pack. Vasysik's Android fork measured
/// an Adreno 710 at 2560x1600 and 16 chunks: 46.05 ms of GPU time (20.09 FPS) with per-section
/// indirect draws, 22.30 ms (38.37 FPS) drawn flat.
pub(super) const ADRENO_MIN_SECTIONS: usize = 512;

/// Where `for_vanilla` holds: sections in view from which a frame is drawn flat.
pub fn vanilla_min_sections(info: &wgpu::AdapterInfo) -> usize {
    if super::super::adreno::adreno_vulkan(info) {
        ADRENO_MIN_SECTIONS
    } else {
        MIN_SECTIONS
    }
}
