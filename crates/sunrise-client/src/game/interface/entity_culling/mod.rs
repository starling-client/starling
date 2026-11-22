//! Entity mesh occlusion and view-distance checks.
use super::{
    Game, Vertex,
    entity::RenderEntityBounds,
    occlusion_culling::{AsyncOcclusionCuller, opaque},
    particle_culling::section_scene,
};
use sunrise_client::graphics::EntityCullingMode;
use sunrise_client::voxel_occlusion::Culler as OcclusionCuller;
pub use sunrise_client_gameplay::entity_occlusion::{EntityCullDecision, at};
pub(super) use sunrise_client_gameplay::entity_occlusion::{
    EntityOcclusion, cull_box, default_range, entity_view_scale, orb_in_view,
};
use sunrise_client_gameplay::occlusion_culling::QuerySource;
use sunrise_core::geometry::{Aabb, Vec3};

impl Game<'_> {
    pub(super) fn prepare_entity_culling(&mut self) {
        // The mods' render hooks: whether any listens this frame, and no entity canceled yet.
        super::native_mob_draw::begin_entity_frame();
        // Spans tell the pack's passes which entities are hidden; without them, nothing changes.
        self.occlusion.shadow_casters = match (&self.renderer, &self.pack_ids) {
            (Some(r), Some(_)) => r.pack_shadow_casters(),
            // A native pack's shadow pass has no spans to hide entities with: it only needs
            // the first-person model.
            (Some(r), None) => (false, r.native_shadow()),
            _ => (false, false),
        };
    }

    pub(super) fn cull_entity_vertices(&self, vertices: &mut Vec<Vertex>, start: usize) -> bool {
        let hidden = self.occlusion_hidden(Some(vertices), start, None);
        if hidden
            && !(self.occlusion.shadow_casters.0 && self.hide_spans_from(start, vertices.len()))
        {
            if self.settings.sunrise.entity_culling == EntityCullingMode::Asynchronous
                && let Some(culler) = &self.occlusion.async_culler
            {
                culler.record_entity_vertices_culled(vertices.len().saturating_sub(start));
            }
            vertices.truncate(start);
        }
        hidden
    }

    pub(super) fn cull_entity_camera_bounds(&self, lo: glam::Vec3, hi: glam::Vec3) -> bool {
        if self.occlusion.occlusion_hidden_camera_bounds(
            &section_scene!(self),
            self.settings.sunrise.entity_culling,
            lo,
            hi,
        ) {
            if self.settings.sunrise.entity_culling == EntityCullingMode::Asynchronous
                && let Some(culler) = &self.occlusion.async_culler
            {
                culler.record_entity_vertices_culled(2);
            }
            return true;
        }
        false
    }

    /// Pre-model visibility hook for dynamic block-entity geometry.
    pub(super) fn block_entity_occluded(&self, bounds: Aabb) -> bool {
        self.occlusion_hidden(None, 0, Some(bounds))
    }

    /// Cull from the shared entity AABB before generating its model vertices.
    pub(super) fn entity_cull_decision<T: RenderEntityBounds>(
        &self,
        entity: &T,
        alpha: f64,
    ) -> EntityCullDecision {
        self.entity_cull_decision_in_bounds(entity, entity.render_bounds(alpha))
    }

    /// Cull from a renderer-computed envelope when the rendered transform has
    /// extra offsets or model geometry that the entity type alone cannot see.
    pub(super) fn entity_cull_decision_in_bounds<T: RenderEntityBounds>(
        &self,
        entity: &T,
        render_bounds: Aabb,
    ) -> EntityCullDecision {
        if self.settings.sunrise.entity_culling == EntityCullingMode::Off
            || self.occlusion.shadow_casters.0
            || entity.render_glowing(self)
        {
            return EntityCullDecision::Bypass;
        }
        // `render_bounds` is already the conservative model and animation
        // envelope. Only cover f32 vertex rounding here; another half-block
        // would make coarse walls falsely visible and force mesh generation.
        let margin = Vec3::new(1.0e-4, 1.0e-4, 1.0e-4);
        let bounds = render_bounds.inflate(margin);
        let scene = section_scene!(self);
        if self.occlusion.terrain_sections_occlude(&scene, bounds) {
            if let Some(culler) = &self.occlusion.async_culler {
                culler.record_entity_culled();
            }
            return EntityCullDecision::Hidden;
        }
        match self.settings.sunrise.entity_culling {
            EntityCullingMode::Off => EntityCullDecision::Bypass,
            EntityCullingMode::Synchronous => {
                if self
                    .occlusion
                    .entity_hidden_sync(&scene, None, 0, Some(bounds))
                {
                    EntityCullDecision::Hidden
                } else {
                    EntityCullDecision::CheckVertices
                }
            }
            EntityCullingMode::Asynchronous => {
                let Some(culler) = &self.occlusion.async_culler else {
                    return if self
                        .occlusion
                        .entity_hidden_sync(&scene, None, 0, Some(bounds))
                    {
                        EntityCullDecision::Hidden
                    } else {
                        EntityCullDecision::CheckVertices
                    };
                };
                if !culler.camera_is_stationary() {
                    return EntityCullDecision::CheckVertices;
                }
                let Some(query) = culler.query_entity_bounds(bounds) else {
                    return EntityCullDecision::CheckVertices;
                };
                culler.sample_entity_aabb(entity.render_id(), &query);
                match culler.query_result(&query, QuerySource::EntityAabb) {
                    Some((false, _)) => EntityCullDecision::CheckVertices,
                    Some((true, Some(proof)))
                        if culler.proof_is_current(
                            scene.world,
                            scene.renderer,
                            scene.streaming,
                            scene.pending_uploads,
                            proof,
                        ) =>
                    {
                        culler.record_entity_culled();
                        EntityCullDecision::Hidden
                    }
                    Some(_) | None => {
                        culler.queue(query);
                        EntityCullDecision::Pending
                    }
                }
            }
        }
    }

    /// Refine an AABB-visible result with the exact bounds of the rendered mesh.
    pub(super) fn finish_entity_occlusion<T: RenderEntityBounds>(
        &self,
        decision: EntityCullDecision,
        entity: &T,
        vertices: &mut Vec<Vertex>,
        start: usize,
    ) {
        if decision.checks_vertices()
            && !self.occlusion.shadow_casters.0
            && !entity.render_glowing(self)
        {
            self.cull_entity_vertices(vertices, start);
        }
    }

    fn occlusion_hidden(
        &self,
        vertices: Option<&[Vertex]>,
        start: usize,
        bounds: Option<Aabb>,
    ) -> bool {
        self.occlusion.occlusion_hidden(
            &section_scene!(self),
            self.settings.sunrise.entity_culling,
            vertices,
            start,
            bounds,
        )
    }
}

