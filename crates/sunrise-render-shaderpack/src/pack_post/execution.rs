//! The frame's work: the centre depth the finished frame copied, `reset_history`, the passes
//! (bind, dispatch and draw, and the flip of each colortex buffer a pass wrote), the resize of
//! `Targets`, the shadow pass and its cache, and `run`, which puts them in order.
//!
//! These are `PackPost`'s inherent methods. The frame inputs come from the decoupled
//! `sunrise_render_core::UniformFrameView` and its own scalars, so the client's `gfx::Frame`
//! never crosses into this crate (its `run`, `shadow_view` and `prepare_entities` shims build
//! them).

use glam::{DMat4, DVec3};
use smallvec::SmallVec;
use sunrise_render_core::UniformFrameView;
use sunrise_shaders::{BUFFERS, Group, SHADOW_COLORS};

use super::construction::*;
use crate::PollLive;
use crate::pack_hand;
use crate::pack_images;
use crate::pack_sky::SkyFrame;
use crate::pack_textures;
use crate::pack_uniforms::{self, PackInputs};

/// The frame's sky inputs, decoupled from the client's `Frame`: what `draw_sky` needs to build a
/// `SkyFrame` (`view` gives the sun, colours and fog).
#[derive(Clone, Copy)]
pub struct SkyInputs<'a> {
    /// The sky's sunset colour (`Sky::sunrise_color`).
    pub sunset_color: [f32; 4],
    /// The stars' rotation and brightness.
    pub stars: Option<(glam::Mat4, f32)>,
    /// The camera is low enough for the void disc.
    pub dark_disc: bool,
    /// Camera-relative sun and moon quads, each four `sunrise_render_core::Vertex`.
    pub celestial: &'a [sunrise_render_core::EntityVertex],
    /// What each quad of `celestial` is (`Frame::celestial_bodies`).
    pub celestial_bodies: &'a [crate::CelestialBody],
    /// The Overworld-style sky draws this frame (`sky_visible`).
    pub visible: bool,
    /// The End's fan draws though vanilla's sky does not (`end_fog_sky`).
    pub end_fog_sky: bool,
}

/// The frame's first-person hand inputs, decoupled from the client's `Frame`: the vertices, the
/// item runs, its own field of view, and the nausea spin the world turns with (`bob`, `spin`) so
/// its projection can take it out.
pub struct HandInputs<'a> {
    pub vertices: &'a [sunrise_render_core::EntityVertex],
    pub items: &'a [(u32, i32)],
    pub fov_degrees: f32,
    pub bob: glam::Mat4,
    pub spin: glam::Mat4,
}

impl PackPost {
    pub fn take_center_depth(&mut self) -> Result<(), String> {
        self.center_depth
            .as_mut()
            .ok_or("the shader pack copied no centre depth")?
            .take_completed()
            .map(|_| ())
    }

    /// Forgets what the pack kept from earlier frames, so the next frame draws as a freshly
    /// loaded pack's first: buffers, images, storage buffers and shadow maps start over (made
    /// again at their sizes when next drawn), the frame uniforms' history, the custom uniforms
    /// and the centre depth too. Compiled programs and the pack's textures stay.
    pub fn reset_history(&mut self, device: &wgpu::Device) {
        self.uniforms.reset_history();
        // A parsed pack's custom uniforms start at tick 0's seed, before any frame of the world.
        self.custom.restart_at(0);
        self.custom.forget_world();
        self.center_depth = None;
        self.targets = None;
        self.main = 0;
        self.mips_valid = 0;
        self.rgb_unset = true;
        self.final_target = None;
        self.blit_group = None;
        self.page_groups.clear();
        for image in &mut self.images {
            image.reset_history();
        }
        self.buffers.reset_history();
        pack_images::reset_stand_ins(device, &mut self.stand_ins);
        if let Some(shadow) = &mut self.shadow_targets {
            shadow.reset_history(device, &self.uniforms.shadow);
        }
        self.shadow.0 = crate::pack_shadow::stand_in(device);
        self.shadow_cleared = false;
        self.shadow_cache = Default::default();
        self.setup_pending = true;
        self.portal_texture = None;
        self.portal_rect = [0.0; 4];
        let sky = self
            .sky
            .iter_mut()
            .flat_map(|s| s.basic.iter_mut().chain(&mut s.textured));
        let entities = self
            .entities
            .iter_mut()
            .flat_map(|e| e.passes.iter_mut().chain(&mut e.shadow).flatten());
        for pass in self
            .passes
            .iter_mut()
            .chain(&mut self.terrain)
            .chain(&mut self.terrain_cutout)
            .chain(&mut self.water)
            .chain(sky)
            .chain(entities)
            .chain(&mut self.clouds)
            .chain(&mut self.lines)
            .chain(self.weather.iter_mut().flatten())
            .chain(self.hand.iter_mut().map(|h| &mut h.0))
            .chain(&mut self.shadow_terrain)
            .chain(&mut self.shadow_water)
            .chain(&mut self.shadow_comp)
        {
            pass.reset_history();
        }
    }
    /// Splits `entities` (`Frame::entities`) between the pack and Sunrise by its runs
    /// (`spans`): the pack's part is uploaded for this frame's draw, Sunrise's goes to `rest`.
    pub fn skins_entities(&self) -> bool {
        self.entities
            .as_ref()
            .is_some_and(|e| e.passes[..3].iter().all(Option::is_some))
    }

    pub fn skin_target(&self) -> Option<&wgpu::Buffer> {
        self.entities.as_ref().map(|e| e.geometry.skin_target())
    }

    /// Which entity runs the shadow pass draws: none without it.
    pub fn shadow_casters(&self) -> crate::pack_entities::Casters {
        let Some(e) = self
            .entities
            .as_ref()
            .filter(|_| self.shadow_targets.is_some())
        else {
            return Default::default();
        };
        crate::pack_entities::Casters {
            entities: e.shadow[0].is_some(),
            only_player: e.only.0,
            blocks: e.shadow[1].is_some(),
            only_lit: e.only.1,
        }
    }

