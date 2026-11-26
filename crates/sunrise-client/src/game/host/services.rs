//! The game's implementations of the mod API's service traits.
use crate::game::{Game, api_values::TryIntoEngine, native_console::Output};
use core::ops::ControlFlow;
use mod_api::{__private::game::CallbackOwner, *};
use std::sync::{Arc, mpsc::Sender};
use sunrise_client::{
    lan::guest::Guest,
    server::{user_cache::UserCache, user_lists::NameAndId},
};
use sunrise_client_render::mod_pipelines::BuiltinRenderType;

impl ClientSession for Game<'_> {
    fn client_server(&self) -> Option<ClientServer<'_>> {
        CallbackOwner::call_host(|| self.native_client_server())
    }
    fn client_language(&self) -> Option<Arc<str>> {
        CallbackOwner::call_host(|| self.native_client_language())
    }
    fn client_latency(&self) -> Option<u32> {
        CallbackOwner::call_host(|| self.native_client_latency())
    }
    fn client_tab_list(&self) -> Result<Option<&dyn ClientTabList>, ClientSessionReadError> {
        Ok(CallbackOwner::call_host(|| {
            self.native_session().map(|_| self as &dyn ClientTabList)
        }))
    }
    fn client_waypoints(&self) -> Option<ClientWaypoints> {
        CallbackOwner::call_host(|| self.native_client_waypoints())
    }
    fn client_player_skins(&self) -> Option<ClientPlayerSkins> {
        CallbackOwner::call_host(|| self.native_client_player_skins())
    }
    fn client_fishing_hook(&self) -> Option<ClientFishingHook> {
        CallbackOwner::call_host(|| self.native_client_fishing_hook())
    }
    fn client_frame_stats(&self) -> Option<ClientFrameStats> {
        CallbackOwner::call_host(|| self.native_client_frame_stats())
    }
    fn client_titles(&self) -> Result<Option<ClientTitles>, ClientSessionReadError> {
        CallbackOwner::call_host(|| self.native_client_titles())
    }
    fn client_boss_bars(&self) -> Result<Option<Vec<ClientBossBarEntry>>, ClientSessionReadError> {
        CallbackOwner::call_host(|| self.native_client_boss_bars())
    }
}
impl Commands for Game<'_> {
    fn refresh_commands(&mut self) {
        CallbackOwner::call_host(|| self.native_refresh_commands())
    }
    fn feedback(
        &mut self,
        line: Text,
        broadcast_to_admins: bool,
    ) -> Result<(), mod_api::text::TextFormError> {
        let line = sunrise_client_lib::api_text::to_serialized_engine(&line)?;
        CallbackOwner::call_host(|| self.native_feedback(line, broadcast_to_admins));
        Ok(())
    }
    fn failure(&mut self, line: Text) -> Result<(), mod_api::text::TextFormError> {
        self.native_failure(sunrise_client_lib::api_text::to_serialized_engine(&line)?);
        Ok(())
    }
    fn run_command(&mut self, line: &str) -> bool {
        CallbackOwner::call_host(|| self.native_run_command(line))
    }
    fn run_command_with_output(&mut self, line: &str) -> mod_api::CommandResult {
        CallbackOwner::call_host(|| self.native_run_command_with_output(line))
    }
    fn run_command_with_feedback(
        &mut self,
        line: &str,
        output: Sender<Result<CommandFeedback, TextFormError>>,
    ) -> bool {
        CallbackOwner::call_host(|| {
            self.run_console_command(line, Some(Output::Stream(output)))
                .success
        })
    }
}
impl Levels for Game<'_> {
    fn visit_levels(
        &self,
        visit: &mut dyn FnMut(LevelRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, ServerReadError> {
        // The caller's visitor runs inside its own callback scope.
        Ok(self.native_visit_levels(visit))
    }
}
impl Messages for Game<'_> {
    fn broadcast(&self, line: Text) -> Result<(), mod_api::text::TextFormError> {
        let line = sunrise_client_lib::api_text::to_serialized_engine(&line)?;
        CallbackOwner::call_host(|| self.native_broadcast(line));
        Ok(())
    }
}
impl Players for Game<'_> {
    fn server_players(&self) -> Vec<PlayerRef> {
        CallbackOwner::call_host(|| self.native_server_players())
    }
    fn player_by_name(&self, name: &str) -> Option<PlayerRef> {
        CallbackOwner::call_host(|| self.native_player_by_name(name))
    }
    fn profile_by_uuid(&self, uuid: u128) -> Option<GameProfile<'_>> {
        self.lookup_profile(|player| player.uuid == uuid, |cache| cache.find_by_id(uuid))
    }
    fn profile_by_name(&self, name: &str) -> Option<GameProfile<'_>> {
        self.lookup_profile(
            |player| player.name.eq_ignore_ascii_case(name),
            |cache| cache.find_by_name(name),
        )
    }
    fn profile_permission_level(&self, uuid: u128, name: &str) -> PermissionLevel {
        if let Some(lists) = &self.server_lists {
            let lists = lists.lock().unwrap_or_else(|e| e.into_inner());
            return lists
                .ops
                .permission_level(uuid)
                .map_or(PermissionLevel::All, PermissionLevel::from_level);
        }
        let Some(host) = &self.lan else {
            return PermissionLevel::All;
        };
        PermissionLevel::from_level(i32::from(host.hub.default_permission(uuid, name)))
    }
    fn acting_player(&self) -> Option<PlayerRef> {
        CallbackOwner::call_host(|| self.native_acting_player())
    }
    fn singleplayer_profile(&self) -> Option<GameProfile<'_>> {
        self.native_singleplayer_profile()
    }
}
impl Profiles for Game<'_> {
    fn resolve_name(&self, name: &str) -> ProfileResolution {
        self.native_resolve_profile(sunrise_client::skins::NameOrId::Name(name.to_owned()))
    }
    fn resolve_id(&self, uuid: u128) -> ProfileResolution {
        self.native_resolve_profile(sunrise_client::skins::NameOrId::Id(uuid))
    }
}
impl Game<'_> {
    /// Borrow from the online records before consulting the offline cache.
    fn lookup_profile(
        &self,
        matches: impl Fn(&Guest) -> bool,
        cached: impl FnOnce(&UserCache) -> Option<&NameAndId>,
    ) -> Option<GameProfile<'_>> {
        if let Some(online) = self
            .lan
            .iter()
            .flat_map(|host| host.hub.online_players())
            .find(|player| matches(player))
        {
            return Some(mod_api::__private::game::game_profile(
                online.uuid,
                &online.name,
                &online.properties,
            ));
        }
        let cached = cached(self.user_cache.as_ref()?)?;
        Some(mod_api::__private::game::game_profile(
            cached.id,
            &cached.name,
            &[],
        ))
    }
}
impl RuntimeLevels for Game<'_> {
    fn open(&mut self, id: &str, level: RuntimeLevel) -> Result<LevelRef, RuntimeLevelError> {
        CallbackOwner::call_host(|| self.runtime_level_open(id, level))
    }
    fn unload(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        CallbackOwner::call_host(|| self.runtime_level_unload(id))
    }
    fn delete(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        CallbackOwner::call_host(|| self.runtime_level_delete(id))
    }
    fn is_open(&mut self, id: &str) -> bool {
        CallbackOwner::call_host(|| self.runtime_level_is_open(id))
    }
}
impl RenderAssets for Game<'_> {
    fn model(&self, id: &Identifier) -> Result<ModelRef, ModelError> {
        CallbackOwner::call_host(|| self.native_model(id))
    }
    fn block_state_models(&self, state: BlockState) -> Result<BakedModelKeys<'_>, ModelError> {
        let state = state
            .try_into_engine(&())
            .map_err(|_| ModelError::UnknownModel)?;
        Ok(CallbackOwner::call_host(|| {
            self.pack.render.block_state_models(state)
        }))
    }
    fn block_state_model(&self, state: BlockState) -> Result<ModelRef, ModelError> {
        let state = state
            .try_into_engine(&())
            .map_err(|_| ModelError::UnknownModel)?;
        CallbackOwner::call_host(|| self.native_block_state_model(state))
    }
    fn baked_model(&self, key: BakedModelKey) -> Result<BakedModelView<'_>, ModelError> {
        CallbackOwner::call_host(|| self.pack.render.baked_model(key))
    }
    fn atlas_sprite_pixels(
        &self,
        sprite: SpriteRef,
    ) -> Result<AtlasSpritePixels, AtlasSpriteError> {
        CallbackOwner::call_host(|| self.native_atlas_sprite_pixels(sprite))
    }
    fn render_pipeline(&self, id: &Identifier) -> Result<RenderPipelineRef, RenderPipelineError> {
        CallbackOwner::call_host(|| self.pack.render.render_pipeline(id))
    }
    fn render_type(&self, id: &Identifier) -> Result<RenderTypeRef, RenderTypeError> {
        CallbackOwner::call_host(|| self.pack.render.render_type(id))
    }
    fn entity_solid(&self, texture: &Identifier) -> Result<RenderTypeRef, RenderTypeError> {
        CallbackOwner::call_host(|| {
            self.pack
                .render
                .textured_render_type(BuiltinRenderType::EntitySolid, texture)
        })
    }
    fn entity_cutout(&self, texture: &Identifier) -> Result<RenderTypeRef, RenderTypeError> {
        CallbackOwner::call_host(|| {
            self.pack
                .render
                .textured_render_type(BuiltinRenderType::EntityCutout, texture)
        })
    }
    fn entity_translucent(&self, texture: &Identifier) -> Result<RenderTypeRef, RenderTypeError> {
        CallbackOwner::call_host(|| {
            self.pack
                .render
                .textured_render_type(BuiltinRenderType::EntityTranslucent, texture)
        })
    }
    fn compute_pipeline(
        &self,
        id: &Identifier,
    ) -> Result<ComputePipelineRef, ComputePipelineError> {
        CallbackOwner::call_host(|| self.pack.render.compute_pipeline(id))
    }
    fn pipeline_modifier(
        &self,
        id: &Identifier,
    ) -> Result<PipelineModifierRef, RenderPipelineError> {
        CallbackOwner::call_host(|| self.pack.render.pipeline_modifier(id))
    }
    fn animation(&self, id: &Identifier) -> Option<AnimationHolder> {
        CallbackOwner::call_host(|| self.pack.render.animation(id))
    }
    fn shader_pack_block_id(&self, state: BlockState) -> Option<i32> {
        let state = state.try_into_engine(&()).ok()?;
        CallbackOwner::call_host(|| self.active_block_state_id(state.0 as usize))
    }
    fn pipeline_pack_bound(&self, pipeline: RenderPipelineRef) -> bool {
        CallbackOwner::call_host(|| {
            self.pack
                .render
                .mod_pipelines
                .slot(pipeline)
                .is_ok_and(|slot| self.pack_slot_bound(slot as usize))
        })
    }
}
