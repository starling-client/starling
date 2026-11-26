//! `light_emission`, the light a `dynamic_light` block emits by where it is
//! (`IBlockExtension.getLightEmission`): asked of its parts on the thread that owns the
//! level as a chunk holding one comes in, as an edit or `level_check_light` reaches one, and
//! on a client as the server's light update reaches its section, the answers kept in the
//! chunk (`Chunk::dynamic_emission`) for the light engine, which never asks on its own.
use super::*;
use sunrise_client::fast_hash::FxHashMap;

impl Game<'_> {
    /// `ServerChunkCache.getChunk`: drive an authoritative FULL read on the level owner.
    /// Receive columns, prepare their contextual emission, then schedule lighting.
    pub(super) fn load_full_column(&mut self, column: (i32, i32)) -> bool {
        self.load_full_chunk(None, column).is_ok()
    }

    /// The chunks the workers brought into this level, asked before a light job reads them.
    /// Leave their load notifications for the normal owner update.
    pub(super) fn ask_loaded_light_emission(&mut self, first: usize) {
        if !self.pack.context.light.any_dynamic() {
            return;
        }
        for i in first..self.world.loaded_chunks.len() {
            let (x, z, _) = self.world.loaded_chunks[i];
            let key = (x, z);
            if self
                .world
                .chunks
                .get(&key)
                .is_some_and(|chunk| chunk.dynamic_pending)
            {
                self.ask_chunk_light_emission(key);
            }
        }
    }
    /// Every `dynamic_light` cell of the resident chunk `key` asked, its answers replacing
    /// the chunk's, and the chunk let through to the light jobs. A server's column is asked
    /// as it is taken in, before any light job can read it.
    pub(super) fn ask_chunk_light_emission(&mut self, key: (i32, i32)) {
        let dimension = self.current_light_emission_dimension();
        self.ask_level_chunk_light_emission(None, dimension, key);
    }
    /// [`Self::ask_chunk_light_emission`] in the level in place (`parked: None`) or in the
    /// parked level whose dimension is `parked`, its cells asked as `dimension`'s.
    pub(super) fn ask_level_chunk_light_emission(
        &mut self,
        parked: Option<&str>,
        dimension: LevelDimension,
        key: (i32, i32),
    ) {
        let world = match parked {
            None => &self.world,
            Some(id) => match self.levels.parked_world_of(id) {
                Some(world) => world,
                None => return,
            },
        };
        let Some(chunk) = world.chunks.get(&key) else {
            return;
        };
        let pending = chunk.dynamic_pending;
        let cells = self.pack.context.light.dynamic_cells(chunk, |_| true);
        if cells.is_empty() && !pending {
            return;
        }
        let mut answers = FxHashMap::default();
        for (cell, [x, y, z], state) in cells {
            if let Some(emission) = self.ask_light_emission(dimension, x, y, z, state) {
                answers.insert(cell, emission);
            }
        }
        let world = match parked {
            None => &mut self.world,
            Some(id) => match self.levels.parked_world_mut_of(id) {
                Some(world) => world,
                None => return,
            },
        };
        if let Some(chunk) = world.chunks.get_mut(&key)
            && (pending || chunk.dynamic_emission != answers)
        {
            // Saved light was not computed from these answers, and admission queues no light
            // job for it: relight the column when they differ.
            let relight = chunk.light_loaded && chunk.dynamic_emission != answers;
            let chunk = Arc::make_mut(chunk);
            chunk.dynamic_emission = answers;
            chunk.dynamic_pending = false;
            if relight {
                world.dirty_light.insert(key);
            }
        }
    }
    /// The cells this update's edits and light checks left in a `dynamic_light` state,
    /// asked before the light engine relights them (`Streaming::relight_edits`).
    /// `Chunk::set` already dropped the answers of the states they replaced.
    pub(super) fn ask_edited_light_emission(&mut self, dim: Dim) {
        let Some(world) = self.light_edit_world(dim) else {
            return;
        };
        // The checks queued so far are asked now: a later one at the same cell queues again.
        world.light_checks.clear();
        let count = world.light_edits.len();
        if count == 0 || !self.pack.context.light.any_dynamic() {
            return;
        }
        let dimension = if dim == self.dim {
            self.current_light_emission_dimension()
        } else {
            dim.level()
        };
        for i in 0..count {
            let Some(world) = self.light_edit_world(dim) else {
                return;
            };
            let ((x, y, z), _, _) = world.light_edits[i];
            let state = world.block(x, y, z).0;
            if !self.pack.context.light.is_dynamic(state) {
                continue;
            }
            let emission = self.ask_light_emission(dimension, x, y, z, state);
            let key = (x >> 4, z >> 4);
            if let Some(world) = self.light_edit_world(dim)
                && let Some(cell) = world.chunks.get(&key).and_then(|c| c.dynamic_cell(x, y, z))
            {
                Self::store_light_emission(world, key, cell, emission);
            }
        }
    }

    /// Access the same world before and after a read-only emission callback, without a level swap.
    fn light_edit_world(&mut self, dim: Dim) -> Option<&mut World> {
        if dim == self.dim {
            Some(&mut self.world)
        } else {
            self.levels.parked_mut(dim).map(|p| &mut p.world)
        }
    }
    /// A server's light update changed the light of `sections` of the column `key`: their
    /// `dynamic_light` cells asked again, each once, so this copy's own relights take what the
    /// server's took (the server asked before it relit them). A Sunrise addition: the contract
    /// reads emission again on no light packet, but a part's answer may read data the client
    /// receives later.
    pub(super) fn ask_updated_light_emission(&mut self, key: (i32, i32), sections: &[usize]) {
        if sections.is_empty() || !self.pack.context.light.any_dynamic() {
            return;
        }
        let Some(chunk) = self.world.chunks.get(&key) else {
            return;
        };
        let cells = self
            .pack
            .context
            .light
            .dynamic_cells(chunk, |s| sections.contains(&s));
        let dimension = self.current_light_emission_dimension();
        for (cell, [x, y, z], state) in cells {
            let emission = self.ask_light_emission(dimension, x, y, z, state);
            Self::store_light_emission(&mut self.world, key, cell, emission);
        }
    }
    /// One cell's answer stored in its chunk (`None` drops the one it had); the chunk is
    /// copied only when the answer changes.
    fn store_light_emission(world: &mut World, key: (i32, i32), cell: u32, emission: Option<u8>) {
        let Some(chunk) = world.chunks.get_mut(&key) else {
            return;
        };
        if chunk.dynamic_emission.get(&cell).copied() == emission {
            return;
        }
        let answers = &mut Arc::make_mut(chunk).dynamic_emission;
        match emission {
            Some(emission) => answers.insert(cell, emission),
            None => answers.remove(&cell),
        };
    }
    /// Use the dimension of the level in play, even for a client's custom dimension.
    fn current_light_emission_dimension(&self) -> LevelDimension {
        self.native_current_level()
            .map_or_else(|| self.dim.level(), Self::light_emission_dimension)
    }

    /// One cell's answer, clamped to a light level as `LightTables` clamps a state's.
    fn ask_light_emission(
        &self,
        dimension: LevelDimension,
        x: i32,
        y: i32,
        z: i32,
        state: u32,
    ) -> Option<u8> {
        self.query_dynamic_light_emission(BlockPos::new(x, y, z), BlockStateId(state), || dimension)
            .flatten()
            .map(|emission| emission.min(15))
    }
}
