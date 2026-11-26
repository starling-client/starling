//! Starling's part of device.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// Where Vulkan keeps the pipelines the driver compiled, inside the game's private folder
/// (`logging::private_home`).
pub fn vulkan_pipeline_cache(home: &std::path::Path) -> std::path::PathBuf {
    home.join("cache").join("vulkan-pipelines.bin")
}

/// One black frame on a surface just configured.
pub(crate) fn clear_surface(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    surface: &wgpu::Surface,
    format: wgpu::TextureFormat,
) {
    let frame = match surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
        _ => return,
    };
    let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
        format: Some(format),
        ..Default::default()
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("First clear"),
    });
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("First clear"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    queue.submit([encoder.finish()]);
    queue.present(frame);
}

/// Whether an adapter whose Depth32Float has `features` can draw into it as a depth target,
/// which every 3D pass does (`open_backend`).
pub(crate) fn renders_depth(features: wgpu::TextureFormatFeatures) -> bool {
    features
        .allowed_usages
        .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
}

/// The name of the GPU Vulkan would give, asked before any device exists: no surface, no
/// device, so nothing reaches the driver's queue.
pub(crate) async fn vulkan_gpu_name(window: &Arc<Window>) -> Option<String> {
    let instance = wgpu::Instance::new(instance_descriptor(
        wgpu::Backends::VULKAN,
        Some(window),
        false,
        wgpu::Dx12Compiler::Fxc,
    ));
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            ..Default::default()
        })
        .await
        .ok()?;
    Some(adapter.get_info().name)
}

/// PowerVR Rogue (GE8320, GE8322, GE8300...) on Vulkan. Diagnostic reports from 0.8.0: the
/// GE8320's driver 1.15 (an HONOR GFY-LX2) and 24.2 (a Xiaomi 23129RN51X) die with SIGSEGV
/// inside `vkQueueSubmit` on the very first submission, `clear_surface`'s black frame, every
/// start; 1.13 (a vivo 1901) gets past it but draws terrain in patches with broken water,
/// which OpenGL ES doesn't (#326, #327). The B-Series (BXM) is not Rogue and keeps Vulkan.
pub(crate) fn rogue_vulkan_crashes(gpu: &str) -> bool {
    gpu.contains("PowerVR Rogue")
}