impl Game<'_> {
    /// Options.getEffectiveRenderDistance: the client setting, capped by the server's view distance.
    pub(super) fn effective_render_distance(&self) -> i32 {
        match &self.remote {
            Some(r) => self.settings.render_distance.min(r.view_distance),
            None => self.settings.render_distance,
        }
    }
    /// Entity.setViewScale: entity distance times the render distance's boost (1 at 8 chunks, 2.5 at 20+).
    pub(super) fn view_scale(&self) -> f64 {
        entity_view_scale(
            self.effective_render_distance(),
            self.settings.entity_distance,
        )
    }
}

impl Game<'_> {
    /// EntityRenderer.shouldRender: the entity within `range` blocks of the
    /// camera, measured from its current `position`, and its culling box
    /// (`lo`..`hi`, interpolated, in world space) grown by 0.5 in the view.
    pub(super) fn entity_in_view(
        &self,
        camera: glam::DVec3,
        position: glam::DVec3,
        range: f64,
        lo: glam::DVec3,
        hi: glam::DVec3,
    ) -> bool {
        if (position - camera).length_squared() >= range * range {
            return false;
        }
        self.view_frustum.as_ref().is_none_or(|view| {
            view.intersects(
                (lo - camera - glam::DVec3::splat(0.5)).as_vec3(),
                (hi - camera + glam::DVec3::splat(0.5)).as_vec3(),
            )
        })
    }
}

impl Game<'_> {
    /// `entity_in_view` for an entity whose box stands on its position,
    /// `width` wide and `height` tall, interpolated by `alpha` and grown by
    /// `grow` (a renderer's getBoundingBoxForCulling).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn standing_entity_in_view(
        &self,
        camera: glam::DVec3,
        previous: glam::DVec3,
        position: glam::DVec3,
        alpha: f64,
        width: f64,
        height: f64,
        range: f64,
        grow: f64,
    ) -> bool {
        let p = previous.lerp(position, alpha);
        let half = glam::DVec3::new(width / 2.0 + grow, grow, width / 2.0 + grow);
        let lo = p - half;
        let hi = p + glam::DVec3::new(half.x, height + grow, half.z);
        self.entity_in_view(camera, position, range, lo, hi)
    }
}

