//! A shader pack's post-processing over Sunrise's own world (spec 2.1 rows 4, 6, 8, 9; 6.1; 6.2;
//! 6.4): the world renders into its own target as with a resolution scale, its colour becomes
//! colortex0 and its depth `depthtex0`, then the pack's `prepare`, `deferred`, `composite` and
//! `final` programs run in order, and `final` draws onto the screen under the hand and HUD.
//!
//! What the pack is, what it builds and the frame's work on it now live in
//! sunrise_render_shaderpack's `pack_post`: this module re-exports them unchanged. The few entry
//! points that still take the client's own `Frame`, entity vertices and skin frame (though the
//! shader pack crate stays clear of those) are `PackPostExt`, a thin bridge that builds the
//! crate's decoupled `UniformFrameView`, `SkyInputs` and `HandInputs` from the frame.

// `types` re-exports the crate's entries and exits; the rest comes straight from the crate.
mod types;

pub use types::*;

pub use sunrise_render_shaderpack::pack_post::construction::*;
pub use sunrise_render_shaderpack::pack_post::execution::*;

use super::pack_entities::PackEntitiesExt as _;
use crate::gfx::Frame;
use crate::gfx::entity_skin::SkinFrame;
use crate::gfx::pack_draw::EntitySpan;
use sunrise_render_core::{SkyView, UniformFrameView};
use sunrise_render_shaderpack::pack_uniforms::PackInputs;

pub(crate) use super::external::ShadowView;

/// The shader pack's `PackPost` methods that still take the client's own `Frame`: `run`, whose
/// `Frame` it turns into the crate's `UniformFrameView`, sky and hand inputs; `shadow_view`; and
/// `prepare_entities`, which uploads the frame's `Vertex` runs and its `SkinFrame`.
pub trait PackPostExt: Sized {
    /// The shadow pass this frame wants, when the pack draws one and this frame doesn't keep the
    /// last shadow map.
    fn shadow_view(&mut self, frame: &Frame) -> Option<ShadowView>;
    /// The entities and block entities the pack draws this frame, into its own buffers.
    fn prepare_entities(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        entities: &[crate::gfx::Vertex],
        spans: &[EntitySpan],
        crumbling: [f32; 4],
        portal: [f32; 4],
        rest: &mut Vec<crate::gfx::Vertex>,
        skin: Option<&SkinFrame>,
        motion: (&[(u32, u64)], &[u32]),
    );
    /// Runs the pack over the world drawn into `world` (colour) and `world_depth`, then draws
    /// the result onto `screen`. Fails once the device is lost (`lost`).
    fn run(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        world: &wgpu::TextureView,
        world_depth: &wgpu::TextureView,
        screen: &wgpu::TextureView,
        frame: &Frame,
        inputs: &PackInputs,
        size: [u32; 2],
        window: [u32; 2],
        pages: &[TerrainPage],
        terrain: &mut TerrainDraw<'_>,
        lost: &std::sync::atomic::AtomicBool,
        poll: sunrise_render_shaderpack::PollLive,
    ) -> Result<(), String>;
    /// The pack's GPU pass timer (`pack_timing::PassTiming` in this crate), in the slot the
    /// renderer's timing API hands out and reads beside Sunrise's own timer.
    fn pass_timer(&mut self) -> &mut Option<sunrise_render_shaderpack::pack_timing::PassTiming>;
    /// The pack's GPU pass timer, read while the renderer collects its readings.
    fn timing_pack(&self) -> &Option<sunrise_render_shaderpack::pack_timing::PassTiming>;
}

