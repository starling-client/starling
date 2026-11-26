//! Immutable, thread-safe chunk column snapshots (`ChunkSnapshot`, `SectionSnapshot`).
use crate::{BiomeKey, BlockState, Heightmap};
use std::fmt;
use std::sync::Arc;

/// Engine-side backing for an immutable [`ChunkSnapshot`].
// hidden: engine-only backend trait; mods interact with `ChunkSnapshot` and `SectionSnapshot`.
#[doc(hidden)]
pub trait ChunkSnapshotBackend: Send + Sync + 'static {
    fn pos(&self) -> [i32; 2];
    fn min_y(&self) -> i32;
    fn height(&self) -> i32;
    fn section_count(&self) -> usize;
    fn has_light(&self) -> bool;
    fn heightmap(&self, kind: Heightmap, lx: u8, lz: u8) -> Option<i32>;
    fn block_state(&self, lx: u8, y: i32, lz: u8) -> Option<BlockState>;
    fn light(&self, lx: u8, y: i32, lz: u8) -> Option<(u8, u8)>;
    fn biome(&self, lx: u8, y: i32, lz: u8) -> Option<u32>;
    fn biome_key(&self, biome_index: u32) -> Option<&BiomeKey>;
    fn section_y(&self, section_index: usize) -> i32;
    fn section_is_empty(&self, section_index: usize) -> bool;
    fn section_uniform_block(&self, section_index: usize) -> Option<BlockState>;
    fn section_uniform_light(&self, section_index: usize) -> Option<(u8, u8)>;
    fn section_uniform_biome(&self, section_index: usize) -> Option<u32>;
    fn section_block_state(
        &self,
        section_index: usize,
        lx: u8,
        ly: u8,
        lz: u8,
    ) -> Option<BlockState>;
    fn section_light(&self, section_index: usize, lx: u8, ly: u8, lz: u8) -> Option<(u8, u8)>;
    fn section_biome_quart(&self, section_index: usize, qx: u8, qy: u8, qz: u8) -> Option<u32>;
}

/// An immutable, thread-safe snapshot of a chunk column's blocks, 3D quart biomes, sky/block
/// light, and heightmaps.
///
/// `ChunkSnapshot` is `Send + Sync + 'static` and may be moved to background worker threads
/// without holding a [`Host`](crate::Host). Edits to the live chunk after capture never affect
/// the snapshot.
///
/// **What is excluded:** Block entities and entities are not included in a `ChunkSnapshot`.
///
/// **Sharing and memory cost:** Capturing a live chunk shares its underlying copy-on-write
/// section allocations (blocks, biomes, and light) with no array copies and no artificial count
/// cap. Holding a snapshot pins its sections in memory after the chunk unloads, and causes the
/// live chunk's next block, biome, or light edit to any still-shared section to copy that section
/// before mutating it.
///
/// **Tracking later updates:** A snapshot reflects the chunk at capture time and does not update
/// automatically. Mods keeping derived data in sync listen to `client_block_changed` /
/// `client_chunk_load` on the client or block-change hooks on the server and re-capture as needed.
// no wire: in-process immutable handle sharing reference-counted section allocations for workers.
// exhaustive: opaque handle with private backend field; constructed only by the engine.
#[derive(Clone)]
pub struct ChunkSnapshot {
    backend: Arc<dyn ChunkSnapshotBackend>,
}

impl fmt::Debug for ChunkSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChunkSnapshot")
            .field("pos", &self.pos())
            .field("min_y", &self.min_y())
            .field("height", &self.height())
            .field("has_light", &self.has_light())
            .finish()
    }
}

impl ChunkSnapshot {
    /// Wrap an engine-owned snapshot backend.
    // hidden: engine-only constructor; mods obtain snapshots from `ChunkRef::snapshot` and peers.
    #[doc(hidden)]
    pub fn from_backend(backend: Arc<dyn ChunkSnapshotBackend>) -> Self {
        Self { backend }
    }

    /// Chunk coordinates `[cx, cz]`.
    pub fn pos(&self) -> [i32; 2] {
        self.backend.pos()
    }

    /// Lowest block Y included in this snapshot (inclusive).
    pub fn min_y(&self) -> i32 {
        self.backend.min_y()
    }

    /// Total vertical block height of this snapshot (`max_y_exclusive = min_y + height`).
    pub fn height(&self) -> i32 {
        self.backend.height()
    }

    /// Number of 16x16x16 vertical sections in this snapshot.
    pub fn section_count(&self) -> usize {
        self.backend.section_count()
    }

    /// True if real sky/block lighting has been computed or loaded for this snapshot.
    /// False for unlit snapshots (such as pre-light detached generation stages).
    pub fn has_light(&self) -> bool {
        self.backend.has_light()
    }

