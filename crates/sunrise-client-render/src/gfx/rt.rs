//! One BLAS per live terrain section's solid quads plus one TLAS over them, built as
//! sections mesh. Runs only on a ray-query device, otherwise stays empty. Positions bake
//! into world space in a triangle soup of their own, decoupled from the render arenas:
//! arena compaction and the shared index buffer's regrow would invalidate BLASes
//! pointing at them. Translucent glass, water, entities and far tiles stay out.

use super::{SectionKey, arena::Arena};
use crate::mesher::TerrainVertex;

/// Sections are this far apart; positions bake into world space at upload.
const SECTION: f32 = 16.0;
/// Only sections within this many blocks of the camera get a BLAS: the near field
/// ray-traced effects read. Farther sections keep no ray-tracing data past
/// `DROP_DISTANCE`.
const BUILD_DISTANCE: f64 = 128.0;
const DROP_DISTANCE: f64 = 256.0;
/// BLAS builds per frame: meshing bursts drain over frames instead of one hitch.
/// 32 warms a loaded area in seconds while staying off the frame's critical path.
const MAX_BUILDS_PER_FRAME: usize = 32;
/// TLAS instances; the gated set fits with headroom (a 128-block radius holds at
/// most a few thousand sections). Beyond it the nearest sections win.
const TLAS_MAX_INSTANCES: u32 = 8192;
/// Identity 3x4 row-major transform: positions already bake into world space.
const IDENTITY: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];

/// World-space triangle soup of a section's solid quads: two triangles per quad, wound
/// as the shared quad index buffer (`Renderer::quad_indices`: 0,1,2 and 0,2,3).
/// `origin` is the section's world-space corner.
pub fn expand_section_positions(solid: &[TerrainVertex], origin: [f32; 3]) -> Vec<[f32; 3]> {
    let mut out = Vec::with_capacity(solid.len() / 4 * 6);
    for quad in solid.as_chunks::<4>().0 {
        let p = [quad[0].pos, quad[1].pos, quad[2].pos, quad[3].pos];
        for v in [p[0], p[1], p[2], p[0], p[2], p[3]] {
            out.push([v[0] + origin[0], v[1] + origin[1], v[2] + origin[2]]);
        }
    }
    out
}

/// World-space origin of `key`'s section.
pub fn section_origin(key: SectionKey) -> [f32; 3] {
    [
        key.0 as f32 * SECTION,
        key.1 as f32 * SECTION,
        key.2 as f32 * SECTION,
    ]
}

/// Whether `key`'s section AABB is within `distance` blocks of `camera`.
pub fn within_distance(key: SectionKey, camera: glam::DVec3, distance: f64) -> bool {
    section_distance(key, camera) <= distance * distance
}

/// Squared section distance, for nearest-first ordering without roots.
fn section_distance(key: SectionKey, camera: glam::DVec3) -> f64 {
    let o = section_origin(key);
    let closest = glam::DVec3::new(
        camera.x.clamp(o[0] as f64, o[0] as f64 + SECTION as f64),
        camera.y.clamp(o[1] as f64, o[1] as f64 + SECTION as f64),
        camera.z.clamp(o[2] as f64, o[2] as f64 + SECTION as f64),
    );
    (closest - camera).length_squared()
}

/// One section's ray-tracing state.
struct RtEntry {
    /// World-space triangle soup, staged until the BLAS is built.
    staged: Option<Vec<[f32; 3]>>,
    /// Range in the ray-tracing arena, once uploaded.
    page: u32,
    start: u64,
    verts: u32,
    /// The built BLAS over the range.
    blas: Option<wgpu::Blas>,
}

/// What the TLAS rebuilds from (`tlas_generation` trails `generation`).
#[derive(Default)]
pub struct RtWorld {
    entries: std::collections::HashMap<SectionKey, RtEntry>,
    arena: Option<Arena>,
    tlas: Option<wgpu::Tlas>,
    /// Bumped on every note/forget/clear and BLAS batch: the TLAS rebuilds while behind.
    generation: u64,
    tlas_generation: u64,
    /// How many instances the TLAS was last built with (to clear the tail).
    tlas_filled: u32,
    /// Whether the TLAS object was ever built: a fresh one traces nothing until it is.
    tlas_built: bool,
    /// Lifetime totals, for the log.
    pub blas_built: u64,
    pub sections_noted: u64,
    warned_oom: bool,
}

