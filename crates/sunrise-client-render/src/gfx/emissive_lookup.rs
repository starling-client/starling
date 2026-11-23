//! Index of emissive sprite overlays for GLES, whose shader compiler struggles with the
//! unrolled uniform lookup. One integer texel per 15-bit base sprite index.

const WIDTH: u32 = 256;
const HEIGHT: u32 = (1 << 15) / WIDTH;
const EXTENT: wgpu::Extent3d = wgpu::Extent3d {
    width: WIDTH,
    height: HEIGHT,
    depth_or_array_layers: 1,
};

pub(super) struct Lookup {
    texture: wgpu::Texture,
    pub(super) view: wgpu::TextureView,
    pairs: Vec<[u32; 4]>,
}

impl Lookup {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Terrain emissive overlay indices"),
            size: EXTENT,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self {
            texture,
            view,
            pairs: Vec::new(),
        }
    }

    pub(super) fn update(&mut self, queue: &wgpu::Queue, pairs: &[[u32; 4]]) {
        let pairs = &pairs[..pairs.len().min(crate::native_emissive::MAX)];
        if self.pairs == pairs {
            return;
        }
        self.pairs.clear();
        self.pairs.extend_from_slice(pairs);
        let mut layers = vec![u32::MAX; (WIDTH * HEIGHT) as usize];
        // Reverse the writes so a duplicate takes its first value, just as the shader did.
        for &[base, overlay, ..] in pairs.iter().rev() {
            if let Some(slot) = layers.get_mut(base as usize) {
                *slot = overlay;
            }
        }
        queue.write_texture(
            self.texture.as_image_copy(),
            bytemuck::cast_slice(&layers),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 4),
                rows_per_image: Some(HEIGHT),
            },
            EXTENT,
        );
    }
}
