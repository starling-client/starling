//! A shader pack's shadow buffers (spec 6.3): the shadow map's depth (`shadowtex0`, and
//! `shadowtex1` without translucent geometry), and the colour buffers `shadowcolor0` to `7`, each
//! with two sides that `shadowcomp` programs flip as composite programs flip colortex (6.1.6).
//! Square, `shadowMapResolution` pixels.

use super::pack_post::SINK;
use sunrise_shaders::SHADOW_COLORS;

type Ops<T> = wgpu::Operations<T>;
type Color = wgpu::Color;

/// The largest shadow map side this device makes, of `device_max`: at most 4096 on a phone. An
/// 8192 map is 256 MiB of depth alone, more than a phone can spare beside the world; 4096 is
/// what the pack gets instead. Adapted from Vasysik/sunrise-android (`max_shadow` in its
/// `gfx/pack_shadow.rs` and `gfx/native_shadow.rs`), MIT, copyright (c) 2026 Ludovic Marie and
/// the Horizon contributors.
#[cfg(target_os = "android")]
pub fn shadow_side_limit(device_max: u32) -> u32 {
    device_max.min(4096)
}

/// The largest shadow map side this device makes: `device_max` here (see the Android one).
#[cfg(not(target_os = "android"))]
pub fn shadow_side_limit(device_max: u32) -> u32 {
    device_max
}

/// Loaded and stored.
pub fn load<T>() -> Ops<T> {
    wgpu::Operations {
        load: wgpu::LoadOp::Load,
        store: wgpu::StoreOp::Store,
    }
}

fn clear(c: [f32; 4]) -> Ops<Color> {
    wgpu::Operations {
        load: wgpu::LoadOp::Clear(wgpu::Color {
            r: c[0].into(),
            g: c[1].into(),
            b: c[2].into(),
            a: c[3].into(),
        }),
        store: wgpu::StoreOp::Store,
    }
}

/// What the shadow pass writes and the programs read.
pub struct ShadowTargets {
    pub size: u32,
    /// `shadowtex0`, written by the shadow pass.
    pub depth: (wgpu::Texture, wgpu::TextureView),
    /// `shadowtex1`: a copy of `shadowtex0` taken before translucent terrain, when read.
    pub depth1: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// Both sides of each shadowcolor buffer used; one texture for both when no `shadowcomp`
    /// program draws into or flips the buffer.
    pub color: [Option<[wgpu::TextureView; 2]>; SHADOW_COLORS],
    /// The copy of each buffer a pass samples while binding it as `shadowcolorimgN`, or whose
    /// copy a dispatch's loads read: taken from "main" just before that pass.
    pub scratch: [Option<wgpu::TextureView>; SHADOW_COLORS],
    /// What a `shadowcomp` program that writes only images draws into: the shadow map's size,
    /// so its fragments cover the buffers' texels.
    pub sink: Option<wgpu::TextureView>,
    /// The clear colour of each buffer cleared every frame.
    pub clears: [Option<[f32; 4]>; SHADOW_COLORS],
    /// Bit n set: B is shadowcolor n's "main".
    pub main: u16,
    /// Nothing cleared yet this frame: the first pass into the shadow buffers clears them as it
    /// begins (`clear_ops`), which saves a pass that loads and stores them all on tiled GPUs.
    fresh: bool,
    /// Bit n set: shadowcolor n was cleared that way.
    cleared: u8,
    /// Depth comparison for `sampler2DShadow` (hardware filtering): linear, as GL filters it.
    pub comparison: wgpu::Sampler,
}

/// How the pack uses the shadowcolor buffers, a mask each (bit n: shadowcolor n).
#[derive(Clone, Copy, Default)]
pub struct ShadowUse {
    /// Used at all.
    pub colors: u16,
    /// Needs a B texture.
    pub doubled: u16,
    /// Bound as `shadowcolorimgN`.
    pub storage: u16,
    /// Has a `scratch` copy.
    pub copied: u16,
    /// A `shadowcomp` program writes only images: the pass has a `sink`.
    pub sink: bool,
}