impl Game<'_> {
    pub(super) fn prepare_occlusion_culling(&mut self, camera: glam::DVec3) {
        self.occlusion.section_camera = None;
        self.occlusion.section_visibility_valid = false;
        let entity_mode = self.settings.sunrise.entity_culling;
        let requested = entity_mode != EntityCullingMode::Off;
        let eligible = requested
            // Packs can replace opaque models, so their terrain is not a safe occluder.
            && self.model_packs_are_vanilla()
            && self.player.is_some()
            && !self.spectator();
        if !eligible {
            self.occlusion.sync_culler = None;
            self.occlusion.async_culler = None;
            self.occlusion.async_failed = false;
            return;
        }
        let at = camera.to_array().map(|v| v.floor() as i32);
        self.occlusion.section_camera = at[1]
            .checked_sub(self.world.bottom_y())
            .map(|y| (at[0] >> 4, y >> 4, at[2] >> 4));
        let camera_facts = self.res.facts().get(self.world.block(at[0], at[1], at[2]));
        if opaque(camera_facts) {
            self.occlusion.sync_culler = None;
            self.occlusion.async_culler = None;
            self.occlusion.async_failed = false;
            return;
        }
        self.occlusion.section_visibility_valid = self.settings.sunrise.cave_culling
            && !camera_facts.solid_render
            && self.occlusion.section_camera.is_some();

        let wants_async = entity_mode == EntityCullingMode::Asynchronous;
        let wants_sync = entity_mode == EntityCullingMode::Synchronous;
        if wants_async {
            if self.occlusion.async_culler.is_none() && !self.occlusion.async_failed {
                match AsyncOcclusionCuller::new() {
                    Ok(culler) => self.occlusion.async_culler = Some(Box::new(culler)),
                    Err(error) => {
                        log::warn!("Sunrise: cannot start world-occlusion-culling worker: {error}");
                        self.occlusion.async_failed = true;
                    }
                }
            }
            if let Some(culler) = &mut self.occlusion.async_culler {
                culler.begin_frame(camera, &self.world);
            }
        } else {
            self.occlusion.async_culler = None;
            self.occlusion.async_failed = false;
        }

        let async_unavailable = wants_async && self.occlusion.async_culler.is_none();
        self.occlusion.sync_culler = (wants_sync || async_unavailable)
            .then(|| OcclusionCuller::new(camera.to_array(), 8_192));
    }

    pub(super) fn finish_async_occlusion_culling(&mut self) {
        if let Some(culler) = &mut self.occlusion.async_culler {
            culler.finish_frame(&self.world, self.res.facts());
        }
    }
}

#[cfg(test)]
mod precheck_envelope_tests {
    use super::super::entity::RenderEntityBounds;
    use std::{path::Path, sync::Arc};
    use sunrise_client::mobs::{Mob, MobData, MobTypes, Species, render};
    use sunrise_core::geometry::Vec3;

    #[test]
    fn vanilla_models_and_animations_stay_inside_the_precheck_envelope() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/sunrise-client/26.3");
        if !root.join("entity-models.json").exists() {
            eprintln!("no prepared resources: skipped model coverage");
            return;
        }
        let types = MobTypes::load(&root).unwrap();
        let data = MobData {
            resolved: Default::default(),
            states: Vec::new(),
            placements: Default::default(),
            heightmaps: Default::default(),
            animations: MobData::load_animations(&root),
            villager_hats: Default::default(),
            frame_models: Default::default(),
            cem: Default::default(),
            random_textures: Default::default(),
            emissive: Default::default(),
            blinks: Default::default(),
            biome_tags: Default::default(),
            pois: Default::default(),
            species_placements: Default::default(),
            mod_spawns: Default::default(),
            vanilla_spawn_placements: None,
        };
        let camera = glam::DVec3::new(3.0, 62.0, -4.0);
        let mut checked = 0;
        for kind in types.types.values() {
            for sample in 0..12 {
                let mut mob = Mob::new(1, Arc::new(kind.clone()), Vec3::new(1.5, 64.0, 3.25), 37.0);
                mob.previous = Vec3::new(1.25, 64.25, 3.0);
                mob.age = sample * 37;
                mob.limb_speed = 0.8;
                mob.limb_position = sample as f32 * 1.7;
                mob.head_pitch = if sample % 2 == 0 { 85.0 } else { -85.0 };
                mob.extra.age = if sample >= 6 { -100 } else { 0 };
                mob.death_time = if sample % 3 == 0 { 10 } else { 0 };
                mob.extra.flag = sample % 2 == 0;
                mob.extra.size = match mob.species() {
                    Species::Slime
                    | Species::MagmaCube
                    | Species::SulfurCube
                    | Species::Phantom => {
                        if sample % 2 == 0 {
                            1
                        } else {
                            4
                        }
                    }
                    _ => mob.extra.size,
                };
                if sample % 3 == 1 {
                    mob.custom_name = Some("Dinnerbone".into());
                }
                for partial in [0.0, 0.4, 1.0] {
                    let bounds = mob.render_bounds(f64::from(partial));
                    let lo = bounds.min_position();
                    let hi = bounds.max_position();
                    let lo = [lo.x, lo.y, lo.z];
                    let hi = [hi.x, hi.y, hi.z];
                    let mut vertices = Vec::new();
                    render::render_with(
                        &mob,
                        partial,
                        camera,
                        &|_| Some([0.0, 0.0, 1.0, 1.0]),
                        0xF0F0,
                        &data,
                        &[],
                        &mut vertices,
                    );
                    for vertex in &vertices {
                        for axis in 0..3 {
                            let world = f64::from(vertex.pos[axis]) + camera[axis];
                            assert!(
                                world >= lo[axis] - 1.0e-4 && world <= hi[axis] + 1.0e-4,
                                "{} sample {sample} partial {partial}: {world} outside {lo:?}..{hi:?}",
                                kind.id,
                            );
                        }
                    }
                    checked += usize::from(!vertices.is_empty());
                }
            }
        }
        assert!(
            checked > 1_000,
            "insufficient real model coverage: {checked}"
        );
    }
}
