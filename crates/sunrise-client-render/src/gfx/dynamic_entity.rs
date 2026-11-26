//! Entity quads that sample a client mod's dynamic texture
//! (`BlockEntityDrawContext::textured_quad`).
//!
//! Each quad names its texture by handle in its vertices (`DYNAMIC_TEXTURE_*_OVERLAY`,
//! `dynamic_texture_handle`), so the batch's regrouping by material keeps the name with the
//! quad. A frame reads the runs of consecutive quads of one texture off the section the batch
//! made of them, resolves each handle against the renderer's store of dynamic textures on the
//! render thread, and draws every run with a bind group of its texture. A texture written to
//! shows on the next frame with no new geometry; one made again (a new size) has a new
//! revision, so its group is made again; one released after its quad was submitted has no view
//! and draws vanilla's missing texture (`TextureManager.release`, `getTexture`).
use super::Vertex;
use crate::hud::{dynamic_textures::Store, missing_image};
use std::collections::HashMap;
use sunrise_render_core::entity::{DynamicTextureRun, dynamic_texture_runs};

/// What an entity bind group is made of besides the texture it samples.
pub(super) struct DynamicEntity {
    layout: wgpu::BindGroupLayout,
    globals: wgpu::Buffer,
    lightmap: (wgpu::TextureView, wgpu::Sampler),
    glint: (wgpu::TextureView, wgpu::Sampler),
    glint_armor: wgpu::TextureView,
    /// `DynamicTexture`'s constructor: nearest, repeating.
    sampler: wgpu::Sampler,
    /// The bind group of each drawn texture and the revision of the texture it was made for.
    groups: HashMap<u64, (u64, wgpu::BindGroup)>,
    /// What a texture with no view draws as, made when first needed: its view and the group
    /// sampling it.
    missing: Option<(wgpu::TextureView, wgpu::BindGroup)>,
    /// This frame's runs, with `first` counted in quads from the start of the entity buffer.
    runs: Vec<DynamicTextureRun>,
    /// Reused by every frame's `prepare`.
    drawn: Vec<u64>,
    changed: Vec<(u64, Option<(wgpu::TextureView, u64)>)>,
}

impl DynamicEntity {
    pub(super) fn new(
        device: &wgpu::Device,
        layout: wgpu::BindGroupLayout,
        globals: wgpu::Buffer,
        lightmap: (wgpu::TextureView, wgpu::Sampler),
        glint: (wgpu::TextureView, wgpu::Sampler),
        glint_armor: wgpu::TextureView,
    ) -> Self {
        Self {
            layout,
            globals,
            lightmap,
            glint,
            glint_armor,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Dynamic texture quads"),
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                ..Default::default()
            }),
            groups: HashMap::new(),
            missing: None,
            runs: Vec::new(),
            drawn: Vec::new(),
            changed: Vec::new(),
        }
    }

    /// The entity bind group (`renderer_construction`'s `entity_group_for`) sampling `view`.
    fn group(&self, device: &wgpu::Device, view: &wgpu::TextureView) -> wgpu::BindGroup {
        let entry = |binding, resource| wgpu::BindGroupEntry { binding, resource };
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Dynamic texture quads"),
            layout: &self.layout,
            entries: &[
                entry(0, self.globals.as_entire_binding()),
                entry(1, wgpu::BindingResource::TextureView(view)),
                entry(2, wgpu::BindingResource::Sampler(&self.sampler)),
                entry(3, wgpu::BindingResource::TextureView(&self.lightmap.0)),
                entry(4, wgpu::BindingResource::Sampler(&self.lightmap.1)),
                entry(5, wgpu::BindingResource::TextureView(&self.glint.0)),
                entry(6, wgpu::BindingResource::Sampler(&self.glint.1)),
                entry(7, wgpu::BindingResource::TextureView(&self.glint_armor)),
            ],
        })
    }

    /// This frame's runs of `section`, the dynamic-texture quads the batch grouped, which start
    /// at quad `first` of the entity buffer; and a bind group for each texture they draw, made
    /// again for one that was made again. Groups no run draws are dropped (the textures stay in
    /// the store).
    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        store: &Store,
        section: &[Vertex],
        first: u32,
    ) {
        self.runs.clear();
        self.runs
            .extend(dynamic_texture_runs(section).map(|run| DynamicTextureRun {
                first: first + run.first,
                ..run
            }));
        let mut drawn = std::mem::take(&mut self.drawn);
        drawn.clear();
        drawn.extend(self.runs.iter().map(|run| run.handle));
        drawn.sort_unstable();
        drawn.dedup();
        self.groups
            .retain(|handle, _| drawn.binary_search(handle).is_ok());
        let mut changed = std::mem::take(&mut self.changed);
        changed.clear();
        store.changed(&drawn, &self.groups, &mut changed);
        for (handle, found) in changed.drain(..) {
            let Some((view, revision)) = found else {
                // Released since the quad was submitted: vanilla's missing texture.
                self.groups.remove(&handle);
                self.missing_view(device, uploads, encoder);
                continue;
            };
            let group = self.group(device, &view);
            self.groups.insert(handle, (revision, group));
        }
        self.drawn = drawn;
        self.changed = changed;
    }

    /// Vanilla's missing texture (`MissingTextureAtlasSprite.generateMissingImage`), made when
    /// first needed.
    fn missing_view(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
    ) -> &wgpu::TextureView {
        if self.missing.is_none() {
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
            let group = self.group(device, &view);
            self.missing = Some((view, group));
        }
        &self.missing.as_ref().expect("made above").0
    }

    /// The view and revision of the dynamic texture `handle` for a shader pack's entity draw: the
    /// missing texture's, at a revision no texture has, when it no longer exists.
    pub(super) fn view(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        store: &Store,
        handle: u64,
    ) -> (wgpu::TextureView, u64) {
        store.view(handle).unwrap_or_else(|| {
            (
                self.missing_view(device, uploads, encoder).clone(),
                u64::MAX,
            )
        })
    }

    /// Draws this frame's runs of `overlay` (`DYNAMIC_TEXTURE_*_OVERLAY`) with the pipeline and
    /// vertex buffer bound: one bind group and one indexed draw a run.
    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, overlay: u32) {
        for run in self.runs.iter().filter(|run| run.overlay == overlay) {
            let Some(group) = self
                .groups
                .get(&run.handle)
                .map(|(_, group)| group)
                .or(self.missing.as_ref().map(|(_, group)| group))
            else {
                continue;
            };
            pass.set_bind_group(0, group, &[]);
            pass.draw_indexed(0..run.quads * 6, (run.first * 4) as i32, 0..1);
        }
    }
}
