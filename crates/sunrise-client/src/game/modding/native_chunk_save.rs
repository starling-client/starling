//! Server-level binding and save admission for chunk disk callbacks.
use super::*;

impl Game<'_> {
    /// Bind a newly opened level's save directory before publishing its streamer.
    /// `Persistent` and the server share this directory; only its submitted records
    /// become disk jobs.
    pub(super) fn bind_chunk_disk_worker(
        &self,
        save: &sunrise_client::save::SaveDir,
        dim: super::Dim,
    ) {
        if self.options.execution_role.is_client()
            || (!mod_api::__private::listened(mod_api::HookId::ChunkDataLoad)
                && !mod_api::__private::listened(mod_api::HookId::ChunkDataSave))
        {
            return;
        }
        if let (Some(world), Some(resources)) = (
            self.native_lifecycle.epoch(),
            sunrise_world::native_worldgen_hooks::context(),
        ) {
            save.set_chunk_disk_context(
                world,
                mod_api::Identifier::parse(dim.id()).expect("registered dimension"),
                resources,
            );
        }
    }
}

impl Game<'_> {
    /// Defer eager saves, streaming/unload and closing-save polling at their
    /// gameplay call sites while an action savepoint holds an entity pass open.
    /// With no save listener these call sites do not take this deferral path.
    pub(super) fn defer_chunk_disk_save(&mut self) -> bool {
        if self.options.execution_role.is_client()
            || !mod_api::__private::listened(mod_api::HookId::ChunkDataSave)
        {
            return false;
        }
        Self::admit_entity_save().is_err()
    }

    /// `ChunkMap.saveChunksEagerly`: snapshot only while the tick and writer
    /// have room. NBT encoding, compression and durable writes stay on workers.
    pub(super) fn save_chunks_eagerly(&mut self) {
        self.save_chunks_eagerly_at(Instant::now());
    }

    fn save_chunks_eagerly_at(&mut self, now: Instant) {
        // No world open (`world_root` is set by `finish_open_world`): the save
        // is `Game::new`'s placeholder under the working directory.
        if self.world.carrier_fences.is_broken()
            || self.remote.is_some()
            || self.world_root.as_os_str().is_empty()
            || self.agent.hold_server
            || self.clock.sprinting()
            || self
                .server_lists
                .as_ref()
                .is_some_and(|lists| !lists.lock().unwrap().autosave)
        {
            return;
        }
        if self.defer_chunk_disk_save() {
            return;
        }
        let deadline = self
            .server_tick_deadline
            .unwrap_or_else(|| Instant::now() + self.clock.tick_duration());
        let mut modified = std::mem::take(&mut self.world.modified);
        modified.save_eagerly(
            now,
            || Instant::now() < deadline && self.save.active_chunk_writes() < 128,
            // Only a lit column is a LevelChunk. An unlit one stays queued.
            |key| self.world.chunks.get(&key).map(|chunk| chunk.lit),
            |key, _| {
                let chunks = self.with_entities([&self.world.chunks[&key]]);
                match self
                    .save
                    .save_later(&chunks, self.pack.assets.registry, world_biomes!(self))
                {
                    Ok(_) => true,
                    Err(e) => {
                        log::warn!("Sunrise: failed to save changed chunks: {e}");
                        false
                    }
                }
            },
        );
        self.world.modified = modified;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::native_entity_refs::tests::with_game;
    use std::time::Duration;
    use sunrise_client::chunk::Chunk;
    use sunrise_client::save::{LevelShape, SaveDir};
    use sunrise_core::nbt::{DISK_WRITE_NESTING_GUARD, Tag};

    #[test]
    #[ignore = "daily"]
    fn eager_ticks_do_not_retry_a_refused_snapshot_without_an_edit() {
        let dir = tempfile::tempdir().unwrap();
        mod_platform::testing::with_mods(Vec::new().into(), || {
            with_game(|game| {
                game.world_root = dir.path().to_path_buf();
                game.save = Arc::new(SaveDir::new(
                    game.world_root.clone(),
                    LevelShape::OVERWORLD,
                    Some(Default::default()),
                ));
                let key = (2, 0);
                let mut chunk = Chunk::with_layout(key.0, key.1, game.world.layout());
                chunk.lit = true;
                let registry = game.pack.assets.registry;
                let stone = registry.default_state("minecraft:stone").unwrap();
                let gold = registry.default_state("minecraft:gold_block").unwrap();
                chunk.set(1, 64, 1, stone.0);
                game.save
                    .save(&[Arc::new(chunk.clone())], registry, world_biomes!(game))
                    .unwrap();
                chunk.set(1, 64, 1, gold.0);
                let invalid = (0..=DISK_WRITE_NESTING_GUARD)
                    .fold(Tag::Int(0), |nested, _| Tag::List(vec![nested]));
                Arc::make_mut(chunk.saved.get_or_insert_with(Default::default))
                    .root
                    .insert("test:invalid".into(), invalid);
                game.world.chunks.insert(key, Arc::new(chunk));
                game.world.modified.insert(key);

                let now = Instant::now();
                game.save_chunks_eagerly_at(now);
                game.save.flush().unwrap();
                assert_eq!(game.save.record_save_failures(), 1);

                // Advance beyond the ordinary cooldown and exercise all later save paths.
                game.save_chunks_eagerly_at(now + Duration::from_secs(11));
                game.save_game_with(false).unwrap();
                game.save_game().unwrap();
                assert_eq!(game.save.record_save_failures(), 1);
                assert_eq!(
                    game.save
                        .load(key.0, key.1, registry, world_biomes!(game))
                        .unwrap()
                        .block(1, 64, 1),
                    stone
                );

                let chunk = Arc::make_mut(game.world.chunks.get_mut(&key).unwrap());
                Arc::make_mut(chunk.saved.as_mut().unwrap())
                    .root
                    .remove(&"test:invalid".into());
                game.world.modified.insert(key);
                game.save_chunks_eagerly_at(now + Duration::from_secs(22));
                game.save.flush().unwrap();
                assert_eq!(
                    game.save
                        .load(key.0, key.1, registry, world_biomes!(game))
                        .unwrap()
                        .block(1, 64, 1),
                    gold
                );
            });
        });
    }
}
