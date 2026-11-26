use super::*;
use mod_api::BlockEntityDataView;
use sunrise_client::containers::BlockEntity;
impl Game<'_> {
    pub(in crate::game) fn native_on_chunk_unloaded(
        &mut self,
        pos: [i32; 3],
        block_entity: BlockEntityRef,
        entity: BlockEntityDataView<'_>,
    ) {
        self.fire_on_chunk_unloaded(pos, block_entity, entity);
    }
}

impl Game<'_> {
    /// Chunk `chunk` of `dimension` left the world (NeoForge's
    /// `ChunkEvent.Unload`), including eviction of idle columns while the level
    /// remains open. Don't read its blocks. Its block entities are still there, but changes made in
    /// this hook aren't saved: as in vanilla, the chunk is saved before it unloads.
    /// Not told when the world closes, which `level_unload` and server shutdown
    /// (`server_stopping` / `server_stopped`) tell.
    /// Server-side.
    pub(in crate::game) fn native_chunk_unload(&mut self, dimension: &str, chunk: [i32; 2]) {
        self.fire_chunk_unload(dimension, chunk);
        if let Some(level) = self.level(dimension) {
            self.forget_composter_chunk(level.dimension(), chunk);
            self.unload_water_tickets_chunk(level, chunk);
        }
    }
    /// Chunk `chunk` of this server level `dimension` left it, after its last save: the
    /// parts of its mod block entities are told, then the mods of the chunk.
    pub(in crate::game) fn native_chunk_and_block_entities_unload(
        &mut self,
        dimension: &str,
        chunk: [i32; 2],
    ) {
        if mod_api::__private::has_part_hook(mod_api::HookId::OnChunkUnloaded)
            && let Some(level) = self.level(dimension)
        {
            self.notify_chunk_block_entities_unloaded(level, (chunk[0], chunk[1]));
        }
        self.native_chunk_unload(dimension, chunk);
    }
    /// Tells the parts of the mod block entities in chunk `chunk` of `level` that it
    /// unloads, each once, while they are still in the world's block entities.
    pub(in crate::game) fn notify_chunk_block_entities_unloaded(
        &mut self,
        level: mod_api::LevelRef,
        chunk: (i32, i32),
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::OnChunkUnloaded) {
            return;
        }
        // Keep the outgoing set and original generations across callbacks. The loans give the
        // mutable side Host no alias to these rows, and hand them back for normal column cleanup.
        let block_entities = &mut self.world.block_entities;
        let positions: Vec<_> = block_entities.mods_in(chunk).copied().collect();
        let unloading: Vec<_> = positions
            .into_iter()
            .filter_map(|key @ (x, y, z)| {
                let generation = block_entities.generation(&key)?;
                let (lease, entity) = block_entities.take(&key)?;
                let pos = [x, y, z];
                let handle = mod_api::__private::game::block_entity_ref(level, pos, generation);
                Some((pos, handle, lease, entity))
            })
            .collect();
        for (pos, handle, lease, row) in unloading {
            if let BlockEntity::Mod(entity) = &row {
                let data = ResidentReadBlockEntity::stored(entity);
                self.native_on_chunk_unloaded(pos, handle, BlockEntityDataView::new(&data));
            }
            self.world.block_entities.put(lease, row);
        }
    }
}