/// The shader pack crate's `UniformFrameView` for the client's `Frame`: the fields it reads.
fn frame_view<'a>(frame: &'a Frame) -> UniformFrameView<'a> {
    UniformFrameView {
        camera: frame.camera,
        yaw: frame.yaw,
        pitch: frame.pitch,
        fov_degrees: frame.fov_degrees,
        eye: frame.eye,
        eye_rotation: frame.eye_rotation,
        first_person: frame.first_person,
        bob: frame.bob,
        spin: frame.spin,
        sky: SkyView {
            sun_angle: frame.sky.sun_angle,
            moon_angle: frame.sky.moon_angle,
            sky_color: frame.sky.sky_color,
            fog_color: frame.sky.fog_color,
        },
        distance: frame.distance,
        detail_distance: frame.detail_distance,
        depth_far: frame.depth_far,
        fog: frame.fog.as_ref().map(|f| sunrise_render_core::FogView {
            color: [f.color[0], f.color[1], f.color[2]],
            environmental_start: f.environmental_start,
            environmental_end: f.environmental_end,
        }),
        night_vision: frame.night_vision,
        darkness: frame.darkness,
        dynamic_lights: frame.dynamic_lights,
        entities: frame.entities,
        lightning_bolt: frame
            .lightning
            .bolts
            .last()
            .map(|bolt| glam::DVec3::from_array(bolt.at)),
    }
}

impl PackPostExt for PackPost {
    /// The shadow camera this frame draws of the terrain, when the pack draws it and this frame
    /// doesn't keep the last shadow map (Starling: one map serves `shadow_every` frames on a phone).
    fn shadow_view(&mut self, frame: &Frame) -> Option<ShadowView> {
        if !self.has_shadow() {
            return None;
        }
        self.uniforms.keep_shadow = self.shadow_wait > 0 && self.uniforms.shadow_near(frame.camera);
        if self.uniforms.keep_shadow {
            self.shadow_wait -= 1;
            return None;
        }
        self.shadow_wait = self.shadow_every - 1;
        let shadow = &self.uniforms.shadow;
        let unculled = match self.shadow_culling {
            sunrise_shaders::ShadowCulling::On => None,
            sunrise_shaders::ShadowCulling::Off => Some(f32::INFINITY),
            sunrise_shaders::ShadowCulling::Reversed => Some(shadow.voxel_distance),
        };
        Some(
            ShadowView::new(
                super::pack_uniforms::shadow_culling(frame, shadow),
                super::pack_uniforms::shadow_light(frame, shadow),
            )
            .with_distance((shadow.render_mul > 0.0).then_some(shadow.distance * shadow.render_mul))
            .with_unculled(unculled)
            .with_every_face(true)
            .with_view_casters(true),
        )
    }

    /// Splits `entities` (`Frame::entities`) between the pack and Sunrise by its runs
    /// (`spans`): the pack's part is uploaded for this frame's draw, Sunrise's goes to `rest`.
    fn prepare_entities(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        entities: &[crate::gfx::Vertex],
        spans: &[EntitySpan],
        crumbling: [f32; 4],
        portal: [f32; 4],
        rest: &mut Vec<crate::gfx::Vertex>,
        skin: Option<&SkinFrame>,
        motion: (&[(u32, u64)], &[u32]),
    ) {
        self.portal_rect = portal;
        let shadow = self.shadow_casters();
        let Some(e) = &mut self.entities else {
            rest.clear();
            rest.extend_from_slice(entities);
            return;
        };
        let drawn = e.passes.each_ref().map(Option::is_some);
        e.geometry.prepare_client(
            device,
            queue,
            &self.draws.layout,
            entities,
            spans,
            drawn,
            (crumbling, self.gui_size),
            shadow,
            rest,
            skin,
            motion,
        );
    }