impl RtWorld {
    /// Stages a section's solid quads for upload and a BLAS build, replacing
    /// whatever the section had (a remesh rebuilds from scratch). Positions bake
    /// into world space; freed on build.
    pub fn note_upload(&mut self, key: SectionKey, positions: Vec<[f32; 3]>) {
        if positions.is_empty() {
            return;
        }
        if let Some(old) = self.entries.remove(&key) {
            self.release_entry(old);
        }
        self.entries.insert(
            key,
            RtEntry {
                staged: Some(positions),
                page: 0,
                start: 0,
                verts: 0,
                blas: None,
            },
        );
        self.sections_noted += 1;
        self.generation += 1;
    }

    /// Drops a section: its BLAS dies with it and its arena range frees.
    /// Missing keys (never noted, e.g. translucent-only) are nothing.
    pub fn forget(&mut self, key: SectionKey) {
        if let Some(old) = self.entries.remove(&key) {
            self.release_entry(old);
            self.generation += 1;
        }
    }

    /// Drops every section of one column pair (a bulk unload's columns).
    pub fn forget_column(&mut self, x: i32, z: i32) {
        let dead: Vec<SectionKey> = self
            .entries
            .keys()
            .filter(|k| k.0 == x && k.2 == z)
            .copied()
            .collect();
        for key in dead {
            self.forget(key);
        }
    }

