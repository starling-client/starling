//! The camera's view frustum for particles: vanilla's always-on point test, and the section
//! cache behind the opt-in Particle Culling setting.

use super::{Particle, ParticleEngine, ParticleQuads, Vertex};
use glam::{DVec3, Quat, Vec3 as FVec3, Vec4};
use std::array;

const PARTICLE_SECTION_CACHE_SIZE: usize = 128;
/// How far back, in blocks, the camera's frustum is pulled for the particle test (vanilla's
/// `Frustum.offset` in `ParticleEngine.extract`).
pub(super) const PARTICLE_FRUSTUM_OFFSET: f32 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SectionFrustumResult {
    Outside,
    Intersects,
    Inside,
}

#[derive(Clone, Copy)]
struct CachedSectionFrustum {
    section: [i32; 3],
    radius_quarters: i32,
    result: SectionFrustumResult,
}

pub(super) struct ParticleSectionFrustumCache {
    entries: [Option<CachedSectionFrustum>; PARTICLE_SECTION_CACHE_SIZE],
}

impl Default for ParticleSectionFrustumCache {
    fn default() -> Self {
        Self {
            entries: [None; PARTICLE_SECTION_CACHE_SIZE],
        }
    }
}

impl ParticleSectionFrustumCache {
    pub(super) fn result(
        &mut self,
        world: [f64; 3],
        camera: DVec3,
        radius: f64,
        frustum: &crate::gfx::JoinedFrusta,
    ) -> SectionFrustumResult {
        let Some(section) = world_section(world) else {
            return SectionFrustumResult::Intersects;
        };
        if !radius.is_finite() || radius < 0.0 || radius * 4.0 > f64::from(i32::MAX) {
            return SectionFrustumResult::Intersects;
        }
        let radius_quarters = (radius * 4.0).ceil() as i32;
        let hash = (section[0] as u32).wrapping_mul(73_856_093)
            ^ (section[1] as u32).wrapping_mul(19_349_663)
            ^ (section[2] as u32).wrapping_mul(83_492_791)
            ^ (radius_quarters as u32).wrapping_mul(2_654_435_761);
        let slot = hash as usize & (PARTICLE_SECTION_CACHE_SIZE - 1);
        if let Some(entry) = self.entries[slot]
            && entry.section == section
            && entry.radius_quarters == radius_quarters
        {
            return entry.result;
        }

        let pad = f64::from(radius_quarters) * 0.25;
        let camera = camera.to_array();
        let min = array::from_fn(|axis| f64::from(section[axis]) * 16.0 - pad - camera[axis]);
        let max = array::from_fn(|axis| min[axis] + 16.0 + 2.0 * pad);
        let result = if min.into_iter().chain(max).any(|v| !v.is_finite()) {
            SectionFrustumResult::Intersects
        } else {
            let min = FVec3::from_array(min.map(|v| v as f32));
            let max = FVec3::from_array(max.map(|v| v as f32));
            if !min.is_finite() || !max.is_finite() {
                SectionFrustumResult::Intersects
            } else if frustum.contains(min, max) {
                SectionFrustumResult::Inside
            } else if frustum.intersects(min, max) {
                SectionFrustumResult::Intersects
            } else {
                SectionFrustumResult::Outside
            }
        };
        self.entries[slot] = Some(CachedSectionFrustum {
            section,
            radius_quarters,
            result,
        });
        result
    }
}

fn world_section(position: [f64; 3]) -> Option<[i32; 3]> {
    let mut block = [0; 3];
    for axis in 0..3 {
        let value = position[axis].floor();
        if !value.is_finite() || value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
            return None;
        }
        block[axis] = value as i32;
    }
    Some([block[0] >> 4, block[1] >> 4, block[2] >> 4])
}

/// Whether `QuadParticleGroup`'s test keeps `particle`: its position this tick (camera-relative, not
/// interpolated) is on or inside `view`, the camera's frustum pulled back [`PARTICLE_FRUSTUM_OFFSET`] blocks.
pub(super) fn in_view(view: &crate::gfx::JoinedFrusta, particle: &Particle, camera: DVec3) -> bool {
    let at = DVec3::from_array(particle.position) - camera;
    view.contains_point(at.as_vec3())
}

/// Vanilla's world particle-extraction frustum. This is not the view used for drawing or
/// the optional particle occlusion checks.
pub struct ParticleCullView {
    origin: DVec3,
    planes: [Vec4; 6],
}

