//! Shared GPU test setup.

/// Find an adapter, reporting an unavailable adapter without hiding other failures.
pub(crate) fn adapter() -> Option<wgpu::Adapter> {
    let instance = instance();
    let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
        Ok(adapter) => adapter,
        Err(error @ wgpu::RequestAdapterError::NotFound { .. }) => {
            eprintln!("skipped: no adapter: {error}");
            return None;
        }
        Err(error) => panic!("GPU test adapter request failed: {error}"),
    };
    Some(adapter)
}

/// Check format usages before constructing a pipeline that needs them.
pub(crate) fn supports_texture_usage(
    adapter: &wgpu::Adapter,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> bool {
    if adapter
        .get_texture_format_features(format)
        .allowed_usages
        .contains(usage)
    {
        return true;
    }
    let info = adapter.get_info();
    eprintln!(
        "skipped: {} / {:?} lacks {format:?} {usage:?}",
        info.name, info.backend
    );
    false
}

/// Naga's GLSL writer cannot lower depth textureLoad (no downlevel flag describes it).
pub(crate) fn supports_depth_load(adapter: &wgpu::Adapter) -> bool {
    let info = adapter.get_info();
    if info.backend != wgpu::Backend::Gl {
        return true;
    }
    eprintln!(
        "skipped: {} / {:?} lacks depth textureLoad in GLSL",
        info.name, info.backend
    );
    false
}

/// Once capabilities are checked, device errors must fail rather than skip.
pub(crate) fn device(adapter: &wgpu::Adapter) -> (wgpu::Device, wgpu::Queue) {
    pollster::block_on(adapter.request_device(&Default::default()))
        .expect("GPU test device creation failed")
}

/// A new instance for one GPU test. On Windows it opens Vulkan and DirectX 12,
/// not OpenGL: WGL instances made and dropped from many test threads at once
/// deadlocked NVIDIA's driver, and the render tests hung most runs (#5238).
/// The game runs there on Vulkan or DirectX 12 unless neither works.
/// `WGPU_BACKEND` and the other `WGPU_*` variables still apply, as in the game.
pub(crate) fn instance() -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    if cfg!(windows) {
        descriptor.backends = wgpu::Backends::PRIMARY;
    }
    wgpu::Instance::new(descriptor.with_env())
}
