//! Far detail coarser than Full.
use super::*;

impl Lod {
    /// Far detail coarser than Full (`graphics::FarDetail::coarseness`: 1
    /// Reduced, 2 Mobile): the next update replans when it changes, and tiles
    /// sampled from then on approximate wide cells (`GeneratorWorker::lod_grid`).
    pub fn set_far_coarseness(&mut self, coarseness: u8) {
        if self.far_coarseness != coarseness {
            let keys: Vec<_> = self
                .ready
                .keys()
                .chain(self.pending.iter())
                .copied()
                .collect();
            for key in keys {
                self.mark_stale(key);
            }
        }
        self.far_coarseness = coarseness;
        self.shared
            .approximate
            .store(coarseness > 0, Ordering::Relaxed);
    }
}
