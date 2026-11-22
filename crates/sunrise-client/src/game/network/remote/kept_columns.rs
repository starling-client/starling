use super::*;

impl Game<'_> {
    /// `ClientChunkCache.Storage.inRange`: within the storage around the
    /// view centre, `calculateStorageRange` (the server's radius, at least
    /// two, plus three) chunks each way.
    pub(in crate::game) fn remote_chunk_in_range(&self, x: i32, z: i32) -> bool {
        if self.replay.playing() {
            return true;
        }
        self.remote
            .as_ref()
            .is_none_or(|r| in_storage_range(r.view_distance, r.chunk_center, x, z))
    }

    fn chunk_inside_remote_range(center: (i32, i32), distance: i32, x: i32, z: i32) -> bool {
        in_storage_range(distance, center, x, z)
    }

    /// The columns a moved or resized storage no longer holds. Returns the
    /// positions it freed (`forget_remote_columns`).
    pub(in crate::game) fn drop_remote_chunks_out_of_range(
        &mut self,
        center: (i32, i32),
        distance: i32,
    ) -> Vec<(i32, i32)> {
        let mut gone: sunrise_client::fast_hash::FxHashSet<(i32, i32)> = self
            .world
            .chunks
            .keys()
            .copied()
            .filter(|&(x, z)| !Self::chunk_inside_remote_range(center, distance, x, z))
            .collect();
        self.forget_remote_columns(&mut gone, true)
    }

    /// Every column goes with the client level. Returns the positions it
    /// freed (`forget_remote_columns`).
    pub(in crate::game) fn forget_all_remote_columns(&mut self) -> Vec<(i32, i32)> {
        let mut gone = self.world.chunks.keys().copied().collect();
        self.forget_remote_columns(&mut gone, false)
    }

    /// Forgets the `gone` columns (and empties the set): their chunks (freed on a thread
    /// of their own when there are many), their sections in the renderer and their block
    /// entities, each store once for all of them. When the level stays open,
    /// far terrain keeps their last state (`server_terrain`). Returns the
    /// positions it freed, in `gone`'s order: only those still resident,
    /// so a later forget of the same positions frees nothing.
    pub(in crate::game) fn forget_remote_columns(
        &mut self,
        gone: &mut sunrise_client::fast_hash::FxHashSet<(i32, i32)>,
        keep_terrain: bool,
    ) -> Vec<(i32, i32)> {
        if gone.is_empty() {
            return Vec::new();
        }
        // The client chunk seam: one unload per column still resident, with
        // its data still there, before `take_remote_columns` frees it.
        let freed_at: Vec<(i32, i32)> = gone
            .iter()
            .filter(|&&(x, z)| self.world.chunks.contains_key(&(x, z)))
            .copied()
            .collect();
        for &(x, z) in &freed_at {
            self.client_chunk_unload(x, z, keep_terrain);
        }
        let freed =
            sunrise_client::streaming::Streaming::take_remote_columns(&mut self.world, gone);
        self.streaming.forget_columns(&mut self.world, gone);
        if keep_terrain && let Some(lod) = &mut self.lod {
            for chunk in &freed {
                lod.observe(chunk.clone(), self.res.facts());
            }
        }
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.remove_columns(gone.iter().copied());
        }
        sunrise_client::chunk::free_columns(freed, gone.len());
        sunrise_client::spans::time("unload_block_entities", || {
            for &column in gone.iter() {
                self.world.block_entities.unload_column(column);
            }
        });
        if let Some(r) = self.remote.as_mut() {
            r.attachments.retain(|holder, _| match *holder {
                Holder::Chunk([x, z]) => !gone.contains(&(x, z)),
                Holder::BlockEntity([x, _, z]) => !gone.contains(&(x >> 4, z >> 4)),
                _ => true,
            });
        }
        gone.clear();
        freed_at
    }
}
