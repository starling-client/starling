//! Captured disk job and its once-only, recoverable completion.
use super::*;
use std::sync::OnceLock;

// no wire: engine-owned recovery snapshot and once-only owner dispatch.
/// One captured disk record. The dirty mark was consumed before capture; the
/// persistence owner retains this while the save attempt is pending.
// hidden: engine-owned save job, built and retained by the persistence owner
#[doc(hidden)]
#[non_exhaustive]
pub struct ChunkDiskJob {
    context: ChunkDiskContext,
    captured: Compound,
    completed: OnceLock<Compound>,
}

impl ChunkDiskJob {
    /// Capture encoded state only when the phase has a subscriber. The empty
    /// route does not clone the identity or store, allocate, or invoke a codec.
    pub fn capture(
        context: &ChunkDiskContext,
        store: &Attachments,
    ) -> Result<Option<Self>, AttachmentEncodeError> {
        let hook = match context.phase {
            ChunkDiskPhase::Load => HookId::ChunkDataLoad,
            ChunkDiskPhase::Save => HookId::ChunkDataSave,
        };
        if !crate::listened(hook) {
            return Ok(None);
        }
        Ok(Some(Self {
            context: context.clone(),
            captured: store.try_to_nbt()?,
            completed: OnceLock::new(),
        }))
    }

    /// Run on the owning server thread. Repeated inspection or recovery
    /// reads borrow the cached outcome and cannot invoke listeners again.
    pub fn run(
        &self,
        resources: &dyn WorldgenHost,
        upgrade_nbt: fn(NbtDataType, i32, Compound) -> Result<Compound, NbtUpgradeError>,
    ) -> &Compound {
        self.completed.get_or_init(|| {
            let mut data = ChunkDiskData::from_nbt(self.captured.clone());
            let context = OwningChunkDisk::new(&self.context, resources, upgrade_nbt);
            match self.context.phase {
                ChunkDiskPhase::Load => native_chunk_data_load(&context, &mut data),
                ChunkDiskPhase::Save => native_chunk_data_save(&context, &mut data),
            }
            data.finish()
        })
    }

    /// The captured envelopes, before any listener ran.
    pub fn captured(&self) -> &Compound {
        &self.captured
    }

    /// Inspect completion without invoking a listener.
    pub fn completed(&self) -> Option<&Compound> {
        self.completed.get()
    }
}