    /// Runs the pack over the world drawn into `world` (colour) and `world_depth`, then draws
    /// the result onto `screen`.
    fn run(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        world: &wgpu::TextureView,
        world_depth: &wgpu::TextureView,
        screen: &wgpu::TextureView,
        frame: &Frame,
        inputs: &PackInputs,
        size: [u32; 2],
        window: [u32; 2],
        pages: &[TerrainPage],
        terrain: &mut TerrainDraw<'_>,
        lost: &std::sync::atomic::AtomicBool,
        poll: sunrise_render_shaderpack::PollLive,
    ) -> Result<(), String> {
        let view = frame_view(frame);
        let celestial_bodies: Vec<_> = frame
            .celestial_bodies
            .iter()
            .map(|body| match body {
                crate::gfx::CelestialBody::Sun => sunrise_render_shaderpack::CelestialBody::Sun,
                crate::gfx::CelestialBody::Moon => sunrise_render_shaderpack::CelestialBody::Moon,
                crate::gfx::CelestialBody::EndFlash => {
                    sunrise_render_shaderpack::CelestialBody::EndFlash
                }
            })
            .collect();
        let sky = SkyInputs {
            sunset_color: frame.sky.sunrise_color,
            stars: frame.stars,
            dark_disc: frame.dark_disc,
            celestial: frame.celestial,
            celestial_bodies: &celestial_bodies,
            visible: super::sky_visible(frame),
            end_fog_sky: super::end_fog_sky(frame),
        };
        let hand = HandInputs {
            vertices: frame.hand,
            items: frame.hand_items,
            fov_degrees: frame.hand_fov_degrees,
            bob: frame.bob,
            spin: frame.spin,
        };
        self.run_frame(
            device,
            queue,
            encoder,
            world,
            world_depth,
            screen,
            &view,
            &sky,
            &hand,
            inputs,
            size,
            window,
            pages,
            terrain,
            frame.shadow_cache,
            frame.shadow_scene_revision,
            frame.ticks,
            frame.pack.day_time,
            lost,
            poll,
        )
    }

    /// The pack's GPU pass timer, as the renderer's timing API reaches it.
    fn pass_timer(&mut self) -> &mut Option<sunrise_render_shaderpack::pack_timing::PassTiming> {
        &mut self.timing
    }

    /// The pack's GPU pass timer, read while the renderer collects its readings.
    fn timing_pack(&self) -> &Option<sunrise_render_shaderpack::pack_timing::PassTiming> {
        &self.timing
    }
}

#[cfg(test)]
mod history_reset_tests;

#[cfg(test)]
mod world_target_tests;

#[cfg(test)]
mod celestial_blend_tests;

#[cfg(test)]
mod sky_switch_tests;

#[cfg(test)]
mod higher_shadowcolor_tests;

#[cfg(test)]
mod shadowcolor_image_tests;

#[cfg(test)]
mod single_side_tests;

#[cfg(test)]
mod mixed_size_tests;

#[cfg(test)]
mod sky_disc_blend_tests;

#[cfg(test)]
mod undrawn_terrain_tests;

#[cfg(test)]
mod line_side_tests;

#[cfg(test)]
mod geometry_input_tests;

#[cfg(test)]
mod full_outputs_tests;

#[cfg(test)]
mod shadow_faces_tests;

#[cfg(test)]
mod r11f_image_tests;

#[cfg(test)]
mod work_group_fold_tests;

#[cfg(test)]
mod forced_flip_tests;

#[cfg(test)]
mod weather_depth_tests;

#[cfg(test)]
mod float_into_integer_tests;

#[cfg(test)]
mod custom_type_tests;

#[cfg(test)]
mod sky_depth_tests;

#[cfg(test)]
mod depth_copy_tests;

#[cfg(test)]
mod end_fan_tests;

#[cfg(test)]
mod runtime_length_tests;

#[cfg(test)]
mod translucent_color_tests;

#[cfg(test)]
mod noise_resolution_tests;

#[cfg(test)]
mod player_boolean_tests;

#[cfg(test)]
mod snorm_tests;

#[cfg(test)]
mod terrain_parts_tests;

#[cfg(test)]
mod clear_budget_tests;

#[cfg(test)]
mod clear_false_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod depth_test;

#[cfg(test)]
mod destination_tests;

#[cfg(test)]
#[path = "../pack_material_tests.rs"]
mod material_tests;

#[cfg(test)]
#[path = "../pack_paged_maps_tests.rs"]
mod paged_maps_tests;

#[cfg(test)]
mod rgb_alpha_tests;

#[cfg(test)]
mod packed_tests;

#[cfg(test)]
mod trinary_tests;

#[cfg(test)]
mod const_offset_tests;

#[cfg(test)]
mod integer_colortex0_tests;

#[cfg(test)]
mod atlas_mip_tests;

#[cfg(test)]
mod wrong_type_uniform_tests;

#[cfg(test)]
mod atlas_name_post_tests;

#[cfg(test)]
mod unknown_sampler_terrain_tests;

#[cfg(test)]
mod mipmap_box_tests;

#[cfg(test)]
mod colortex_mip_blend_tests;

#[cfg(test)]
mod material_route_tests;
