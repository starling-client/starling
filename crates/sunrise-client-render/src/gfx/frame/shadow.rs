use super::super::*;

impl Renderer {
    pub(in super::super) fn draw_shadow_passes(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &Frame,
        shadow_view: Option<&external::ShadowView>,
        flat_frame: bool,
        shadow_first: u32,
        shadow_solid: u32,
        shadow_translucent: u32,
        entity_quads: u32,
        shadow_only_quads: u32,
        outline_base: u32,
    ) {
        // One of Sunrise's own packs with `[shadow]`: the solid terrain the light sees, from the
        // shadow camera's lists, into the pack's shadow map (a pack without one gets its 1×1 map
        // cleared to "far" once).
        if !frame.title
            && let Some(native) = &mut self.native
        {
            native.frame.shadow_map.clear_blank(encoder);
        }
        if !frame.title
            && shadow_view.is_some()
            && let Some(native) = &self.native
            && let Some(shadow) = &native.shadow
        {
            let mut pass = native.frame.shadow_map.begin(
                encoder,
                "Native shader pack shadow",
                self.own_timing.as_mut(),
            );
            pass.set_bind_group(0, &self.group, &[]);
            pass.set_bind_group(sunrise_shader_api::stages::GROUP, &shadow.group, &[]);
            if flat_frame
                && let Some(flat) = &self.flat
                && let Some(pipeline) = &shadow.flat
            {
                pass.set_pipeline(pipeline);
                let [first, translucent] = flat.shadow_calls;
                flat.draw(
                    &mut pass,
                    &flat.calls[first..translucent],
                    flat_vertices(&self.arena, self.translucent_arena.as_ref()),
                );
            } else if !flat_frame {
                pass.set_pipeline(&shadow.solid);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_vertex_buffer(1, self.instances.slice(..));
                self.draw_solid_sections(&mut pass, &self.indirect, 0, shadow_first, shadow_solid);
            }
            // `[displacement]`'s grid casts in place of the flat quads it took over.
            if let Some((d, picks, pipeline)) = native
                .displacement
                .as_ref()
                .and_then(|d| Some((d, d.shadow.as_ref()?, shadow.displaced.as_ref()?)))
            {
                d.draw(&mut pass, picks, pipeline);
            }
            drop(pass);
            let mut pass = native.frame.shadow_map.begin_translucent(
                encoder,
                "Native shader pack translucent shadow",
                self.own_timing.as_mut(),
            );
            pass.set_bind_group(0, &self.group, &[]);
            pass.set_bind_group(sunrise_shader_api::stages::GROUP, &shadow.group, &[]);
            if flat_frame
                && let Some(flat) = &self.flat
                && let Some(pipeline) = &shadow.flat_translucent
            {
                pass.set_pipeline(pipeline);
                flat.draw(
                    &mut pass,
                    flat.shadow_translucent_calls(),
                    flat_vertices(&self.arena, self.translucent_arena.as_ref()),
                );
            } else if !flat_frame {
                pass.set_pipeline(&shadow.translucent);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_vertex_buffer(1, self.instances.slice(..));
                self.draw_sections(
                    &mut pass,
                    &self.indirect,
                    0,
                    shadow_first + shadow_solid,
                    shadow_translucent,
                );
            }
            drop(pass);
            if entity_quads > 0 || shadow_only_quads > 0 {
                let mut pass = native
                    .frame
                    .shadow_map
                    .begin_entities(encoder, "Native shader pack entity shadow");
                pass.set_pipeline(&shadow.entity);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_bind_group(sunrise_shader_api::stages::GROUP, &shadow.group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..entity_quads * 6, 0, 0..1);
                // Before the GPU-posed outlines (`EntityBatch::append_shadow_only`).
                let first = outline_base - shadow_only_quads * 4;
                pass.draw_indexed(0..shadow_only_quads * 6, first as i32, 0..1);
            }
        }
    }
}
