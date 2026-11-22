//! What the renderer holds, for status, soak tests and memory reports (`sunrise-ctl memory`,
//! `scene_replay --memory`): resident sections and far tiles, and the bytes of its largest
//! GPU resources and of its per-section bookkeeping.

use std::{collections::HashMap, mem::size_of, ops::Deref};

use super::{Renderer, SectionKey, arena, flat, lod, texture_bytes, visbuffer};
use crate::fast_hash::FxHashMap;

type SectionBlock = ([i32; 3], u32);
type SectionBlocks = FxHashMap<SectionKey, Vec<SectionBlock>>;

/// Block-entity or light-source lists, with their total capacity maintained as
/// meshes come and go. Only read access to the backing map is exposed.
#[derive(Default)]
pub(crate) struct SectionLists {
    lists: SectionBlocks,
    entries: usize,
}

impl SectionLists {
    /// Replace the list and report whether the membership or state changed.
    pub(super) fn update(&mut self, key: SectionKey, blocks: &[SectionBlock]) -> bool {
        if blocks.is_empty() {
            return self.remove(&key);
        }
        let list = blocks.to_vec();
        self.entries += list.capacity();
        let old = self.lists.insert(key, list);
        self.entries -= old.as_ref().map_or(0, Vec::capacity);
        old.as_deref() != Some(blocks)
    }

    pub(super) fn remove(&mut self, key: &SectionKey) -> bool {
        let Some(old) = self.lists.remove(key) else {
            return false;
        };
        self.entries -= old.capacity();
        true
    }

    pub(super) fn remove_position(&mut self, key: SectionKey, pos: [i32; 3]) {
        if let Some(entries) = self.lists.get_mut(&key) {
            // Retain does not change the backing allocation, even when empty.
            entries.retain(|(at, _)| *at != pos);
        }
    }

    pub(super) fn retain(&mut self, mut keep: impl FnMut(&SectionKey) -> bool) {
        self.lists.retain(|key, blocks| {
            let kept = keep(key);
            if !kept {
                self.entries -= blocks.capacity();
            }
            kept
        });
    }

    pub(super) fn clear(&mut self) {
        self.lists.clear();
        self.entries = 0;
    }

    fn heap_bytes(&self) -> usize {
        self.lists.capacity() * (size_of::<SectionKey>() + size_of::<Vec<SectionBlock>>() + 1)
            + self.entries * size_of::<SectionBlock>()
    }
}

impl Deref for SectionLists {
    type Target = SectionBlocks;

    fn deref(&self) -> &Self::Target {
        &self.lists
    }
}

