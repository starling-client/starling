//! Isolated attachment stores for chunk-worker disk callbacks.
use crate::*;

#[path = "chunk_disk/data.rs"]
mod data;
pub use data::ChunkDiskData;

// no wire: identifies the representation of an in-process chunk disk job.
/// Which serialized chunk representation owns a disk job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChunkDiskKind {
    /// A completed chunk.
    Full,
    /// A chunk still in generation.
    Proto,
}

// no wire: identifies the phase of an in-process chunk disk job.
/// The boundary at which a disk job runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChunkDiskPhase {
    /// After decoding, before residency publication.
    Load,
    /// After capture, before encoding.
    Save,
}

// no wire: captured context for an in-process chunk disk callback.
/// Identity captured by the existing world and chunk residency owner.
/// Borrow it from [`ChunkDiskHost::job`]; it grants no live lookup or write authority.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ChunkDiskContext {
    /// The existing world's lifetime, never a second chunk generation.
    pub world: WorldEpoch,
    /// The owning dimension's resource identifier.
    pub dimension: Identifier,
    /// Chunk coordinates, in chunks.
    pub pos: [i32; 2],
    /// Full or proto disk representation.
    pub kind: ChunkDiskKind,
    /// Load or save callback boundary.
    pub phase: ChunkDiskPhase,
}

impl ChunkDiskContext {
    /// Capture the existing world identity and the disk record's coordinates.
    pub fn new(
        world: WorldEpoch,
        dimension: Identifier,
        pos: [i32; 2],
        kind: ChunkDiskKind,
        phase: ChunkDiskPhase,
    ) -> Self {
        Self {
            world,
            dimension,
            pos,
            kind,
            phase,
        }
    }
}

// no wire: local refusal of an in-process chunk disk job.
/// Why a chunk disk job refused, with the failing mod and attachment when known.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ChunkDiskFailure {
    /// The listener's owner; empty for input validation before dispatch.
    pub mod_id: String,
    /// The attachment key, or empty for a failure outside a typed operation.
    pub key: String,
    /// The failed operation or missing capability.
    pub reason: String,
}

/// A disk callback's shared Host. It has no live backend and is never a
/// `ServerHost`: live handle methods refuse, and world writes do not compile.
/// Registry and settings reads use the frozen resource generation retained by
/// the disk worker. Neither getter reads files. The inherited
/// [`ConfigHost::setting`](prelude::ConfigHost::setting) still returns an owned `Option<String>`.
pub trait ChunkDiskHost: Host {
    /// Borrow the captured world, dimension, chunk, representation and phase.
    fn job(&self) -> &ChunkDiskContext;
    /// Borrows registry names in ID order for the lifetime of the host borrow,
    /// or returns `None` if the registry is unavailable in the captured generation.
    fn registry(&self, name: &str) -> Option<&[String]>;
}

// no wire: a borrowed worker-only Host.
/// Engine-side capability lasting only for one callback dispatch.
// hidden: engine-side worker capability, built only by the persistence owner
#[doc(hidden)]
#[non_exhaustive]
pub struct OwningChunkDisk<'a> {
    job: &'a ChunkDiskContext,
    resources: &'a dyn WorldgenHost,
}

impl<'a> OwningChunkDisk<'a> {
    /// Bind the captured identity to immutable resource-generation reads.
    pub fn new(job: &'a ChunkDiskContext, resources: &'a dyn WorldgenHost) -> Self {
        Self { job, resources }
    }
}

impl Host for OwningChunkDisk<'_> {}
crate::impl_host_areas!(OwningChunkDisk<'_>, except(levels, config));
impl prelude::LevelsHost for OwningChunkDisk<'_> {
    fn lit_block_name(&self, _: [i32; 3]) -> Option<crate::BlockKey> {
        None
    }
    fn is_loaded(&self, id: &str) -> bool {
        // Installed membership, as `LevelsHost` promises, not the startup-enabled `mods()` view.
        crate::__private::loaded_mods().get(id).is_some()
    }
    fn chunk(&self, _: LevelRef, _: [i32; 2]) -> Option<ChunkRef> {
        None
    }
}
impl prelude::ConfigHost for OwningChunkDisk<'_> {
    fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        self.resources.setting(mod_id, key).map(str::to_owned)
    }
}
impl ChunkDiskHost for OwningChunkDisk<'_> {
    fn job(&self) -> &ChunkDiskContext {
        self.job
    }
    fn registry(&self, name: &str) -> Option<&[String]> {
        self.resources.registry(name)
    }
}

/// The worker's load entry point, discovered by the hook catalog generator.
// hidden: worker entry point found by the hook catalog generator
#[doc(hidden)]
#[inline(never)]
pub fn native_chunk_data_load(context: &OwningChunkDisk<'_>, data: &mut ChunkDiskData) {
    crate::dispatch_chunk_data_load(context, data);
}

/// The worker's save entry point, discovered by the hook catalog generator.
// hidden: worker entry point found by the hook catalog generator
#[doc(hidden)]
#[inline(never)]
pub fn native_chunk_data_save(context: &OwningChunkDisk<'_>, data: &mut ChunkDiskData) {
    crate::dispatch_chunk_data_save(context, data);
}

#[path = "chunk_disk/job.rs"]
mod job;
// hidden: engine-owned save job, built and retained by the persistence owner
#[doc(hidden)]
pub use job::ChunkDiskJob;

#[cfg(test)]
#[path = "chunk_disk/tests.rs"]
mod tests;

/// Build the job refusal recorded when a chunk-disk listener unwinds through
/// [`crate::runtime::invoke`].
pub(crate) fn unwound_failure(mod_id: &'static str) -> ChunkDiskFailure {
    ChunkDiskFailure {
        mod_id: mod_id.into(),
        key: String::new(),
        reason: "native listener unwound".into(),
    }
}
