//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The server host's methods of the `services` area, implemented by the game and reached through [`ServerHost`].
#[sunrise_mod_api_macros::host_area]
pub trait ServicesServerHost {
    /// The commands service.
    #[service]
    fn commands(&mut self) -> &mut dyn Commands;
    /// The levels service.
    #[service]
    fn levels(&self) -> &dyn Levels;
    /// The messages service.
    #[service]
    fn messages(&self) -> &dyn Messages;
    /// The players service.
    #[service]
    fn players(&self) -> &dyn Players;
    /// The runtime_levels service.
    #[service]
    fn runtime_levels(&mut self) -> &mut dyn RuntimeLevels;
}
/// The client host's methods of the `services` area, implemented by the game and reached through [`crate::ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait ServicesClientHost {
    /// The calling mod's handle for submitting work to the client thread from any thread.
    /// Obtain it in `client_setup` and clone it into workers that cannot borrow a host.
    /// Returns `None` when the client loop is unavailable or closing, on restricted hosts
    /// such as the client-side `entity_entering_section` callback, or after a task from
    /// the calling mod panicked in this client session.
    fn client_executor(&self) -> Option<ClientExecutor>;
    /// Persistent model groups.
    #[service]
    fn instance_groups(&self) -> &dyn InstanceGroups;
    /// The client_session service.
    #[service]
    fn client_session(&self) -> &dyn ClientSession;
    /// The render_assets service.
    #[service]
    fn render_assets(&self) -> &dyn RenderAssets;
}
/// The client_session service, reached through its host.
pub trait ClientSession {
    /// Side: client.
    /// The connected server's borrowed text, not a stale reconnect target. None outside
    /// a logged-in client world. External display names are not retained; memory
    /// connections have no address. Friend-invite secrets are redacted.
    fn client_server(&self) -> Option<ClientServer<'_>> {
        unimplemented!("ClientSession::client_server")
    }
    /// Side: client.
    /// A shared snapshot of the applied language code, not a pending options selection.
    /// None outside a logged-in client world. The snapshot can outlive the host borrow
    /// and does not change when a different language is applied. Read `ClientHost::is_paused`
    /// for the applied pause state.
    fn client_language(&self) -> Option<Arc<str>> {
        unimplemented!("ClientSession::client_language")
    }
    /// Side: client.
    /// The local player's latency in milliseconds as the tab list draws it
    /// (`PlayerInfo.getLatency`): the server's smoothed keep-alive average, refreshed by its own
    /// broadcast. 0 before the server's first measurement, as vanilla; None outside a logged-in
    /// client world or before the server lists this player. Sends nothing.
    fn client_latency(&self) -> Option<u32> {
        unimplemented!("ClientSession::client_latency")
    }
    /// Side: client.
    /// The player list as the Tab overlay draws it: each listed player's drawn name, profile
    /// name, UUID, game mode and latency in drawn order, at most the 80 it draws, with the
    /// header and footer. What this client has been sent; None outside a logged-in client
    /// world. Sends nothing.
    fn client_tab_list(&self) -> Option<ClientTabList> {
        unimplemented!("ClientSession::client_tab_list")
    }
    /// Side: client.
    /// Existing local frame counters; None outside a logged-in client world.
    /// This read collects no history and sends nothing anywhere.
    fn client_frame_stats(&self) -> Option<ClientFrameStats> {
        unimplemented!("ClientSession::client_frame_stats")
    }
}
/// The commands service, reached through its host.
pub trait Commands {
    /// Side: server.
    /// A mod changed the commands it registers (`register_commands`):
    /// at the start of the next server tick every mod is asked again, the
    /// server dispatcher and its executors are replaced, and every player on the server
    /// is sent their command tree again, after `player_commands_sent`. The tree contains
    /// no executors, and parses from the previous dispatcher are no longer executable. Nothing
    /// happens where no world is open to players, and nothing for a server
    /// whose mods never call it. API @API:refresh_commands@.
    fn refresh_commands(&mut self) {
        unimplemented!("Commands::refresh_commands")
    }
    /// Side: server.
    /// An executor can send success feedback to the command's originating source, using
    /// vanilla's silence, send-command-feedback and block/cart track-output rules.
    /// This sends a message; the executor's return value determines the command result.
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn feedback(&mut self, line: Text) -> Result<(), TextFormError> {
        unimplemented!("Commands::feedback")
    }
    /// Side: server.
    /// Runs `line` (no slash) as the server's console does: permission
    /// level 4, from the world spawn in its level, with no output. A teleport to
    /// another level (`execute in minecraft:the_nether run tp Alex 0 70 0`)
    /// happens at the next tick's start, as the console's. Whether it ran
    /// without failing. During a guest's turn, live players (including the
    /// acting player) are reachable; displaced outer players are excluded
    /// from name, UUID and selector resolution on this path only.
    fn run_command(&mut self, line: &str) -> bool {
        unimplemented!("Commands::run_command")
    }
    /// Side: server.
    /// `run_command` with its feedback kept: the same line, source and run
    /// path (one leading slash is dropped here), answering with whether it
    /// ran without failing (`success`, what `run_command` returns) and what the
    /// console would have printed for it as plain text (`feedback`, as a mod
    /// that answers a remote console wants; Paper's `Server.dispatchCommand`
    /// with a capturing `CommandSender`). The feedback of the command alone is
    /// kept, never the lines other code logs meanwhile; a call made while
    /// another one runs captures its own lines and the outer capture goes on
    /// unchanged. At most 256 lines of 4096 bytes each
    /// come back (a longer line is cut at a character boundary). A command that ran and failed
    /// is `success: false` with its message in `feedback`. A line run where no server
    /// is open to players, or empty once its whitespace and slash are
    /// dropped, runs nothing and is `success: false` with no feedback, exactly as
    /// `run_command` answers `false` for it; no refusal type tells those
    /// apart from a failed command, as `run_command` has none either. The
    /// dedicated console's own commands (`stop`, `op`, `whitelist`, `ban`...)
    /// are not reached: they answer as an unknown command.
    fn run_command_with_output(&mut self, line: &str) -> CommandResult {
        unimplemented!("Commands::run_command_with_output")
    }
}
/// The levels service, reached through its host.
pub trait Levels {
    /// Side: server.
    /// Loaded server levels, in vanilla dimension order. Empty without a server world.
    fn list_levels(&self) -> Vec<LevelRef> {
        unimplemented!("Levels::list_levels")
    }
}
/// The messages service, reached through its host.
pub trait Messages {
    /// Side: server.
    /// A system line in every player's chat and the server's log.
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn broadcast(&self, line: Text) -> Result<(), TextFormError> {
        unimplemented!("Messages::broadcast")
    }
}
/// The players service, reached through its host.
pub trait Players {
    /// Side: server.
    /// The real players on this server, in join order, including an integrated
    /// server's own player even when the world is not open to LAN. Empty on a client.
    fn server_players(&self) -> Vec<PlayerRef> {
        unimplemented!("Players::server_players")
    }
    /// Side: server.
    /// Find a connected player by their account name, case-insensitively,
    /// including an integrated server's own player.
    fn player_by_name(&self, name: &str) -> Option<PlayerRef> {
        unimplemented!("Players::player_by_name")
    }
    /// Side: server.
    /// The player whose action or command a hook hears, as their state is
    /// swapped in (a command executor, `right_click_block`,
    /// `native_use_on`, `break_block`...): None outside such a
    /// hook.
    fn acting_player(&self) -> Option<PlayerRef> {
        unimplemented!("Players::acting_player")
    }
}
/// The runtime_levels service, reached through its host.
/// Success from `open` or `unload` admits a queued request, not its completion.
/// Apply failures are logged with the requesting mod and level id. A failed open
/// leaves the id unpublished and free to retry; a failed unload keeps it open for retry.
pub trait RuntimeLevels {
    /// Side: server.
    /// Open the level `id` (`mymod:instance/ab`), in the calling mod's namespace, as `level`
    /// describes. The request is checked now and applied after this server tick's levels have
    /// ticked, never inside a level's tick: the level is not open, and `is_open` says false, until
    /// then. The handle names this open of the level and never a later one. A folder that
    /// exists under the id is reopened as it was saved. `level_load` fires when it opens.
    /// Refused with [`RuntimeLevelError`] for an id outside the mod's namespace, one already
    /// open or waiting, an unknown dimension type or generator, a noise generator, and when
    /// the server's cap of open runtime levels (`sunrise-max-runtime-levels`, 64) is reached.
    fn open(&mut self, id: &str, level: RuntimeLevel) -> Result<LevelRef, RuntimeLevelError> {
        unimplemented!("RuntimeLevels::open")
    }
    /// Side: server.
    /// Save and close the level `id` the calling mod opened, keeping its folder, once this server
    /// tick's levels have ticked: `level_unload` fires then. Refused with
    /// [`RuntimeLevelError::PlayersInside`] while a player or an arrival not yet landed is in the
    /// level, now and again when it is applied: the mod moves its players out first
    /// (`entity_teleport_to_level`).
    fn unload(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        unimplemented!("RuntimeLevels::unload")
    }
    /// Side: server.
    /// Remove the level `id` the calling mod owns, folder and all, once this server tick's levels
    /// have ticked. A level that is open is saved and closed first (`level_unload` fires), as by
    /// `unload`; a level that is not open, such as the leftovers of a crash, only has its files
    /// removed, and a level with no files is nothing to delete. The structure references the
    /// game keeps beside the world for the level go with its folder, unless another id would
    /// share the file (`a_b:c` and `a:b_c` do) and that level exists. Links under `dimensions` or
    /// `data` are never followed: the removal fails and is logged. Refused, now and again when
    /// it is applied, with [`RuntimeLevelError::PlayersInside`] while a player or an arrival not
    /// yet landed is in the level, with [`RuntimeLevelError::LevelsBelow`] while a level below
    /// the id exists (`mymod:instance/ab` below `mymod:instance`: the folders nest), and with
    /// [`RuntimeLevelError::AlreadyOpen`] for a level still waiting to open. An `open` of a level
    /// that is open and being deleted is `AlreadyOpen` until the delete is applied; one of a
    /// level that was not open, made after the `delete`, is applied after it and starts empty.
    /// If the level closed but its files could not all be removed, it stays closed and
    /// `is_open` is false, `open` of the id is [`RuntimeLevelError::Removing`] and an `open`
    /// already queued is dropped with a log line, until a second `delete` has removed them.
    fn delete(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        unimplemented!("RuntimeLevels::delete")
    }
    /// Side: server.
    /// Whether the level `id` is open: false until a queued `open` is applied, true until a
    /// queued `unload` or `delete` is. Takes `&mut self` as the service's other methods do: a frame
    /// resource is reached one way.
    fn is_open(&mut self, id: &str) -> bool {
        unimplemented!("RuntimeLevels::is_open")
    }
}
/// The render_assets service, reached through its host.
pub trait RenderAssets {
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// Resolve a registered standalone model once in `client_setup` and again in
    /// `client_resources_reloaded`, not each frame. Resource-pack overrides and other
    /// namespaces use the normal shared model store; no resource is read or baked here.
    fn model(&self, id: &Identifier) -> Result<ModelRef, ModelError> {
        unimplemented!("RenderAssets::model")
    }
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// Resolve a pipeline registered in `register_render_pipelines` once in `client_setup` and
    /// again in `client_resources_reloaded`, not each frame. `Unknown` for an id no mod
    /// registered, `Invalid` for one whose shader the current resources don't give or that
    /// didn't compile. Nothing is read or compiled here.
    fn render_pipeline(&self, id: &Identifier) -> Result<RenderPipelineRef, RenderPipelineError> {
        unimplemented!("RenderAssets::render_pipeline")
    }
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// The keyframe animation `id`, from `assets/<namespace>/neoforge/animations/entity/<path>.json`
    /// in the mod's or a pack's resources (NeoForge's `Model.getAnimation`), as the latest resource
    /// load read it; `None` when no file gives it or the file failed to load. Get and
    /// [`AnimationHolder::bake`] it once in `client_setup` and again in
    /// `client_resources_reloaded`, not each frame.
    fn animation(&self, id: &Identifier) -> Option<AnimationHolder> {
        unimplemented!("RenderAssets::animation")
    }
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// The model of block state `state` as vanilla draws a block state outside the terrain (a
    /// moving piston block, a falling block, a block display): all the parts its blockstate file
    /// selects for its properties, weighted alternatives picked at the fixed seed
    /// `BlockEntityDrawContext::block` uses, tint layers from the block's tint sources without a
    /// position. Any state: a mod's, vanilla's, a resource pack's. It draws through
    /// [`WorldDrawContext::draw_model_instances`](crate::WorldDrawContext::draw_model_instances),
    /// [`InstanceGroupRef`] and `BlockEntityDrawContext::terrain_model`. `ModelError::UnknownModel` for a
    /// state with no baked model (air, a block drawn by a special renderer) or one the live
    /// block-state index lacks (a block it does not hold, or an ordinal past the block's states).
    /// Resolve once per distinct state in `client_setup` and again in `client_resources_reloaded`,
    /// not each frame; the handle goes stale on a reload.
    fn block_state_model(&self, state: BlockState) -> Result<ModelRef, ModelError> {
        unimplemented!("RenderAssets::block_state_model")
    }
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// Keys of every baked model `state` can draw in the current published generation, in bake
    /// order without repeats: each weighted alternative and each multipart part its properties
    /// select, whichever one a position's seed would pick. Borrow the list from this generation;
    /// a valid state with no baked binding has no keys. A state the live block-state index lacks
    /// (a block it does not hold, or an ordinal past the block's states) returns
    /// `ModelError::UnknownModel`. Retain the keys until the next `model_baking_completed`.
    fn block_state_models(&self, state: BlockState) -> Result<BakedModelKeys<'_>, ModelError> {
        unimplemented!("RenderAssets::block_state_models")
    }
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// Borrow the baked model `key` names: its geometry and sprite names, before draw-time
    /// tints, lighting or material selection. `ModelError::StaleModel` once a successful resource
    /// reload supersedes the key's generation; `ModelError::UnknownModel` for a slot the store
    /// lacks. Retain the key and borrow the model when reading its quads, without copying them.
    fn baked_model(&self, key: BakedModelKey) -> Result<BakedModelView<'_>, ModelError> {
        unimplemented!("RenderAssets::baked_model")
    }
}

/// Client-side persistent copies of resolved models.
pub trait InstanceGroups {
    /// Allocate up to 16,384 copies, initially length zero. Each mod may reserve 65,536
    /// copies total (an empty group reserves one slot). Only visible copies in the drawn
    /// length, not capacity, consume the view budget. At most 16,384 groups per view.
    /// Create at setup/reload, not each frame. Visible GPU-routed copies upload each drawn frame.
    /// Reload, world close, disabling the owning mod, or explicit destroy retires the group.
    /// Re-enabling a mod never revives its old handles; recreate groups after publication.
    fn create_group(&self, model: ModelRef, capacity: u32) -> Result<InstanceGroupRef, GroupError>;
}