impl ParticleCullView {
    /// `Camera.createProjectionMatrixForCulling`, then `LevelExtractor` offsets its origin
    /// by -3 along `Frustum.calculateFrustum`'s transposed clip-Z row.
    pub fn new(
        camera: DVec3,
        yaw_pitch: [f32; 2],
        size: [u32; 2],
        configured_fov: f32,
        effective_fov: f32,
        far: f32,
    ) -> Self {
        let [yaw, pitch] = yaw_pitch;
        let projection = crate::camera::perspective(
            configured_fov.max(effective_fov),
            size[0] as f32,
            size[1] as f32,
            0.05,
            far,
            true,
        );
        let rotation = crate::camera::view_rotation_matrix(yaw, pitch).0;
        let x = rotation.row(0) * projection[0];
        let y = rotation.row(1) * projection[5];
        let z = (rotation.row(2) * projection[10]).with_w(projection[14]);
        let w = -rotation.row(2);
        // JOML FrustumIntersection always uses both w +/- z planes, even with
        // a zero-to-one projection. Test points after subtracting the offset in f64.
        let origin = camera + (z.truncate() * -PARTICLE_FRUSTUM_OFFSET).as_dvec3();
        let planes = [w + x, w - x, w + y, w - y, w + z, w - z].map(|plane| {
            let [x, y, z, _] = plane.to_array();
            plane * (1.0 / ((f64::from(x * x + y * y + z * z)).sqrt() as f32))
        });
        Self { origin, planes }
    }

    pub(super) fn contains(&self, world_position: [f64; 3]) -> bool {
        let p = (DVec3::from_array(world_position) - self.origin).as_vec3();
        self.planes
            .iter()
            .all(|plane| plane.x * p.x + plane.y * p.y + plane.z * p.z + plane.w >= 0.0)
    }
}

/// Optional Sunrise culling of stock particle geometry. It does not decide whether the
/// world extract callback runs or cull any explicit quads it emits.
pub struct ParticleWorldCull<'a> {
    pub pixel_focal_length: f32,
    pub frustum: Option<&'a crate::gfx::JoinedFrusta>,
    pub occluded: &'a mut dyn FnMut([f64; 3], [f64; 3]) -> bool,
}

impl ParticleEngine {
    /// [`Self::render`], keeping only the particles inside `view`, the camera's frustum pulled
    /// back [`PARTICLE_FRUSTUM_OFFSET`] blocks (vanilla's `ParticleEngine.extract` and
    /// `QuadParticleGroup`): the test runs whatever the Particle Culling setting says. With
    /// `None`, every particle is drawn.
    pub fn render_in_view(
        &self,
        camera: DVec3,
        base: Quat,
        partial: f32,
        view: Option<&crate::gfx::JoinedFrusta>,
        light_at: &dyn Fn([i32; 3]) -> u8,
        out: &mut Vec<Vertex>,
    ) {
        self.render_inner::<false>(
            camera,
            base,
            partial,
            0.0,
            view,
            None,
            &mut |_, _| false,
            light_at,
            out,
        );
    }

    /// Draw the level, including provider extraction against the vanilla cull frustum.
    /// The ordinary render entry is also used by live scenes, which have only stock quads.
    pub fn render_world(
        &self,
        camera: DVec3,
        base: Quat,
        partial: f32,
        providers: Option<(&ParticleCullView, &mut ParticleQuads)>,
        stock_cull: Option<ParticleWorldCull<'_>>,
        light_at: &dyn Fn([i32; 3]) -> u8,
        out: &mut Vec<Vertex>,
    ) {
        if let Some(cull) = stock_cull {
            self.render_inner::<true>(
                camera,
                base,
                partial,
                cull.pixel_focal_length,
                cull.frustum,
                providers,
                cull.occluded,
                light_at,
                out,
            );
        } else {
            self.render_inner::<false>(
                camera,
                base,
                partial,
                0.0,
                None,
                providers,
                &mut |_, _| false,
                light_at,
                out,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn particle_cull_offset_uses_the_projection_derived_view_vector() {
        let view = ParticleCullView::new(DVec3::ZERO, [0.0, 0.0], [512, 512], 70.0, 70.0, 8.0);
        // With near .05 and far 8 the Z row length is 8/7.95, not a unit vector.
        // The near test is w + z, so the first point falls inside only with that offset.
        assert!(view.contains([0.0, 0.0, -2.99]));
        assert!(!view.contains([0.0, 0.0, -3.0]));
    }
}
