//! `ChunkBatchSizeCalculator`: how many chunks a tick a client asks the
//! server for, from how long its last chunk batches took to arrive. The game's
//! network thread and the load bots time their batches with it.

use std::time::Instant;

/// The moving average of the nanoseconds a batch's chunks took each, its
/// weight and when the current batch started.
pub struct ChunkBatchSize {
    aggregated_nanos_per_chunk: f64,
    old_samples_weight: i32,
    batch_start: Instant,
}

impl Default for ChunkBatchSize {
    fn default() -> Self {
        Self {
            aggregated_nanos_per_chunk: 2_000_000.0,
            old_samples_weight: 1,
            batch_start: Instant::now(),
        }
    }
}

impl ChunkBatchSize {
    /// `onBatchStart`, on `ClientboundChunkBatchStartPacket`.
    pub fn on_batch_start(&mut self) {
        self.batch_start = Instant::now();
    }

    /// `onBatchFinished`, on `ClientboundChunkBatchFinishedPacket`: the batch's
    /// `batch_size` chunks took the time since its start.
    pub fn on_batch_finished(&mut self, batch_size: i32) {
        let nanos = self.batch_start.elapsed().as_nanos() as f64;
        self.record(batch_size, nanos);
    }

    /// The sample of one batch: each chunk's share of `batch_nanos`, kept
    /// within a third and three times the average, weighs in as one of up to
    /// 50.
    fn record(&mut self, batch_size: i32, batch_nanos: f64) {
        if batch_size > 0 {
            let nanos_per_chunk = batch_nanos / f64::from(batch_size);
            let average = self.aggregated_nanos_per_chunk;
            // Mth.clamp(double): below the minimum is the minimum, else at most the maximum.
            let clamped = if nanos_per_chunk < average / 3.0 {
                average / 3.0
            } else {
                nanos_per_chunk.min(average * 3.0)
            };
            self.aggregated_nanos_per_chunk = (average * f64::from(self.old_samples_weight)
                + clamped)
                / f64::from(self.old_samples_weight + 1);
            self.old_samples_weight = (self.old_samples_weight + 1).min(49);
        }
    }

    /// `getDesiredChunksPerTick`: what `ServerboundChunkBatchReceivedPacket`
    /// asks for.
    pub fn desired_chunks_per_tick(&self) -> f32 {
        (7_000_000.0 / self.aggregated_nanos_per_chunk) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::ChunkBatchSize;

    #[test]
    fn follows_the_batches_it_measured() {
        let mut size = ChunkBatchSize::default();
        // Before any batch: 7e6 / 2e6.
        assert_eq!(size.desired_chunks_per_tick(), 3.5);
        // 10 chunks in 10 ms, 1e6 each, within [2e6 / 3, 6e6]: (2e6 + 1e6) / 2 = 1.5e6.
        size.record(10, 10_000_000.0);
        assert_eq!(
            size.desired_chunks_per_tick(),
            (7_000_000.0_f64 / 1_500_000.0) as f32
        );
        // 64 chunks in 1 ms is held at a third of 1.5e6 = 5e5: (1.5e6 * 2 + 5e5) / 3.
        size.record(64, 1_000_000.0);
        assert_eq!(
            size.desired_chunks_per_tick(),
            (7_000_000.0_f64 / (3_500_000.0 / 3.0)) as f32
        );
        // An empty batch changes nothing.
        size.record(0, 50_000_000.0);
        assert_eq!(
            size.desired_chunks_per_tick(),
            (7_000_000.0_f64 / (3_500_000.0 / 3.0)) as f32
        );
    }
}
