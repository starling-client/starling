use super::*;

/// Whether section `key` lies in the eye's visible terrain set: within horizontal and vertical
/// render distance, reached by the bounded occlusion graph, and passing the eye frustum /
/// outside-height section-tree test (`LevelExtractor.extractVisibleBlockEntities`).
pub fn section_in_eye_view(
    key: SectionKey,
    camera: DVec3,
    min_y: i32,
    sections: i32,
    radius: i32,
    frustum: Option<&impl Cull>,
    reachable: impl Fn(SectionKey) -> bool,
) -> bool {
    if !crate::streaming::in_view_distance(resident::camera_column(camera), radius, key.0, key.2) {
        return false;
    }
    let Some((lo_y, hi_y)) = resident::vertical_section_range(
        resident::camera_section_y(camera, min_y),
        radius,
        sections,
    ) else {
        return false;
    };
    if key.1 < lo_y || key.1 > hi_y || !reachable(key) {
        return false;
    }
    let Some(frustum) = frustum else {
        return true;
    };
    let min = resident::origin(key, min_y, camera).as_vec3();
    let max = min + Vec3::splat(16.0);
    if let Some(tree) = resident::outside_height_tree(camera, min_y, sections, radius) {
        let offset = frustum.offset_for_camera_cube(camera, 8);
        let state = match &offset {
            Some(offset) => tree.branch_state(key.0, key.2, camera, offset),
            None => tree.branch_state(key.0, key.2, camera, frustum),
        };
        if state == resident::BRANCH_OUTSIDE {
            return false;
        }
        if state == resident::BRANCH_INSIDE {
            return true;
        }
        return match &offset {
            Some(offset) => offset.intersects(min, max),
            None => frustum.intersects(min, max),
        };
    }
    frustum.intersects(min, max)
}

/// Pairs each faded-in section's block entities with whether that section lies in the eye's
/// visible terrain set (`LevelExtractor.extractVisibleBlockEntities`).
fn visible_block_entities_in_eye_view<'a>(
    block_entities: impl Iterator<Item = (&'a SectionKey, &'a [([i32; 3], u32)])> + 'a,
    camera: DVec3,
    min_y: i32,
    sections: i32,
    frustum: Option<JoinedFrusta>,
    radius: i32,
    reachable: impl Fn(SectionKey) -> bool + 'a,
) -> impl Iterator<Item = (&'a ([i32; 3], u32), bool)> + 'a {
    block_entities.flat_map(move |(key, blocks)| {
        let eye_visible = section_in_eye_view(
            *key,
            camera,
            min_y,
            sections,
            radius,
            frustum.as_ref(),
            &reachable,
        );
        blocks.iter().map(move |block| (block, eye_visible))
    })
}

/// Filters `block_entities` in range of `camera`, restricting ordinary block entities to
/// eye-visible sections while keeping global renderers (`is_global`) on their existing path.
#[allow(clippy::too_many_arguments)]
fn filter_block_entities_in_range<'a>(
    block_entities: impl Iterator<Item = (&'a SectionKey, &'a [([i32; 3], u32)])> + 'a,
    camera: DVec3,
    min_y: i32,
    sections: i32,
    frustum: Option<JoinedFrusta>,
    radius: i32,
    reachable: impl Fn(SectionKey) -> bool + 'a,
    keep_long_range: impl Fn(u32) -> bool + 'a,
    is_global: impl Fn(u32) -> bool + Copy + 'a,
) -> impl Iterator<Item = BlockEntityInRange<'a>> + 'a {
    block_entities
        .filter_map(move |(key, blocks)| {
            let eye_visible = section_in_eye_view(
                *key,
                camera,
                min_y,
                sections,
                radius,
                frustum.as_ref(),
                &reachable,
            );
            if !eye_visible && !blocks.iter().any(|(_, state)| is_global(*state)) {
                return None;
            }
            let origin = DVec3::new(
                (i64::from(key.0) * 16 - 1) as f64,
                (i64::from(min_y) + i64::from(key.1) * 16 - 1) as f64,
                (i64::from(key.2) * 16 - 1) as f64,
            ) - camera;
            let min = origin.as_vec3();
            // The last block's expanded bound ends at section_origin + 17.
            let max = min + Vec3::splat(18.0);
            let distance_squared = Vec3::ZERO.max(min).min(max).length_squared();
            let near = distance_squared <= 64.0 * 64.0;
            let in_range = near
                || (distance_squared <= 256.0 * 256.0
                    && blocks.iter().any(|(_, state)| keep_long_range(*state)));
            in_range.then(|| {
                let section_inside = frustum.is_some_and(|view| view.contains(min, max));
                (blocks, eye_visible, section_inside, near)
            })
        })
        .flat_map(move |(blocks, eye_visible, section_inside, near)| {
            blocks.iter().filter_map(move |block| {
                (eye_visible || is_global(block.1)).then_some(BlockEntityInRange {
                    block,
                    section_inside,
                    near,
                })
            })
        })
}

/// `mesh`'s solid sources and where its solid range names their quads, for
/// `Arena::write_sources`: by record in the compact layout (`packed`, one record per quad in
/// mesh order), by first vertex otherwise.
pub(super) fn solid_sources(
    mesh: &SectionMesh,
    packed: bool,
) -> (arena::QuadSlots, &[crate::mesher::TerrainSource]) {
    let quads = mesh.solid.len() as u64 / 4;
    let names = if packed {
        arena::QuadSlots::records(quads)
    } else {
        arena::QuadSlots::vertices(4 * quads)
    };
    (names, &mesh.solid_sources)
}

/// A block entity of a section in range of the camera (`Renderer::block_entities_in_range`).
pub struct BlockEntityInRange<'a> {
    /// Its block and block state.
    pub block: &'a ([i32; 3], u32),
    /// The whole section is inside the camera frustum, so the game's finer per-block check
    /// cannot change the answer for it.
    pub section_inside: bool,
    /// The section is within the 64 blocks every block entity is drawn to, before
    /// `keep_long_range`.
    pub near: bool,
}

/// The normal stream of the arena page a run's `key` names (`draw_sections_paged`): the
/// translucent arena's past `flat::TRANSLUCENT_KEYS`, as far as its vertices are bound. Only
/// while the arenas keep one (`follow_arena_layout`).
pub(super) fn normals_binding_in<'a>(
    arena: &'a arena::Arena,
    translucent_arena: &'a Option<arena::Arena>,
    key: u32,
) -> wgpu::BufferSlice<'a> {
    let normals = match (translucent_arena, key.checked_sub(flat::TRANSLUCENT_KEYS)) {
        (Some(arena), Some(page)) => arena.normals_binding(page),
        _ => arena.normals_binding(key),
    };
    normals.expect("every page has a normal stream while a pack reads it")
}

