//! On-demand GUI pictures. The atlas keeps weak references; only a frame and
//! the renderer keep decoded pixels/textures alive, so leaving the screen frees both.
//! A mod's standalone texture (`GuiCanvas::texture`) is held instead for the atlas's
//! resource generation, as vanilla's `TextureManager` keeps a texture until a reload.
use super::*;
use std::sync::{
    Weak,
    atomic::{AtomicU64, Ordering},
};
use sunrise_core::identifier::Identifier;

pub struct Picture {
    pub(super) id: u64,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) pixels: Vec<u8>,
    pub(super) flags: TextureFlags,
}

#[derive(Default)]
pub(super) struct Cache {
    roots: Vec<crate::resource_source::ResourceSource>,
    entries: HashMap<String, Option<Weak<Picture>>>,
    /// By namespace, then path: looked up by `&str` every frame without building a key.
    textures: HashMap<String, HashMap<String, Arc<Picture>>>,
    /// By namespace, then path, whether [`Cache::has`] found the resource in a root.
    present: HashMap<String, HashMap<String, bool>>,
    missing: Option<Arc<Picture>>,
}

fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Cache {
    pub(super) fn new(
        root: &crate::resource_source::ResourceSource,
        defaults: &crate::resource_source::ResourceSource,
    ) -> Self {
        Self {
            roots: vec![root.clone(), defaults.clone()],
            ..Self::default()
        }
    }
    /// Whether the resource `assets/<namespace>/<path>` of `id` is in a root: an image that is
    /// there but cannot be decoded still is, and draws vanilla's missing texture.
    /// Answered once per resource generation, as the cache dies with its atlas on a reload; a
    /// hit builds no name.
    fn has(&mut self, id: &Identifier) -> bool {
        let (namespace, path) = (id.namespace(), id.path());
        if let Some(&known) = self.present.get(namespace).and_then(|t| t.get(path)) {
            return known;
        }
        let name = format!("assets/{namespace}/{path}");
        let found = crate::resource_source::stays_inside(&name)
            && self.roots.iter().any(|root| root.contains(&name));
        self.present
            .entry(namespace.to_owned())
            .or_default()
            .insert(path.to_owned(), found);
        found
    }
    /// The standalone texture `id`, the exact resource `assets/<namespace>/<path>`, from the
    /// first root that has it (`ResourceManager.getResource`), sampled as its `.mcmeta` says:
    /// the topmost one from the image's root or a root above it
    /// (`FallbackResourceManager.createStackMetadataFinder`), as the pack stack itself keeps only
    /// a `.mcmeta` from the image's pack or above. Decoded once per resource generation; a missing
    /// or undecodable image, or a `.mcmeta` that cannot be read or parsed
    /// ([`TextureFlags::read_mcmeta`]), is vanilla's missing texture
    /// (`TextureManager.loadContentsSafe`, `TextureContents.createMissing`).
    fn texture(&mut self, id: &Identifier) -> Arc<Picture> {
        let (namespace, path) = (id.namespace(), id.path());
        if let Some(texture) = self.textures.get(namespace).and_then(|t| t.get(path)) {
            return texture.clone();
        }
        let name = format!("assets/{namespace}/{path}");
        let found = crate::resource_source::stays_inside(&name)
            .then(|| self.roots.iter().position(|root| root.contains(&name)))
            .flatten()
            .and_then(|winner| {
                let (width, height, pixels) = decode_resource(&self.roots[winner], &name).ok()?;
                let meta = format!("{name}.mcmeta");
                let flags = match self.roots[..=winner].iter().find(|r| r.contains(&meta)) {
                    None => TextureFlags::default(),
                    Some(root) => {
                        let text = root.read_to_string(&meta).ok()?;
                        TextureFlags::read_mcmeta(&text).ok()?
                    }
                };
                Some(Arc::new(Picture {
                    id: next_id(),
                    width,
                    height,
                    pixels,
                    flags,
                }))
            });
        let texture = found.unwrap_or_else(|| {
            self.missing
                .get_or_insert_with(|| {
                    let (width, height, pixels) = missing_image();
                    Arc::new(Picture {
                        id: next_id(),
                        width,
                        height,
                        pixels,
                        flags: TextureFlags::default(),
                    })
                })
                .clone()
        });
        self.textures
            .entry(namespace.to_owned())
            .or_default()
            .insert(path.to_owned(), texture.clone());
        texture
    }
    fn get(&mut self, name: &str) -> Option<Arc<Picture>> {
        if let Some(cached) = self.entries.get(name) {
            let weak = cached.as_ref()?;
            if let Some(image) = weak.upgrade() {
                return Some(image);
            }
        }
        let image = self
            .roots
            .iter()
            .find_map(|root| {
                let path = crate::atlas_sources::texture_resource(name, "textures");
                let image = decode_resource(root, &path).ok()?;
                let flags = root
                    .read_to_string(&format!("{path}.mcmeta"))
                    .ok()
                    .as_deref()
                    .and_then(TextureFlags::from_mcmeta)
                    .unwrap_or_default();
                Some((image, flags))
            })
            .map(|((width, height, pixels), flags)| {
                Arc::new(Picture {
                    id: next_id(),
                    width,
                    height,
                    pixels,
                    flags,
                })
            });
        self.entries
            .insert(name.into(), image.as_ref().map(Arc::downgrade));
        image
    }
}