fn target(
    device: &wgpu::Device,
    label: &str,
    size: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | usage,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

/// The depth stand-in is initialized to no shadow on the next pack render.
pub fn stand_in(device: &wgpu::Device) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("Shader pack shadow stand-in"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

impl ShadowTargets {
    pub fn reset_history(&mut self, device: &wgpu::Device, shadow: &sunrise_shaders::Shadow) {
        let mut uses = ShadowUse::default();
        let formats = std::array::from_fn(|n| match &self.color[n] {
            Some([a, b]) => {
                uses.colors |= 1 << n;
                if a.texture() != b.texture() {
                    uses.doubled |= 1 << n;
                }
                if (a.texture().usage()).contains(wgpu::TextureUsages::STORAGE_BINDING) {
                    uses.storage |= 1 << n;
                }
                if self.scratch[n].is_some() {
                    uses.copied |= 1 << n;
                }
                a.texture().format()
            }
            None => wgpu::TextureFormat::Rgba8Unorm,
        });
        uses.sink = self.sink.is_some();
        *self = Self::new(device, shadow, formats, uses, self.depth1.is_some());
    }

    /// The buffers of `shadow` the pack uses (`uses`), and `depth1` when some program reads
    /// `shadowtex1`. The resolution is kept within the device's limit.
    pub fn new(
        device: &wgpu::Device,
        shadow: &sunrise_shaders::Shadow,
        formats: [wgpu::TextureFormat; SHADOW_COLORS],
        uses: ShadowUse,
        depth1: bool,
    ) -> Self {
        let ShadowUse {
            colors,
            doubled,
            storage,
            copied,
            sink,
        } = uses;
        let size = shadow.resolution.clamp(
            1,
            shadow_side_limit(device.limits().max_texture_dimension_2d),
        );
        let copy = wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST;
        let depth = wgpu::TextureFormat::Depth32Float;
        // A buffer bound as an image, as colortex buffers are (`PackPost::resize`).
        let image = |n: usize| {
            if storage & 1 << n == 0 {
                return wgpu::TextureUsages::empty();
            }
            let atomic = device.features().contains(wgpu::Features::TEXTURE_ATOMIC)
                && matches!(
                    formats[n],
                    wgpu::TextureFormat::R32Sint | wgpu::TextureFormat::R32Uint
                );
            if atomic {
                wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::STORAGE_ATOMIC
            } else {
                wgpu::TextureUsages::STORAGE_BINDING
            }
        };
        let color = std::array::from_fn(|n| {
            (colors & 1 << n != 0).then(|| {
                let copy_src = if copied & 1 << n != 0 {
                    wgpu::TextureUsages::COPY_SRC
                } else {
                    wgpu::TextureUsages::empty()
                };
                let side = |side: &str| {
                    let label = format!("Shader pack shadowcolor{n} {side}");
                    target(device, &label, size, formats[n], image(n) | copy_src).1
                };
                let a = side("A");
                let b = if doubled & 1 << n != 0 {
                    side("B")
                } else {
                    a.clone()
                };
                [a, b]
            })
        });
        let scratch = std::array::from_fn(|n| {
            (colors & copied & 1 << n != 0).then(|| {
                let label = format!("Shader pack shadowcolor{n} copy");
                let usage = image(n) | wgpu::TextureUsages::COPY_DST;
                target(device, &label, size, formats[n], usage).1
            })
        });
        Self {
            size,
            depth: target(device, "Shader pack shadowtex0", size, depth, copy),
            depth1: depth1.then(|| target(device, "Shader pack shadowtex1", size, depth, copy)),
            color,
            scratch,
            sink: sink.then(|| {
                let usage = wgpu::TextureUsages::empty();
                target(device, "Shader pack shadow sink", size, SINK, usage).1
            }),
            clears: std::array::from_fn(|n| {
                let b = &shadow.colors[n];
                b.clear.then(|| b.clear_color.unwrap_or([1.0; 4]))
            }),
            main: 0,
            fresh: false,
            cleared: 0,
            comparison: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Shader pack shadow comparison"),
                compare: Some(wgpu::CompareFunction::LessEqual),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
        }
    }

    /// The shadowcolor buffers not cleared every frame (bit n: shadowcolor n): what a pass
    /// writes there builds on the frame before.
    pub fn kept(&self) -> u16 {
        (0..SHADOW_COLORS)
            .filter(|&n| self.clears[n].is_none())
            .fold(0, |m, n| m | 1 << n)
    }

    /// Side `alt` (of "main" and "alt") of shadowcolor `n`.
    pub fn color(&self, n: usize, alt: bool) -> Option<&wgpu::TextureView> {
        let side = usize::from(self.main & 1 << n != 0) ^ usize::from(alt);
        self.color.get(n)?.as_ref().map(|c| &c[side])
    }

    /// A new frame: the shadow map and the shadowcolor buffers cleared every frame (6.3) are
    /// cleared by the first pass that begins (`clear_ops`), or else by `clear_rest`.
    pub fn begin_frame(&mut self) {
        self.fresh = true;
        self.cleared = 0;
    }

    /// The load operations of a pass into the shadow map and shadowcolor buffers `writes`:
    /// clears in the frame's first one, loads after.
    pub fn clear_ops(
        &mut self,
        writes: impl Iterator<Item = usize>,
    ) -> (Ops<f32>, [Ops<Color>; SHADOW_COLORS]) {
        if !std::mem::take(&mut self.fresh) {
            return (load(), [load(); SHADOW_COLORS]);
        }
        let mut colors = [load(); SHADOW_COLORS];
        for n in writes.filter(|&n| n < SHADOW_COLORS) {
            if let Some(c) = self.clears[n] {
                colors[n] = clear(c);
                self.cleared |= 1 << n;
            }
        }
        let depth = wgpu::Operations {
            load: wgpu::LoadOp::Clear(1.0),
            store: wgpu::StoreOp::Store,
        };
        (depth, colors)
    }

    /// Clears what no pass has cleared this frame: all of it when no shadow program drew, the
    /// cleared shadowcolor buffers the first one did not write.
    pub fn clear_rest(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let depth = std::mem::take(&mut self.fresh);
        let cleared = std::mem::replace(&mut self.cleared, u8::MAX);
        self.clear(encoder, !cleared, depth);
    }

    /// Clears the shadowcolor buffers `images` (bit n: shadowcolor n) not cleared yet this
    /// frame, before a shadow program binds them as images: the first pass clears only what it
    /// draws into, and `clear_rest` after it would wipe its stores.
    pub fn clear_images(&mut self, encoder: &mut wgpu::CommandEncoder, images: u16) {
        let buffers = images as u8 & !self.cleared;
        self.cleared |= buffers;
        self.clear(encoder, buffers, false);
    }

    /// Clears the shadowcolor buffers `buffers` cleared every frame, and the shadow map with
    /// `depth`. One buffer a pass: eight wide formats together would pass the device's
    /// bytes-per-sample limit.
    fn clear(&self, encoder: &mut wgpu::CommandEncoder, buffers: u8, mut depth: bool) {
        let attachments = (0..SHADOW_COLORS)
            .filter(|n| buffers & 1 << n != 0)
            .filter_map(|n| {
                Some(Some(wgpu::RenderPassColorAttachment {
                    view: self.color(n, false)?,
                    depth_slice: None,
                    resolve_target: None,
                    ops: clear(self.clears[n]?),
                }))
            });
        let passes: Vec<Vec<_>> = attachments.map(|a| vec![a]).collect();
        let passes = if passes.is_empty() && depth {
            vec![Vec::new()]
        } else {
            passes
        };
        for attachments in &passes {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shader pack shadow clear"),
                color_attachments: attachments,
                depth_stencil_attachment: std::mem::take(&mut depth).then_some(
                    wgpu::RenderPassDepthStencilAttachment {
                        view: &self.depth.1,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    },
                ),
                ..Default::default()
            });
        }
    }

    /// `shadowtex1`: the shadow map as it is now (before translucent geometry).
    pub fn copy_depth1(&self, encoder: &mut wgpu::CommandEncoder) {
        if let Some((copy, _)) = &self.depth1 {
            encoder.copy_texture_to_texture(
                self.depth.0.as_image_copy(),
                copy.as_image_copy(),
                self.depth.0.size(),
            );
        }
    }
}