    /// The centre depth the frame copied: its readback, started by calling it right after the
    /// frame's submission.
    pub fn submitted(&mut self) -> Option<impl FnOnce() + Send + 'static> {
        self.center_depth.as_mut()?.submitted()
    }

    /// Whether the pack draws entities or block entities itself.
    pub fn has_entities(&self) -> bool {
        self.entities.is_some()
    }

    /// Whether the pack draws the block outline itself.
    pub fn has_lines(&self) -> bool {
        self.lines.is_some()
    }

    /// The block outline to draw this frame (`None`: none).
    pub fn set_lines(&mut self, lines: Option<Lines>) {
        self.line_draw = lines.filter(|l| l.count > 0);
    }

    /// Whether the pack draws the clouds itself (or hides them).
    pub fn has_clouds(&self) -> bool {
        self.hides_clouds
    }

    /// The clouds to draw this frame (`None`: none).
    pub fn set_clouds(&mut self, clouds: Option<Clouds>) {
        self.cloud_draw = clouds.filter(|c| c.quads > 0);
    }

    /// Whether the pack draws rain and snow itself.
    pub fn has_weather(&self) -> bool {
        self.weather.is_some()
    }

    /// The rain and snow to draw this frame (`None`: none).
    pub fn set_weather(&mut self, weather: Option<Weather>) {
        self.weather_draw = weather;
    }

    pub fn has_terrain(&self) -> bool {
        self.terrain.is_some() || self.terrain_cutout.is_some() || self.terrain_undrawn
    }

    /// Whether the pack's shadow pass draws terrain (`shadow_view`).
    pub fn has_shadow(&self) -> bool {
        self.shadow_targets.is_some()
            && (self.shadow_terrain.is_some() || self.shadow_water.is_some())
    }

    /// Whether the pack draws the sky (disc, sunset, stars, void) itself.
    /// What the world's background is when the pack draws the sky: colortex0's clear colour
    /// (`None`: the fog colour, its default).
    pub fn sky_clear(&self) -> Option<[f32; 4]> {
        self.sky_clear.filter(|_| self.has_sky())
    }

    /// Whether the pack has a sky program, drawing or not: vanilla's sky is then not drawn.
    pub fn has_sky(&self) -> bool {
        self.hides_sky
    }

    /// Whether the pack has the sun and moon's program, drawing or not.
    pub fn has_celestial(&self) -> bool {
        self.hides_celestial
    }

    /// `sky`, `stars`, `sun` and `moon`: the parts of the vanilla sky the pack leaves out, which
    /// the game's sky leaves out too when the pack has no program for them.
    pub fn sky_switches(&self) -> sunrise_shaders::SkySwitches {
        self.sky_switches
    }

    /// Whether the pack draws translucent terrain itself.
    pub fn has_water(&self) -> bool {
        self.water.is_some() || self.water_undrawn
    }

    /// The names of the programs that run: the full-screen ones in order, then those that draw
    /// the world. A pack may have only these (Round World).
    pub fn programs(&self) -> Vec<&str> {
        let sky = self
            .sky
            .iter()
            .flat_map(|s| s.basic.iter().chain(&s.textured));
        let entities = self.entities.iter().flat_map(|e| e.passes.iter().flatten());
        self.passes
            .iter()
            .chain(&self.terrain)
            .chain(&self.water)
            .chain(sky)
            .chain(entities)
            .chain(&self.clouds)
            .chain(self.weather.iter().flatten())
            .chain(self.hand.iter().map(|h| &h.0))
            .map(|p| p.name.as_str())
            .collect()
    }

    /// The programs that run and the notes of loading them.
    pub fn report(&self) -> (Vec<String>, Vec<String>) {
        let programs = self.programs().into_iter().map(str::to_owned).collect();
        (programs, self.notes.clone())
    }

    /// Whether the pack draws the first-person hand itself.
    pub fn has_hand(&self) -> bool {
        self.hand.is_some()
    }

    /// Runs the pack over the world drawn into `world` (colour) and `world_depth`, then draws
    /// the result onto `screen`.
    pub fn run_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        world: &wgpu::TextureView,
        world_depth: &wgpu::TextureView,
        screen: &wgpu::TextureView,
        frame_view: &UniformFrameView<'_>,
        sky: &SkyInputs<'_>,
        hand: &HandInputs<'_>,
        inputs: &PackInputs,
        size: [u32; 2],
        window: [u32; 2],
        pages: &[TerrainPage],
        terrain: &mut TerrainDraw<'_>,
        shadow_cache: bool,
        shadow_scene_revision: (u64, u64),
        ticks: u64,
        day_time: i64,
        lost: &std::sync::atomic::AtomicBool,
        poll: PollLive,
    ) -> Result<(), String> {
        self.resize(device, size, lost, poll)?;
        // Last frame's water patches (or a new texture's zeros) out: depth 1 is no patch.
        if self.water_mask_dirty
            && let Some((_, view)) = self.targets.as_ref().and_then(|t| t.water_mask.as_ref())
        {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shader pack water patches clear"),
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            self.water_mask_dirty = false;
        }
        // A resolution scale leaves the picture smaller (or larger) than the window: the pack's
        // `final` draws it at the picture's size, and one copy takes it to the window (#9291).
        let final_view = if size != window && self.passes.iter().any(|p| p.group == Group::Final) {
            if self.final_target.as_ref().is_none_or(|t| t.size != size) {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Shader pack final"),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.screen,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                self.final_target = Some(FinalTarget {
                    size,
                    view: texture.create_view(&Default::default()),
                });
            }
            self.final_target.as_ref().map(|t| t.view.clone())
        } else {
            self.final_target = None;
            None
        };
        if let Some((layout, ids, rects)) = &self.block_ids {
            // A page that grew has a new buffer.
            self.page_groups
                .retain(|p, (b, s, _)| pages.iter().any(|(q, c, t)| q == p && c == b && s == t));
            for (page, buffer, sources) in pages {
                self.page_groups.entry(*page).or_insert_with(|| {
                    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Shader pack terrain"),
                        layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: sources.as_ref().unwrap_or(ids).as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: rects.as_entire_binding(),
                            },
                        ],
                    });
                    (buffer.clone(), sources.clone(), group)
                });
            }
        }
        if let Some(t) = &mut self.timing {
            t.begin(device, queue, lost, poll)?;
        }
        self.uniforms.center_depth_raw = self
            .center_depth
            .get_or_insert_with(|| crate::pack_center_depth::CenterDepth::new(device))
            .collect();
        self.uniforms.update(frame_view, inputs, size);
        // `smooth` starts on the first frame that knows the camera's biome.
        self.custom.world_frame(inputs.biome_known);
        self.uniforms.eval_custom(&mut self.custom);
        if self.targets.is_none() {
            return Ok(());
        }
        // Images start the frame cleared, before the shadow pass writes them.
        if let Some(dump) = &mut self.dump {
            dump.read(device, lost, poll)?;
            dump.copy(device, encoder, &self.images);
        }
        pack_images::clear(
            device,
            encoder,
            &self.images,
            self.gpu.clear,
            &mut self.zeros,
        );
        let Some(targets) = &self.targets else {
            return Ok(());
        };
        // Every frame starts on A. A buffer that isn't cleared and ended the last frame on B has B
        // copied back to A, and B keeps what was last written into it: a model that fits Iris 1.11.7's
        // values in probe packs of our own (#9009), not a documented Iris rule. colortex0's A is
        // overwritten by the world below.
        for b in (1..BUFFERS).filter(|&b| self.main & 1 << b != 0 && self.clears[b].is_none()) {
            if let Some([a, b_side]) = &targets.color[b] {
                encoder.copy_texture_to_texture(
                    b_side.texture().as_image_copy(),
                    a.texture().as_image_copy(),
                    a.texture().size(),
                );
            }
        }
        self.main = 0;
        let main = |m: u16, i: usize| usize::from(m & 1 << i != 0);
        let view = |m: u16, i: usize, alt: bool| {
            targets.color[i]
                .as_ref()
                .map(|t| &t[main(m, i) ^ usize::from(alt)])
        };

        if !self.shadow_cleared {
            self.shadow_cleared = true;
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shader pack shadow stand-in"),
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow.0,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
        }
        // Three-channel buffers start out with alpha 1 on both sides: a pass writes the side the
        // frame's clear didn't touch.
        if self.rgb_unset {
            self.rgb_unset = false;
            let start = wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            };
            for b in (0..BUFFERS).filter(|&b| self.rgb & 1 << b != 0) {
                for alt in [false, true]
                    .into_iter()
                    .take(1 + usize::from(self.doubled & 1 << b != 0))
                {
                    if let Some(v) = view(self.main, b, alt) {
                        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("Shader pack three-channel start"),
                            color_attachments: &[attach(v, start)],
                            ..Default::default()
                        });
                    }
                }
            }
        }
        let clear_op = |b: usize| {
            let c = self.clears[b].unwrap_or_default();
            wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: c[0].into(),
                    g: c[1].into(),
                    b: c[2].into(),
                    a: c[3].into(),
                }),
                store: wgpu::StoreOp::Store,
            }
        };
        // Overwritten whole: clearing lets a tiled GPU skip loading the old content.
        let overwrite = wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            store: wgpu::StoreOp::Store,
        };
        // Attachments share a size and stay within the device's count and byte limits.
        let limits = device.limits();
        let max_attachments = (limits.max_color_attachments as usize).min(8);
        // The buffers the blit doesn't clear, and the other texture of every cleared buffer.
        let alt_clears = (0..BUFFERS)
            .filter(|&b| self.clears[b].is_some() && self.doubled & 1 << b != 0)
            .filter(|&b| view(self.main, b, true).is_some())
            .map(|b| (b, true));
        let mut by_size: Vec<([u32; 2], Vec<(usize, bool)>)> = Vec::new();
        for (b, alt) in self
            .other_clears
            .iter()
            .map(|&b| (b, false))
            .chain(alt_clears)
        {
            let at = targets.sizes[b];
            match by_size.iter_mut().find(|(s, buffers)| {
                let bytes = buffers
                    .iter()
                    .fold(0, |bytes, &(b, _)| attachment_bytes(bytes, self.formats[b]));
                *s == at
                    && buffers.len() < max_attachments
                    && attachment_bytes(bytes, self.formats[b])
                        <= limits.max_color_attachment_bytes_per_sample
            }) {
                Some((_, l)) => l.push((b, alt)),
                None => by_size.push((at, vec![(b, alt)])),
            }
        }
        for (_, chunk) in &by_size {
            let attachments: Vec<_> = chunk
                .iter()
                .map(|&(b, alt)| attach(view(self.main, b, alt).expect("used buffer"), clear_op(b)))
                .collect();
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shader pack clear"),
                color_attachments: &attachments,
                timestamp_writes: self.timing.as_mut().and_then(|t| t.writes("clear")),
                ..Default::default()
            });
        }

        // The world into colortex0 and depthtex0, flipped to GL's rows, and the clears.
        if let Some(blit) = &self.blit
            && self
                .blit_group
                .as_ref()
                .is_none_or(|g| g.0[0] != *world || g.0[1] != *world_depth)
        {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Shader pack world"),
                layout: &blit.1,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(world),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(world_depth),
                    },
                ],
            });
            self.blit_group = Some(([world.clone(), world_depth.clone()], group));
        }
        if let Some(blit) = &self.blit {
            // The blit leaves a three-channel colortex0's alpha at the 1 it starts with.
            let world_ops = if self.rgb & 1 != 0 {
                wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                }
            } else {
                overwrite
            };
            let mut attachments = vec![if holds_floats(self.formats[0]) {
                attach(view(self.main, 0, false).expect("colortex0"), world_ops)
            } else {
                None
            }];
            for &b in &self.blit_clears {
                attachments.push(attach(
                    view(self.main, b, false).expect("used buffer"),
                    clear_op(b),
                ));
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shader pack world"),
                color_attachments: &attachments,
                timestamp_writes: self.timing.as_mut().and_then(|t| t.writes("world")),
                depth_stencil_attachment: self.blit_depth.then_some(
                    wgpu::RenderPassDepthStencilAttachment {
                        view: &targets.depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    },
                ),
                ..Default::default()
            });
            pass.set_pipeline(&blit.0);
            pass.set_bind_group(0, self.blit_group.as_ref().map(|g| &g.1), &[]);
            pass.draw(0..3, 0..1);
        }

        // The world and the clears just rewrote the buffers' first levels.
        self.mips_valid = 0;
        let mut passes = std::mem::take(&mut self.passes);
        let mut seen = (false, false);
        let mut shown = false;
        // A water-only pack still needs the opaque phase to populate requested snapshots.
        let mut terrain_drawn = self.terrain.is_none()
            && self.terrain_cutout.is_none()
            && self.sky.is_none()
            && !self.entities.as_ref().is_some_and(Entities::drawn)
            && self.clouds.is_none()
            && self.lines.is_none()
            && self.hand.is_none()
            && !self.depth1
            && !self.depth2;
        let late_entities = self.entities.as_ref().is_some_and(|e| {
            e.passes
                .iter()
                .zip(e.late)
                .any(|(p, late)| p.is_some() && late)
        });
        let mut water_drawn = self.water.is_none() && self.weather.is_none() && !late_entities;
        let mut shadow_drawn = false;
        for pass in &mut passes {
            if pass.group == Group::Setup && !self.setup_pending {
                continue;
            }
            let early = matches!(pass.group, Group::Setup | Group::Begin);
            if !early && !shadow_drawn {
                self.run_shadow(
                    device,
                    queue,
                    encoder,
                    terrain,
                    frame_view,
                    shadow_cache,
                    shadow_scene_revision,
                    ticks,
                    day_time,
                );
                shadow_drawn = true;
            }
            if !terrain_drawn && !early && pass.group != Group::Prepare {
                terrain_drawn = true;
                self.draw_opaque(device, queue, encoder, size, frame_view, sky, hand, terrain);
            }
            // As if the pack didn't have it: no compute stages, flips or stage changes.
            if self.sizes_differ(pass) {
                continue;
            }
            if !water_drawn && matches!(pass.group, Group::Composite | Group::Final) {
                water_drawn = true;
                self.draw_translucent(device, queue, encoder, size, terrain);
            }
            if pass.group == Group::Deferred && !seen.0 {
                seen.0 = true;
                self.main ^= self.deferred_pre;
            }
            if pass.group == Group::Composite && !seen.1 {
                seen.1 = true;
                self.main ^= self.composite_pre;
            }
            let mut compute = std::mem::take(&mut pass.compute);
            for c in &mut compute {
                self.dispatch(device, queue, encoder, c, size);
            }
            pass.compute = compute;
            if matches!(pass.pipeline, Pipeline::Compute { .. }) {
                self.dispatch(device, queue, encoder, pass, size);
                continue;
            }
            let stale = pass.mipmaps & self.mipped & !self.mips_valid;
            for b in (0..BUFFERS).filter(|b| stale & 1 << b != 0) {
                self.mipmap(encoder, b);
            }
            self.mips_valid |= stale;
            let final_pass = pass.group == Group::Final;
            // A pass writing buffers of a size of their own draws at that size (they share it).
            // `final` draws the picture at the world's size; a copy takes it to the window.
            let out = match pass.writes.first() {
                Some(&(_, b)) => self.targets.as_ref().map_or(size, |t| t.sizes[b]),
                None => size,
            };
            let group = self.bind(device, queue, encoder, pass, out);
            let targets = self.targets.as_ref().expect("resized");
            let view = |i: usize| {
                targets.color[i]
                    .as_ref()
                    .map(|t| &t[usize::from(self.main & 1 << i == 0)])
            };
            let mut attachments: SmallVec<[Option<wgpu::RenderPassColorAttachment>; BUFFERS]> =
                SmallVec::new();
            // Vertex coverage and fragment discard can leave alternate pixels untouched.
            // The unused sink can still skip loading its old contents.
            let ops = if !pass.sink || pass.viewport.is_some() {
                wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }
            } else {
                overwrite
            };
            if final_pass {
                attachments.push(attach(final_view.as_ref().unwrap_or(screen), overwrite));
                shown = true;
            } else if pass.sink {
                attachments.push(attach(&targets.sink, ops));
            } else {
                for &(location, buffer) in &pass.writes {
                    let l = location as usize;
                    if attachments.len() <= l {
                        attachments.resize(l + 1, None);
                    }
                    attachments[l] = view(buffer).and_then(|v| attach(v, ops));
                }
            }
            {
                let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(&pass.name),
                    color_attachments: &attachments,
                    timestamp_writes: self.timing.as_mut().and_then(|t| t.writes(&pass.name)),
                    ..Default::default()
                });
                rp.set_pipeline(pass.render());
                rp.set_bind_group(0, &group, &[]);
                if pass.buffers
                    && let Some(g) = &self.buffers.group
                {
                    rp.set_bind_group(4, g, &[]);
                }
                if let Some(scale) = pass.viewport {
                    let [x, y, w, h] = viewport(out, scale);
                    rp.set_viewport(x, y, w, h, 0.0, 1.0);
                }
                rp.draw(0..6, 0..1);
            }
            self.main ^= pass.flip;
            self.mips_valid &= !pass.written;
        }
        self.passes = passes;
        self.setup_pending = false;
        if !shadow_drawn {
            self.run_shadow(
                device,
                queue,
                encoder,
                terrain,
                frame_view,
                shadow_cache,
                shadow_scene_revision,
                ticks,
                day_time,
            );
        }
        if !terrain_drawn {
            self.draw_opaque(device, queue, encoder, size, frame_view, sky, hand, terrain);
        }
        if !water_drawn {
            self.draw_translucent(device, queue, encoder, size, terrain);
        }
        let targets = self.targets.as_ref().expect("resized");
        // depthtex0 as the frame leaves it, for next frames' `centerDepthSmooth`.
        if let Some(center) = &mut self.center_depth {
            center.copy(device, encoder, &targets.depth_texture);
        }
        let view = |m: u16, i: usize, alt: bool| {
            targets.color[i]
                .as_ref()
                .map(|t| &t[main(m, i) ^ usize::from(alt)])
        };
        // Without `final`, colortex0 holds floats (`new` refuses an integer one).
        if !shown {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Shader pack colortex0"),
                layout: &self.copy.1,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            view(self.main, 0, false).expect("colortex0"),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.linear),
                    },
                ],
            });
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shader pack colortex0"),
                color_attachments: &[attach(screen, overwrite)],
                ..Default::default()
            });
            rp.set_pipeline(&self.copy.0);
            rp.set_bind_group(0, &group, &[]);
            rp.draw(0..3, 0..1);
        } else if let Some(picture) = &final_view {
            // The picture at the world's size onto the window: one upscale, smoothly.
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Shader pack final"),
                layout: &self.upscale.1,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(picture),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.linear),
                    },
                ],
            });
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shader pack final"),
                color_attachments: &[attach(screen, overwrite)],
                timestamp_writes: self.timing.as_mut().and_then(|t| t.writes("final copy")),
                ..Default::default()
            });
            rp.set_pipeline(&self.upscale.0);
            rp.set_bind_group(0, &group, &[]);
            rp.draw(0..3, 0..1);
        }
        Ok(())
    }
}

