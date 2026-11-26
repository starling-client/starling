//! Client block-change tracking, connected-texture groups and narration.

pub mod block_changed {
    use mod_api::{ClientBlockChangedEvent, ClientHost, Hooks};

    #[derive(Default)]
    struct ChunkCache;

    impl ChunkCache {
        fn stale(&self, _dimension: &str, _chunk: [i32; 2]) {}
    }

    #[mod_api::sunrise_mod]
    impl Hooks for ChunkCache {
        // ANCHOR: client_block_changed
        fn client_block_changed(
            &self,
            _host: &dyn ClientHost,
            event: &mut ClientBlockChangedEvent<'_>,
        ) {
            // The chunk to redraw: [x, z], in chunks.
            self.stale(event.dimension, [event.pos.x >> 4, event.pos.z >> 4]);
        }
        // ANCHOR_END: client_block_changed
    }
}

pub mod break_progress {
    use mod_api::{ClientBreakProgressEvent, ClientHost, Hooks};
    use std::sync::atomic::{AtomicI32, Ordering};

    /// The crack stage last shown on the block the player mines, -1 for none.
    #[derive(Default)]
    struct MiningHelper {
        stage: AtomicI32,
    }

    #[mod_api::sunrise_mod]
    impl Hooks for MiningHelper {
        // ANCHOR: client_break_progress
        fn client_break_progress(
            &self,
            host: &dyn ClientHost,
            event: &mut ClientBreakProgressEvent<'_>,
        ) {
            let own = host
                .local_player()
                .is_some_and(|player| u32::try_from(event.breaker) == Ok(player.entity().id()));
            if own {
                // 0 to 9 is a crack stage; any other value means the crack went.
                let stage = if (0..10).contains(&event.progress) {
                    event.progress
                } else {
                    -1
                };
                self.stage.store(stage, Ordering::Relaxed);
            }
        }
        // ANCHOR_END: client_break_progress
    }
}

pub mod connected {
    use mod_api::{LevelRef, LevelWriteError, ServerHost};

    pub fn assign_group(
        level: LevelRef,
        host: &mut dyn ServerHost,
        pos: [i32; 3],
        tank_controller_id: u64,
    ) -> Result<bool, LevelWriteError> {
        // ANCHOR: connection_group
        let changed = level.set_connection_group(host, pos, Some(tank_controller_id))?;
        // ANCHOR_END: connection_group
        Ok(changed)
    }
}

pub mod narration {
    // ANCHOR: target_changed
    use mod_api::{ClientHost, NarrateError, Narration, Text};

    pub fn target_changed(host: &mut dyn ClientHost, name: &Text) {
        match host.narrate(name, Narration::Now) {
            Ok(()) => {}
            // The player turned narration off, or chose Chat only: nothing to do.
            Err(NarrateError::Off) => {}
            Err(other) => log::debug!("not narrated: {other}"),
        }
    }
    // ANCHOR_END: target_changed
}
