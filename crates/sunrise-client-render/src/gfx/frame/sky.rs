use super::super::*;

impl Renderer {
    /// The timed native sky disc, before the engine draws the remaining sky features.
    pub(in super::super) fn draw_timed_native_sky(
        native: &mut Option<native_pack::NativePack>,
        device: &wgpu::Device,
        group: &wgpu::BindGroup,
        encoder: &mut wgpu::CommandEncoder,
        (world_view, world_depth, format): (
            &wgpu::TextureView,
            &wgpu::TextureView,
            wgpu::TextureFormat,
        ),
        clear: wgpu::Color,
    ) {
        if let Some(native) = native
            && let Some(sky) = &native.sky
        {
            if let Some(t) = &mut native.timing {
                t.mark(device, encoder, (world_view, format));
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(&sky.label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: world_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: world_depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: native.timing.as_mut().and_then(|t| t.writes(&sky.label)),
                ..Default::default()
            });
            pass.set_bind_group(0, group, &[]);
            sky.draw(&mut pass, &native.frame);
        }
    }

    pub(in super::super) fn draw_sky_pass_content(
        &self,
        pass: &mut pack_timing::CountedPass<'_>,
        frame: &Frame,
        sky_visible: bool,
        pack_draws_sky: bool,
        native_sky_pass: bool,
        celestial_vertices: usize,
        pack_draws_celestial: bool,
        sky_switches: sunrise_shaders::SkySwitches,
        hdr_gain: bool,
    ) {
        if frame.title || (sky_visible && (frame.skybox || frame.end_sky)) {
            if !frame.title && frame.skybox && !pack_draws_sky && !native_sky_pass {
                match self
                    .native
                    .as_ref()
                    .and_then(|n| Some((n.sky.as_ref()?, &n.frame)))
                {
                    Some((sky, native)) => sky.draw(pass, native),
                    None => {
                        // A shader pack's `sky=false` leaves out the disc and the sunrise fan.
                        if sky_switches.sky {
                            match self.core_sky.as_ref().filter(|c| c.draws_disc()) {
                                Some(core) => {
                                    core.draw_disc(pass);
                                    pass.set_bind_group(0, &self.group, &[]);
                                }
                                None => {
                                    pass.set_pipeline(&self.sky_base);
                                    pass.draw(0..3, 0..1);
                                }
                            }
                        }
                        if sky_switches.sky && frame.sky.sunrise_color[3] > 0.001 {
                            pass.set_pipeline(&self.sunrise_pipeline);
                            pass.set_vertex_buffer(0, self.sunrise_buffer.slice(..));
                            pass.draw(0..sunrise_fan::VERTICES as u32, 0..1);
                        }
                        // OptiFine custom sky layers, over the disc and sunrise and under the
                        // sun, moon and stars.
                        if self.custom_sky.is_drawing() {
                            self.custom_sky.draw(pass);
                            pass.set_bind_group(0, &self.group, &[]);
                        }
                    }
                }
            }
            if frame.title {
                self.panorama.draw(pass);
                pass.set_bind_group(0, &self.group, &[]);
            } else if celestial_vertices > 0 || frame.end_sky {
                pass.set_pipeline(&self.celestial.added);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                if frame.end_sky {
                    self.end_sky.draw(pass);
                    // The End's custom sky layers go over its sky box.
                    if self.custom_sky.is_drawing() && !pack_draws_sky && self.native.is_none() {
                        self.custom_sky.draw(pass);
                        pass.set_pipeline(&self.celestial.added);
                        pass.set_bind_group(0, &self.entity_group, &[]);
                    }
                }
                if celestial_vertices > 0 && !pack_draws_celestial {
                    // A pack sky drawing its own sun skips vanilla's.
                    let own_sun = self
                        .native
                        .as_ref()
                        .is_some_and(|n| n.sky.is_some() && !n.vanilla_sun);
                    // A resource pack's core/sky isn't vanilla's sky times the sky's
                    // lift, which the sun's disc pass assumes: today's add there.
                    let lifted = hdr_gain
                        && self.hdr.as_ref().is_some_and(|h| h.lighting)
                        && !self.core_sky.as_ref().is_some_and(|c| c.draws_disc());
                    let bodies = vanilla_bodies(
                        frame.celestial_bodies,
                        celestial_vertices / 4,
                        own_sun,
                        sky_switches,
                    );
                    pass.set_vertex_buffer(0, self.celestial_buffer.slice(..));
                    self.celestial.draw_bodies(pass, lifted, bodies);
                }
                pass.set_bind_group(0, &self.group, &[]);
            }
            let own_stars = self
                .native
                .as_ref()
                .is_some_and(|n| n.sky.is_some() && !n.vanilla_stars);
            if !frame.title
                && frame.stars.is_some()
                && !pack_draws_sky
                && !own_stars
                && sky_switches.stars
            {
                match self.core_sky.as_ref().filter(|c| c.draws_stars()) {
                    Some(core) => core.draw_stars(pass),
                    None => self.stars.draw(pass),
                }
                pass.set_bind_group(0, &self.group, &[]);
            }
            if !frame.title && frame.dark_disc && !pack_draws_sky && sky_switches.sky {
                pass.set_pipeline(&self.dark_disc);
                pass.draw(0..3, 0..1);
            }
        }
    }
}

/// The celestial quads the game draws itself, by their place in `bodies` (what each quad is,
/// `Frame::celestial_bodies`), of the `quads` uploaded: `own_sun` (a pack sky drawing its own)
/// or a shader pack's `sun=false` leaves the sun out, `moon=false` the moon. The End's flash is
/// neither and always draws. An entry past the quads uploaded has no quad to draw.
fn vanilla_bodies(
    bodies: &[CelestialBody],
    quads: usize,
    own_sun: bool,
    switches: sunrise_shaders::SkySwitches,
) -> impl Iterator<Item = (u32, CelestialBody)> + '_ {
    let drawn = move |body: CelestialBody| match body {
        CelestialBody::Sun => !own_sun && switches.sun,
        CelestialBody::Moon => switches.moon,
        CelestialBody::EndFlash => true,
    };
    (0u32..)
        .zip(bodies.iter().copied().take(quads))
        .filter(move |&(_, body)| drawn(body))
}

#[cfg(test)]
#[path = "sky_switch_tests.rs"]
mod sky_switch_tests;