impl HudAtlas {
    /// Whether the standalone texture `id` is in the resource stack ([`HudBatch::texture`]).
    pub fn has_texture(&self, id: &Identifier) -> bool {
        self.pictures.lock().unwrap().has(id)
    }
}

impl HudBatch {
    /// Draw a full GUI picture at its exact place in the HUD's ordered runs.
    pub fn picture(
        &mut self,
        atlas: &HudAtlas,
        name: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        tint: u32,
    ) {
        let Some(picture) = atlas.pictures.lock().unwrap().get(name) else {
            return;
        };
        self.picture_quad(picture, [x, y, w, h], [0.0, 0.0, 1.0, 1.0], tint);
    }

    /// `GuiGraphicsExtractor.blit` of the standalone texture `id` (see `GuiCanvas::texture`):
    /// `innerBlit`'s quad at `x, y, w, h`, sampling `uv / tex` to `(uv + src) / tex` unclamped,
    /// tinted `tint` (the batch's RGBA), at its place in the HUD's ordered runs.
    pub fn texture(
        &mut self,
        atlas: &HudAtlas,
        id: &Identifier,
        at: [f32; 4],
        uv: [f32; 2],
        [src, tex]: [[i32; 2]; 2],
        tint: u32,
    ) {
        let picture = atlas.pictures.lock().unwrap().texture(id);
        let (tw, th) = (tex[0] as f32, tex[1] as f32);
        let uvs = [
            uv[0] / tw,
            uv[1] / th,
            (uv[0] + src[0] as f32) / tw,
            (uv[1] + src[1] as f32) / th,
        ];
        self.picture_quad(picture, at, uvs, tint);
    }

    /// `GuiGraphicsExtractor.blit` of the dynamic texture `handle`, `size` pixels, as
    /// [`Self::texture`] draws a standalone one.
    pub fn dynamic_texture(
        &mut self,
        handle: u64,
        size: [u32; 2],
        [x, y, w, h]: [f32; 4],
        uv: [f32; 2],
        [src, tex]: [[i32; 2]; 2],
        tint: u32,
    ) {
        let (tw, th) = (tex[0] as f32, tex[1] as f32);
        let rect = Rect {
            u0: uv[0] / tw,
            v0: uv[1] / th,
            u1: (uv[0] + src[0] as f32) / tw,
            v1: (uv[1] + src[1] as f32) / th,
            w: size[0],
            h: size[1],
        };
        let start = self.vertices.len();
        self.rect_uv(x, y, w, h, rect, tint);
        self.blend_since(start, Blend::Dynamic(handle));
    }

    fn picture_quad(
        &mut self,
        picture: Arc<Picture>,
        [x, y, w, h]: [f32; 4],
        [u0, v0, u1, v1]: [f32; 4],
        tint: u32,
    ) {
        let start = self.vertices.len();
        let rect = Rect {
            u0,
            v0,
            u1,
            v1,
            w: picture.width,
            h: picture.height,
        };
        self.rect_uv(x, y, w, h, rect, tint);
        self.blend_since(start, Blend::Picture(picture.id));
        if !self.pictures.iter().any(|p| p.id == picture.id) {
            self.pictures.push(picture);
        }
    }
}

/// `ReloadableTexture.setSampler`: `blur` filters linearly, else nearest; `clamp` clamps to the
/// edge, else repeats. Always level 0, even of a mipmapped dynamic texture: `lod_max_clamp`
/// is 0 (no mip filter either), as the book page says.
fn sampler(flags: TextureFlags) -> wgpu::SamplerDescriptor<'static> {
    let filter = match flags.blur {
        true => wgpu::FilterMode::Linear,
        false => wgpu::FilterMode::Nearest,
    };
    let address = match flags.clamp {
        true => wgpu::AddressMode::ClampToEdge,
        false => wgpu::AddressMode::Repeat,
    };
    wgpu::SamplerDescriptor {
        mag_filter: filter,
        min_filter: filter,
        address_mode_u: address,
        address_mode_v: address,
        lod_max_clamp: 0.0,
        ..Default::default()
    }
}