impl PackPost {
    /// Writes `pass`'s uniforms for a target of `out` pixels; its bind group for the buffers'
    /// current mapping. The buffers it both samples and stores to are copied first, and it
    /// samples the copies.
    pub fn bind(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pass: &mut Pass,
        out: [u32; 2],
    ) -> wgpu::BindGroup {
        pack_images::copy(encoder, &self.images, pass.image_copies);
        for &(_, image) in &pass.images {
            if let Image::StandIn(format, dimension) = image
                && let Some(s) = pack_images::stand_in(&self.stand_ins, format, dimension)
            {
                s.clear(encoder);
            }
        }
        if let Some(targets) = &self.targets {
            for i in (0..BUFFERS).filter(|i| pass.copies & 1 << i != 0) {
                if let (Some(t), Some(copy)) = (&targets.color[i], &targets.scratch[i]) {
                    let side = usize::from(self.main & 1 << i != 0);
                    encoder.copy_texture_to_texture(
                        t[side].texture().as_image_copy(),
                        copy.texture().as_image_copy(),
                        t[side].texture().size(),
                    );
                }
            }
        }
        if let Some(shadow) = &self.shadow_targets {
            for n in (0..SHADOW_COLORS).filter(|n| pass.shadow_copies & 1 << n != 0) {
                if let (Some(main), Some(copy)) = (shadow.color(n, false), &shadow.scratch[n]) {
                    encoder.copy_texture_to_texture(
                        main.texture().as_image_copy(),
                        copy.texture().as_image_copy(),
                        main.texture().size(),
                    );
                }
            }
        }
        let fog = self.uniforms.fog();
        // Our block members first (`hz_target`, `hz_fog_color`, `hz_fog`, `hz_depth_linear`),
        // then the pack's.
        let ours = [
            [
                out[0] as f32,
                out[1] as f32,
                1.0 / out[0] as f32,
                1.0 / out[1] as f32,
            ],
            fog[0].to_array(),
            fog[1].to_array(),
            pass.depth_linear,
        ];
        let head: &[u8] = bytemuck::cast_slice(&ours);
        let n = head.len().min(pass.block.len());
        pass.block[..n].copy_from_slice(&head[..n]);
        for (index, ty, offset) in &pass.uniforms {
            let Some((i, v)) = index.and_then(|i| Some((i, self.uniforms.values.get(i)?))) else {
                continue;
            };
            // A built-in of another type keeps its initial value (0); custom uniforms were
            // checked when the pass was built.
            if pack_uniforms::NAMES
                .get(i)
                .is_none_or(|name| v.read_as(name, ty))
            {
                v.write(ty, &mut pass.block, *offset as usize);
            }
        }
        if !pass.block.is_empty() {
            queue.write_buffer(&pass.ubo, 0, &pass.block);
        }
        let targets = self.targets.as_ref().expect("resized");
        let shadow = self.shadow_targets.as_ref();
        let shadow_main = shadow.map_or(0, |s| s.main);
        let shadow_space = pass.group.shadow();
        // A chain is sampled only while it is current: a buffer written since its last chain
        // reads its first level, never stale levels (6.1.7). The shadow buffers have none.
        let (main, current) = if shadow_space {
            (shadow_main, 0)
        } else {
            (self.main, self.mips_valid & self.mipped & pass.reads)
        };
        let key = u64::from(main & pass.reads)
            | u64::from(current) << 16
            | u64::from(shadow_main & pass.shadow_reads) << 32;
        let shadow_scratch = |i: usize| shadow.and_then(|t| t.scratch[i].as_ref());
        let group = pass.groups.entry(key).or_insert_with(|| {
            let mut entries = vec![wgpu::BindGroupEntry {
                binding: 0,
                resource: pass.ubo.as_entire_binding(),
            }];
            for s in &pass.samplers {
                let shadow_color = |i: usize, side: usize| {
                    shadow.and_then(|t| t.color[i].as_ref()).map(|c| &c[side])
                };
                let texture = match s.source {
                    Source::Color(i) if shadow_space && pass.shadow_copies & 1 << i != 0 => {
                        shadow_scratch(i).unwrap_or(&self.dummy)
                    }
                    Source::ShadowColor(i) if pass.shadow_copies & 1 << i != 0 => {
                        shadow_scratch(i).unwrap_or(&self.dummy)
                    }
                    Source::Color(i) if shadow_space => {
                        shadow_color(i, usize::from(key & 1 << i != 0)).unwrap_or(&self.dummy)
                    }
                    Source::ShadowColor(i) => {
                        shadow_color(i, usize::from(key & 1 << (32 + i) != 0))
                            .unwrap_or(&self.dummy)
                    }
                    Source::ShadowDepth(i, _) => match shadow {
                        Some(t) => match (i, &t.depth1) {
                            (1, Some(d)) => &d.1,
                            _ => &t.depth.1,
                        },
                        None => &self.shadow.0,
                    },
                    Source::Color(i) if pass.copies & 1 << i != 0 => {
                        targets.scratch[i].as_ref().unwrap_or(&self.dummy)
                    }
                    Source::Color(i) => {
                        let side = usize::from(key & 1 << i != 0);
                        match (&targets.mips[i], &targets.color[i]) {
                            (Some(m), _) if current & 1 << i != 0 => &m[side].all,
                            (_, Some(t)) => &t[side],
                            (_, None) => &self.dummy,
                        }
                    }
                    Source::Depth => &targets.depth,
                    Source::WaterMask => targets.water_mask.as_ref().map_or(&self.dummy, |m| &m.1),
                    // Before translucent terrain the two are the same.
                    Source::Depth1 => match &targets.depth1 {
                        Some(d)
                            if matches!(
                                pass.group,
                                Group::Deferred
                                    | Group::Composite
                                    | Group::Final
                                    | Group::Translucent
                            ) =>
                        {
                            &d.1
                        }
                        _ => &targets.depth,
                    },
                    Source::Depth2 => match &targets.depth2 {
                        Some(d)
                            if matches!(
                                pass.group,
                                Group::Deferred
                                    | Group::Composite
                                    | Group::Final
                                    | Group::Translucent
                            ) =>
                        {
                            &d.1
                        }
                        _ => match &targets.depth1 {
                            Some(d)
                                if matches!(
                                    pass.group,
                                    Group::Deferred
                                        | Group::Composite
                                        | Group::Final
                                        | Group::Translucent
                                ) =>
                            {
                                &d.1
                            }
                            _ => &targets.depth,
                        },
                    },
                    Source::Noise => &self.noise,
                    Source::Dummy if s.dimension == wgpu::TextureViewDimension::D3 => {
                        &self.dummy_3d
                    }
                    Source::Dummy if s.dimension == wgpu::TextureViewDimension::D1 => {
                        &self.dummy_1d
                    }
                    Source::Normals if pass.paged => {
                        self.paged_maps[0].as_ref().unwrap_or(&self.dummy_normal)
                    }
                    Source::Specular if pass.paged => {
                        self.paged_maps[1].as_ref().unwrap_or(&self.dummy_black)
                    }
                    Source::Normals => self
                        .material_maps
                        .as_ref()
                        .filter(|_| pass.material_maps)
                        .map_or(&self.dummy_normal, |maps| &maps[0]),
                    Source::Specular => self
                        .material_maps
                        .as_ref()
                        .filter(|_| pass.material_maps)
                        .map_or(&self.dummy_black, |maps| &maps[1]),
                    Source::Dummy => &self.dummy,
                    Source::Image(k) if pass.image_copies & 1 << k != 0 => {
                        self.images[k].copy.as_ref().unwrap_or(&self.dummy)
                    }
                    Source::Image(k) => self.images[k].view.as_ref().unwrap_or(&self.dummy),
                    Source::Custom { index, .. } => match &self.textures[index].image {
                        pack_textures::Image::View(v) => v,
                        _ => &self.dummy,
                    },
                    Source::Shadow => &self.shadow.0,
                    Source::Atlas => pass
                        .atlas
                        .as_ref()
                        .or(self.atlas.as_ref())
                        .unwrap_or(&self.dummy),
                    Source::Lightmap => &self.lightmap,
                };
                let sampler = match SamplerKind::of(s.source, s.filtering) {
                    SamplerKind::Linear => &self.linear,
                    SamplerKind::Nearest => &self.nearest,
                    SamplerKind::Repeat => &self.repeat,
                    SamplerKind::NearestRepeat => &self.nearest_repeat,
                    SamplerKind::Atlas => &self.atlas_sampler,
                    SamplerKind::Buffer => &self.buffer_sampler,
                    SamplerKind::Comparison => shadow.map_or(&self.shadow.1, |t| &t.comparison),
                };
                entries.push(wgpu::BindGroupEntry {
                    binding: s.binding,
                    resource: wgpu::BindingResource::TextureView(texture),
                });
                if s.sampler == s.binding + 1 {
                    entries.push(wgpu::BindGroupEntry {
                        binding: s.sampler,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    });
                }
            }
            // An image is the side its buffer's sampler reads.
            for &(binding, image) in &pass.images {
                let view = match image {
                    Image::Color(i) => {
                        let side = usize::from(key & 1 << i != 0);
                        targets.color[i].as_ref().map_or(&self.dummy, |t| &t[side])
                    }
                    Image::Custom(k) => self.images[k].view.as_ref().unwrap_or(&self.dummy),
                    Image::ShadowColor(i) => shadow
                        .and_then(|t| t.color[i].as_ref())
                        .map_or(&self.dummy, |c| &c[usize::from(key & 1 << (32 + i) != 0)]),
                    Image::ColorBefore(i) => targets.scratch[i].as_ref().unwrap_or(&self.dummy),
                    Image::ShadowColorBefore(i) => shadow_scratch(i).unwrap_or(&self.dummy),
                    Image::CustomBefore(k) => self.images[k].before.as_ref().unwrap_or(&self.dummy),
                    Image::StandIn(format, dimension) => {
                        pack_images::stand_in(&self.stand_ins, format, dimension)
                            .map_or(&self.dummy, |s| &s.view)
                    }
                };
                entries.push(wgpu::BindGroupEntry {
                    binding,
                    resource: wgpu::BindingResource::TextureView(view),
                });
            }
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&pass.name),
                layout: &pass.layout,
                entries: &entries,
            })
        });
        group.clone()
    }

    /// Dispatches compute pass `pass` over a target of `size`.
    pub fn dispatch(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pass: &mut Pass,
        size: [u32; 2],
    ) {
        let Pipeline::Compute {
            pipeline,
            groups,
            render,
            local,
        } = &pass.pipeline
        else {
            return;
        };
        let pipeline = pipeline.get().expect("compiled with its pack");
        let groups = groups.unwrap_or_else(|| {
            let n = |k: usize| (size[k] as f32 * render[k] / local[k] as f32).ceil() as u32;
            [n(0), n(1), 1]
        });
        let (pipeline, groups) = (pipeline.clone(), groups);
        // wgpu panics on a dispatch over its limit. The translator reshapes a large
        // `workGroups`; a `workGroupsRender` that large is not run.
        let most = device.limits().max_compute_workgroups_per_dimension;
        if groups.iter().any(|&n| n > most) {
            log::warn!(
                "shader pack: {}: {groups:?} work groups is over {most} per dimension; not run",
                pass.name
            );
            return;
        }
        for image in (0..self.images.len()).filter(|k| pass.snapshots & 1 << k != 0) {
            pack_images::snapshot(encoder, &self.images[image]);
        }
        let group = self.bind(device, queue, encoder, pass, size);
        let mut cp = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(&pass.name),
            timestamp_writes: self
                .timing
                .as_mut()
                .and_then(|t| t.compute_writes(&format!("{} (compute)", pass.name))),
        });
        cp.set_pipeline(&pipeline);
        cp.set_bind_group(0, &group, &[]);
        if pass.buffers
            && let Some(g) = &self.buffers.group
        {
            cp.set_bind_group(4, g, &[]);
        }
        cp.dispatch_workgroups(groups[0], groups[1], groups[2]);
        drop(cp);
        self.mips_valid &= !pass.written;
    }

    /// A render pass into the buffers `pass` writes, in place, tested against the world's depth.
    pub fn begin<'e>(
        &mut self,
        encoder: &'e mut wgpu::CommandEncoder,
        pass: &Pass,
    ) -> wgpu::RenderPass<'e> {
        self.begin_with(encoder, pass, false)
    }

    /// `begin`, or with `water_mask` into the water patches' depth instead of the frame's.
    pub fn begin_with<'e>(
        &mut self,
        encoder: &'e mut wgpu::CommandEncoder,
        pass: &Pass,
        water_mask: bool,
    ) -> wgpu::RenderPass<'e> {
        let targets = self.targets.as_ref().expect("resized");
        let load = wgpu::Operations {
            load: wgpu::LoadOp::Load,
            store: wgpu::StoreOp::Store,
        };
        // The shadow pass draws into the shadow buffers and the shadow map, the frame's first
        // one clearing them.
        let (depth_ops, shadow_ops) = match &mut self.shadow_targets {
            Some(s) if pass.group.shadow() => s.clear_ops(pass.writes.iter().map(|w| w.1)),
            _ => (crate::pack_shadow::load(), [load; SHADOW_COLORS]),
        };
        let shadow = self.shadow_targets.as_ref().filter(|_| pass.group.shadow());
        let mut attachments: SmallVec<[Option<wgpu::RenderPassColorAttachment>; BUFFERS]> =
            SmallVec::new();
        for &(location, buffer) in &pass.writes {
            let l = location as usize;
            if attachments.len() <= l {
                attachments.resize(l + 1, None);
            }
            attachments[l] = match shadow {
                Some(s) => s
                    .color(buffer, false)
                    .and_then(|v| attach(v, shadow_ops.get(buffer).copied().unwrap_or(load))),
                None => targets.color[buffer]
                    .as_ref()
                    .and_then(|t| attach(&t[usize::from(self.main & 1 << buffer != 0)], load)),
            };
        }
        let timestamp_writes = self.timing.as_mut().and_then(|t| t.writes(&pass.name));
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(&pass.name),
            color_attachments: &attachments,
            timestamp_writes,
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: match &targets.water_mask {
                    Some((_, view)) if water_mask => view,
                    _ => shadow.map_or(&targets.depth, |s| &s.depth.1),
                },
                depth_ops: Some(depth_ops),
                stencil_ops: None,
            }),
            ..Default::default()
        })
    }

    /// Terrain (or, `translucent`, translucent terrain) with the pack's program; `draw` issues
    /// the game's draws. Solid terrain in two parts (a pack's `gbuffers_terrain_solid` and
    /// `_cutout`) has its cutout part drawn second, from the same draws.
    pub fn draw_terrain(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        list: TerrainList,
        draw: &mut TerrainDraw<'_>,
    ) {
        self.draw_terrain_part(device, queue, encoder, size, list, false, draw);
        if list == TerrainList::Solid {
            self.draw_terrain_part(device, queue, encoder, size, list, true, draw);
        }
    }

    /// The pass that draws `list`, or the cutout part of the solid list.
    pub fn terrain_slot(&mut self, list: TerrainList, cutout: bool) -> &mut Option<Pass> {
        match list {
            TerrainList::Solid if cutout => &mut self.terrain_cutout,
            TerrainList::Solid => &mut self.terrain,
            TerrainList::Translucent => &mut self.water,
            TerrainList::ShadowSolid => &mut self.shadow_terrain,
            TerrainList::ShadowTranslucent => &mut self.shadow_water,
        }
    }

    pub fn draw_terrain_part(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        list: TerrainList,
        cutout: bool,
        draw: &mut TerrainDraw<'_>,
    ) {
        let Some(mut pass) = self.terrain_slot(list, cutout).take() else {
            return;
        };
        let translucent = list.translucent();
        let group = self.bind(device, queue, encoder, &mut pass, size);
        if let Some((slot, _)) = &pass.prepass
            && let Some(prepass) = slot.get()
        {
            let targets = self.targets.as_ref().expect("resized");
            let timestamp_writes = self.timing.as_mut().and_then(|t| t.writes("terrain depth"));
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shader pack terrain depth"),
                color_attachments: &[],
                timestamp_writes,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(crate::pack_shadow::load()),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            rp.set_pipeline(prepass);
            rp.set_bind_group(0, &group, &[]);
            if pass.draw {
                self.draws.bind(&mut rp, "TERRAIN_SOLID");
            }
            let (groups, ids) = (&self.page_groups, pass.ids);
            let page = |rp: &mut wgpu::RenderPass<'_>, page: u32| {
                if let Some((_, _, group)) = groups.get(&page).filter(|_| ids) {
                    rp.set_bind_group(3, group, &[]);
                }
            };
            draw(&mut rp, list, &page, None);
        }
        {
            let mut rp = self.begin(encoder, &pass);
            rp.set_pipeline(pass.render());
            rp.set_bind_group(0, &group, &[]);
            if pass.buffers
                && let Some(g) = &self.buffers.group
            {
                rp.set_bind_group(4, g, &[]);
            }
            if pass.draw {
                let stage = if translucent {
                    "TERRAIN_TRANSLUCENT"
                } else {
                    "TERRAIN_SOLID"
                };
                self.draws.bind(&mut rp, stage);
            }
            let (groups, ids) = (&self.page_groups, pass.ids);
            let page = |rp: &mut wgpu::RenderPass<'_>, page: u32| {
                if let Some((_, _, group)) = groups.get(&page).filter(|_| ids) {
                    rp.set_bind_group(3, group, &[]);
                }
            };
            let geometry = self
                .indices
                .as_ref()
                .filter(|_| pass.geometry > 0)
                .map(|indices| (pass.geometry, indices));
            draw(&mut rp, list, &page, geometry);
        }
        if !pass.group.shadow() {
            self.mips_valid &= !pass.written;
        }
        *self.terrain_slot(list, cutout) = Some(pass);
    }

    /// The sky's parts in vanilla's order, each with the program that draws it; one render pass
    /// while consecutive programs write the same buffers.
    pub fn draw_sky(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        view: &UniformFrameView<'_>,
        sky: &SkyInputs<'_>,
    ) {
        let Some(mut sky_parts) = self.sky.take() else {
            return;
        };
        let sky_frame = SkyFrame {
            sun_angle: view.sky.sun_angle,
            sky_color: view.sky.sky_color,
            fog_color: view.sky.fog_color,
            sunset_color: sky.sunset_color,
            fog: view.fog.map(|f| [f.color[0], f.color[1], f.color[2]]),
            stars: sky.stars,
            dark_disc: sky.dark_disc,
            detail_distance: view.detail_distance,
            celestial: bytemuck::cast_slice(sky.celestial),
            celestial_bodies: sky.celestial_bodies,
            end_fog_sky: sky.end_fog_sky,
        };
        let parts = sky_parts.geometry.prepare(
            queue,
            &sky_frame,
            self.uniforms.shadow.sun_path_rotation,
            self.sky_switches,
        );
        let mut groups = [None, None];
        for (i, pass) in [&mut sky_parts.basic, &mut sky_parts.textured]
            .into_iter()
            .enumerate()
        {
            if let Some(pass) = pass {
                groups[i] = Some(self.bind(device, queue, encoder, pass, size));
                self.mips_valid &= !pass.written;
            }
        }
        let passes = [&sky_parts.basic, &sky_parts.textured];
        let mut open: Option<(wgpu::RenderPass<'_>, usize)> = None;
        for part in parts {
            let i = usize::from(part.textured);
            let (Some(pass), Some(group)) = (passes[i], &groups[i]) else {
                continue;
            };
            let current = open.as_ref().map(|o| o.1);
            if current != Some(i) {
                let same = current
                    .and_then(|c| passes[c].as_ref())
                    .is_some_and(|p| p.writes == pass.writes);
                if !same {
                    drop(open.take());
                    open = Some((self.begin(encoder, pass).forget_lifetime(), i));
                }
                let rp = &mut open.as_mut().expect("open").0;
                rp.set_pipeline(pass.render());
                rp.set_bind_group(0, group, &[]);
                // A sky program that reads the pack's SSBOs (Noble's `gbuffers_textured`, the
                // sun's fallback) binds them, as the other passes do.
                if pass.buffers
                    && let Some(g) = &self.buffers.group
                {
                    rp.set_bind_group(4, g, &[]);
                }
                sky_parts.geometry.bind(rp, part.textured);
                open.as_mut().expect("open").1 = i;
            }
            let rp = &mut open.as_mut().expect("open").0;
            // Each part with vanilla's blend for it: one program, three pipelines.
            let part_pipeline =
                pass.sky_parts
                    .as_ref()
                    .and_then(|[plain, stars]| match part.stage {
                        "SKY" | "VOID" => plain.get(),
                        "STARS" => stars.get(),
                        _ => None,
                    });
            rp.set_pipeline(part_pipeline.unwrap_or(pass.render()));
            if pass.draw {
                self.draws.bind(rp, part.stage);
            }
            rp.draw(part.range, 0..1);
        }
        drop(open);
        self.sky = Some(sky_parts);
    }

    /// Entities, block entities, translucent entity parts and particles with the pack's
    /// programs, those drawn before deferred or (`late`) after: each run `prepare_entities` kept
    /// is one draw with its own `entityId` or `blockEntityId` and `entityColor`.
    pub fn draw_entities(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        late: bool,
    ) {
        let Some(mut entities) = self.entities.take() else {
            return;
        };
        let geometry = &entities.geometry;
        for (program, pass) in entities.passes.iter_mut().enumerate() {
            let Some(pass) = pass.as_mut().filter(|_| entities.late[program] == late) else {
                continue;
            };
            let mut draws = geometry
                .draws
                .iter()
                .filter(|d| d.pack && d.program == program && !d.water_mask)
                .peekable();
            let masks: Vec<_> = (geometry.draws.iter())
                .filter(|d| d.pack && d.program == program && d.water_mask)
                .collect();
            if draws.peek().is_none() {
                continue;
            }
            let group = self.bind(device, queue, encoder, pass, size);
            let portal_group = self.portal_group(
                device,
                queue,
                encoder,
                pass,
                size,
                geometry
                    .draws
                    .iter()
                    .any(|d| d.pack && d.program == program && d.portal),
            );
            {
                let mut rp = self.begin(encoder, pass);
                rp.set_pipeline(pass.render());
                rp.set_bind_group(0, &group, &[]);
                if pass.buffers
                    && let Some(g) = &self.buffers.group
                {
                    rp.set_bind_group(4, g, &[]);
                }
                rp.set_bind_group(1, &geometry.table_group, &[]);
                rp.set_vertex_buffer(0, geometry.vertices());
                rp.set_index_buffer(geometry.indices.slice(..), wgpu::IndexFormat::Uint32);
                let mut on_portal = false;
                for d in draws {
                    if let Some(portal) = &portal_group
                        && d.portal != on_portal
                    {
                        rp.set_bind_group(0, if d.portal { portal } else { &group }, &[]);
                        on_portal = d.portal;
                    }
                    if pass.draw {
                        rp.set_bind_group(2, &geometry.group, &[geometry.offset(d.slot)]);
                    }
                    rp.draw_indexed(0..d.quads * 6, d.base as i32, 0..1);
                }
            }
            // The boats' water patches, depth only, into their own depth: translucent terrain
            // discards what lies behind them, and the frame's depth stays the pack's.
            if let Some(mask) = pass.water_mask.as_ref().and_then(|m| m.get())
                && !masks.is_empty()
                && self
                    .targets
                    .as_ref()
                    .is_some_and(|t| t.water_mask.is_some())
            {
                let mut rp = self.begin_with(encoder, pass, true);
                rp.set_pipeline(mask);
                rp.set_bind_group(0, &group, &[]);
                if pass.buffers
                    && let Some(g) = &self.buffers.group
                {
                    rp.set_bind_group(4, g, &[]);
                }
                rp.set_bind_group(1, &geometry.table_group, &[]);
                rp.set_vertex_buffer(0, geometry.vertices());
                rp.set_index_buffer(geometry.indices.slice(..), wgpu::IndexFormat::Uint32);
                for d in &masks {
                    if pass.draw {
                        rp.set_bind_group(2, &geometry.group, &[geometry.offset(d.slot)]);
                    }
                    rp.draw_indexed(0..d.quads * 6, d.base as i32, 0..1);
                }
                drop(rp);
                self.water_mask_dirty = true;
            }
            self.mips_valid &= !pass.written;
        }
        self.entities = Some(entities);
    }

    /// End portal faces bind `end_portal.png` as the atlas, a texture of its own, so packs that
    /// wrap it with their own coordinates sample the portal sprite only. `None` when no portal
    /// face draws in this pass.
    pub fn portal_group(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pass: &mut Pass,
        size: [u32; 2],
        wanted: bool,
    ) -> Option<wgpu::BindGroup> {
        let atlas = pass.atlas.clone().or_else(|| self.atlas.clone())?;
        if !wanted || self.portal_rect[2] <= 0.0 {
            return None;
        }
        if self
            .portal_texture
            .as_ref()
            .is_none_or(|(rect, _)| *rect != self.portal_rect)
        {
            let view = crate::pack_weather::copy_sprite(
                device,
                encoder,
                atlas.texture(),
                self.portal_rect,
            );
            self.portal_texture = Some((self.portal_rect, view));
            pass.portal_groups.clear();
        }
        // `bind` caches groups by a key without the atlas: the portal groups are kept in a cache
        // of their own, swapped in with the texture.
        let view = self.portal_texture.as_ref().map(|p| p.1.clone());
        let saved = std::mem::replace(&mut pass.atlas, view);
        let groups = std::mem::replace(&mut pass.groups, std::mem::take(&mut pass.portal_groups));
        let group = self.bind(device, queue, encoder, pass, size);
        pass.portal_groups = std::mem::replace(&mut pass.groups, groups);
        pass.atlas = saved;
        Some(group)
    }

    /// Entities and block entities into the shadow map, each run with the per-draw uniforms
    /// of its gbuffers draw (2.4): opaque, so before the `shadowtex1` copy.
    pub fn draw_shadow_entities(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
    ) {
        let Some(mut entities) = self.entities.take() else {
            return;
        };
        let geometry = &entities.geometry;
        for (program, pass) in entities.shadow.iter_mut().enumerate() {
            let Some(pass) = pass else {
                continue;
            };
            let mut draws = (geometry.draws.iter())
                .filter(|d| d.shadow && crate::pack_entities::shadow_program(d.program) == program)
                .peekable();
            if draws.peek().is_none() {
                continue;
            }
            let group = self.bind(device, queue, encoder, pass, size);
            let portal_group = self.portal_group(
                device,
                queue,
                encoder,
                pass,
                size,
                (geometry.draws.iter()).any(|d| {
                    d.shadow
                        && crate::pack_entities::shadow_program(d.program) == program
                        && d.portal
                }),
            );
            let mut on_portal = false;
            let mut rp = self.begin(encoder, pass);
            rp.set_pipeline(pass.render());
            rp.set_bind_group(0, &group, &[]);
            if pass.buffers
                && let Some(g) = &self.buffers.group
            {
                rp.set_bind_group(4, g, &[]);
            }
            rp.set_bind_group(1, &geometry.table_group, &[]);
            rp.set_vertex_buffer(0, geometry.vertices());
            rp.set_index_buffer(geometry.indices.slice(..), wgpu::IndexFormat::Uint32);
            for d in draws {
                if let Some(portal) = &portal_group
                    && d.portal != on_portal
                {
                    rp.set_bind_group(0, if d.portal { portal } else { &group }, &[]);
                    on_portal = d.portal;
                }
                if pass.draw {
                    rp.set_bind_group(2, &geometry.group, &[geometry.offset(d.slot)]);
                }
                if pass.geometry > 0 {
                    // A geometry stage reads its vertices itself: corner `c` of the run is
                    // vertex `c` past the run's first quad's six (`geometry_inputs`), one
                    // instance per triangle the stage can emit.
                    let first = d.base / 4 * 6;
                    rp.draw(first..first + d.quads * 6, 0..pass.geometry);
                } else {
                    rp.draw_indexed(0..d.quads * 6, d.base as i32, 0..1);
                }
            }
        }
        self.entities = Some(entities);
    }

    /// The sky, when it shows, then terrain, entities and block entities.
    pub fn draw_opaque(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        view: &UniformFrameView<'_>,
        sky: &SkyInputs<'_>,
        hand: &HandInputs<'_>,
        terrain: &mut TerrainDraw<'_>,
    ) {
        if sky.visible || sky.end_fog_sky {
            self.draw_sky(device, queue, encoder, size, view, sky);
        }
        self.draw_terrain(device, queue, encoder, size, TerrainList::Solid, terrain);
        self.draw_entities(device, queue, encoder, size, false);
        self.draw_lines(device, queue, encoder, size);
        self.draw_clouds(device, queue, encoder, size);
        // Only depth2 excludes the opaque first-person hand.
        if let Some(t) = &self.targets
            && let Some((copy, _)) = &t.depth2
        {
            encoder.copy_texture_to_texture(
                t.depth_texture.as_image_copy(),
                copy.as_image_copy(),
                t.depth_texture.size(),
            );
        }
        self.draw_hand(device, queue, encoder, size, view, hand);
        // Depth1 includes that hand, but still precedes translucent terrain.
        if let Some(t) = &self.targets
            && let Some((copy, _)) = &t.depth1
        {
            encoder.copy_texture_to_texture(
                t.depth_texture.as_image_copy(),
                copy.as_image_copy(),
                t.depth_texture.size(),
            );
        }
    }

    /// The first-person hand and held items, with `gbuffers_hand` at the hand's projection.
    pub fn draw_hand(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        view: &UniformFrameView<'_>,
        hand: &HandInputs<'_>,
    ) {
        let Some((mut pass, mut geometry)) = self.hand.take() else {
            return;
        };
        // The hand draws through `gbufferModelView`, which holds the nausea spin the world
        // turns with; the hand doesn't (as vanilla's), so its projection takes the spin out.
        let unspin = hand.bob * hand.spin.inverse() * hand.bob.inverse();
        let projection = pack_hand::projection(
            hand.fov_degrees,
            size[0] as f32 / size[1].max(1) as f32,
            view.depth_far,
        ) * unspin;
        geometry.prepare(device, queue, hand.vertices, hand.items, projection);
        if geometry.quads > 0 {
            let group = self.bind(device, queue, encoder, &mut pass, size);
            {
                let mut rp = self.begin(encoder, &pass);
                rp.set_pipeline(pass.render());
                rp.set_bind_group(0, &group, &[]);
                if pass.buffers
                    && let Some(g) = &self.buffers.group
                {
                    rp.set_bind_group(4, g, &[]);
                }
                rp.set_bind_group(1, &geometry.group, &[]);
                rp.set_vertex_buffer(0, geometry.vertices());
                rp.set_index_buffer(geometry.indices.slice(..), wgpu::IndexFormat::Uint32);
                // The arm and each held item: their own `currentRenderedItemId`. Translucent
                // quads wait for `gbuffers_hand_water` when the pack has one.
                let late = self.hand_water.is_some();
                let runs = geometry
                    .runs
                    .iter()
                    .chain(geometry.translucent_runs.iter().filter(|_| !late));
                for &(first, quads, offset) in runs {
                    if pass.draw {
                        rp.set_bind_group(2, &geometry.slot_group, &[offset]);
                    }
                    rp.draw_indexed(first * 6..(first + quads) * 6, 0, 0..1);
                }
            }
            self.mips_valid &= !pass.written;
        }
        self.hand = Some((pass, geometry));
    }

    /// The clouds `set_clouds` gave for this frame, with `gbuffers_clouds`.
    pub fn draw_clouds(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
    ) {
        // The pass stays for later frames when this one has no clouds.
        let Some(clouds) = self.cloud_draw.take() else {
            return;
        };
        let Some(mut pass) = self.clouds.take() else {
            return;
        };
        let group = self.bind(device, queue, encoder, &mut pass, size);
        {
            let mut rp = self.begin(encoder, &pass);
            rp.set_pipeline(pass.render());
            rp.set_bind_group(0, &group, &[]);
            if pass.buffers
                && let Some(g) = &self.buffers.group
            {
                rp.set_bind_group(4, g, &[]);
            }
            rp.set_bind_group(1, &clouds.group, &[]);
            if pass.draw {
                self.draws.bind(&mut rp, "CLOUDS");
            }
            rp.set_vertex_buffer(0, clouds.vertices.slice(..));
            rp.set_index_buffer(clouds.indices.slice(..), wgpu::IndexFormat::Uint32);
            rp.draw_indexed(0..clouds.quads * 6, 0, 0..1);
        }
        self.mips_valid &= !pass.written;
        self.clouds = Some(pass);
    }

    /// The block outline `set_lines` gave for this frame, with the pack's lines program.
    pub fn draw_lines(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
    ) {
        // The pass stays for later frames when this one has no outline.
        let Some(lines) = self.line_draw.take() else {
            return;
        };
        let Some(mut pass) = self.lines.take() else {
            return;
        };
        let group = self.bind(device, queue, encoder, &mut pass, size);
        {
            let mut rp = self.begin(encoder, &pass);
            rp.set_pipeline(pass.render());
            rp.set_bind_group(0, &group, &[]);
            if pass.buffers
                && let Some(g) = &self.buffers.group
            {
                rp.set_bind_group(4, g, &[]);
            }
            if pass.draw {
                self.draws.bind(&mut rp, "OUTLINE");
            }
            rp.set_vertex_buffer(0, lines.edges.slice(..));
            // Each edge reads the width of the stroke drawn over it from the second buffer:
            // the next stroke's edge (the outline buffer from that stroke's first edge on, as
            // this stroke's edges are instances from its own first edge on), or the last
            // stroke's own.
            let stroke_bytes = u64::from(lines.count) * 32;
            for stroke in 0..lines.strokes {
                let over = if stroke + 1 < lines.strokes {
                    stroke_bytes
                } else {
                    0
                };
                rp.set_vertex_buffer(1, lines.edges.slice(over..));
                rp.draw(0..6, stroke * lines.count..(stroke + 1) * lines.count);
            }
        }
        self.mips_valid &= !pass.written;
        self.lines = Some(pass);
    }

    /// Translucent terrain, then the entity parts drawn after deferred.
    pub fn draw_translucent(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        terrain: &mut TerrainDraw<'_>,
    ) {
        self.draw_terrain(
            device,
            queue,
            encoder,
            size,
            TerrainList::Translucent,
            terrain,
        );
        self.draw_entities(device, queue, encoder, size, true);
        self.draw_hand_water(device, queue, encoder, size);
        self.draw_weather(device, queue, encoder, size);
    }

    /// The hand's translucent quads (uploaded by `draw_hand` this frame), with
    /// `gbuffers_hand_water`.
    pub fn draw_hand_water(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
    ) {
        let Some(mut pass) = self.hand_water.take() else {
            return;
        };
        let hand = self.hand.take();
        if let Some((_, geometry)) = hand
            .as_ref()
            .filter(|(_, g)| g.quads > 0 && !g.translucent_runs.is_empty())
        {
            let group = self.bind(device, queue, encoder, &mut pass, size);
            {
                let mut rp = self.begin(encoder, &pass);
                rp.set_pipeline(pass.render());
                rp.set_bind_group(0, &group, &[]);
                if pass.buffers
                    && let Some(g) = &self.buffers.group
                {
                    rp.set_bind_group(4, g, &[]);
                }
                rp.set_bind_group(1, &geometry.group, &[]);
                rp.set_vertex_buffer(0, geometry.vertices());
                rp.set_index_buffer(geometry.indices.slice(..), wgpu::IndexFormat::Uint32);
                for &(first, quads, offset) in &geometry.translucent_runs {
                    if pass.draw {
                        rp.set_bind_group(2, &geometry.slot_group, &[offset]);
                    }
                    rp.draw_indexed(first * 6..(first + quads) * 6, 0, 0..1);
                }
            }
            self.mips_valid &= !pass.written;
        }
        self.hand = hand;
        self.hand_water = Some(pass);
    }

    /// The rain, then the snow, `set_weather` gave for this frame, with `gbuffers_weather`.
    pub fn draw_weather(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
    ) {
        // The passes stay for later frames when this one has no rain or snow.
        let Some(weather) = self.weather_draw.take() else {
            return;
        };
        let Some(mut passes) = self.weather.take() else {
            return;
        };
        if weather.generation != self.weather_generation {
            self.weather_generation = weather.generation;
            for (pass, sprite) in passes.iter_mut().zip(&weather.sprites) {
                pass.atlas = sprite.clone();
                pass.groups.clear();
            }
        }
        for (pass, quads) in passes.iter_mut().zip(weather.quads.clone()) {
            if quads.is_empty() {
                continue;
            }
            let group = self.bind(device, queue, encoder, pass, size);
            {
                let mut rp = self.begin(encoder, pass);
                rp.set_pipeline(pass.render());
                rp.set_bind_group(0, &group, &[]);
                if pass.buffers
                    && let Some(g) = &self.buffers.group
                {
                    rp.set_bind_group(4, g, &[]);
                }
                if pass.draw {
                    self.draws.bind(&mut rp, "RAIN_SNOW");
                }
                rp.set_vertex_buffer(0, weather.vertices.slice(..));
                rp.set_index_buffer(weather.indices.slice(..), wgpu::IndexFormat::Uint32);
                rp.draw_indexed(quads.start * 6..quads.end * 6, 0, 0..1);
            }
            self.mips_valid &= !pass.written;
        }
        self.weather = Some(passes);
    }

    /// Regenerates the mip chain of buffer `b`'s side that programs read.
    pub fn mipmap(&mut self, encoder: &mut wgpu::CommandEncoder, b: usize) {
        let (Some(targets), Some((pipeline, _))) = (&self.targets, self.down.get(&self.formats[b]))
        else {
            return;
        };
        let Some(mips) = &targets.mips[b] else { return };
        let side = &mips[usize::from(self.main & 1 << b != 0)];
        // Timed as one pass per buffer (its first level), so the pass timer shows the chains.
        let label = format!("mipmaps colortex{b}");
        let mut first = true;
        for (attachment, group) in &side.levels {
            let timestamp_writes = if std::mem::take(&mut first) {
                self.timing.as_mut().and_then(|t| t.writes(&label))
            } else {
                None
            };
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shader pack mipmap"),
                timestamp_writes,
                color_attachments: &[attach(
                    attachment,
                    wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                )],
                ..Default::default()
            });
            rp.set_pipeline(pipeline);
            rp.set_bind_group(0, group, &[]);
            rp.draw(0..3, 0..1);
        }
    }

    /// Whether the buffers `pass` writes have different sizes at the current render size: then
    /// it is skipped, as one whose `size.buffer` sizes never meet is when the pack loads. Only
    /// a pass writing buffers sized relative to the render size and in pixels can.
    pub fn sizes_differ(&self, pass: &Pass) -> bool {
        let Some(targets) = &self.targets else {
            return false;
        };
        let mut sizes = pass.writes.iter().map(|&(_, b)| targets.sizes[b]);
        let first = sizes.next();
        !pass.group.shadow() && pass.group != Group::Final && sizes.any(|s| Some(s) != first)
    }

    /// Drops every pass's cached bind groups, compute stages included: they name views that
    /// were made again.
    pub fn forget_groups(&mut self) {
        let sky = self
            .sky
            .iter_mut()
            .flat_map(|s| s.basic.iter_mut().chain(&mut s.textured));
        let entities = self
            .entities
            .iter_mut()
            .flat_map(|e| e.passes.iter_mut().chain(&mut e.shadow).flatten());
        let passes = self
            .passes
            .iter_mut()
            .chain(&mut self.terrain)
            .chain(&mut self.terrain_cutout)
            .chain(&mut self.water)
            .chain(sky)
            .chain(entities)
            .chain(&mut self.clouds)
            .chain(&mut self.lines)
            .chain(self.weather.iter_mut().flatten())
            .chain(self.hand.iter_mut().map(|h| &mut h.0))
            .chain(&mut self.hand_water)
            .chain(&mut self.shadow_terrain)
            .chain(&mut self.shadow_water)
            .chain(&mut self.shadow_comp);
        for pass in passes {
            pass.groups.clear();
            for c in &mut pass.compute {
                c.groups.clear();
            }
        }
    }

    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        size: [u32; 2],
        lost: &std::sync::atomic::AtomicBool,
        poll: PollLive,
    ) -> Result<(), String> {
        if pack_images::resize_images(device, &mut self.images, size) {
            self.forget_groups();
        }
        self.buffers.resize(device, size);
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return Ok(());
        }
        self.setup_pending = true;
        self.rgb_unset = true;
        // Free the old targets before making the new ones: a pack with large buffers can't
        // hold both sets at once on a card it otherwise fits (a window resize ran out of video
        // memory). The bind groups hold the old views too, and the GPU may still be using
        // them, so wait for it; a resize is rare.
        if self.targets.take().is_some() {
            self.forget_groups();
            // A lost device stops the frame here, before the new targets are made.
            poll(device, lost, wgpu::PollType::wait_indefinitely())?;
        }
        let sizes: [[u32; 2]; BUFFERS] =
            std::array::from_fn(|i| self.sizes[i].map_or(size, |s| s.resolve(size)));
        let full_chain = |size: [u32; 2]| 32 - size[0].max(size[1]).max(1).leading_zeros();
        let texture_sized = |label: &str, format, usage, levels, size: [u32; 2]| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: levels,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: usage | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let texture = |label: &str, format, usage| texture_sized(label, format, usage, 1, size);
        let level = |t: &wgpu::Texture, level: u32| {
            t.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let mut mips: [Option<[Mips; 2]>; BUFFERS] = Default::default();
        let color = std::array::from_fn(|i| {
            (self.used & 1 << i != 0).then(|| {
                let label = format!("colortex{i}");
                let mipped = self.mipped & 1 << i != 0;
                let levels = if mipped { full_chain(sizes[i]) } else { 1 };
                let mut attachment = wgpu::TextureUsages::RENDER_ATTACHMENT;
                if self.storage & 1 << i != 0 {
                    attachment |= wgpu::TextureUsages::STORAGE_BINDING;
                    // Atomics on a texture need it made for them (Metal ignores them otherwise).
                    if device.features().contains(wgpu::Features::TEXTURE_ATOMIC)
                        && matches!(
                            self.formats[i],
                            wgpu::TextureFormat::R32Sint | wgpu::TextureFormat::R32Uint
                        )
                    {
                        attachment |= wgpu::TextureUsages::STORAGE_ATOMIC;
                    }
                }
                if self.copied & 1 << i != 0 {
                    attachment |= wgpu::TextureUsages::COPY_SRC;
                }
                // A buffer that isn't cleared starts each frame with B copied back to A.
                if i != 0 && self.clears[i].is_none() {
                    attachment |= wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST;
                }
                let side = || texture_sized(&label, self.formats[i], attachment, levels, sizes[i]);
                let a = side();
                let sides = [
                    a.clone(),
                    if self.doubled & 1 << i != 0 {
                        side()
                    } else {
                        a
                    },
                ];
                if mipped && let Some((_, layout)) = self.down.get(&self.formats[i]) {
                    mips[i] = Some(sides.each_ref().map(|t| {
                        Mips {
                            all: t.create_view(&Default::default()),
                            levels: (1..levels)
                                .map(|l| {
                                    let group =
                                        device.create_bind_group(&wgpu::BindGroupDescriptor {
                                            label: Some(&label),
                                            layout,
                                            entries: &[
                                                wgpu::BindGroupEntry {
                                                    binding: 0,
                                                    resource: wgpu::BindingResource::TextureView(
                                                        &level(t, l - 1),
                                                    ),
                                                },
                                                wgpu::BindGroupEntry {
                                                    binding: 2,
                                                    resource: wgpu::BindingResource::Sampler(
                                                        &self.linear,
                                                    ),
                                                },
                                            ],
                                        });
                                    (level(t, l), group)
                                })
                                .collect(),
                        }
                    }));
                }
                sides.map(|t| level(&t, 0))
            })
        });
        let depth32 = wgpu::TextureFormat::Depth32Float;
        let depth_texture = texture(
            "depthtex0",
            depth32,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth1 = self.depth1.then(|| {
            let t = texture("depthtex1", depth32, wgpu::TextureUsages::COPY_DST);
            let view = t.create_view(&Default::default());
            (t, view)
        });
        let depth2 = self.depth2.then(|| {
            let t = texture("depthtex2", depth32, wgpu::TextureUsages::COPY_DST);
            let view = t.create_view(&Default::default());
            (t, view)
        });
        let water_mask = self.water_mask.then(|| {
            let t = texture(
                "Shader pack water patches",
                depth32,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            );
            let view = t.create_view(&Default::default());
            (t, view)
        });
        self.water_mask_dirty = true;
        let scratch = std::array::from_fn(|i| {
            (self.copied & 1 << i != 0).then(|| {
                // A copy also read as an image before a dispatch.
                let image = if self.storage & 1 << i != 0 {
                    wgpu::TextureUsages::STORAGE_BINDING
                } else {
                    wgpu::TextureUsages::empty()
                };
                texture_sized(
                    &format!("colortex{i} copy"),
                    self.formats[i],
                    wgpu::TextureUsages::COPY_DST | image,
                    1,
                    sizes[i],
                )
                .create_view(&Default::default())
            })
        });
        let sink = texture(
            "Shader pack sink",
            SINK,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        )
        .create_view(&Default::default());
        self.targets = Some(Targets {
            size,
            sink,
            sizes,
            scratch,
            color,
            depth: depth_texture.create_view(&Default::default()),
            depth_texture,
            depth1,
            depth2,
            water_mask,
            mips,
        });
        self.forget_groups();
        for pass in self.passes.iter().filter(|p| self.sizes_differ(p)) {
            log::info!(
                "shader pack: {}: writes buffers of different sizes at {}x{} (size.buffer); skipped",
                pass.name,
                size[0],
                size[1]
            );
        }
        Ok(())
    }

    /// The shadow pass (2.1, passes 2 and 3): the shadow buffers cleared, then the `shadowcomp`
    /// programs, each drawing into the "alt" side of what it writes and flipping it after.
    pub fn run_shadow(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        terrain: &mut TerrainDraw<'_>,
        view: &UniformFrameView<'_>,
        shadow_cache: bool,
        shadow_scene_revision: (u64, u64),
        ticks: u64,
        day_time: i64,
    ) {
        // The last shadow map serves this frame too (`shadow_every`).
        if self.uniforms.keep_shadow {
            return;
        }
        // Opt-in only (`sunrise-graphics.txt` shadowCache). Off: always redraw, drop any cache.
        if !shadow_cache {
            self.shadow_cache = ShadowCache::default();
            self.draw_shadow_pass(device, queue, encoder, terrain);
            return;
        }

        // Compute the current frame's cache key before borrowing shadow_targets mutably.
        let cache_key = self.compute_shadow_cache_key(view, shadow_scene_revision, ticks, day_time);

        // Entity vertices are interpolated each frame, so their shadows cannot be reused.
        let has_shadow_entities = self
            .entities
            .as_ref()
            .is_some_and(|entities| entities.geometry.draws.iter().any(|draw| draw.shadow));
        // Custom images are cleared each frame (`pack_images::clear`) before the
        // shadow pass writes them, so a pack with any redraws it too. A pass after the shadow
        // pass that binds a shadowcolor buffer as an image may change it, so the map kept isn't
        // the shadow pass's any more. And a shadow program writing a buffer that isn't cleared
        // every frame builds on the frame before (an accumulating `shadowcomp` dispatch): the
        // same inputs give another result. The pack's storage buffers (`bufferObject.*`) are
        // never cleared and aren't in the key, so a shadow program that binds them, reading or
        // writing, is never kept either.
        let cache_hit = !has_shadow_entities
            && self.images.is_empty()
            && self.shadow_images_outside == 0
            && !self.shadow_buffers
            && self.shadow_cache.key == Some(cache_key)
            && self
                .shadow_targets
                .as_ref()
                .is_some_and(|t| self.shadow_writes & t.kept() == 0);
        if cache_hit {
            // The targets still hold this map (and `main`, the A/B side
            // `shadowcomp` left current).
            return;
        }
        self.draw_shadow_pass(device, queue, encoder, terrain);
        self.shadow_cache.key = self.shadow_targets.is_some().then_some(cache_key);
    }

    /// The shadow pass body: terrain, entities, clears, and `shadowcomp`.
    pub fn draw_shadow_pass(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        terrain: &mut TerrainDraw<'_>,
    ) {
        let Some(t) = &mut self.shadow_targets else {
            return;
        };
        t.begin_frame();
        t.clear_images(encoder, self.shadow_pass_images);
        let size = [t.size; 2];
        self.draw_terrain(
            device,
            queue,
            encoder,
            size,
            TerrainList::ShadowSolid,
            terrain,
        );
        self.draw_shadow_entities(device, queue, encoder, size);
        let t = self.shadow_targets.as_mut().expect("shadow pass");
        t.clear_rest(encoder);
        t.copy_depth1(encoder);
        self.draw_terrain(
            device,
            queue,
            encoder,
            size,
            TerrainList::ShadowTranslucent,
            terrain,
        );
        let mut comp = std::mem::take(&mut self.shadow_comp);
        for pass in &mut comp {
            let mut compute = std::mem::take(&mut pass.compute);
            for c in &mut compute {
                self.dispatch(device, queue, encoder, c, size);
            }
            pass.compute = compute;
            if matches!(pass.pipeline, Pipeline::Compute { .. }) {
                self.dispatch(device, queue, encoder, pass, size);
                continue;
            }
            let group = self.bind(device, queue, encoder, pass, size);
            let t = self.shadow_targets.as_ref().expect("shadow pass");
            let mut attachments: SmallVec<[Option<wgpu::RenderPassColorAttachment>; BUFFERS]> =
                SmallVec::new();
            // A program that writes only images draws into the sink, which keeps nothing.
            if pass.sink {
                let ops = wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Discard,
                };
                attachments.push(t.sink.as_ref().and_then(|v| attach(v, ops)));
            }
            for &(location, buffer) in &pass.writes {
                let l = location as usize;
                if attachments.len() <= l {
                    attachments.resize(l + 1, None);
                }
                // A shadowcomp shader can leave part of its destination untouched.
                let ops = wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                };
                attachments[l] = t.color(buffer, true).and_then(|v| attach(v, ops));
            }
            {
                let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(&pass.name),
                    color_attachments: &attachments,
                    timestamp_writes: self.timing.as_mut().and_then(|t| t.writes(&pass.name)),
                    ..Default::default()
                });
                rp.set_pipeline(pass.render());
                rp.set_bind_group(0, &group, &[]);
                if pass.buffers
                    && let Some(g) = &self.buffers.group
                {
                    rp.set_bind_group(4, g, &[]);
                }
                rp.draw(0..6, 0..1);
            }
            drop(attachments);
            if let Some(t) = &mut self.shadow_targets {
                t.main ^= pass.flip;
            }
        }
        self.shadow_comp = comp;
    }

    /// Computes the cache key for the current frame.
    pub fn compute_shadow_cache_key(
        &self,
        view: &UniformFrameView<'_>,
        shadow_scene_revision: (u64, u64),
        ticks: u64,
        day_time: i64,
    ) -> ShadowCacheKey {
        let shadow = &self.uniforms.shadow;
        // Use the same logic as shadow_camera to compute the light direction and snapped camera.
        // Use the public shadow_light function which returns the normalized light direction.
        let light = pack_uniforms::shadow_light(view, shadow);
        let light = light.as_dvec3();
        let up = if light.z.abs() > 0.99 {
            DVec3::Y
        } else {
            DVec3::Z
        };
        let rotation = DMat4::look_to_rh(DVec3::ZERO, -light, up);
        let at = rotation.transform_vector3(view.camera);
        let step = f64::from(shadow.interval);
        let snapped_camera = if step > 0.0 {
            Some((at / step).floor() * step)
        } else {
            Some(at)
        };

        // Hash the shadow settings that affect the shadow map.
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        shadow.resolution.hash(&mut hasher);
        shadow.distance.to_bits().hash(&mut hasher);
        shadow.render_mul.to_bits().hash(&mut hasher);
        shadow.interval.to_bits().hash(&mut hasher);
        shadow.sun_path_rotation.to_bits().hash(&mut hasher);
        shadow.fov.map(|f| f.to_bits()).hash(&mut hasher);
        shadow.near.map(f32::to_bits).hash(&mut hasher);
        shadow.far.map(f32::to_bits).hash(&mut hasher);
        for hf in &shadow.hardware_filtering {
            hf.hash(&mut hasher);
        }
        for color in &shadow.colors {
            format!("{:?}", color.format).hash(&mut hasher);
            color.clear.hash(&mut hasher);
            if let Some(cc) = color.clear_color {
                for c in cc {
                    c.to_bits().hash(&mut hasher);
                }
            }
            color.mipmap.hash(&mut hasher);
        }
        let shadow_settings_hash = hasher.finish();
        let shadow_uniforms_hash = self.shadow_uniforms_hash();

        ShadowCacheKey {
            sun_angle: view.sky.sun_angle,
            moon_angle: view.sky.moon_angle,
            snapped_camera,
            shadow_settings_hash,
            shadow_scene_revision,
            ticks,
            day_time,
            shadow_uniforms_hash,
        }
    }

    pub fn shadow_uniforms_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let passes = self
            .shadow_terrain
            .iter()
            .chain(self.shadow_water.iter())
            .chain(self.shadow_comp.iter())
            .chain(
                self.entities
                    .iter()
                    .flat_map(|entities| entities.shadow.iter().flatten()),
            )
            // Their compute stages read uniforms of their own.
            .flat_map(|p| std::iter::once(p).chain(&p.compute));
        for pass in passes {
            pass.name.hash(&mut hasher);
            for (index, ty, offset) in &pass.uniforms {
                index.hash(&mut hasher);
                ty.hash(&mut hasher);
                offset.hash(&mut hasher);
                let Some(value) = index.and_then(|index| self.uniforms.values.get(index)) else {
                    continue;
                };
                match value {
                    pack_uniforms::Value::F(values) => {
                        0u8.hash(&mut hasher);
                        for value in values {
                            value.to_bits().hash(&mut hasher);
                        }
                    }
                    pack_uniforms::Value::I(values) => {
                        1u8.hash(&mut hasher);
                        values.hash(&mut hasher);
                    }
                }
            }
        }
        hasher.finish()
    }
}
