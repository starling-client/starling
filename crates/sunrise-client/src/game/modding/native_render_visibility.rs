//! States a client mod hides from the mesher (`native_block_render_visibility`): ore
//! scanners, builder cutaways, cinematic shots. The answer is per state and resolved here, so
//! meshing reads one table and never a mod.

use super::{BlockStateId, Game};
use std::sync::Arc;

impl Game<'_> {
    /// Asks `native_block_render_visibility` again for every state, and the fluid-face parts
    /// for every state and fluid (`resolve_part_tables`), and, when the answers differ from
    /// those in use, remeshes every loaded section with them. While a mod listens to
    /// `section_visibility`, rebuilds the player's level's meshes and prepares their visibility
    /// again. Call it when a decision changes and when the player joins, not every frame.
    pub(super) fn refresh_render_visibility(&mut self) {
        // With no section listener now or before, the section policy does not change.
        let listening = mod_api::__private::listened(mod_api::HookId::SectionVisibility);
        self.streaming
            .refresh_section_visibility(&mut self.world, listening);
        let asked = mod_api::__private::has_scoped_hook(mod_api::HookId::BlockRenderVisibility);
        let mut hidden: Arc<[bool]> = if asked {
            (0..self.pack.assets.registry.len())
                .map(|state| {
                    let mut hidden = false;
                    self.native_block_render_visibility(BlockStateId(state as u32), &mut hidden);
                    hidden
                })
                .collect()
        } else {
            Arc::from([])
        };
        // A table that hides nothing is no table at all.
        if !hidden.contains(&true) {
            hidden = Arc::from([]);
        }
        let part_tables = self.resolve_part_tables();
        if hidden == self.pack.context.hidden_states
            && part_tables == *self.pack.context.part_tables
        {
            return;
        }
        let old = &self.pack.context;
        self.pack.context = Arc::new(sunrise_client::streaming::WorkerContext {
            assets: old.assets.clone(),
            render: old.render.clone(),
            tables: old.tables.clone(),
            biomes: old.biomes.clone(),
            light: old.light.clone(),
            hidden_states: hidden,
            part_tables: Arc::new(part_tables),
        });
        let context = self.worker_context();
        self.streaming.reload_resources(&mut self.world, context);
    }
}
