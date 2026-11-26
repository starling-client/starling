//! GPU ownership of the textures stitched by registered atlas ID.
use super::{AnimatedPlacement, HudRenderer, RegisteredAtlasPage};
use crate::sprites::SpriteAnimation;

pub(crate) struct UploadedAtlasPage {
    pub(super) texture: wgpu::Texture,
    pub(super) view: wgpu::TextureView,
    pub(super) animations: Vec<(AnimatedPlacement, SpriteAnimation, (usize, usize, u32))>,
}

pub(super) struct AtlasPageGpu {
    pub(super) page: UploadedAtlasPage,
    pub(super) group: wgpu::BindGroup,
}

pub(crate) fn upload_atlas_page(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    page: &RegisteredAtlasPage,
) -> Option<UploadedAtlasPage> {
    let [width, height] = page.size;
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let attempt = || -> Result<_, String> {
        let bytes_per_row = width
            .checked_mul(4)
            .ok_or("RGBA8 row exceeds the GPU address space")?;
        // TextureAtlas.createTexture and uploadInitialContents operate on the owning atlas.
        // Capture allocation/upload failures here, before any draw can bind the texture.
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&page.atlas),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &page.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(height),
            },
            size,
        );
        let memory = pollster::block_on(memory.pop());
        let internal = pollster::block_on(internal.pop());
        let validation = pollster::block_on(validation.pop());
        if let Some(error) = memory.or(internal).or(validation) {
            return Err(error.to_string());
        }
        let view = texture.create_view(&Default::default());
        Ok(UploadedAtlasPage {
            texture,
            view,
            animations: page
                .animation_placements
                .iter()
                .zip(&page.animations)
                .map(|(&placement, (_, a))| (placement, a.clone(), (usize::MAX, 0, 0)))
                .collect(),
        })
    };
    match attempt() {
        Ok(page) => Some(page),
        Err(error) => {
            log::error!(
                "Sunrise: texture atlas '{}' (mod {:?}) could not be uploaded: {error}",
                page.atlas,
                page.mod_id
            );
            None
        }
    }
}

impl HudRenderer {
    pub(crate) fn atlas_page_views(&self) -> impl Iterator<Item = Option<&wgpu::TextureView>> {
        self.atlas_pages
            .iter()
            .map(|page| page.as_ref().map(|p| &p.page.view))
    }
}