    /// Borrows section `section_index` (`0..self.section_count()`, bottom up), or `None` if out
    /// of range.
    pub fn section(&self, section_index: usize) -> Option<SectionSnapshot<'_>> {
        (section_index < self.backend.section_count()).then_some(SectionSnapshot {
            backend: &*self.backend,
            index: section_index,
        })
    }

    /// Heightmap value at chunk-local block coordinates `(lx, lz)` (`0..16`), matching vanilla's
    /// `LevelReader.getHeight(Heightmap.Types, x, z)` and `Heightmap.getFirstAvailable(x, z)`
    /// (the first block Y above the highest matching block, or `min_y` if the column is empty).
    /// Returns `None` if `lx >= 16 || lz >= 16`.
    pub fn heightmap(&self, kind: Heightmap, lx: u8, lz: u8) -> Option<i32> {
        self.backend.heightmap(kind, lx, lz)
    }

    /// Block state at chunk-local block coordinates `(lx, y, lz)` (`lx, lz` in `0..16`, `y` in
    /// `min_y..min_y + height`). Returns `None` if out of bounds.
    pub fn block_state(&self, lx: u8, y: i32, lz: u8) -> Option<BlockState> {
        self.backend.block_state(lx, y, lz)
    }

    /// Packed light at chunk-local block coordinates `(lx, y, lz)`: `(sky_light, block_light)`,
    /// each `0..=15`. Returns `None` if out of bounds. Check [`Self::has_light`] before relying
    /// on light values for snapshots produced without a light pass.
    pub fn light(&self, lx: u8, y: i32, lz: u8) -> Option<(u8, u8)> {
        self.backend.light(lx, y, lz)
    }

    /// Registered biome stored at chunk-local block coordinates `(lx, y, lz)` (`lx, lz` in
    /// `0..16`, `y` in `min_y..min_y + height`), mapped to its 4x4x4 quart cell via vanilla's
    /// `>> 2` shift (`ChunkAccess.getNoiseBiome`, before biome-manager zoom). Returns `None` if
    /// out of bounds, or if the stored slot names no registered biome (a save placeholder for a
    /// biome the world's registry lacks).
    ///
    /// Equal keys name the same biome; parse a key once with [`BiomeKey::parse`] and compare it
    /// against each cell off-thread, with no [`Host`](crate::Host).
    pub fn biome(&self, lx: u8, y: i32, lz: u8) -> Option<&BiomeKey> {
        self.backend
            .biome(lx, y, lz)
            .and_then(|slot| self.backend.biome_key(slot))
    }
}

/// Borrowed view of one 16x16x16 vertical section inside a [`ChunkSnapshot`].
// no wire: borrowed section view into an in-process `ChunkSnapshot`.
// exhaustive: opaque borrowed view with private fields; constructed only via `ChunkSnapshot::section`.
#[derive(Clone, Copy)]
pub struct SectionSnapshot<'a> {
    backend: &'a dyn ChunkSnapshotBackend,
    index: usize,
}

impl fmt::Debug for SectionSnapshot<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SectionSnapshot")
            .field("section_y", &self.section_y())
            .field("is_empty", &self.is_empty())
            .finish()
    }
}

impl<'a> SectionSnapshot<'a> {
    /// Section Y coordinate (`SectionPos.y`, i.e. lowest block Y in the section `>> 4`).
    pub fn section_y(&self) -> i32 {
        self.backend.section_y(self.index)
    }

    /// True if every block in this 16x16x16 section is air (`LevelChunkSection.hasOnlyAir()`).
    pub fn is_empty(&self) -> bool {
        self.backend.section_is_empty(self.index)
    }

    /// Uniform block state if every block in this section has the same state (O(1) fast path).
    pub fn uniform_block(&self) -> Option<BlockState> {
        self.backend.section_uniform_block(self.index)
    }

    /// Uniform `(sky_light, block_light)` if light does not vary across this section.
    pub fn uniform_light(&self) -> Option<(u8, u8)> {
        self.backend.section_uniform_light(self.index)
    }

