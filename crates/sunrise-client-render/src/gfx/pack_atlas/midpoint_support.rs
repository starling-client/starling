//! A test array of sprites for the atlas's midpoint probes, as the game builds them.

use wgpu::util::DeviceExt;

/// An array of `sprite`-pixel layers (`sprite` the largest of `native`): each layer scaled up by
/// whole pixels at level 0, its own pixels from the level where it has its own size. Every pixel
/// of a layer's own image differs: (layer, x, y, 255).
pub(super) fn upscaled_array(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    native: &[u32],
) -> wgpu::Texture {
    let sprite = native.iter().copied().max().unwrap();
    let levels = sprite.ilog2() + 1;
    let mut data = Vec::new();
    for (layer, &side) in native.iter().enumerate() {
        for level in 0..levels {
            let edge = sprite >> level;
            // Above its own size a level repeats each pixel; below, it shrinks to the first.
            let scale = (edge / side).max(1);
            for y in 0..edge {
                for x in 0..edge {
                    let (x, y) = if edge >= side {
                        (x / scale, y / scale)
                    } else {
                        (0, 0)
                    };
                    data.extend([layer as u8 * 50, x as u8 * 16, y as u8 * 16, 255]);
                }
            }
        }
    }
    device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: sprite,
                height: sprite,
                depth_or_array_layers: native.len() as u32,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &data,
    )
}
