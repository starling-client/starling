//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
use std::sync::mpsc::Sender;
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
    /// The profiles service.
    #[service]
    fn profiles(&self) -> &dyn Profiles;
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
    /// Ask the local client to quit, even from the title screen without a world open.
    /// The client uses the normal Quit Game path after the calling hook returns,
    /// disconnecting from any server and waiting for an integrated server to save
    /// and release its world lock before exiting. This is not a server operation.
    /// Returns `true` if the request was recorded, not if shutdown succeeded;
    /// returns `false` if there is no live client or a quit is pending or underway.
    #[decline_unavailable]
    fn request_client_quit(&self) -> bool;
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
    /// Borrow the live Tab overlay view, without copying rows or text. `Ok(None)` means
    /// no logged-in client world; an empty list in a logged-in world still has a view.
    /// An unavailable session service returns [`ClientSessionReadError::Unavailable`].
    /// Sends nothing.
    fn client_tab_list(&self) -> Result<Option<&dyn ClientTabList>, ClientSessionReadError> {
        Err(ClientSessionReadError::Unavailable)
    }
    /// Side: client.
    /// The locator bar's tracked waypoints: each waypoint's identifier, target, icon style and
    /// icon colour, farthest first by squared distance from the camera entity when one
    /// exists (ties in no promised order; no promised order without one). What this client
    /// has been sent; None outside a logged-in client world. Sends nothing.
    fn client_waypoints(&self) -> Option<ClientWaypoints> {
        unimplemented!("ClientSession::client_waypoints")
    }
    /// Side: client.
    /// The listed players' skins as the Tab overlay draws them: each player's skin texture path
    /// and whether it uses the slim model. What this client has been sent; None outside a
    /// logged-in client world. Sends nothing.
    fn client_player_skins(&self) -> Option<ClientPlayerSkins> {
        unimplemented!("ClientSession::client_player_skins")
    }
    /// The local player's fishing hook: its position and whether a fish is biting. What
    /// this client tracks; None outside a logged-in client world or when no hook is out.
    /// Sends nothing.
    fn client_fishing_hook(&self) -> Option<ClientFishingHook> {
        unimplemented!("ClientSession::client_fishing_hook")
    }
    /// Side: client.
    /// Existing local frame counters; None outside a logged-in client world.
    /// This read collects no history and sends nothing anywhere.
    fn client_frame_stats(&self) -> Option<ClientFrameStats> {
        unimplemented!("ClientSession::client_frame_stats")
    }
    /// Side: client.
    /// The title, subtitle and action bar on screen now. Each is None once its timer has run
    /// out; the subtitle shows only with a title. What this client has been sent; `Ok(None)`
    /// outside a logged-in client world. A text conversion error is returned for the failing
    /// read. No plaintext is substituted. Sends nothing.
    fn client_titles(&self) -> Result<Option<ClientTitles>, ClientSessionReadError> {
        Err(ClientSessionReadError::Unavailable)
    }
    /// Side: client.
    /// Every boss bar the client holds, in server order: each bar's name and
    /// progress (0 to 1, as drawn). The HUD draws only those that fit within
    /// `guiHeight / 3`. What this client has been sent; `Ok(None)` outside a
    /// logged-in client world. A text conversion error is returned for the failing read.
    /// No plaintext is substituted. Sends nothing.
    fn client_boss_bars(&self) -> Result<Option<Vec<ClientBossBarEntry>>, ClientSessionReadError> {
        Err(ClientSessionReadError::Unavailable)
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
    /// An executor's success line, as vanilla's `CommandSourceStack.sendSuccess(line,
    /// broadcast_to_admins)`. A silent source (`execute` run quietly, a function, a mod's
    /// suppressed run) sends nothing at all. Otherwise the line goes to the originating
    /// source when it takes success feedback: a player while `send_command_feedback` is on,
    /// the console and RCON always, a command block or minecart that tracks output while
    /// `send_command_feedback` is on; never to the `execute as` entity.
    ///
    /// With `broadcast_to_admins`, and when the origin informs operators (a player always,
    /// a block or minecart that tracks output while `command_block_output` is on, the
    /// console and RCON per `broadcast-console-to-ops` / `broadcast-rcon-to-ops`), the line
    /// is also wrapped as `[name: line]` (`chat.type.admin`, gray and italic, with the
    /// display name of the `execute as` entity, else the block's, minecart's or server's).
    /// That line goes to every operator (`PlayerList.isOp`, a level-0 operator too) but
    /// the source player while `send_command_feedback` is on, and to the server log while
    /// `log_admin_commands` is on unless the console is the source; neither rule gates
    /// the other. Pass `false` for a query or list, as vanilla does.
    ///
    /// The text is converted once. Delivering to no one is still `Ok(())`, and the
    /// executor's return value alone determines the command result.
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn feedback(&mut self, line: Text, broadcast_to_admins: bool) -> Result<(), TextFormError> {
        unimplemented!("Commands::feedback")
    }
    /// Side: server.
    /// An executor can send a failure line to the command's originating source, as vanilla's
    /// `CommandSourceStack.sendFailure` (the same call a NeoForge command registered in
    /// `RegisterCommandsEvent` makes on its source): the line in red, shown unless the source is
    /// silent or does not accept failures (a command block or minecart with track output off),
    /// and independent of `sendCommandFeedback`. A command block or minecart keeps it as its last
    /// output. It changes nothing else: the executor still returns the command's result, so
    /// Java's `source.sendFailure(text); return 0;` is `failure(text)` then `Ok(0)`: `execute
    /// store result` reads 0, without the `Err` path of a failed command (`execute store success`
    /// reads 1 and a command block counts a success, as for any vanilla command that returns 0).
    /// Refuses text deeper than 512 component nodes before
    /// applying any changes; a line whose red formatting no longer fits a packet or a saved
    /// command block is replaced by `command.failed`, as a returned failure is.
    fn failure(&mut self, line: Text) -> Result<(), TextFormError> {
        unimplemented!("Commands::failure")
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
    /// Runs a console command from the world spawn level (`MinecraftServer.createCommandSourceStack`)
    /// and returns a synchronous snapshot of every source message emitted before the call returns,
    /// as plain text in order and without truncation. Messages sent by delayed command listeners
    /// after dispatch returns (such as `/test` completion summaries) are not waited for or
    /// included; use [`Self::run_command_with_feedback`] to receive structured messages including
    /// delayed ones, and to tell success, failure and direct system messages apart. Both methods
    /// have the same execution result and source routing. A command can fail after already sending
    /// success feedback. Public `/say` and `/me` announcements are chat, not source feedback.
    fn run_command_with_output(&mut self, line: &str) -> CommandResult {
        unimplemented!("Commands::run_command_with_output")
    }
    /// Side: server.
    /// Runs `line` as the server console from the world spawn level
    /// (`MinecraftServer.createCommandSourceStack`), dropping one leading slash, and sends the
    /// command source's structured messages to `output` in order as they occur. Logs, public
    /// chat, chat to other recipients and operator broadcasts are not source feedback.
    /// Outgoing confirmations addressed to a non-player command source (such as `/msg`) are delivered as
    /// [`CommandFeedback::System`]. Nested calls have their own output. Output is not truncated;
    /// the receiver decides what to keep or display. A component that cannot be represented is
    /// sent as a `TextFormError` in its place. Dropping the receiver stops delivery but does not
    /// interrupt the command.
    ///
    /// The channel closes once the command returns and every delayed listener bound to that
    /// command source (such as a `/test` runner's summary displayer or `/fetchprofile`) finishes
    /// and drops its source. On the server thread, drain with `try_iter()` or `try_recv()` rather
    /// than blocking iteration so delayed commands can keep ticking; a background reader thread
    /// may block until disconnection.
    ///
    /// Returns whether the command ran without failing. A blank line or no open server runs
    /// nothing and returns `false` without sending a message. Commands handled only by the
    /// dedicated console (`stop`, `op`, `whitelist`, `ban`...) are unknown to this dispatcher.
    fn run_command_with_feedback(
        &mut self,
        line: &str,
        output: Sender<Result<CommandFeedback, TextFormError>>,
    ) -> bool {
        unimplemented!("Commands::run_command_with_feedback")
    }
}
/// The levels service, reached through its host.
pub trait Levels {
    /// Side: server.
    /// Visit loaded server levels in creation order: the Overworld first, then the other
    /// configured levels in registry traversal, then later opens. Return `Break(())` to stop.
    /// Without an open world, the visit completes without a call. An unavailable service returns
    /// `Unavailable`.
    fn visit_levels(
        &self,
        visit: &mut dyn FnMut(LevelRef) -> core::ops::ControlFlow<()>,
    ) -> Result<core::ops::ControlFlow<()>, ServerReadError> {
        let _ = visit;
        Err(ServerReadError::Unavailable)
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
    /// An online player's profile by UUID, else an unexpired user-cache entry without login
    /// properties. Does not contact the profile service or synthesize an offline identity.
    fn profile_by_uuid(&self, uuid: u128) -> Option<GameProfile<'_>> {
        unimplemented!("Players::profile_by_uuid")
    }
    /// Side: server.
    /// An online player's profile by account name (case-insensitive), else an unexpired
    /// user-cache entry without login properties. Does not contact the profile service or
    /// synthesize an offline identity.
    fn profile_by_name(&self, name: &str) -> Option<GameProfile<'_>> {
        unimplemented!("Players::profile_by_name")
    }
    /// Side: server.
    /// The permission set for a profile (`MinecraftServer.getProfilePermissions`), even before
    /// the player joins. Dedicated servers look up the UUID in the op list. Integrated servers
    /// check the owner's name case-insensitively and account for world and guest command access.
    /// An ordinary player has [`PermissionLevel::All`] (level 0).
    fn profile_permission_level(&self, uuid: u128, name: &str) -> PermissionLevel {
        unimplemented!("Players::profile_permission_level")
    }
    /// Side: server.
    /// The player whose action or command a hook hears, as their state is
    /// swapped in (a command executor, `right_click_block`,
    /// `native_use_on`, `break_block`...): None outside such a
    /// hook.
    fn acting_player(&self) -> Option<PlayerRef> {
        unimplemented!("Players::acting_player")
    }
    /// Side: server.
    /// `MinecraftServer.getSingleplayerProfile`: the profile an integrated server was created
    /// with, its owner's name, UUID and login properties, borrowed from the host. Fixed when
    /// the server is created, so every read returns the same profile, from the
    /// `server_about_to_start` and `server_starting` hooks on, before the owner joins and
    /// after they leave. None on a dedicated server and on a client.
    /// [`PlayerRef::is_singleplayer_owner`] matches a player to it by name.
    fn singleplayer_profile(&self) -> Option<GameProfile<'_>> {
        unimplemented!("Players::singleplayer_profile")
    }
}
/// The profiles service, reached through its host: account profiles by name or UUID through
/// the server's `ProfileResolver.Cached`, the one `/fetchprofile` and dynamic player heads use,
/// also on an offline-mode server. Each name and each UUID is kept 10 minutes after its last
/// lookup, at most 256 of each, so polling a lookup keeps its answer. A lookup that failed
/// or found no account is kept the same way, as `Done(None)`; a new request is sent only
/// once it has expired.
pub trait Profiles {
    /// Side: server.
    /// The profile of the account named `name`, any case (`ProfileResolver.fetchByName`):
    /// its UUID from the name lookup, then the session service's signed profile. A name
    /// `StringUtil.isValidPlayerName` refuses (over 16 UTF-16 units, or anything but printable
    /// ASCII other than a space) answers `Done(None)` without a request. `Done(None)` off a
    /// server.
    fn resolve_name(&self, name: &str) -> ProfileResolution {
        unimplemented!("Profiles::resolve_name")
    }
    /// Side: server.
    /// The session service's signed profile of the account `uuid`
    /// (`ProfileResolver.fetchById`). `Done(None)` off a server.
    fn resolve_id(&self, uuid: u128) -> ProfileResolution {
        unimplemented!("Profiles::resolve_id")
    }
}
/// The runtime_levels service, reached through its host.
/// Success from `open`, `unload` or `delete` admits a queued request, not its completion.
/// These requests are not restricted to the calling mod's namespace.
/// Apply failures are logged with the requesting mod and level id. A failed open
/// leaves the id unpublished and free to retry; a failed unload keeps it open for retry.
pub trait RuntimeLevels {
    /// Side: server.
    /// Open the level `id` (`mymod:instance/ab`), as `level` describes. The request is checked
    /// now and applied after this server tick's levels have ticked, never inside a level's tick:
    /// the level is not open, and `is_open` says false, until then. The handle names this open of
    /// the level and never a later one. A folder that exists under the id is reopened as it was
    /// saved. `level_load` fires when it opens. Refused with [`RuntimeLevelError`] for an invalid
    /// or reserved id, one already open or waiting, an unknown dimension type or generator, or an
    /// unknown, invalid or incompatible biome source.
    fn open(&mut self, id: &str, level: RuntimeLevel) -> Result<LevelRef, RuntimeLevelError> {
        unimplemented!("RuntimeLevels::open")
    }
    /// Side: server.
    /// Save and close the open runtime level `id`, keeping its folder, once this server
    /// tick's levels have ticked: `level_unload` fires then. Refused with
    /// [`RuntimeLevelError::PlayersInside`] while a player or an arrival not yet landed is in the
    /// level, now and again when it is applied: the mod moves its players out first
    /// (`entity_teleport_to_level`). [`RuntimeLevelError::NotOpen`] means no runtime level
    /// of that id is open to unload.
    fn unload(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        unimplemented!("RuntimeLevels::unload")
    }
    /// Side: server.
    /// Remove the runtime level `id`, folder and all, once this server tick's levels
    /// have ticked. A level that is open is saved and closed first (`level_unload` fires), as by
    /// `unload`; a level that is not open, such as the leftovers of a crash, only has its files
    /// removed, and a level with no files is nothing to delete. The structure references the
    /// game keeps beside the world for the level go with its folder, unless another id would
    /// share the file (`a_b:c` and `a:b_c` do) and that level exists. Links under `dimensions` or
    /// `data` are never followed: the removal fails and is logged. Refused, now and again when
    /// it is applied, with [`RuntimeLevelError::PlayersInside`] while a player or an arrival not
    /// yet landed is in the level, with [`RuntimeLevelError::LevelsBelow`] while a level below
    /// the id exists (`mymod:instance/ab` below `mymod:instance`: the folders nest), and with
    /// [`RuntimeLevelError::AlreadyOpen`] for a level still waiting to open. A vanilla, startup
    /// or data-pack id is refused with [`RuntimeLevelError::ReservedId`]. An `open` of a level
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
    /// Resolve the handle of a registered render pipeline by ID once in `client_setup` and
    /// again in `client_resources_reloaded`, not each frame. `Unknown` for an ID
    /// no mod registered, `Invalid` for one whose shader the current resources don't give or that
    /// didn't compile. Nothing is read or compiled here.
    fn render_pipeline(&self, id: &Identifier) -> Result<RenderPipelineRef, RenderPipelineError> {
        let _ = id;
        unimplemented!("RenderAssets::render_pipeline")
    }
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// Resolve the handle of a built-in ([`BUILTIN_RENDER_TYPE_IDS`]) or registered custom
    /// [`RenderSetup`] render type by ID once in `client_setup` and again in
    /// `client_resources_reloaded`, not each frame. Returns [`RenderTypeError::Unknown`] for an
    /// unregistered ID, or [`RenderTypeError::Invalid`] when a custom render type's underlying
    /// optional pipeline failed to build in the current resource load.
    fn render_type(&self, id: &Identifier) -> Result<RenderTypeRef, RenderTypeError> {
        let _ = id;
        unimplemented!("RenderAssets::render_type")
    }
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// The render type `RenderTypes.entitySolid(texture)`: the built-in `minecraft:entity_solid`
    /// drawing the dynamic texture `texture`, for `BlockEntityDrawContext::textured_quad`.
    /// Memoized per texture as vanilla's is, so the same `texture` gives the same handle in one
    /// resource load; resolve it once in `client_setup` and again in
    /// `client_resources_reloaded`, not each frame. The texture need not exist yet: a draw asks
    /// whether it does. Only `textured_quad` takes this handle; any other draw refuses it as
    /// [`RenderTypeError::Unknown`].
    fn entity_solid(&self, texture: &Identifier) -> Result<RenderTypeRef, RenderTypeError> {
        let _ = texture;
        unimplemented!("RenderAssets::entity_solid")
    }
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// As [`Self::entity_solid`] for `RenderTypes.entityCutout(texture)`: texels under alpha 0.1
    /// are discarded.
    fn entity_cutout(&self, texture: &Identifier) -> Result<RenderTypeRef, RenderTypeError> {
        let _ = texture;
        unimplemented!("RenderAssets::entity_cutout")
    }
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// As [`Self::entity_solid`] for `RenderTypes.entityTranslucent(texture)`: alpha blended,
    /// depth written, quads sorted far to near.
    fn entity_translucent(&self, texture: &Identifier) -> Result<RenderTypeRef, RenderTypeError> {
        let _ = texture;
        unimplemented!("RenderAssets::entity_translucent")
    }
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// Resolve the handle of a registered compute pipeline ([`ComputePipeline`]) by ID once in
    /// `client_setup` and again in `client_resources_reloaded`, not each frame. Returns
    /// [`ComputePipelineError::Unknown`] for an unregistered ID,
    /// [`ComputePipelineError::Invalid`] when the shader did not build in the current resource
    /// load or the device refuses it, and [`ComputePipelineError::UnsupportedFeature`] when it
    /// needs subgroup operations the adapter lacks.
    fn compute_pipeline(
        &self,
        id: &Identifier,
    ) -> Result<ComputePipelineRef, ComputePipelineError> {
        let _ = id;
        unimplemented!("RenderAssets::compute_pipeline")
    }
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// Resolve the handle of a registered pipeline modifier by ID once in `client_setup`
    /// (also valid in `client_resources_reloaded`). `Unknown` for an ID no mod registered.
    /// Modifiers are static across resource reloads.
    fn pipeline_modifier(
        &self,
        id: &Identifier,
    ) -> Result<PipelineModifierRef, RenderPipelineError> {
        let _ = id;
        unimplemented!("RenderAssets::pipeline_modifier")
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
    /// position. Any state: a mod's, vanilla's, a resource pack's. A block vanilla also draws with
    /// a special renderer (the blocks vanilla's `BuiltInBlockModels` lists: chests, shulker
    /// boxes, banners, heads and skulls, the bell, conduit, decorated pot, enchanting table's
    /// book, copper golem statues, the end portal and gateway) is their composite: the
    /// blockstate parts, then the special renderer's geometry with no block entity data (a
    /// closed lid, a closed book lying on the enchanting table, a still banner with no
    /// patterns, every face of the end portal and gateway, no profile), as a block display
    /// shows it. It draws through
    /// [`WorldDrawContext::draw_model_instances`](crate::WorldDrawContext::draw_model_instances),
    /// [`InstanceGroupRef`] and `BlockEntityDrawContext::terrain_model`; the special part takes
    /// the entity materials, no tint and no hidden faces. `ModelError::UnknownModel` for a state
    /// that draws nothing (air, a block with no model) or one the live block-state index lacks
    /// (a block it does not hold, or an ordinal past the block's states).
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
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// The pixels of one blocks-atlas sprite, as `TextureAtlas.getSprite(..)`'s `SpriteContents`
    /// holds them: the first frame of an animated sprite (the one its `.mcmeta` starts on, not the
    /// whole strip), untinted, RGBA8, read on the CPU from the published resources with no GPU
    /// readback. `sprite` is the reference `texture_sprites` and `block_sprites` answer; it names
    /// the blocks atlas, the only atlas a `SpriteRef` indexes (`TextureAtlas::Blocks`).
    ///
    /// The size is the sprite's own frame as the engine stitched it, larger than the texture
    /// budget lets the atlas array hold it included: such a sprite comes back at its own size, not
    /// shrunk to the array's edge. A frame the atlas squared (a `.mcmeta` frame that is not
    /// square) comes back squared. `AtlasSpriteError::NoSuchSprite` only for a reference whose array or layer is
    /// out of range; `AtlasSpriteError::NotReady` before any resources are published. A
    /// `SpriteRef` carries no generation: one kept across a resource reload may name a different
    /// sprite and is read as that sprite, so resolve references again whenever
    /// `block_sprites_version` changes. Each call copies the pixels: read once in `client_setup` or
    /// `client_resources_reloaded` to bake a lookup texture, a colour table or a preview, not
    /// each frame.
    fn atlas_sprite_pixels(
        &self,
        sprite: SpriteRef,
    ) -> Result<AtlasSpritePixels, AtlasSpriteError> {
        unimplemented!("RenderAssets::atlas_sprite_pixels")
    }
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// The active `shaders/` pack's `block.properties` integer id (`mc_Entity`) for `state`, as
    /// Sunrise's shader-pack loader maps it: the one the engine's own terrain carries for it.
    /// `None` when no pack is active and rendering (the title screen and a pack still loading
    /// included), when the active pack is not a `shaders/` pack, when the pack gives `state` no
    /// id, and when `state` is not in the live block-state index.
    ///
    /// The answer changes when the active pack is loaded, switched or turned off, and no signal
    /// of such a change exists yet: nothing tells a mod when a pack reloads, and
    /// `shader_pack_active()` stays `true` across a switch from one pack to another. A mod that
    /// needs the current id asks again when it needs it; keep no cache that nothing invalidates.
    /// Item and entity ids are not part of this read.
    fn shader_pack_block_id(&self, state: BlockState) -> Option<i32> {
        let _ = state;
        unimplemented!("RenderAssets::shader_pack_block_id")
    }
    /// Side: client.
    /// Whether a shader pack is active and compiled a pack program for `pipeline`'s
    /// [`PackBinding`], so the pack draws it (Iris's `DHCompatInternal.shouldOverride` and
    /// `shouldOverrideShadow`; `incompatiblePack` is the opposite). `false` when no pack is
    /// active (the title screen included), when `pipeline` is stale or invalid, and when the
    /// pack has no matching program or the pipeline has no `PackBinding`: the engine then draws
    /// the pipeline through the mod's own shaders. Read in the render callbacks, as the state
    /// was at the start of the frame; [`ShaderPackState::generation`] says when to ask again.
    fn pipeline_pack_bound(&self, pipeline: RenderPipelineRef) -> bool {
        let _ = pipeline;
        unimplemented!("RenderAssets::pipeline_pack_bound")
    }
}

/// Client-side persistent copies of resolved models.
pub trait InstanceGroups {
    /// Allocate storage for `capacity` copies, initially length zero.
    /// Create at setup/reload, not each frame. Visible GPU-routed copies upload each drawn frame.
    /// Reload, world close, disabling the owning mod, or explicit destroy retires the group.
    /// Re-enabling a mod never revives its old handles; recreate groups after publication.
    fn create_group(&self, model: ModelRef, capacity: u32) -> Result<InstanceGroupRef, GroupError>;
}