    /// Uniform registered biome if every 4x4x4 quart cell in this section has the same biome.
    /// `None` if the cells differ or the shared slot names no registered biome.
    pub fn uniform_biome(&self) -> Option<&'a BiomeKey> {
        self.backend
            .section_uniform_biome(self.index)
            .and_then(|slot| self.backend.biome_key(slot))
    }

    /// Block state at section-local block coordinates `(lx, ly, lz)` (`0..16` on each axis).
    /// Returns `None` if any coordinate is `>= 16`.
    pub fn block_state(&self, lx: u8, ly: u8, lz: u8) -> Option<BlockState> {
        self.backend.section_block_state(self.index, lx, ly, lz)
    }

    /// Packed `(sky_light, block_light)` at section-local block coordinates `(lx, ly, lz)`
    /// (`0..16` on each axis). Returns `None` if any coordinate is `>= 16`.
    pub fn light(&self, lx: u8, ly: u8, lz: u8) -> Option<(u8, u8)> {
        self.backend.section_light(self.index, lx, ly, lz)
    }

    /// Registered biome stored at section-local quart coordinates `(qx, qy, qz)` (`0..4` on each
    /// axis, 4 blocks per quart cell). Returns `None` if any coordinate is `>= 4`, or if the
    /// stored slot names no registered biome.
    pub fn biome_quart(&self, qx: u8, qy: u8, qz: u8) -> Option<&'a BiomeKey> {
        self.backend
            .section_biome_quart(self.index, qx, qy, qz)
            .and_then(|slot| self.backend.biome_key(slot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubBackend(BiomeKey);

    impl ChunkSnapshotBackend for StubBackend {
        fn pos(&self) -> [i32; 2] {
            [3, -5]
        }
        fn min_y(&self) -> i32 {
            -64
        }
        fn height(&self) -> i32 {
            32
        }
        fn section_count(&self) -> usize {
            2
        }
        fn has_light(&self) -> bool {
            false
        }
        fn heightmap(&self, _: Heightmap, lx: u8, lz: u8) -> Option<i32> {
            (lx < 16 && lz < 16).then_some(-64)
        }
        fn block_state(&self, _: u8, _: i32, _: u8) -> Option<BlockState> {
            None
        }
        fn light(&self, lx: u8, y: i32, lz: u8) -> Option<(u8, u8)> {
            (lx < 16 && lz < 16 && (-64..-32).contains(&y)).then_some((15, 0))
        }
        fn biome(&self, lx: u8, y: i32, lz: u8) -> Option<u32> {
            (lx < 16 && lz < 16 && (-64..-32).contains(&y)).then_some(7)
        }
        fn biome_key(&self, biome_index: u32) -> Option<&BiomeKey> {
            (biome_index == 7).then_some(&self.0)
        }
        fn section_y(&self, section_index: usize) -> i32 {
            -4 + section_index as i32
        }
        fn section_is_empty(&self, _: usize) -> bool {
            true
        }
        fn section_uniform_block(&self, _: usize) -> Option<BlockState> {
            None
        }
        fn section_uniform_light(&self, _: usize) -> Option<(u8, u8)> {
            Some((15, 0))
        }
        fn section_uniform_biome(&self, section_index: usize) -> Option<u32> {
            Some(if section_index == 0 { 8 } else { 7 })
        }
        fn section_block_state(&self, _: usize, _: u8, _: u8, _: u8) -> Option<BlockState> {
            None
        }
        fn section_light(&self, _: usize, lx: u8, ly: u8, lz: u8) -> Option<(u8, u8)> {
            (lx < 16 && ly < 16 && lz < 16).then_some((15, 0))
        }
        fn section_biome_quart(&self, _: usize, qx: u8, qy: u8, qz: u8) -> Option<u32> {
            (qx < 4 && qy < 4 && qz < 4).then_some(7)
        }
    }

    #[test]
    fn chunk_snapshot_is_send_sync_and_wraps_typed_biomes() {
        fn assert_send_sync_static<T: Send + Sync + 'static>() {}
        assert_send_sync_static::<ChunkSnapshot>();

        let desert = BiomeKey::parse("minecraft:desert").unwrap();
        let snap = ChunkSnapshot::from_backend(Arc::new(StubBackend(desert.clone())));
        assert_eq!(snap.pos(), [3, -5]);
        assert_eq!(snap.min_y(), -64);
        assert_eq!(snap.height(), 32);
        assert_eq!(snap.section_count(), 2);
        assert!(!snap.has_light());

        assert_eq!(snap.biome(0, -64, 0), Some(&desert));
        assert_eq!(snap.biome(0, -32, 0), None);
        // An unregistered slot (a save placeholder) names no biome.
        assert_eq!(snap.section(0).unwrap().uniform_biome(), None);

        let sec1 = snap.section(1).unwrap();
        assert_eq!(sec1.section_y(), -3);
        assert!(sec1.is_empty());
        assert_eq!(sec1.uniform_biome(), Some(&desert));
        assert_eq!(sec1.biome_quart(3, 3, 3), Some(&desert));
        assert_eq!(sec1.biome_quart(4, 0, 0), None);
        assert!(snap.section(2).is_none());
    }
}
