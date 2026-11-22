//! Hardware ray queries, only on request (#13780, after the #1180 spike): `HORIZON_RAY_TRACING=1`
//! at startup asks the device for wgpu's `EXPERIMENTAL_RAY_QUERY` when the GPU has it. Nothing
//! draws with it yet. Without the variable, and in a build without `horizon-extras` (a Horizon
//! extra, docs/layers.md), the device is opened exactly as before.

/// The terrain's acceleration structures (step 2).
#[path = "ray_scene.rs"]
pub(in crate::gfx) mod ray_scene;

/// The ray query feature.
const RAY_QUERY: wgpu::Features = wgpu::Features::EXPERIMENTAL_RAY_QUERY;

/// Whether `device` was opened with ray queries.
pub(in crate::gfx) fn on(device: &wgpu::Device) -> bool {
    device.features().contains(RAY_QUERY)
}

/// What buffers that BLAS builds read need on top of their own usage: nothing without ray
/// queries, so the off path's buffers are main's.
pub(in crate::gfx) fn blas_input(device: &wgpu::Device) -> wgpu::BufferUsages {
    if on(device) {
        wgpu::BufferUsages::BLAS_INPUT
    } else {
        wgpu::BufferUsages::empty()
    }
}

/// The index buffer of `quads` quads (triangles 0 1 2 and 0 2 3), which the terrain draws
/// and BLAS builds both read.
pub(in crate::gfx) fn quad_indices(device: &wgpu::Device, quads: u32) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    let indices: Vec<u32> = (0..quads)
        .flat_map(|q| {
            let b = q * 4;
            [b, b + 1, b + 2, b, b + 2, b + 3]
        })
        .collect();
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Quad indices"),
        contents: bytemuck::cast_slice(&indices),
        usage: wgpu::BufferUsages::INDEX | blas_input(device),
    })
}

/// Whether the player asked for ray tracing (the device needs the feature from its creation):
/// `HORIZON_RAY_TRACING`, or Starling's Ray Tracing switch (experimental, #470), which shader
/// packs with `[ray_tracing]` trace against (`gfx/rt.rs`).
fn requested() -> bool {
    horizon_scene() || crate::graphics::ray_tracing_wanted()
}

/// Whether Horizon's own scene (`ray_scene`) is built: only on `HORIZON_RAY_TRACING`. Starling's
/// switch builds #470's structures (`gfx/rt.rs`) instead, and two would cost twice.
pub(in crate::gfx) fn horizon_scene() -> bool {
    asked(std::env::var("HORIZON_RAY_TRACING").ok().as_deref())
}

/// The Ray Tracing switch's offer (#470): greyed out where the adapter has no ray queries,
/// with the device extensions it lacks, which the log names too.
pub(super) fn note_offer(adapter: &wgpu::Adapter) {
    let info = adapter.get_info();
    let offered = adapter.features().contains(RAY_QUERY);
    crate::graphics::offer_ray_tracing(offered);
    if offered {
        crate::graphics::set_ray_tracing_missing(None);
        log::info!(
            "Horizon renderer: {} offers ray queries (the Ray Tracing switch is available)",
            info.name
        );
        return;
    }
    // wgpu offers ray queries only when the driver lists all four extensions.
    #[cfg(not(target_os = "macos"))]
    {
        let missing = horizon_hdr::ray_query_missing_extensions(adapter).unwrap_or_default();
        crate::graphics::set_ray_tracing_missing((!missing.is_empty()).then_some(&missing));
        if missing.is_empty() {
            log::info!(
                "Horizon renderer: {} lists the ray-query extensions, but wgpu offers no ray \
                 queries; the Ray Tracing switch stays off",
                info.name
            );
        } else {
            log::info!(
                "Horizon renderer: {} offers no ray queries; driver lacks {}. The Ray Tracing \
                 switch stays off",
                info.name,
                missing.join(", ")
            );
        }
    }
    #[cfg(target_os = "macos")]
    {
        crate::graphics::set_ray_tracing_missing(None);
        log::info!(
            "Horizon renderer: Metal offers no ray queries; the Ray Tracing switch stays off"
        );
    }
}

/// Whether `HORIZON_RAY_TRACING` set to `var` asks for ray tracing: only in a `horizon-extras`
/// build.
fn asked(var: Option<&str>) -> bool {
    cfg!(feature = "horizon-extras") && var == Some("1")
}

/// The features to request on `adapter`: ray queries, when asked for and offered.
pub(super) fn features(adapter: &wgpu::Adapter) -> wgpu::Features {
    let features = pick(requested(), adapter.features());
    if requested() {
        let info = adapter.get_info();
        if features.is_empty() {
            log::warn!(
                "Horizon renderer: ray tracing asked for, but {} on {:?} has no hardware ray \
                 queries; it stays off",
                info.name,
                info.backend
            );
        } else {
            log::info!(
                "Horizon renderer: ray queries on ({} on {:?})",
                info.name,
                info.backend
            );
        }
    }
    features
}

fn pick(requested: bool, offered: wgpu::Features) -> wgpu::Features {
    if requested {
        offered & RAY_QUERY
    } else {
        wgpu::Features::empty()
    }
}

/// The device's opt-in to `features`' experimental ones.
pub(super) fn experimental(features: wgpu::Features) -> wgpu::ExperimentalFeatures {
    if features.contains(RAY_QUERY) {
        horizon_hdr::experimental_features()
    } else {
        wgpu::ExperimentalFeatures::disabled()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_extras_build_asks_for_ray_tracing() {
        assert_eq!(asked(Some("1")), cfg!(feature = "horizon-extras"));
        assert!(!asked(None));
        assert!(!asked(Some("0")));
    }

    #[test]
    fn ray_queries_only_when_asked_and_offered() {
        let all = wgpu::Features::all();
        assert_eq!(pick(true, all), RAY_QUERY);
        assert!(pick(false, all).is_empty());
        assert!(pick(true, all - RAY_QUERY).is_empty());
        assert!(!experimental(wgpu::Features::empty()).is_enabled());
    }
}
