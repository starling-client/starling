//! The GPU tests' adapter and device helpers.
//!
//! The client render's `gfx::test_gpu` hands out the same helpers, each spelled the same and
//! with the same missing-adapter policy (`SUNRISE_GPU_TESTS_REQUIRE_ADAPTER`), and its own
//! extra helpers (the DX12 compilers, the depth-load probe) are its tests' alone, so this crate
//! keeps only what it calls.

use std::sync::{Mutex, MutexGuard};

static GPU_TEST: Mutex<()> = Mutex::new(());

/// Serialize an entire GPU test, including driver initialization and teardown.
pub(crate) fn lock() -> MutexGuard<'static, ()> {
    GPU_TEST.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// A new instance for one GPU test. On Windows it opens Vulkan and DirectX 12, not OpenGL:
/// WGL instances made and dropped from many test threads at once deadlocked NVIDIA's driver,
/// and the render tests hung most runs (#5238).
pub(crate) fn instance() -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    if cfg!(windows) {
        descriptor.backends = wgpu::Backends::PRIMARY;
    }
    wgpu::Instance::new(descriptor.with_env())
}

/// Find an adapter, reporting an unavailable adapter without hiding other failures.
pub(crate) fn adapter() -> Option<wgpu::Adapter> {
    match pollster::block_on(instance().request_adapter(&Default::default())) {
        Ok(adapter) => Some(adapter),
        Err(error @ wgpu::RequestAdapterError::NotFound { .. }) => {
            assert!(!require_adapter(), "no GPU adapter: {error}");
            eprintln!("skipped: no adapter: {error}");
            None
        }
        Err(error) => panic!("GPU test adapter request failed: {error}"),
    }
}

/// CI sets this where every GPU test must run: a missing adapter or device fails instead of skipping.
fn require_adapter() -> bool {
    std::env::var_os("SUNRISE_GPU_TESTS_REQUIRE_ADAPTER").is_some_and(|value| value == "1")
}

/// Device creation may skip locally, but must succeed under the CI switch.
pub(crate) fn optional_device_with(
    adapter: &wgpu::Adapter,
    descriptor: &wgpu::DeviceDescriptor<'_>,
) -> Option<(wgpu::Device, wgpu::Queue)> {
    match pollster::block_on(adapter.request_device(descriptor)) {
        Ok(pair) => Some(pair),
        Err(error) => {
            assert!(
                !require_adapter(),
                "GPU test device creation failed: {error}"
            );
            eprintln!("skipped: no GPU device: {error}");
            None
        }
    }
}

pub(crate) fn device(adapter: &wgpu::Adapter) -> (wgpu::Device, wgpu::Queue) {
    pollster::block_on(adapter.request_device(&Default::default()))
        .expect("GPU test device creation failed")
}