    /// Drops everything (a dimension or layout change re-meshes the world). The TLAS
    /// object itself stays: bind groups hold it, and the next sync rebuilds it empty.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.arena = None;
        self.warned_oom = false;
        self.tlas_built = false;
        self.generation += 1;
    }

    /// The TLAS for pack bind groups, creating it empty on first use. The object is
    /// stable afterwards (rebuilds go into it), so groups bound once stay valid.
    pub fn ensure_tlas(&mut self, device: &wgpu::Device) -> &wgpu::Tlas {
        self.tlas.get_or_insert_with(|| {
            device.create_tlas(&wgpu::CreateTlasDescriptor {
                label: Some("World TLAS"),
                max_instances: TLAS_MAX_INSTANCES,
                flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                update_mode: wgpu::AccelerationStructureUpdateMode::Build,
            })
        })
    }

    fn release_entry(&mut self, entry: RtEntry) {
        if entry.verts > 0
            && let Some(arena) = &mut self.arena
        {
            arena.release(entry.page, entry.start, u64::from(entry.verts));
        }
        // The BLAS drops here; wgpu defers destruction past in-flight builds.
    }

    /// Builds pending BLASes (bounded per frame) and rebuilds the TLAS while
    /// the live set trails it. Runs only on a ray-query device; anywhere else
    /// there is nothing staged and this is a generation check.
    pub fn sync(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, camera: glam::DVec3) {
        // Far sections first out: beyond the drop margin their staged data and
        // BLAS free (a remesh re-stages them; teleports remesh everything).
        let dead: Vec<SectionKey> = self
            .entries
            .keys()
            .filter(|k| !within_distance(**k, camera, DROP_DISTANCE))
            .copied()
            .collect();
        for key in dead {
            self.forget(key);
        }
        if self.entries.is_empty() {
            // Bind groups may already reference the TLAS (the pack activates before
            // the first section meshes): an unbuilt TLAS is a validation error on
            // use, so build it empty once.
            if !self.tlas_built {
                self.ensure_tlas(device);
                self.rebuild_tlas(device, queue, camera);
            }
            return;
        }
        // Phase 1: nearest staged sections first, so a backlog fills in around
        // the player. Uploads the positions, writes them and creates the BLAS.
        let mut staged: Vec<(SectionKey, f64)> = self
            .entries
            .iter()
            .filter(|(_, e)| e.staged.is_some())
            .map(|(k, _)| (*k, section_distance(*k, camera)))
            .collect();
        staged.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let mut built = Vec::new();
        for (key, _) in staged.into_iter().take(MAX_BUILDS_PER_FRAME) {
            if !within_distance(key, camera, BUILD_DISTANCE) {
                continue;
            }
            let Some(entry) = self.entries.get_mut(&key) else {
                continue;
            };
            let Some(positions) = entry.staged.take() else {
                continue;
            };
            let verts = positions.len() as u32;
            let arena = self.arena.get_or_insert_with(|| Arena::ray_tracing(device));
            let range = arena.alloc_existing(u64::from(verts)).or_else(|| {
                // Growing the arena records a copy: submit it before anything
                // reads the new page.
                let mut grow = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Ray tracing arena grow"),
                });
                let range = arena.alloc(device, &mut grow, u64::from(verts));
                if range.is_some() {
                    queue.submit([grow.finish()]);
                }
                range
            });
            let Some((page, start)) = range else {
                entry.staged = Some(positions);
                if !self.warned_oom {
                    self.warned_oom = true;
                    log::warn!(
                        "Sunrise renderer: no ray-tracing video memory for a section; \
                         its rays wait for a remesh"
                    );
                }
                continue;
            };
            queue.write_buffer(
                arena.buffer(page),
                start * 12,
                bytemuck::cast_slice(&positions),
            );
            let blas = device.create_blas(
                &wgpu::CreateBlasDescriptor {
                    label: Some("Section BLAS"),
                    flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                    update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                },
                wgpu::BlasGeometrySizeDescriptors::Triangles {
                    descriptors: vec![triangle_size(verts)],
                },
            );
            entry.page = page;
            entry.start = start;
            entry.verts = verts;
            entry.blas = Some(blas);
            built.push(key);
        }
        // Phase 2: build what phase 1 created. Sizes fill first: pushing while
        // borrowed refs exist is rejected, and a reallocation would move them.
        if !built.is_empty() {
            let arena = self.arena.as_ref().expect("phase 1 uploaded");
            let mut sizes = Vec::with_capacity(MAX_BUILDS_PER_FRAME.max(built.len()));
            for key in &built {
                sizes.push(triangle_size(self.entries[key].verts));
            }
            let mut geometries = Vec::with_capacity(sizes.len());
            for (key, size) in built.iter().zip(sizes.iter()) {
                let entry = &self.entries[key];
                geometries.push(wgpu::BlasTriangleGeometry {
                    size,
                    vertex_buffer: arena.buffer(entry.page),
                    first_vertex: entry.start as u32,
                    vertex_stride: 12,
                    index_buffer: None,
                    first_index: None,
                    transform_buffer: None,
                    transform_buffer_offset: None,
                });
            }
            let blas_entries: Vec<wgpu::BlasBuildEntry> = built
                .iter()
                .map(|key| self.entries[key].blas.as_ref().expect("phase 1 built it"))
                .zip(geometries)
                .map(|(blas, geometry)| wgpu::BlasBuildEntry {
                    blas,
                    geometry: wgpu::BlasGeometries::TriangleGeometries(vec![geometry]),
                })
                .collect();
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Ray tracing builds"),
            });
            encoder.build_acceleration_structures(
                blas_entries.iter(),
                std::iter::empty::<&wgpu::Tlas>(),
            );
            queue.submit([encoder.finish()]);
            self.blas_built += built.len() as u64;
            self.warned_oom = false;
            self.generation += 1;
            if built.len() as u64 == self.blas_built || self.blas_built % 1024 < built.len() as u64
            {
                log::info!(
                    "Sunrise renderer: ray tracing BLASes built: {}",
                    self.blas_built
                );
            }
        }
        // The TLAS follows membership, fresh builds, and a fresh object: an unbuilt
        // TLAS traces nothing, so bind groups never read one.
        if !self.tlas_built || self.tlas_generation != self.generation {
            self.rebuild_tlas(device, queue, camera);
        }
    }

    /// Rewrites the TLAS instances (nearest first, capped) and builds it. Only
    /// sections with a built BLAS take part; staged ones join once built.
    fn rebuild_tlas(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, camera: glam::DVec3) {
        let mut live: Vec<(SectionKey, f64)> = self
            .entries
            .iter()
            .filter(|(_, e)| e.blas.is_some())
            .map(|(k, _)| (*k, section_distance(*k, camera)))
            .collect();
        live.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        if live.len() > TLAS_MAX_INSTANCES as usize {
            log::warn!(
                "Sunrise renderer: {} ray-tracing sections built, TLAS takes {}; \
                 the rest wait for a remesh",
                live.len(),
                TLAS_MAX_INSTANCES
            );
            live.truncate(TLAS_MAX_INSTANCES as usize);
        }
        let tlas = self.tlas.get_or_insert_with(|| {
            device.create_tlas(&wgpu::CreateTlasDescriptor {
                label: Some("World TLAS"),
                max_instances: TLAS_MAX_INSTANCES,
                flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                update_mode: wgpu::AccelerationStructureUpdateMode::Build,
            })
        });
        let filled = live.len() as u32;
        if let Some(slots) = tlas.get_mut_slice(0..filled as usize) {
            for (i, (slot, (key, _))) in slots.iter_mut().zip(live.iter()).enumerate() {
                let blas = self.entries[key]
                    .blas
                    .as_ref()
                    .expect("filtered to built entries");
                *slot = Some(wgpu::TlasInstance::new(blas, IDENTITY, i as u32, 0xff));
            }
        }
        if filled < self.tlas_filled
            && let Some(tail) = tlas.get_mut_slice(filled as usize..self.tlas_filled as usize)
        {
            for slot in tail.iter_mut() {
                *slot = None;
            }
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Ray tracing TLAS"),
        });
        encoder.build_acceleration_structures(
            std::iter::empty::<&wgpu::BlasBuildEntry>(),
            std::iter::once(tlas as &wgpu::Tlas),
        );
        queue.submit([encoder.finish()]);
        self.tlas_filled = filled;
        self.tlas_generation = self.generation;
        self.tlas_built = true;
        log::info!("Sunrise renderer: TLAS rebuilt with {filled} ray-tracing sections");
    }
}