impl Renderer {
    /// Whether `upload_mesh` puts `mesh`'s solid quads in the compact layout: where the
    /// mesher made it and the passes read it (`packed_quads`).
    fn uploads_packed(&self, mesh: &SectionMesh) -> bool {
        self.packed_quads && !mesh.packed.is_empty() && mesh.solid.len() >= 4
    }
    /// The bytes `upload` copies for `mesh`, for the player's upload budget: its vertices at
    /// their own size (`TerrainVertex`, as the budget has always counted them, compact records
    /// included, though those copy about a quarter of it), and what the arenas keep beside them
    /// as `upload_mesh` writes it: each
    /// vertex's normal-stream entry where a pack's per-section draws read the stream
    /// (`Arena::stream_bytes`), and each slot's source where draws read sources
    /// (`Arena::source_bytes`).
    pub fn mesh_upload_bytes(&self, mesh: &SectionMesh) -> u64 {
        let stride = std::mem::size_of::<TerrainVertex>() as u64;
        let (solid, translucent) = (mesh.solid.len() as u64, mesh.translucent.len() as u64);
        let packed = self.uploads_packed(mesh);
        let solid_slots = if packed {
            mesh.packed.len() as u64
        } else {
            solid
        };
        let (names, sources) = solid_sources(mesh, packed);
        let translucent_arena = self.translucent_arena.as_ref().unwrap_or(&self.arena);
        let beside = self.arena.source_bytes(names, sources.len(), solid_slots)
            + translucent_arena.source_bytes(
                arena::QuadSlots::vertices(translucent),
                mesh.translucent_sources.len(),
                translucent,
            )
            // No stream beside compact records (`upload_mesh`).
            + if packed { 0 } else { self.arena.stream_bytes(solid) }
            + translucent_arena.stream_bytes(translucent);
        (solid + translucent) * stride + beside
    }
    pub fn upload(&mut self, key: SectionKey, mesh: &SectionMesh) {
        if let Some(copies) = &mut self.capture_copies {
            copies.section(key, mesh);
        }
        let facts = MeshFacts::of(mesh);
        let fading = self.before_mesh(key, &facts);
        let section = self.upload_mesh(mesh);
        let uploaded = section.is_some();
        self.after_mesh(key, &facts, section, fading);
        // Ray tracing's triangle soup (#470, experimental), on a ray-query device with a
        // ray-tracing pack: solid quads baked into world space for one BLAS per section.
        if uploaded && self.rt_enabled() && !mesh.solid.is_empty() {
            let origin = rt::section_origin(key);
            self.rt
                .note_upload(key, rt::expand_section_positions(&mesh.solid, origin));
        }
    }
    /// Section `key`'s new mesh drawn from now on, its quads uploaded (`section`: none when it
    /// has none, or the arena had no room).
    fn after_mesh(
        &mut self,
        key: SectionKey,
        mesh: &MeshFacts,
        section: Option<SectionGpu>,
        fading: Option<Instant>,
    ) {
        self.ray_mark(key);
        let section = section.map(|section| SectionGpu { fading, ..section });
        self.set_detail(key, section.is_some() || mesh.empty);
        if let Some(mut section) = section {
            let sections = self.layout.sections();
            if let Some(gpu) = &mut self.gpu_culling
                && section.gpu_recorded(key, sections)
            {
                // A fade starts with its section: flagged from the first frame that
                // draws it, once a frame has decided the flags (`mark_fades`).
                if let Some(fade_ms) = gpu.fades.fade_ms {
                    let f = FadeInputs {
                        now: Instant::now(),
                        fade_ms,
                        uploaded: &self.section_uploaded,
                        retiring: &self.lod_retiring,
                    };
                    section.gpu_fading = decide_fade(&section, key, &mut gpu.fades, f);
                }
                section.gpu_id = Some(gpu.records.insert(section.record(key, mesh.bounds)));
            }
            let marked = section.listed(key, sections);
            self.sections
                .insert_marked(key, section, mesh.bounds, marked);
        }
    }
    pub(super) fn upload_mesh(&mut self, mesh: &SectionMesh) -> Option<SectionGpu> {
        let solid = (mesh.solid.len() / 4) as u32;
        let translucent = (mesh.translucent.len() / 4) as u32;
        if solid + translucent == 0 {
            return None;
        }
        let max = solid.max(translucent);
        if max > self.index_quads {
            self.index_quads = max.next_power_of_two();
            self.indices = Self::quad_indices(&self.device, self.index_quads);
        }
        let packed = self.uploads_packed(mesh);
        let encoder = self.section_copies.get_or_insert_with(|| {
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Section uploads"),
                })
        });
        // The solid quads, and the translucent ones after them unless they have
        // an arena of their own. With video and system memory both full, the
        // section is not drawn. The solid ones in the compact layout where the
        // mesher made it and the passes read it (`packed_quads`).
        let solid_slots = if packed {
            mesh.packed.len() as u64
        } else {
            mesh.solid.len() as u64
        };
        let split = self.translucent_arena.is_some();
        let main = if split {
            solid_slots
        } else {
            solid_slots + mesh.translucent.len() as u64
        };
        let (page, start) = if main > 0 {
            self.arena.alloc(&self.device, encoder, main)?
        } else {
            (0, 0)
        };
        let release_main = |arena: &mut arena::Arena| {
            if main > 0 {
                arena.release(page, start, main);
            }
        };
        let translucent_vertices = mesh.translucent.len() as u64;
        let translucent_at = match &mut self.translucent_arena {
            Some(arena) if translucent > 0 => {
                match arena.alloc(&self.device, encoder, translucent_vertices) {
                    Some(at) => at,
                    None => {
                        release_main(&mut self.arena);
                        return None;
                    }
                }
            }
            _ => (page, start + solid_slots),
        };
        let sorted_indices = if translucent > 0 {
            let indices = self.sorted_indices.alloc(
                &self.device,
                encoder,
                translucency::index_words(translucent),
            );
            if indices.is_none() {
                release_main(&mut self.arena);
                if let Some(arena) = &mut self.translucent_arena {
                    arena.release(translucent_at.0, translucent_at.1, translucent_vertices);
                }
                return None;
            }
            indices
        } else {
            None
        };
        // Flat draws name the blocks of its ranges by its id: its range in `arena` and,
        // split, its translucent quads' in `translucent_arena` (`flat::TRANSLUCENT_KEYS`).
        let translucent_key = (split && translucent > 0)
            .then(|| (translucent_at.0 + flat::TRANSLUCENT_KEYS, translucent_at.1));
        let flat_id = self.flat.as_mut().and_then(|f| {
            let ranges = [(main > 0).then_some((page, start, main))]
                .into_iter()
                .chain([translucent_key.map(|(key, first)| (key, first, translucent_vertices))]);
            let mut id = None;
            for (key, first, vertices) in ranges.flatten() {
                let uploads = &mut self.section_uploads;
                match id {
                    None => id = Some(f.add(&self.device, encoder, uploads, key, first, vertices)),
                    Some(id) => f.place(&self.device, encoder, uploads, id, key, first, vertices),
                }
            }
            id
        });
        let stride = std::mem::size_of::<TerrainVertex>() as u64;
        // The sources' bytes count toward the submission threshold as the vertices' do.
        let mut source_bytes = 0;
        if solid_slots > 0 {
            let (names, sources) = solid_sources(mesh, packed);
            source_bytes += self.arena.write_sources(
                &self.device,
                encoder,
                &mut self.section_uploads,
                (page, start, solid_slots),
                names,
                sources,
            );
        }
        if translucent_vertices > 0 {
            source_bytes += self
                .translucent_arena
                .as_mut()
                .unwrap_or(&mut self.arena)
                .write_sources(
                    &self.device,
                    encoder,
                    &mut self.section_uploads,
                    (translucent_at.0, translucent_at.1, translucent_vertices),
                    arena::QuadSlots::vertices(translucent_vertices),
                    &mesh.translucent_sources,
                );
        }
        self.uploaded_bytes += source_bytes;
        self.section_copy_bytes += source_bytes;
        // The normal stream beside the vertices, where the pack's per-section draws read one.
        // Never beside compact records: no per-section pack draw reads them
        // (`follow_packed_layout`).
        // With each quad's block id under the running pack's `block.properties`.
        let ids = self.pack_block_ids.as_deref().map_or(&[][..], |ids| &ids.0);
        let mut normal_bytes = 0;
        if !packed {
            let uploads = &mut self.section_uploads;
            let (vertices, sources) = (&mesh.solid, &mesh.solid_sources);
            normal_bytes +=
                self.arena
                    .write_normals(encoder, uploads, (page, start), vertices, sources, ids);
        }
        normal_bytes += self
            .translucent_arena
            .as_mut()
            .unwrap_or(&mut self.arena)
            .write_normals(
                encoder,
                &mut self.section_uploads,
                translucent_at,
                &mesh.translucent,
                &mesh.translucent_sources,
                ids,
            );
        // The stream's bytes count toward the submission threshold as the vertices' do.
        self.uploaded_bytes += normal_bytes;
        self.section_copy_bytes += normal_bytes;
        let translucent_arena = self.translucent_arena.as_ref().unwrap_or(&self.arena);
        let solid_bytes: &[u8] = if packed {
            bytemuck::cast_slice(&mesh.packed)
        } else {
            bytemuck::cast_slice(&mesh.solid)
        };
        for (bytes, arena, (part_page, first)) in [
            (solid_bytes, &self.arena, (page, start)),
            (
                bytemuck::cast_slice(&mesh.translucent),
                translucent_arena,
                translucent_at,
            ),
        ] {
            if !bytes.is_empty() {
                self.uploaded_bytes += bytes.len() as u64;
                self.section_copy_bytes += bytes.len() as u64;
                self.section_uploads.buffer(
                    encoder,
                    arena.buffer(part_page),
                    first * stride,
                    bytes,
                );
            }
        }
        if self.section_copy_bytes >= SECTION_COPY_LIMIT
            && let Some(copies) = self.take_section_copies()
        {
            self.queue.submit([copies]);
        }
        let animated_layers =
            collect_animated_layers(mesh, |layer| self.animations.is_animated(layer));
        Some(SectionGpu {
            page,
            start,
            translucent_at,
            solid,
            solid_slots,
            packed,
            packed_far: packed && mesh.packed_far,
            sides: if mesh.sides.quads() == solid {
                mesh.sides
            } else {
                Sides::unsorted(solid)
            },
            translucent,
            sort: translucency::SortState::of_quads(
                mesh.translucent
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|q| q.map(|v| v.pos)),
            ),
            sorted_indices,
            flat_id,
            gpu_id: None,
            gpu_fading: false,
            fading: None,
            leaving: None,
            animated_layers,
        })
    }
    /// The section copies recorded since the last submission, closed.
    pub(super) fn take_section_copies(&mut self) -> Option<wgpu::CommandBuffer> {
        let encoder = self.section_copies.take()?;
        self.section_copy_bytes = 0;
        self.section_uploads.finish(&encoder);
        Some(encoder.finish())
    }
    /// Draws `count` section draws from `first` with the arenas and section
    /// origins bound: one indirect multi-draw, or a loop without it.
    /// Consecutive draws in one arena page share a vertex buffer binding,
    /// keeping the list's order (translucent sections go back to front).
    /// `indirect` holds the list's draws from `base`. Leaves the quad
    /// indices bound, as the draws after it expect.
    pub(super) fn draw_sections(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        indirect: &wgpu::Buffer,
        base: u32,
        first: u32,
        count: u32,
    ) {
        self.draw_sections_from(pass, indirect, base, first, count, false);
    }
    /// `draw_sections` of a list from its start (`base` 0), calling `page` with each run's
    /// arena page before drawing from it: a shader pack's terrain programs that read the page
    /// (`pack_atlas::TerrainDraws::Pages`) bind it there. Translucent arena pages are named by
    /// their key past `flat::TRANSLUCENT_KEYS`, as flat draws name them.
    pub(super) fn draw_sections_paged(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        indirect: &wgpu::Buffer,
        first: u32,
        count: u32,
        page: &dyn Fn(&mut wgpu::RenderPass<'_>, u32),
    ) {
        self.draw_sections_with(pass, indirect, 0, first..first + count, false, page);
    }
    /// `draw_sections` through the solid pipelines that read the arena page themselves
    /// (`vs_section`): each page's group is bound as their group 1.
    pub(super) fn draw_solid_sections(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        indirect: &wgpu::Buffer,
        base: u32,
        first: u32,
        count: u32,
    ) {
        self.draw_sections_from(pass, indirect, base, first, count, self.flat.is_some());
    }
    pub(super) fn draw_sections_from(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        indirect: &wgpu::Buffer,
        base: u32,
        first: u32,
        count: u32,
        pulled: bool,
    ) {
        self.draw_sections_with(
            pass,
            indirect,
            base,
            first..first + count,
            pulled,
            &|_, _| {},
        );
    }
    /// `draw_sections_from` over the draws `list`, calling `bind_page` with each run's page key
    /// before its draws.
    fn draw_sections_with(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        indirect: &wgpu::Buffer,
        base: u32,
        list: std::ops::Range<u32>,
        pulled: bool,
        bind_page: &dyn Fn(&mut wgpu::RenderPass<'_>, u32),
    ) {
        let (draws, pages) = (&self.draws[..], &self.draw_pages[..]);
        let (mut i, end) = (list.start as usize, list.end as usize);
        while i < end {
            let (page, index_page) = pages[i];
            let run = pages[i..end]
                .iter()
                .take_while(|&&p| p == (page, index_page))
                .count();
            // Translucent draws, and only they, have sorted indices; split, their large
            // pages are bound up to their last range in use (`Arena::binding`).
            let (vertices, indices, key) = match (&self.translucent_arena, index_page) {
                (Some(arena), Some(p)) => (
                    arena.binding(page),
                    self.sorted_indices.binding(p),
                    page + flat::TRANSLUCENT_KEYS,
                ),
                (_, p) => (
                    self.arena.buffer(page).slice(..),
                    p.map_or(&self.indices, |p| self.sorted_indices.buffer(p))
                        .slice(..),
                    page,
                ),
            };
            pass.set_vertex_buffer(0, vertices);
            bind_page(pass, key);
            if pulled
                && index_page.is_none()
                && let Some(group) = self.flat.as_ref().and_then(|f| f.page_group(page))
            {
                pass.set_bind_group(1, group, &[]);
            }
            pass.set_index_buffer(indices, wgpu::IndexFormat::Uint32);
            if !self.base_vertex {
                // Android fork fallback: GLES 3.0 has no base-vertex draws.
                // Rebase the selected arena's bounded binding instead.
                for d in &draws[i..i + run] {
                    let offset = u64::from(d[3]) * std::mem::size_of::<TerrainVertex>() as u64;
                    pass.set_vertex_buffer(0, vertices.slice(offset..));
                    pass.draw_indexed(d[2]..d[2] + d[0], 0, d[4]..d[4] + 1);
                }
            } else if self.multi_draw {
                let offset = (i - base as usize) as u64 * 20;
                pass.multi_draw_indexed_indirect(indirect, offset, run as u32);
            } else {
                for d in &draws[i..i + run] {
                    pass.draw_indexed(d[2]..d[2] + d[0], d[3] as i32, d[4]..d[4] + 1);
                }
            }
            i += run;
        }
        if pages[list.start as usize..end]
            .iter()
            .any(|(_, sorted)| sorted.is_some())
        {
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        }
    }
    pub(super) fn release_section(&mut self, key: SectionKey) {
        if let Some(s) = self.sections.remove(key) {
            self.release_mesh(s);
        }
    }
    pub(super) fn release_mesh(&mut self, s: SectionGpu) {
        if let (Some(flat), Some(id)) = (&mut self.flat, s.flat_id) {
            flat.remove(id);
        }
        if let (Some(gpu), Some(id)) = (&mut self.gpu_culling, s.gpu_id) {
            gpu.records.remove(id);
        }
        if let Some((page, start)) = s.sorted_indices {
            self.sorted_indices
                .release(page, start, translucency::index_words(s.translucent));
        }
        let (main, translucent) = s.vertex_ranges(self.translucent_arena.is_some());
        if let Some((page, start, vertices)) = main {
            self.arena.release(page, start, vertices);
        }
        if let (Some(arena), Some((page, start, vertices))) =
            (&mut self.translucent_arena, translucent)
        {
            arena.release(page, start, vertices);
        }
    }
    /// Video memory the arena keeps because fragmentation holds its pages
    /// partly used (a render distance that went down, far tiles that were
    /// replaced): moves the meshes off the sparsest page, a few MiB a frame,
    /// and `Arena::trim` returns the page once it is empty. Frames with the
    /// arena in system memory (video memory full) are slow, so this is what
    /// keeps the terrain in video memory. The section arena's: the large pages
    /// of the translucent arena and the sorted indices are `shrink_translucent`'s.
    /// Returns what is left of the frame's `COMPACT_BUDGET`.
    pub(super) fn compact_arena(&mut self) -> u64 {
        let mut budget = COMPACT_BUDGET;
        if self.arena.closing().is_none()
            && let Some(page) = self.arena.sparsest()
        {
            self.arena.close(page);
        }
        let Some(page) = self.arena.closing() else {
            return budget;
        };
        let encoder = self.section_copies.get_or_insert_with(|| {
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Section uploads"),
                })
        });
        // GPU culling's records move with the sections, sourced geometry's with its meshes.
        let (sourced, sourced_ids) = self.sourced.moves();
        let meshes = self.sections.values_mut().map(|s| (s, None)).chain(
            self.lods
                .iter_mut()
                .flat_map(|(key, tile)| tile.recorded_meshes_mut(sourced_ids(*key))),
        );
        let flat = self
            .flat
            .as_mut()
            .map(|f| (f, &self.device, &mut self.section_uploads));
        let gpu = self.gpu_culling.as_mut().map(|g| &mut g.records);
        let split = self.translucent_arena.is_some();
        let (arena, part) = (&mut self.arena, Part::Section { split });
        if !move_off_page(
            arena,
            encoder,
            flat,
            gpu,
            sourced,
            meshes,
            page,
            part,
            &mut budget,
        ) {
            // The other pages have no room after all.
            arena.reopen();
        }
        // With every mesh moved off, the page stays closed so this frame's
        // uploads can't refill it: `trim` returns it and clears `closing`.
        budget
    }
    /// Shrinks the large pages of the translucent arena and the sorted indices
    /// once terrain has settled (`set_terrain_loading`): one far larger than what
    /// it holds moves into a new page of their size (`begin_shrink`), through
    /// `move_off_page` with what `compact_arena` left of the frame's budget
    /// (`left`), and `trim` returns it once empty. One page at a time, vertices
    /// first; only starts wait, a move under way finishes even if terrain comes
    /// again (stopped, the page would stay closed while the new one fills).
    pub(super) fn shrink_translucent(&mut self, left: u64) {
        let Some(vertices) = self.translucent_arena.as_mut() else {
            return; // split off: nothing to shrink
        };
        let now = Instant::now();
        let mut arenas = [
            (vertices, Part::Translucent),
            (&mut self.sorted_indices, Part::Indices),
        ];
        if arenas.iter().all(|(arena, _)| arena.closing().is_none()) {
            if !self.shrink_gate.may_start(now, left) {
                return;
            }
            // The next start in a second, or `SHRINK_RETRY` after a try found no room.
            let mut wait = SHRINK_SETTLE;
            for (arena, part) in &mut arenas {
                let meshes = self
                    .sections
                    .values_mut()
                    .chain(self.lods.values_mut().flat_map(|tile| tile.meshes_mut()));
                match begin_shrink(arena, &self.device, meshes, *part) {
                    Some(true) => break,
                    Some(false) => wait = SHRINK_RETRY, // the other arena may still go
                    None => {}
                }
            }
            self.shrink_gate.wait(now, wait);
        }
        let Some((arena, part, page)) = arenas.into_iter().find_map(|(arena, part)| {
            let page = arena.closing()?;
            Some((arena, part, page))
        }) else {
            return;
        };
        let encoder = self.section_copies.get_or_insert_with(|| {
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Section uploads"),
                })
        });
        let meshes = self
            .sections
            .values_mut()
            .chain(self.lods.values_mut().flat_map(|tile| tile.meshes_mut()))
            .map(|s| (s, None));
        // As many quads a frame either way: a quad's indices are a quarter of its bytes.
        let cap = match part {
            Part::Indices => COMPACT_BUDGET / 4,
            _ => COMPACT_BUDGET,
        };
        let mut budget = left.min(cap);
        // Flat draws name translucent quads' blocks by their section's id; indices have none.
        let flat = match part {
            Part::Translucent => self.flat.as_mut(),
            _ => None,
        }
        .map(|f| (f, &self.device, &mut self.section_uploads));
        // GPU culling's records follow the section arena alone.
        if !move_off_page(
            arena,
            encoder,
            flat,
            None,
            None,
            meshes,
            page,
            part,
            &mut budget,
        ) {
            // New ranges took the room: both pages stay, drawn, until a later try.
            arena.reopen();
            self.shrink_gate.wait(now, SHRINK_RETRY);
        }
    }
    /// Block-entity blocks of every meshed section (see `SectionMesh::block_entities`).
    pub fn block_entities(&self) -> impl Iterator<Item = &([i32; 3], u32)> {
        self.block_entities.values().flatten()
    }
    /// `block_entities` of sections far enough into their fade-in to draw them
    /// (`LevelExtractor.extractVisibleBlockEntities`: `getVisibility(now, fade) >= 0.3`),
    /// paired with whether each block's section is in the eye's visible terrain set.
    pub fn visible_block_entities<'a>(
        &'a self,
        fade_seconds: f32,
        camera: DVec3,
        frustum: Option<JoinedFrusta>,
        radius: i32,
    ) -> impl Iterator<Item = (&'a ([i32; 3], u32), bool)> + 'a {
        let fade = fade_millis(fade_seconds);
        let now = std::time::Instant::now();
        let all = !self.sections_fading_in(fade, now);
        let min_y = self.layout.min_y();
        let sections = self.layout.sections() as i32;
        let faded = self.block_entities.iter().filter(move |(key, _)| {
            all || chunk_visibility(self.section_elapsed_ms(**key, now), fade)
                >= CHUNK_VISIBILITY_THRESHOLD
        });
        visible_block_entities_in_eye_view(
            faded.map(|(k, v)| (k, v.as_slice())),
            camera,
            min_y,
            sections,
            frustum,
            radius,
            move |k| self.occlusion.contains(k),
        )
    }
    /// Whether the section holding block `pos` lies in the eye's visible terrain set
    /// (`LevelExtractor.extractVisibleBlockEntities`).
    pub fn block_entity_section_in_eye_view(
        &self,
        pos: [i32; 3],
        camera: DVec3,
        frustum: Option<&JoinedFrusta>,
        radius: i32,
    ) -> bool {
        let Some(sy) = self.layout.section_index(pos[1]) else {
            return false;
        };
        let key = (pos[0] >> 4, sy as i32, pos[2] >> 4);
        section_in_eye_view(
            key,
            camera,
            self.layout.min_y(),
            self.layout.sections() as i32,
            radius,
            frustum,
            |k| self.occlusion.contains(k),
        )
    }
    /// `LevelRenderer.isSectionCompiledAndVisible`: the entity's section must have finished
    /// its mesh (including empty sections) and reached the same fade threshold as block entities.
    pub fn entity_section_visible(
        &self,
        [x, y, z]: [i32; 3],
        fade_seconds: f32,
        now: Instant,
    ) -> bool {
        let Some(sy) = self.layout.section_index(y) else {
            return false;
        };
        let column = (x >> 4, z >> 4);
        self.detail_columns
            .get(&column)
            .is_some_and(|mask| mask.contains(sy))
            && chunk_visibility(
                self.section_elapsed_ms((column.0, sy as i32, column.1), now),
                fade_millis(fade_seconds),
            ) >= CHUNK_VISIBILITY_THRESHOLD
    }

    /// Whether a section may still be fading in over `fade_ms` (vanilla's chunk fade): one was
    /// uploaded more recently than that. Otherwise every section is whole.
    pub(super) fn sections_fading_in(&self, fade_ms: u64, now: std::time::Instant) -> bool {
        fade_ms > 0
            && self
                .newest_upload
                .is_some_and(|t| (now.saturating_duration_since(t).as_millis() as u64) < fade_ms)
    }
    /// Whether a section may be fading in or out over far terrain (`SectionGpu::fade`):
    /// otherwise every section's code is 0.
    pub(super) fn sections_fading_over_lods(&self, now: std::time::Instant) -> bool {
        !self.lod_retiring.is_empty()
            || !self.lod_leaving.is_empty()
            || self
                .newest_lod_fade
                .is_some_and(|t| now.saturating_duration_since(t) < lod::FADE)
    }
    /// Block entities of every section in range of `camera`, restricting ordinary block entities
    /// to eye-visible sections (`LevelExtractor.extractVisibleBlockEntities`) while keeping global
    /// renderers (`is_global`) on their existing path.
    pub fn block_entities_in_range<'a>(
        &'a self,
        fade_seconds: f32,
        camera: DVec3,
        frustum: Option<JoinedFrusta>,
        radius: i32,
        keep_long_range: impl Fn(u32) -> bool + 'a,
        is_global: impl Fn(u32) -> bool + Copy + 'a,
    ) -> impl Iterator<Item = BlockEntityInRange<'a>> + 'a {
        let fade = fade_millis(fade_seconds);
        let now = Instant::now();
        // No section uploaded within the fade: all are whole, none is looked up.
        let all = !self.sections_fading_in(fade, now);
        let min_y = self.layout.min_y();
        let sections = self.layout.sections() as i32;
        let faded = self.block_entities.iter().filter(move |(key, _)| {
            all || chunk_visibility(self.section_elapsed_ms(**key, now), fade)
                >= CHUNK_VISIBILITY_THRESHOLD
        });
        filter_block_entities_in_range(
            faded.map(|(k, v)| (k, v.as_slice())),
            camera,
            min_y,
            sections,
            frustum,
            radius,
            move |k| self.occlusion.contains(k),
            keep_long_range,
            is_global,
        )
    }
    /// The block entities of the sections a shader pack's shadow pass drew in the last frame
    /// (`shadow_visible`), within the 64 blocks `block_entities_in_range` keeps.
    /// Our probe packs' Iris 1.11.7 shadow maps hold a block entity wherever the shadow pass
    /// draws its section's terrain, whether or not the camera's view takes it in (#8464).
    pub fn shadow_pass_block_entities(
        &self,
        camera: DVec3,
    ) -> impl Iterator<Item = &([i32; 3], u32)> + '_ {
        block_entities_within_reach(
            self.shadow_visible.iter().map(|v| v.key),
            &self.block_entities,
            camera,
            self.layout.min_y(),
        )
    }
    /// Whether a shader pack's shadow pass drew the section holding block `pos` in the last
    /// frame (`shadow_pass_block_entities`).
    pub fn shadow_pass_drew(&self, pos: [i32; 3]) -> bool {
        let key = (
            pos[0] >> 4,
            resident::block_to_section_y(pos[1], self.layout.min_y()),
            pos[2] >> 4,
        );
        self.shadow_visible.iter().any(|v| v.key == key)
    }
    /// `LevelRenderer.isSectionCompiledAndVisible` for the section holding block `pos`: it
    /// has had a mesh since its column came (an empty one too), and it is 30% through its
    /// fade-in of `fade_ms` since its first upload. A mesh still waiting to upload is not
    /// compiled yet.
    pub fn section_compiled_and_visible(&self, pos: [i32; 3], fade_ms: u64) -> bool {
        let Some(sy) = self.layout.section_index(pos[1]) else {
            return false;
        };
        let key = (pos[0] >> 4, sy as i32, pos[2] >> 4);
        let compiled = self
            .detail_columns
            .get(&(key.0, key.2))
            .is_some_and(|mask| mask.contains(sy));
        compiled_and_visible(
            compiled,
            self.section_uploaded.get(&key),
            fade_ms,
            std::time::Instant::now(),
        )
    }
    /// Milliseconds since `key` was uploaded; a section not in `section_uploaded` is old.
    fn section_elapsed_ms(&self, key: SectionKey, now: std::time::Instant) -> u64 {
        self.section_uploaded.get(&key).map_or(u64::MAX, |t| {
            now.saturating_duration_since(*t).as_millis() as u64
        })
    }
    /// A value that changes whenever `block_entities` gives something else.
    pub fn block_entity_revision(&self) -> u64 {
        self.block_entity_revision
    }
    /// Both mesh/light and block-entity membership revisions for a section
    /// key (`x`, section-y relative to the dimension's minimum, `z`).
    pub fn block_entity_section_revision(
        &self,
        section: [i32; 3],
    ) -> Option<StaticSectionRevision> {
        let key = (section[0], section[1], section[2]);
        Some(StaticSectionRevision {
            mesh: *self.block_entity_section_revisions.get(&key)?,
            block_entities: *self.block_entity_membership_revisions.get(&key)?,
        })
    }
    /// Block-entity cache work and upload volume from the last rendered frame.
    pub fn static_block_entity_frame_stats(&self) -> StaticCacheFrameStats {
        self.static_entity_frame_stats
    }
    /// Drops every section, for a level whose chunks hold `layout`.
    pub fn clear_sections(&mut self, layout: crate::chunk::ColumnLayout) {
        if let Some(copies) = &mut self.capture_copies {
            *copies = capture::Copies::default();
        }
        self.block_entities.clear();
        self.block_entity_section_revisions.clear();
        self.block_entity_membership_revisions.clear();
        self.section_uploaded.clear();
        self.newest_upload = None;
        self.newest_lod_fade = None;
        self.block_entity_revision = next_block_entity_revision();
        self.sections.clear();
        self.ray_clear();
        self.lods.clear();
        self.lod_vertex_bytes = 0;
        self.lod_cells.clear();
        self.sourced.clear();
        self.lod_cutout.clear();
        self.lod_retiring.clear();
        self.lod_leaving.clear();
        self.lod_pending = None;
        self.detail_columns.clear();
        self.detail_revision += 1;
        // A dimension change drops every page at once: freed later, off this thread. One
        // page's buffer stays for the next level's first sections, or is made now, while
        // its loading screen shows (`Arena::prewarm`).
        self.graveyard.add(self.arena.clear());
        self.arena.prewarm(&self.device);
        if let Some(gpu) = &mut self.gpu_culling {
            gpu.records.clear();
            // The chunk fade-in time they were decided with stays.
            gpu.fades.clear();
        }
        if let Some(arena) = &mut self.translucent_arena {
            self.graveyard.add(arena.clear());
        }
        if let Some(flat) = &mut self.flat {
            flat.clear();
        }
        self.graveyard.add(self.sorted_indices.clear());
        self.translucency_cursor = 0;
        if layout == self.layout {
            self.occlusion.clear();
        } else {
            self.layout = layout;
            self.occlusion = occlusion::Occlusion::new(layout.min_y(), layout.sections() as i32);
            self.sections.set_min_y(layout.min_y());
            self.sections.set_sections(layout.sections() as i32);
        }
    }
    /// Lays the sections out as `translucent_split` says for the shader pack that
    /// runs, before a world frame draws them. A change drops every section and far
    /// tile, as a new renderer has none, and changes the sorted indices' arena with
    /// them (large pages beside the translucent arena, else pages flat draws bind):
    /// the game meshes them again (`take_dropped_sections`).
    pub(super) fn follow_translucent_layout(&mut self) {
        let pack = self.pack_post.as_ref().and_then(|p| p.1.as_ref());
        let pending = self.pack_pending().unwrap_or_default();
        let split = translucent_split(
            self.translucent_batching,
            self.flat.is_some() && self.flat_on,
            self.pack_reads_pages(),
            // A pack still building draws what its programs will (`pack_pending`).
            pending.terrain || pack.is_some_and(|p| p.has_terrain()),
            // One of Sunrise's own packs with `[shadow]` draws from the same lists (`shadow_view`).
            pending.shadow || pack.is_some_and(|p| p.has_shadow()) || self.native_shadow(),
        );
        if split == self.translucent_arena.is_some() {
            return;
        }
        log::info!(
            "Sunrise: terrain laid out again: translucent quads {}",
            if split {
                "in an arena of their own"
            } else {
                "in the section arena"
            }
        );
        self.sections_dropped |=
            self.sections.len() > 0 || !self.lods.is_empty() || self.lod_pending.is_some();
        self.clear_sections(self.layout);
        (self.sorted_indices, self.translucent_arena) = arena::Arena::translucent_arenas(
            &self.device,
            std::mem::size_of::<TerrainVertex>() as u64,
            self.flat.is_some(),
            split,
            self.arena.has_normals(),
        );
        if let (Some(flat), Some(arena)) = (&mut self.flat, &self.translucent_arena) {
            flat.set_translucent_page_vertices(arena.page_vertices());
        }
    }
    /// Keeps the section arena's quad sources while meshes carry them for a `shaders/` pack
    /// (`on`, the game's `shader_pack` meshing switch) and flat draws, or the pack's
    /// per-section draws that read the pages (`pack_reads_pages`), can read them
    /// (`pack_atlas`'s `hz_sources`); none otherwise. Switching off frees every sidecar, and
    /// meshes still carrying a dropped pack's sources are kept without them. The translucent
    /// arena, which no draw binds as storage, never keeps any.
    pub fn keep_pack_sources(&mut self, on: bool) {
        self.pack_sources_on = on;
        self.follow_pack_sources();
    }
    /// Has the section arena keep its quad sources or not, as `keep_pack_sources` says.
    fn follow_pack_sources(&mut self) {
        let read = self.pack_sources_on && (self.flat.is_some() || self.pack_reads_pages());
        if read != self.arena.sources_read() {
            let released = self.arena.read_sources(read);
            self.graveyard.add(released);
        }
    }
    /// What a `shaders/` pack being built (`load_shader_pack`) and not yet run
    /// (`poll_shader_pack`) will draw from the game's sections (`PackLoading::draws_of`); `None`
    /// with none. The layouts the sections are in (`follow_arena_layout`,
    /// `follow_translucent_layout`, `follow_packed_layout`) take such a pack as present, drawing
    /// that, with its terrain drawn as `pack_terrain_inputs` decided for it: the frames drawn
    /// meanwhile keep the layout it will draw in rather than drop the sections for the frames
    /// without it and again once it runs.
    pub(super) fn pack_pending(&self) -> Option<PackDraws> {
        // Starling: what the pack draws once its build thread has translated it.
        (self.pack_loading.as_ref()).map(|l| l.draws.get().copied().unwrap_or_default())
    }
    /// Whether a `shaders/` pack, running or being built (`pack_pending`), has programs that
    /// draw the game's sections (`PackPost::draws_sections`): a pack with none, one of
    /// `composite` and `final` programs alone, reads no pages and no stream.
    fn pack_draws_sections(&self) -> bool {
        let pack = self.pack_post.as_ref().and_then(|p| p.1.as_ref());
        self.pack_pending().is_some_and(|d| d.sections) || pack.is_some_and(|p| p.draws_sections())
    }
    /// Whether a `shaders/` pack's terrain or shadow programs read the section arena's pages in
    /// per-section draws (`pack_atlas::TerrainDraws::Pages`).
    pub(super) fn pack_reads_pages(&self) -> bool {
        self.pack_terrain_draws() == pack_atlas::TerrainDraws::Pages && self.pack_draws_sections()
    }
    /// Whether a `shaders/` pack's terrain or shadow programs read the normal stream in
    /// per-section draws (`pack_atlas::TerrainDraws::Normals`).
    pub(super) fn pack_reads_normals(&self) -> bool {
        self.pack_terrain_draws() == pack_atlas::TerrainDraws::Normals && self.pack_draws_sections()
    }
    /// Makes the section arena's pages storage, or not, as `arena_storage` says for the shader
    /// pack that runs, and gives its pages and the translucent arena's a normal stream while
    /// that pack's per-section draws read one (`pack_reads_normals`), before a world frame draws
    /// them. A change drops every section and far tile, as a new renderer has none: the game
    /// meshes them again (`take_dropped_sections`). The arena keeps its quad sources as
    /// `keep_pack_sources` says for that pack.
    pub(super) fn follow_arena_layout(&mut self) {
        let storage = arena_storage(self.flat.is_some(), self.pack_reads_pages());
        let normals = self.pack_reads_normals();
        let normals_changed = normals != self.arena.has_normals();
        if storage == self.arena_storage && !normals_changed {
            self.follow_pack_sources();
            return;
        }
        log::info!(
            "Sunrise: terrain laid out again: section pages {}, normal stream {}",
            if storage {
                "storage"
            } else {
                "vertex buffers alone"
            },
            if normals { "on" } else { "off" }
        );
        self.sections_dropped |=
            self.sections.len() > 0 || !self.lods.is_empty() || self.lod_pending.is_some();
        self.clear_sections(self.layout);
        self.arena = arena::Arena::new(
            &self.device,
            std::mem::size_of::<TerrainVertex>() as u64,
            1 << 20,
            storage,
        )
        .with_normals(normals);
        self.arena_storage = storage;
        // The new arena keeps quad sources from its first upload where draws read them, flat
        // ones as they did the old arena's, or the pack's per-section ones reading the pages
        // (`keep_pack_sources`).
        self.follow_pack_sources();
        if normals_changed && self.translucent_arena.is_some() {
            (self.sorted_indices, self.translucent_arena) = arena::Arena::translucent_arenas(
                &self.device,
                std::mem::size_of::<TerrainVertex>() as u64,
                self.flat.is_some(),
                true,
                normals,
            );
            if let (Some(flat), Some(arena)) = (&mut self.flat, &self.translucent_arena) {
                flat.set_translucent_page_vertices(arena.page_vertices());
            }
        }
    }
    /// Uploads solid quads in the compact layout (`packed_quads`) unless a pack reads the
    /// arena's vertices as `TerrainVertex`es: a `shaders/` pack's terrain and shadow
    /// programs outside flat draws (flat ones unpack the records, `pack_atlas::PULL_FLAT`).
    /// Leaving it drops every section and far tile for the game to mesh again.
    pub(super) fn follow_packed_layout(&mut self) {
        // Any pack, a post-only one too, as once it runs; one still building counts.
        let pack = self.pack_pending().is_some()
            || self.pack_post.as_ref().and_then(|p| p.1.as_ref()).is_some();
        let want = self.flat.is_some() && (!pack || self.pack_terrain_flat) && self.packed_on;
        if want == self.packed_quads {
            return;
        }
        log::info!(
            "Sunrise: compact solid terrain {}",
            if want { "on" } else { "off" }
        );
        self.packed_quads = want;
        packed::pack_quads(want);
        let laid_out = self.sections.values().any(|s| s.packed)
            || self.lods.values().any(|t| t.holds_packed());
        if !want && laid_out {
            self.sections_dropped = true;
            self.clear_sections(self.layout);
        }
    }
    /// Allows the compact layout of solid quads or not, for A/B checks: every section and far
    /// tile is meshed again in the layout that follows. Replies whether sections now use it.
    pub fn set_packed_layout(&mut self, on: bool) -> bool {
        if on != self.packed_on {
            self.packed_on = on;
            self.follow_packed_layout();
            self.sections_dropped = true;
            self.clear_sections(self.layout);
        }
        self.packed_quads
    }
    /// Whether every section and far tile was dropped since the last call, to be
    /// laid out again (`follow_translucent_layout`): the game meshes them again,
    /// as for a new renderer.
    pub fn take_dropped_sections(&mut self) -> bool {
        std::mem::take(&mut self.sections_dropped)
    }
    /// Whether terrain is on its way (the view not all in, events, chunks or
    /// uploads still waiting, far tiles sampled): a large translucent page shrinks
    /// once that, new translucent sections (`upload`) and far tiles to upload
    /// (`upload_lods`) have been over for `SHRINK_SETTLE`, not at the low point
    /// between a view's columns leaving and the next one's arriving, after which
    /// it would grow back by whole copies.
    pub fn set_terrain_loading(&mut self, loading: bool) {
        if loading {
            self.shrink_gate.wait(Instant::now(), SHRINK_SETTLE);
        }
    }
    /// A column whose every section has its mesh (`lod_detail_columns`).
    fn complete_column(&self) -> SectionMask {
        SectionMask::below(self.layout.sections())
    }
    pub fn lod_detail_columns(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        let complete = self.complete_column();
        self.detail_columns
            .iter()
            .filter_map(move |(&column, &mask)| (mask == complete).then_some(column))
    }
    /// Changes whenever `lod_detail_columns` does.
    pub fn lod_detail_revision(&self) -> u64 {
        self.detail_revision
    }
    /// Whether the section at `key` has its mesh, for `lod_detail_columns`.
    fn set_detail(&mut self, key: SectionKey, meshed: bool) {
        let column = (key.0, key.2);
        let mask = self
            .detail_columns
            .get(&column)
            .copied()
            .unwrap_or_default();
        let mut next = mask;
        if meshed {
            next.insert(key.1 as usize);
        } else {
            next.remove(key.1 as usize);
        }
        let complete = self.complete_column();
        if (mask == complete) != (next == complete) {
            self.detail_revision += 1;
        }
        if next.is_empty() {
            self.detail_columns.remove(&column);
        } else {
            self.detail_columns.insert(column, next);
        }
    }
    /// Every section of these columns goes: one lookup per column where the
    /// tables are by column, and the meshes' ranges freed in one batch. A
    /// teleport removes a few thousand columns in one frame.
    /// A section whose column far terrain now draws fades out over it, chunk by
    /// chunk as it came, and is released once it has (`release_left`): taken
    /// away at once, the column's detail dropped to the far terrain's in one
    /// frame.
    pub(super) fn leave_section(&mut self, key: SectionKey) {
        let now = std::time::Instant::now();
        if let Some(section) = self.sections.find_mut(key)
            && section.leaving.is_none()
        {
            section.leaving = Some(now);
            self.lod_leaving.push((key, now));
            if let Some(gpu) = &mut self.gpu_culling {
                hold_fading(&mut self.sections, &mut gpu.records, key);
            }
        }
    }
    /// Releases the sections that have faded out (`leave_section`).
    pub(super) fn release_left(&mut self, now: std::time::Instant) {
        let mut done = Vec::new();
        self.lod_leaving.retain(|&(key, since)| {
            let left = now.saturating_duration_since(since) >= lod::FADE;
            if left {
                done.push((key, since));
            }
            !left
        });
        for (key, since) in done {
            // Meshed again since (the player came back): it stays.
            if self
                .sections
                .find(key)
                .is_some_and(|s| s.leaving == Some(since))
            {
                self.release_section(key);
            }
        }
    }
    pub fn remove_columns(&mut self, columns: impl IntoIterator<Item = (i32, i32)>) {
        let columns: Vec<(i32, i32)> = columns.into_iter().collect();
        if let Some(copies) = &mut self.capture_copies {
            copies.remove_columns(&columns);
        }
        // Most of the view at once (a teleport at render distance 32 drops some 3700
        // columns): a pass over each store instead of a lookup per section key, which
        // took 10 ms of one frame. Far terrain fades the sections out instead.
        let keys = columns.len() * self.layout.sections();
        if self.lods.is_empty()
            && self.lod_cutout.is_empty()
            && keys >= self.sections.len().max(BULK_REMOVAL_KEYS)
        {
            self.remove_columns_at_once(&columns);
            return;
        }
        let mut released = Vec::new();
        let mut block_entities = false;
        for (x, z) in columns {
            if self.detail_columns.remove(&(x, z)) == Some(self.complete_column()) {
                self.detail_revision += 1;
            }
            self.occlusion.remove_column(x, z);
            let retiring = !self.lod_cutout.is_empty() && self.lod_cutout.contains(&(x, z));
            for sy in 0..self.layout.sections() as i32 {
                let key = (x, sy, z);
                block_entities |= self.block_entities.remove(&key);
                self.block_entity_section_revisions.remove(&key);
                self.block_entity_membership_revisions.remove(&key);
                if self.light_blocks.remove(&key) {
                    self.light_revision += 1;
                }
                self.section_uploaded.remove(&key);
                if retiring {
                    // Distant tiles still leave its column out: drawn until they don't.
                    self.lod_retiring.insert(key);
                    if let Some(gpu) = &mut self.gpu_culling {
                        hold_fading(&mut self.sections, &mut gpu.records, key);
                    }
                } else if !self.lods.is_empty() && self.sections.contains(key) {
                    // Far terrain draws the column: it fades out over it.
                    self.leave_section(key);
                } else if let Some(section) = self.sections.remove(key) {
                    self.rt.forget(key);
                    released.push(section);
                }
            }
        }
        if block_entities {
            self.block_entity_revision = next_block_entity_revision();
        }
        self.release_meshes(released);
    }
    /// `remove_columns` for a large share of the resident sections: each store filtered
    /// once by column.
    fn remove_columns_at_once(&mut self, columns: &[(i32, i32)]) {
        let gone: crate::fast_hash::FxHashSet<(i32, i32)> = columns.iter().copied().collect();
        let complete = self.complete_column();
        for &(x, z) in columns {
            if self.detail_columns.remove(&(x, z)) == Some(complete) {
                self.detail_revision += 1;
            }
        }
        self.occlusion.remove_columns(&gone);
        let kept = |key: &SectionKey| !gone.contains(&(key.0, key.2));
        let before = self.block_entities.len();
        self.block_entities.retain(kept);
        if self.block_entities.len() != before {
            self.block_entity_revision = next_block_entity_revision();
        }
        let before = self.light_blocks.len();
        self.light_blocks.retain(kept);
        self.light_revision += (before - self.light_blocks.len()) as u64;
        self.section_uploaded.retain(|key, _| kept(key));
        let released = self.sections.remove_columns(&gone);
        for &(x, z) in columns {
            self.rt.forget_column(x, z);
        }
        self.release_meshes(released);
    }
    /// Gives the removed sections' quads back to their arenas; the sections themselves
    /// (their sort centres) drop on a thread of their own when there are many.
    fn release_meshes(&mut self, released: Vec<SectionGpu>) {
        if let Some(flat) = &mut self.flat {
            for id in released.iter().filter_map(|s| s.flat_id) {
                flat.remove(id);
            }
        }
        if let Some(gpu) = &mut self.gpu_culling {
            gpu.records
                .remove_all(released.iter().filter_map(|s| s.gpu_id));
        }
        self.sorted_indices
            .release_all(released.iter().filter_map(|s| {
                let (page, start) = s.sorted_indices?;
                Some((page, start, translucency::index_words(s.translucent)))
            }));
        let split = self.translucent_arena.is_some();
        self.arena
            .release_all(released.iter().filter_map(|s| s.vertex_ranges(split).0));
        if let Some(arena) = &mut self.translucent_arena {
            arena.release_all(released.iter().filter_map(|s| s.vertex_ranges(true).1));
        }
        if released.len() >= BULK_REMOVAL_KEYS {
            arena::free_later(released);
        }
    }
}

