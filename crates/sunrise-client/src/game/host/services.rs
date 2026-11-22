//! The game's implementations of the mod API's service traits.
use crate::game::{Game, api_values::TryIntoEngine};
use mod_api::*;
use std::sync::Arc;

impl ClientSession for Game<'_> {
    fn client_server(&self) -> Option<ClientServer<'_>> {
        self.native_client_server()
    }
    fn client_language(&self) -> Option<Arc<str>> {
        self.native_client_language()
    }
    fn client_latency(&self) -> Option<u32> {
        self.native_client_latency()
    }
    fn client_tab_list(&self) -> Option<ClientTabList> {
        self.native_client_tab_list()
    }
    fn client_frame_stats(&self) -> Option<ClientFrameStats> {
        self.native_client_frame_stats()
    }
}
impl Commands for Game<'_> {
    fn refresh_commands(&mut self) {
        self.native_refresh_commands()
    }
    fn feedback(&mut self, line: Text) -> Result<(), mod_api::text::TextFormError> {
        self.native_feedback(sunrise_client_lib::api_text::to_serialized_engine(&line)?);
        Ok(())
    }
    fn run_command(&mut self, line: &str) -> bool {
        self.native_run_command(line)
    }
    fn run_command_with_output(&mut self, line: &str) -> mod_api::CommandResult {
        self.native_run_command_with_output(line)
    }
}
impl Levels for Game<'_> {
    fn list_levels(&self) -> Vec<LevelRef> {
        self.native_list_levels()
    }
}
impl Messages for Game<'_> {
    fn broadcast(&self, line: Text) -> Result<(), mod_api::text::TextFormError> {
        self.native_broadcast(sunrise_client_lib::api_text::to_serialized_engine(&line)?);
        Ok(())
    }
}
impl Players for Game<'_> {
    fn server_players(&self) -> Vec<PlayerRef> {
        self.native_server_players()
    }
    fn player_by_name(&self, name: &str) -> Option<PlayerRef> {
        self.native_player_by_name(name)
    }
    fn acting_player(&self) -> Option<PlayerRef> {
        self.native_acting_player()
    }
}
impl RuntimeLevels for Game<'_> {
    fn open(&mut self, id: &str, level: RuntimeLevel) -> Result<LevelRef, RuntimeLevelError> {
        self.runtime_level_open(id, level)
    }
    fn unload(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        self.runtime_level_unload(id)
    }
    fn delete(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        self.runtime_level_delete(id)
    }
    fn is_open(&mut self, id: &str) -> bool {
        self.runtime_level_is_open(id)
    }
}
impl RenderAssets for Game<'_> {
    fn model(&self, id: &Identifier) -> Result<ModelRef, ModelError> {
        self.native_model(id)
    }
    fn block_state_models(&self, state: BlockState) -> Result<BakedModelKeys<'_>, ModelError> {
        let state = state
            .try_into_engine(&())
            .map_err(|_| ModelError::UnknownModel)?;
        Ok(self.pack.render.block_state_models(state))
    }
    fn block_state_model(&self, state: BlockState) -> Result<ModelRef, ModelError> {
        let state = state
            .try_into_engine(&())
            .map_err(|_| ModelError::UnknownModel)?;
        self.native_block_state_model(state)
    }
    fn baked_model(&self, key: BakedModelKey) -> Result<BakedModelView<'_>, ModelError> {
        self.pack.render.baked_model(key)
    }
    fn render_pipeline(&self, id: &Identifier) -> Result<RenderPipelineRef, RenderPipelineError> {
        self.pack.render.render_pipeline(id)
    }
    fn animation(&self, id: &Identifier) -> Option<AnimationHolder> {
        self.pack.render.animation(id)
    }
}
