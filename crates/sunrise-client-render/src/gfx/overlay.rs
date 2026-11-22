//! Overlay counts and offsets for Renderer::draw_overlay, and the overlay pass itself.

use super::*;

/// Counts and offsets `Renderer::draw_overlay` draws from this frame's buffers.
pub(super) struct Overlay {
    /// Zero when a shader pack draws the hand.
    pub(super) hand_quads: u32,
    pub(super) first_hand_vertex: i32,
    pub(super) crosshair: u32,
    pub(super) first_crosshair: u32,
    /// The world drew into `depth` (post effects may read it), not a scaled target's.
    pub(super) world_depth: bool,
    /// The real frame, not frame generation's made one: the world icon grabs it.
    pub(super) real: bool,
    /// The game drew the first-person hand, in its own pass or at the end of the world's
    /// (`hand_in_world`), not a shader pack.
    pub(super) hand_drawn: bool,
    /// Camera-relative world to clip space for the hand: its own field of view and the bob.
    pub(super) hand_view_proj: Mat4,
}

impl Renderer {
    /// A frame of a paused menu from the blurred frame `MenuBlur::keep` kept: that frame, then
    /// the menu over it, with the section copies and HUD uploads of the frame, submitted and
    /// presented as `draw_frame` presents a plain one. The world and the blur are not drawn.
    /// Adapted from Vasysik/sunrise-android (the cached path of its `Renderer::render`, commit
    /// 77b9e17f76), MIT, copyright (c) 2026 Ludovic Marie and the Horizon contributors.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_kept_menu(
        &mut self,
        frame: &Frame,
        mut encoder: wgpu::CommandEncoder,
        has_section_copies: bool,
        target_texture: &wgpu::Texture,
        surface_frame: Option<wgpu::SurfaceTexture>,
        reconfigure: bool,
        started: Instant,
    ) -> Result<bool> {
        let size = [self.config.width, self.config.height];
        self.hud.prepare(
            &self.device,
            &mut self.uploads,
            &mut encoder,
            frame.hud,
            size,
        );
        // `has_menu_blur_cache` holds only where the HUD draws in the frame's own format.
        let view_target = target_texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.output.frame.format),
            ..Default::default()
        });
        if let Some(blur) = &self.menu_blur {
            blur.draw_kept(&self.device, &self.queue, &mut encoder, &view_target);
        }
        self.hud.draw_range(
            &mut encoder,
            &view_target,
            None,
            &self.depth,
            frame.hud,
            size,
            frame.hud.blur_at..frame.hud.vertices.len() as u32,
            None,
        );
        self.uploads.finish(&encoder);
        if has_section_copies {
            self.section_copy_bytes = 0;
            self.section_uploads.finish(&encoder);
        }
        if let Some(image) = surface_frame
            .as_ref()
            .map(|f| &f.texture)
            .or(self.offscreen.as_ref())
        {
            self.readback.present(&self.device, &mut encoder, image);
        }
        let commands = crate::spans::time("encode", || encoder.finish());
        let submission = crate::spans::time("submit", || self.queue.submit([commands]));
        self.last_wait += self.queue.take_waited();
        let present = Instant::now();
        if let Some(surface_frame) = surface_frame {
            if self.config.present_mode == wgpu::PresentMode::Fifo {
                self.window.pre_present_notify();
            }
            self.queue.present(surface_frame);
            if reconfigure {
                self.configure_surface();
            }
        }
        if self.surface.is_none() {
            self.in_flight.push_back(submission);
            if !self.paced_elsewhere {
                self.wait_in_flight();
            }
        }
        self.last_wait += present.elapsed();
        self.queue.take_waited();
        self.last_encode = started.elapsed().saturating_sub(self.last_wait);
        Ok(true)
    }
    /// What draws over the world, on the real frame and frame generation's made one:
    /// the hand, entity outlines, post effects, screen effects, the debug crosshair and
    /// the HUD. Returns the world icon's readback, taken on the real frame.
    pub(super) fn draw_overlay(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &Frame,
        view_target: &wgpu::TextureView,
        target_texture: &wgpu::Texture,
        overlay: &Overlay,
    ) -> Option<(ScreenshotRequest, wgpu::Buffer)> {
        // Sunrise's own pass timings (#5887) time the real frame's overlay only: a made frame's
        // would count the same passes twice.
        let mut timing = if overlay.real {
            self.own_timing.take()
        } else {
            None
        };
        // HDR output's layered frame keeps the world after the hand and the camera overlays,
        // as light (`Hdr::layered`); else it is kept here, before them.
        let layered = self.hdr.as_ref().is_some_and(|h| h.layered());
        if let Some(hdr) = &self.hdr {
            if layered {
                hdr.begin(&self.queue, !frame.title);
            } else {
                hdr.keep_world(encoder, timing.as_mut());
            }
        }
        if overlay.hand_quads > 0 {
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view: view_target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("First-person hand"),
                timestamp_writes: timing.as_mut().and_then(|t| t.writes("Hand")),
                color_attachments: &attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        // HDR output's hand marks test against it (below).
                        store: if self.hdr.is_some() {
                            wgpu::StoreOp::Store
                        } else {
                            wgpu::StoreOp::Discard
                        },
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(self.overlay_hand.as_ref().unwrap_or(&self.entity_pipeline));
            pass.set_bind_group(0, &self.hand_group, &[]);
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
            pass.draw_indexed(0..overlay.hand_quads * 6, overlay.first_hand_vertex, 0..1);
            drop(pass);
            if let Some(hdr) = &self.hdr
                && !no_pack_hdr_gain(layered, self.shader_pack_kind().is_some(), !frame.title)
            {
                hdr.mark_hand(encoder, timing.as_mut());
                // The non-layered 0.2 hand keeps its UI-brightness sentinel, not light marks.
                if light_marks(self.output.frame.format, true)
                    && let Some(view) = hdr.hand_light_view()
                {
                    // The light sources in the hand may shine, the rest of it stays at the
                    // world's brightness: nothing the world kept here shows through it.
                    let attachments = [Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })];
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("HDR hand marks"),
                        timestamp_writes: timing.as_mut().and_then(|t| t.writes("HDR hand marks")),
                        color_attachments: &attachments,
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &self.depth,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Discard,
                            }),
                            stencil_ops: None,
                        }),
                        ..Default::default()
                    });
                    pass.set_bind_group(0, &self.hand_group, &[]);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                    let quads = 0..overlay.hand_quads * 6;
                    pass.set_pipeline(&self.hdr_hand_unmark_pipeline);
                    pass.draw_indexed(quads.clone(), overlay.first_hand_vertex, 0..1);
                    pass.set_pipeline(&self.hdr_entity_mark_pipeline);
                    pass.draw_indexed(quads, overlay.first_hand_vertex, 0..1);
                }
            }
        }
        // A native mod's turn after the hand, on the real frame only.
        if overlay.real && !frame.title {
            native_render::hand_drawn(&mut native_render::HandDrawn {
                device: &self.device,
                queue: &self.queue,
                encoder: &mut *encoder,
                color: target_texture,
                color_view: view_target,
                color_format: self.output.frame.overlay_format,
                size: [self.config.width, self.config.height],
                hand: if overlay.hand_drawn { frame.hand } else { &[] },
                view_proj: overlay.hand_view_proj,
                camera: frame.camera,
                atlas: self.hud.atlas_texture.as_ref(),
                quad_indices: &self.indices,
            });
        }
        // GameRenderer.render: LevelRenderer.blitEntityOutline after the level and the hand.
        self.entity_outline.draw(
            encoder,
            view_target,
            &self.entity_group,
            &self.indices,
            &self.entity_buffer,
            timing.as_mut(),
        );
        // The world icon grabs here: after the level and the hand, before
        // the effects and the HUD draw over them (`tryTakeScreenshotIfNeeded`
        // runs right after `renderLevel`).
        let readback = self
            .screenshot
            .take_if(|request| overlay.real && request.size.is_some())
            .and_then(|request| self.take_readback(request, encoder, target_texture));
        // GameRenderer.render: applyPostEffects, after the level and before the GUI.
        if !frame.title
            && !frame.post_effects.effects.is_empty()
            && target_texture
                .usage()
                .contains(wgpu::TextureUsages::COPY_SRC)
        {
            let device = &self.device;
            let post = self
                .post_effects
                .get_or_insert_with(|| crate::post_chain::PostEffects::new(device));
            // The chains a spectated mob applies compile in the background from the
            // first frame in a world, not in the frame that first needs one.
            post.preload(
                &self.device,
                &self.queue,
                frame.post_effects.root,
                crate::spectator_shaders::chains(),
                target_texture.format(),
            );
            post.apply(
                &self.device,
                &self.queue,
                encoder,
                target_texture,
                view_target,
                [self.config.width, self.config.height],
                // The scaled world pass drew its own depth buffer.
                overlay.world_depth.then_some(&self.depth),
                &frame.post_effects,
                timing.as_mut(),
            );
        }
        // The layered frame's light eases into the peak, before the camera overlays draw on it,
        // unless a 0.2 pack's own `[tonemap]` did.
        let pack_tonemap = self
            .native
            .as_ref()
            .and_then(|n| n.post.as_ref())
            .is_some_and(native_post::NativePost::tonemaps);
        if sunrise_tone_curve(layered, !frame.title, pack_tonemap)
            && let Some(hdr) = &self.hdr
        {
            hdr.tonemap(encoder, timing.as_mut());
        }
        if !frame.screen.is_empty() {
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view: view_target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Screen effects"),
                timestamp_writes: timing.as_mut().and_then(|t| t.writes("Screen effects")),
                color_attachments: &attachments,
                ..Default::default()
            });
            pass.set_bind_group(0, &self.entity_group, &[]);
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.set_vertex_buffer(0, self.screen_buffer.slice(..));
            for (i, quad) in frame.screen.iter().enumerate() {
                let blend = match quad.blend {
                    crate::screen_effects::Blend::Alpha => 0,
                    crate::screen_effects::Blend::Vignette => 4,
                };
                let tint = if quad.world_tint && self.hdr.is_some() {
                    8
                } else {
                    0
                };
                pass.set_pipeline(
                    &self.screen_pipelines[tint + blend + (quad.sampling & 3) as usize],
                );
                pass.draw_indexed(0..6, (i * 4) as i32, 0..1);
            }
            drop(pass);
            // A tint leaves the frame's alpha and goes over the kept world too: light sources
            // under water still show the world, and still shine.
            if let Some(view) = self.hdr.as_ref().and_then(|h| h.hand_light_view())
                && frame.screen.iter().any(|quad| quad.world_tint)
            {
                let attachments = [Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })];
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Screen tints over the kept world"),
                    timestamp_writes: None,
                    color_attachments: &attachments,
                    ..Default::default()
                });
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_vertex_buffer(0, self.screen_buffer.slice(..));
                for (i, quad) in frame.screen.iter().enumerate() {
                    if quad.world_tint {
                        pass.set_pipeline(&self.screen_pipelines[8 + (quad.sampling & 3) as usize]);
                        pass.draw_indexed(0..6, (i * 4) as i32, 0..1);
                    }
                }
            }
        }
        if layered && let Some(hdr) = &self.hdr {
            hdr.keep_world(encoder, timing.as_mut());
        }
        // What the HUD and menus draw on: the layered frame's HUD target, else the frame.
        let hud = self
            .hdr
            .as_ref()
            .and_then(|h| h.hud())
            .map(|t| t.create_view(&Default::default()));
        let hud_target = hud.as_ref().unwrap_or(view_target);
        let coverage = self.hdr.as_ref().and_then(|h| h.coverage_view());
        if let Some(view) = &coverage {
            hud_coverage::clear(encoder, view);
        }
        // GameRenderer.render3dHud: the debug crosshair after the hand and
        // the screen effects.
        if overlay.crosshair > 0 {
            let mut attachments = vec![Some(wgpu::RenderPassColorAttachment {
                view: hud_target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })];
            if let Some(view) = &coverage {
                attachments.push(Some(hud_coverage::attachment(view)));
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3d crosshair"),
                timestamp_writes: timing.as_mut().and_then(|t| t.writes("Crosshair")),
                color_attachments: &attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(match &coverage {
                Some(_) => self.debug_crosshair.as_ref().unwrap(),
                None => self
                    .overlay_crosshair
                    .as_ref()
                    .unwrap_or(&self.debug_lines_on_top),
            });
            pass.set_bind_group(0, &self.crosshair_group, &[]);
            pass.set_vertex_buffer(0, self.debug_line_buffer.slice(..));
            let first = overlay.first_crosshair;
            pass.draw(0..4, first..first + overlay.crosshair);
        }
        let menu_blur = frame
            .menu_blur
            .filter(|_| self.config.usage.contains(wgpu::TextureUsages::COPY_SRC));
        let blur_at = menu_blur.map_or(0, |_| frame.hud.blur_at);
        self.hud.draw_range(
            encoder,
            hud_target,
            coverage.as_ref(),
            &self.depth,
            frame.hud,
            [self.config.width, self.config.height],
            0..blur_at,
            timing.as_mut(),
        );
        if let Some(radius) = menu_blur {
            let format = self.output.frame.overlay_format;
            let device = &self.device;
            let software = is_software(self.info.device_type);
            let blur = self
                .menu_blur
                .get_or_insert_with(|| crate::menu_blur::MenuBlur::new(device, format, software));
            let size = [self.config.width, self.config.height];
            match self.hdr.as_ref().filter(|_| layered) {
                // The blur keeps light above white: it blurs the picture so far, the HUD over
                // the world's light, which becomes the world the menu draws over.
                Some(hdr) => {
                    let (_, source) = blur.source(device, size);
                    hdr.composite(encoder, &source, timing.as_mut());
                    blur.blur(
                        device,
                        &self.queue,
                        encoder,
                        view_target,
                        size,
                        radius,
                        timing.as_mut(),
                    );
                    hdr.keep_world(encoder, timing.as_mut());
                }
                None => {
                    blur.apply(
                        device,
                        &self.queue,
                        encoder,
                        target_texture,
                        view_target,
                        size,
                        radius,
                        timing.as_mut(),
                    );
                    // HDR output never draws the kept frame back (`has_menu_blur_cache`).
                    if frame.menu_blur_cache && overlay.real {
                        blur.keep(encoder, target_texture, size, radius);
                    }
                }
            }
            if let Some(view) = &coverage {
                hud_coverage::clear(encoder, view);
            }
        } else if let Some(blur) = &mut self.menu_blur {
            blur.release();
        }
        self.hud.draw_range(
            encoder,
            hud_target,
            coverage.as_ref(),
            &self.depth,
            frame.hud,
            [self.config.width, self.config.height],
            blur_at..frame.hud.vertices.len() as u32,
            timing.as_mut(),
        );
        if overlay.real {
            self.own_timing = timing;
        }
        readback
    }
}