impl HudRenderer {
    pub(super) fn draw_picture(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        id: u64,
        range: std::ops::Range<u32>,
        coverage: bool,
    ) {
        if let Some((_, group)) = self.pictures.get(&id) {
            pass.set_bind_group(0, group, &[]);
            pass.set_pipeline(self.picture_pipeline.get(coverage));
            pass.draw(range, 0..1);
            pass.set_bind_group(0, &self.group, &[]);
        }
    }

    /// The bind group `picture_pipeline` draws a standalone texture `view` with, sampled by
    /// `sampler`.
    fn picture_group(
        &self,
        device: &wgpu::Device,
        view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("GUI picture"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&self.atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&self.atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&self.atlas_view),
                },
            ],
        })
    }

    pub(super) fn draw_dynamic(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        handle: u64,
        range: std::ops::Range<u32>,
        coverage: bool,
    ) {
        if let Some(group) = self
            .dynamic_groups
            .get(&handle)
            .map(|(_, group)| group)
            .or(self.dynamic_missing.as_ref())
        {
            pass.set_bind_group(0, group, &[]);
            pass.set_pipeline(self.picture_pipeline.get(coverage));
            pass.draw(range, 0..1);
            pass.set_bind_group(0, &self.group, &[]);
        }
    }

    /// Bind groups for the dynamic textures `batch` draws, made again for a texture that was
    /// made again; those no run of this frame draws are dropped (the textures stay in the
    /// store). Sampled nearest with repeat, at level 0 only, however many levels the texture has:
    /// `DynamicTexture`'s constructor.
    pub(super) fn prepare_dynamic(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut crate::gfx::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        batch: &HudBatch,
    ) {
        let mut drawn = std::mem::take(&mut self.dynamic_drawn);
        drawn.clear();
        drawn.extend(batch.blends.iter().filter_map(|(_, blend)| match blend {
            Blend::Dynamic(handle) => Some(*handle),
            _ => None,
        }));
        drawn.sort_unstable();
        drawn.dedup();
        self.dynamic_groups
            .retain(|handle, _| drawn.binary_search(handle).is_ok());
        let mut changed = std::mem::take(&mut self.dynamic_changed);
        changed.clear();
        self.dynamic
            .changed(&drawn, &self.dynamic_groups, &mut changed);
        let mut shared = None;
        for (handle, found) in changed.drain(..) {
            let Some((view, revision)) = found else {
                // Released since the draw was submitted: vanilla's missing texture.
                self.dynamic_groups.remove(&handle);
                if self.dynamic_missing.is_none() {
                    let (width, height, pixels) = missing_image();
                    let texture = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("Missing dynamic texture"),
                        size: wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    });
                    uploads.texture(encoder, texture.as_image_copy(), width, height, &pixels);
                    let view = texture.create_view(&Default::default());
                    let sampler = device.create_sampler(&sampler(TextureFlags::default()));
                    self.dynamic_missing = Some(self.picture_group(device, &view, &sampler));
                }
                continue;
            };
            let shared = shared
                .get_or_insert_with(|| device.create_sampler(&sampler(TextureFlags::default())));
            let group = self.picture_group(device, &view, shared);
            self.dynamic_groups.insert(handle, (revision, group));
        }
        self.dynamic_drawn = drawn;
        self.dynamic_changed = changed;
    }

    pub(super) fn prepare_pictures(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut crate::gfx::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        batch: &HudBatch,
    ) {
        // A picture this frame doesn't draw keeps its texture while something besides the renderer
        // holds it: a standalone texture, which its atlas holds until the next resource reload, so
        // returning to its screen uploads nothing. A Realms picture, held weakly, is released as
        // soon as no frame holds it.
        self.pictures.retain(|id, (picture, _)| {
            Arc::strong_count(picture) > 1 || batch.pictures.iter().any(|p| p.id == *id)
        });
        for picture in &batch.pictures {
            if picture.width > device.limits().max_texture_dimension_2d
                || picture.height > device.limits().max_texture_dimension_2d
            {
                continue;
            }
            if self.pictures.contains_key(&picture.id) {
                continue;
            }
            let entry = {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("GUI picture"),
                    size: wgpu::Extent3d {
                        width: picture.width,
                        height: picture.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                uploads.texture(
                    encoder,
                    texture.as_image_copy(),
                    picture.width,
                    picture.height,
                    &picture.pixels,
                );
                let view = texture.create_view(&Default::default());
                let sampler = device.create_sampler(&sampler(picture.flags));
                let group = self.picture_group(device, &view, &sampler);
                (picture.clone(), group)
            };
            self.pictures.insert(picture.id, entry);
        }
    }
}

#[cfg(test)]
mod tests;