/// One triangle geometry's size: non-indexed `Float32x3` positions, marked opaque so
/// committed hits need no any-hit confirmation (there is none: cutout leaves block
/// whole quads for rays, like `shadow.solid_materials` does for the shadow map).
fn triangle_size(vertex_count: u32) -> wgpu::BlasTriangleGeometrySizeDescriptor {
    wgpu::BlasTriangleGeometrySizeDescriptor {
        vertex_format: wgpu::VertexFormat::Float32x3,
        vertex_count,
        index_format: None,
        index_count: None,
        flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesher::TerrainVertex;

    fn quad_at(origin: [f32; 3]) -> [TerrainVertex; 4] {
        [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ]
        .map(|p| {
            let mut v = TerrainVertex::new(p, [0.0, 0.0], 0, 0, 0);
            v.pos = [
                v.pos[0] + origin[0],
                v.pos[1] + origin[1],
                v.pos[2] + origin[2],
            ];
            v
        })
    }

    #[test]
    fn quads_expand_to_triangles_wound_like_the_index_buffer() {
        // Quad indices are 0,1,2 and 0,2,3 per quad (Renderer::quad_indices).
        let solid: Vec<TerrainVertex> = quad_at([2.0, 0.0, 0.0]).into();
        let tris = expand_section_positions(&solid, [16.0, 0.0, 32.0]);
        assert_eq!(tris.len(), 6);
        // Triangle 1: verts 0,1,2; triangle 2: verts 0,2,3, all offset by origin.
        assert_eq!(tris[0], [18.0, 0.0, 32.0]);
        assert_eq!(tris[1], [19.0, 0.0, 32.0]);
        assert_eq!(tris[2], [19.0, 1.0, 32.0]);
        assert_eq!(tris[3], [18.0, 0.0, 32.0]);
        assert_eq!(tris[4], [19.0, 1.0, 32.0]);
        assert_eq!(tris[5], [18.0, 1.0, 32.0]);
    }

    #[test]
    fn section_origins_follow_the_grid() {
        assert_eq!(section_origin((0, 0, 0)), [0.0, 0.0, 0.0]);
        assert_eq!(section_origin((1, -1, 2)), [16.0, -16.0, 32.0]);
    }

    #[test]
    fn distance_gate_covers_the_near_field_only() {
        let camera = glam::DVec3::new(0.0, 80.0, 0.0);
        assert!(within_distance((0, 5, 0), camera, BUILD_DISTANCE));
        assert!(!within_distance((100, 5, 0), camera, BUILD_DISTANCE));
        assert!(!within_distance((100, 5, 0), camera, DROP_DISTANCE));
    }
}