/// An owned renderer sample for a native capture worker. The vertex and CPU
/// counts are from the requesting frame; the backend allocator is queried
/// when the worker asks for its report, not when the snapshot is taken.
pub struct RendererMemorySnapshot {
    device: wgpu::Device,
    vertices: [u64; 2],
    cpu: [(&'static str, usize); 4],
}

#[cfg(not(target_arch = "wasm32"))]
const _: fn() = || {
    fn assert_send<T: Send>() {}
    assert_send::<RendererMemorySnapshot>();
};

impl RendererMemorySnapshot {
    /// Vertex arena allocation and vertex bytes in the current near/far meshes.
    pub fn section_vertex_bytes(&self) -> [u64; 2] {
        self.vertices
    }

    /// Heap bytes of renderer per-section bookkeeping, by name.
    pub fn cpu_census(&self) -> [(&'static str, usize); 4] {
        self.cpu
    }

    /// Allocated and reserved device bytes, with allocation labels largest first.
    /// Runs the backend allocator census on the calling thread.
    pub fn gpu_memory_by_label(&self) -> Option<(u64, u64, Vec<(String, u64)>)> {
        allocator_memory(&self.device)
    }
}

fn allocator_memory(device: &wgpu::Device) -> Option<(u64, u64, Vec<(String, u64)>)> {
    let report = device.generate_allocator_report()?;
    let mut by_label: HashMap<String, u64> = HashMap::new();
    for a in report.allocations {
        *by_label.entry(a.name).or_default() += a.size;
    }
    let mut labels: Vec<_> = by_label.into_iter().collect();
    labels.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Some((
        report.total_allocated_bytes,
        report.total_reserved_bytes,
        labels,
    ))
}

impl Renderer {
    /// Copy only maintained counts and the device handle. This does not walk
    /// any residency store or ask the backend to enumerate GPU allocations.
    pub fn memory_snapshot(&self) -> RendererMemorySnapshot {
        RendererMemorySnapshot {
            device: self.device.clone(),
            vertices: self.section_vertex_bytes(),
            cpu: self.cpu_census(),
        }
    }

    /// Sections with GPU buffers, for soak tests watching residency.
    pub fn resident_sections(&self) -> usize {
        self.sections.len()
    }
    /// Quads of the far terrain tiles with GPU buffers.
    pub fn lod_quads(&self) -> u64 {
        self.lods
            .values()
            .map(|tile| u64::from(tile.mesh.solid + tile.mesh.translucent))
            .sum()
    }
    /// Tiles and quads of the far terrain tiles with GPU buffers, by level.
    pub fn lod_levels(&self) -> Vec<[u64; 2]> {
        let mut levels = Vec::new();
        for (key, tile) in &self.lods {
            let level = usize::from(key.level);
            if levels.len() <= level {
                levels.resize(level + 1, [0, 0]);
            }
            levels[level][0] += 1;
            levels[level][1] += u64::from(tile.mesh.solid + tile.mesh.translucent);
        }
        levels
    }
    /// Whether solid quads go to the arena in the compact layout
    /// (`packed_quads`), and whether far tiles' own records are on
    /// (`lod::packed::far_packing`).
    pub fn packed_layout(&self) -> [bool; 2] {
        [self.packed_quads, crate::lod::packed::far_packing()]
    }
    /// Bytes of the arena the resident chunk sections hold, and of the one far
    /// tiles hold: the same split `gpu_census` reports, for `status`, so the
    /// compact layouts can be seen working.
    pub fn vertex_bytes_split(&self) -> [u64; 2] {
        [self.sections.vertex_bytes(), self.lod_vertex_bytes]
    }
    /// Bytes of the section vertex arenas' buffers, and of the vertices held
    /// by resident sections and current far tiles.
    pub fn section_vertex_bytes(&self) -> [u64; 2] {
        let [near, far] = self.vertex_bytes_split();
        let translucent = self.translucent_arena.as_ref();
        let arenas = self.arena.bytes() + translucent.map_or(0, arena::Arena::bytes);
        [arenas, near + far]
    }
    /// Bytes of the renderer's largest GPU resources, by name (`scene_replay --memory`):
    /// the terrain arenas (their pages, and what resident sections and far tiles use of
    /// them), the quad indices, the block sprites with their mips, the LabPBR maps, flat
    /// draws' buffers and the visibility buffer's.
    pub fn gpu_census(&self) -> Vec<(&'static str, u64)> {
        let [near, far] = self.vertex_bytes_split();
        let textures = |t: &[wgpu::Texture]| t.iter().map(texture_bytes).sum::<u64>();
        let translucent = self.translucent_arena.as_ref();
        vec![
            ("section_arena_pages", self.arena.bytes()),
            (
                "translucent_arena_pages",
                translucent.map_or(0, arena::Arena::bytes),
            ),
            ("sorted_index_pages", self.sorted_indices.bytes()),
            ("near_section_vertices", near),
            ("far_tile_vertices", far),
            ("quad_indices", self.indices.size()),
            ("block_sprites", textures(&self.sprite_textures)),
            (
                "pbr_maps",
                self.pbr_maps
                    .as_ref()
                    .map_or(0, |maps| maps.iter().flatten().map(texture_bytes).sum()),
            ),
            (
                "flat_draws",
                self.flat.as_ref().map_or(0, flat::Flat::bytes),
            ),
            (
                "visibility_buffer",
                self.visbuf.as_ref().map_or(0, visbuffer::Visbuffer::bytes),
            ),
        ]
    }
    /// Device memory the backend's allocator holds, by the label of what holds
    /// it, largest first, with its allocated and reserved totals (Vulkan and
    /// DirectX 12; none on Metal and OpenGL) (`sunrise-ctl memory`).
    pub fn gpu_memory_by_label(&self) -> Option<(u64, u64, Vec<(String, u64)>)> {
        allocator_memory(&self.device)
    }
    /// Heap bytes of the renderer's per-section bookkeeping (`sunrise-ctl memory`):
    /// resident sections, far tiles, block entities and light sources by section.
    pub fn cpu_census(&self) -> [(&'static str, usize); 4] {
        [
            ("resident_sections", self.sections.heap_bytes()),
            (
                "far_tiles",
                self.lods.capacity()
                    * (size_of::<crate::lod::TileKey>() + size_of::<lod::Tile>() + 1),
            ),
            ("block_entities", self.block_entities.heap_bytes()),
            ("light_sources", self.light_blocks.heap_bytes()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_list_census_tracks_capacity_through_mesh_changes() {
        let mut lists = SectionLists::default();
        let a = (0, 0, 0);
        let b = (1, 0, 0);
        let c = (2, 0, 0);
        let first = ([0, 0, 0], 1);
        let second = ([1, 0, 0], 2);
        let check = |lists: &SectionLists, slots| {
            assert_eq!(
                lists.heap_bytes(),
                lists.capacity() * (size_of::<SectionKey>() + size_of::<Vec<SectionBlock>>() + 1)
                    + slots * size_of::<SectionBlock>()
            );
        };

        assert!(lists.update(a, &[first, second]));
        assert!(!lists.update(a, &[first, second]));
        assert!(lists.update(b, &[second]));
        check(&lists, 3);
        lists.remove_position(b, second.0);
        check(&lists, 3);
        assert!(lists.update(a, &[first]));
        assert!(lists.update(c, &[first, second]));
        check(&lists, 4);
        lists.retain(|key| *key != c);
        check(&lists, 2);
        assert!(lists.update(b, &[]));
        assert!(!lists.remove(&b));
        check(&lists, 1);
        lists.clear();
        check(&lists, 0);
    }
}