#[cfg(test)]
mod block_entity_visibility_tests {
    use super::*;
    use crate::fast_hash::FxHashMap;

    /// Ordinary block entities follow eye-visible sections (vertical clamp + occlusion + frustum),
    /// while global renderers (`is_global`) and shadow-pass block entities stay on their own paths.
    #[test]
    fn ordinary_block_entities_follow_eye_visible_sections() {
        // Overworld min_y = -64, 24 sections: camera at Y = 111.0 (world sy = 6, 0-based y = 10),
        // rd = 2 -> visible vertical band is world sy 4..=8 (0-based y 8..=12, block Y 64..=143).
        let camera = DVec3::new(8.5, 111.0, 8.5);
        let min_y = -64;
        let sections = 24;
        let rd = 2;
        const CHEST: u32 = 1;
        const BEACON: u32 = 2;
        let outside_key = (0, 7, 0); // world sy = 3, block Y 48..64
        let edge_key = (0, 8, 0); // world sy = 4, block Y 64..80
        let mut map: FxHashMap<SectionKey, Vec<([i32; 3], u32)>> = FxHashMap::default();
        map.insert(outside_key, vec![([8, 60, 8], CHEST), ([9, 60, 8], BEACON)]);
        map.insert(edge_key, vec![([8, 64, 8], CHEST)]);

        let proj = world_projection(70.0, 16.0 / 9.0, 192.0);
        let view = crate::camera::view_matrix(0.0, 90.0);
        let frustum = JoinedFrusta::from(Frustum::from_planes(frustum(proj * view)));

        let mut o = occlusion::Occlusion::new(min_y, sections);
        for x in -2..=2 {
            for z in -2..=2 {
                o.load_column(x, z);
            }
        }
        o.settle(camera, rd, true);

        let mut kept: Vec<([i32; 3], u32)> = filter_block_entities_in_range(
            map.iter().map(|(k, v)| (k, v.as_slice())),
            camera,
            min_y,
            sections,
            Some(frustum),
            rd,
            |k| o.contains(k),
            |_| false,
            |state| state == BEACON,
        )
        .map(|entry| *entry.block)
        .collect();
        kept.sort_unstable();
        assert_eq!(
            kept,
            vec![([8, 64, 8], CHEST), ([9, 60, 8], BEACON)],
            "chest at Y=60 excluded, chest at Y=64 included, global beacon at Y=60 kept"
        );

        let mut mod_flags: Vec<([i32; 3], u32, bool)> = visible_block_entities_in_eye_view(
            map.iter().map(|(k, v)| (k, v.as_slice())),
            camera,
            min_y,
            sections,
            Some(frustum),
            rd,
            |k| o.contains(k),
        )
        .map(|(&(pos, state), eye_visible)| (pos, state, eye_visible))
        .collect();
        mod_flags.sort_unstable();
        assert_eq!(
            mod_flags,
            vec![
                ([8, 60, 8], CHEST, false),
                ([8, 64, 8], CHEST, true),
                ([9, 60, 8], BEACON, false),
            ],
            "native mod block entity iterator shares the same eye-section filter"
        );

        // Shadow-pass block entities remain independent of eye vertical clamping.
        let shadow_kept: Vec<_> =
            block_entities_within_reach([outside_key, edge_key].into_iter(), &map, camera, min_y)
                .copied()
                .collect();
        assert!(shadow_kept.contains(&([8, 60, 8], CHEST)));
        assert!(shadow_kept.contains(&([8, 64, 8], CHEST)));
    }
}
