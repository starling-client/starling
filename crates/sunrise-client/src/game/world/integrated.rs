//! The integrated server (`IntegratedServer`): single player's world runs on
//! its own "Server thread", as vanilla's does, and the window joins it as a
//! player through memory, so a world tick never holds up a frame.
//!
//! The window asks its server for what only the server knows (Open to LAN,
//! the world's options) with [`Integrated::ask`] and an `OwnerRequest`, as vanilla's client calls
//! into its `IntegratedServer`.
//!
//! Every single-player world runs this way; LAN guests and dedicated servers
//! use the same server code.
use super::{
    Dim, Game, Options, PackResources, Resources, UiResources, WorldIcon,
    execution_role::ExecutionRole,
    lan::{
        LanSettings, Owner,
        sync::{SyncPreload, preload_sync_data},
    },
    owner_link::{OwnerRequest, Reply},
    world_loading::{WorldLoader, WorldRequest},
};
use std::{
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI32, AtomicU16, AtomicU32, AtomicU64, Ordering},
        mpsc::{Receiver, RecvTimeoutError, Sender, channel, sync_channel},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use sunrise_client::level_loading::{STATUS_RADIUS, Stage};
use sunrise_client::spans::startup;
use sunrise_client::{
    Result,
    audio::Audio,
    chunk_io::Reports,
    lan::{Lan, MAX_PLAYERS, NetConfig, OwnerProfile, hub::HubConfig},
    settings::Settings,
    title::WorldOptions,
};
use sunrise_protocol::connection::Pipe;
use sunrise_protocol::{crypto::offline_uuid, text::translatable};

#[path = "integrated/capture.rs"]
pub(super) mod capture;

/// How long the window waits for the server thread to answer an `ask`.
const CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// The error of an `ask` without an answer: the server stopped, or it was
/// busy past `CALL_TIMEOUT` (the log says which).
pub(super) const NO_ANSWER: &str =
    "the integrated server stopped or did not answer within 10 s (the game log says which)";

/// The server thread holds its `Game` by value and runs every tick on it. It uses the game
/// worker stack floor or a larger `RUST_MIN_STACK` requested by a harness. std only reads that
/// variable for threads spawned without an explicit size.
fn server_thread_stack() -> usize {
    static SIZE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *SIZE.get_or_init(|| stack_for(std::env::var("RUST_MIN_STACK").ok().as_deref()))
}

/// Interpret `min_stack` as a byte count, as std parses `RUST_MIN_STACK`.
fn stack_for(min_stack: Option<&str>) -> usize {
    min_stack
        .and_then(|bytes| bytes.parse::<usize>().ok())
        .unwrap_or_default()
        .max(sunrise_threads::GAME_THREAD_STACK_BYTES)
}

/// `LevelLoadProgressTracker.EXPECTED_PLAYER_CHUNKS`: the 7x7 chunks around the spawn.
const PLAYER_CHUNKS: usize = 49;

/// What the window and its server thread share.
#[derive(Default)]
struct Flags {
    #[cfg(test)]
    fail_tick: AtomicBool,
    /// `Minecraft.pause`: a single-player pause screen is open.
    paused: AtomicBool,
    stop: AtomicBool,
    finished: AtomicBool,
    failure: Mutex<Option<String>>,
    /// The owning window's report inbox, shared with this server and its region workers.
    reports: Arc<Reports>,
    /// `IntegratedServer.getPort`: the port it is open to the LAN on, 0
    /// while it is not.
    published: AtomicU16,
    smoothed_tick_ms: AtomicU32,
    /// `options.renderDistance` and `options.simulationDistance`, which the
    /// server follows while it runs (IntegratedServer.tickServer).
    render_distance: AtomicI32,
    simulation_distance: AtomicI32,
    /// `options.entityDistanceScaling` (an `f64`'s bits), which
    /// `IntegratedServer.getScaledTrackingDistance` reads on every call.
    entity_distance: AtomicU64,
    /// `MinecraftServer.isCurrentlySaving`: the server is autosaving.
    saving: AtomicBool,
    /// The generators of the levels the server has run, by dimension: what
    /// F3's chunk generation entry asks `ServerChunkCache.getGenerator`.
    terrain: Mutex<
        std::collections::HashMap<&'static str, Arc<sunrise_client::vanillagen::VanillaTerrain>>,
    >,
    /// What the server's `LevelLoadListener` reports, for the window's
    /// `LevelLoadingScreen` (`LevelLoadTracker.serverProgress/statusView`).
    load: Arc<sunrise_client::level_loading::ServerLoadView>,
    /// What the world's upgrade has done (`UpgradeProgress`): the window shows it on
    /// `FileFixerProgressScreen` while the file fixers run, and its Cancel stops them.
    upgrade: Arc<sunrise_world::datafix::progress::UpgradeProgress>,
    /// Where the window's debug entries want the server's lines for (its
    /// dimension and feet), and the lines of the last tick there.
    debug_at: Mutex<Option<(Dim, [i32; 3])>>,
    debug: Mutex<super::debug_lines::ServerDebug>,
}

impl Flags {
    fn emergency_save(&self, server: std::thread::ThreadId, timeout: Duration) -> bool {
        if std::thread::current().id() == server || self.finished.load(Ordering::Acquire) {
            return true;
        }
        log::warn!("Sunrise: saving the world before the crash");
        self.stop.store(true, Ordering::Relaxed);
        let started = Instant::now();
        while !self.finished.load(Ordering::Acquire) {
            let Some(left) = timeout.checked_sub(started.elapsed()) else {
                log::warn!(
                    "Sunrise: the world did not save within {} s",
                    timeout.as_secs()
                );
                return false;
            };
            std::thread::sleep(left.min(Duration::from_millis(10)));
        }
        true
    }
}

// Signal even if serve returns an error or unwinds; no game data is touched here.
struct Finished(Arc<Flags>);
impl Drop for Finished {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::Release);
    }
}

/// The window's handle on its server thread.
pub(super) struct Integrated {
    flags: Arc<Flags>,
    thread: Option<JoinHandle<()>>,
    emergency_save: Option<sunrise_client::crash::EmergencySave>,
    tasks: Sender<OwnerRequest>,
    /// The world's name (`level.dat`'s `LevelName`), for the pause menu.
    pub world_name: String,
    pub save_folder: String,
    pub rule_registry: Arc<sunrise_client::gamerules::RuleRegistry>,
}
impl Integrated {
    /// The window's handle on a server fixture, with no server thread behind it.
    #[cfg(test)]
    pub(super) fn fixture_for(serving: &Serving) -> Self {
        let (tasks, _receiver) = channel();
        Self {
            flags: serving.flags.clone(),
            thread: None,
            emergency_save: None,
            tasks,
            world_name: "test".into(),
            save_folder: "test".into(),
            rule_registry: Default::default(),
        }
    }

    /// Queue every change, even if the currently published tree still has the requested state:
    /// an earlier update may be waiting for a busy server thread.
    fn queue_pregen_option(&self, enabled: bool) {
        self.tell(OwnerRequest::SetPregen(enabled));
    }
    /// The generator of the server's level in `dimension`, once it has run.
    pub fn terrain(
        &self,
        dimension: &str,
    ) -> Option<Arc<sunrise_client::vanillagen::VanillaTerrain>> {
        self.flags.terrain.lock().ok()?.get(dimension).cloned()
    }
    /// The server's load progress and chunk status map.
    pub fn load_view(&self) -> &Arc<sunrise_client::level_loading::ServerLoadView> {
        &self.flags.load
    }
    /// The world's upgrade while the server opens it.
    pub fn upgrade(&self) -> &sunrise_world::datafix::progress::UpgradeProgress {
        &self.flags.upgrade
    }
    pub fn smoothed_tick_ms(&self) -> f32 {
        f32::from_bits(self.flags.smoothed_tick_ms.load(Ordering::Relaxed))
    }
    /// Asks for the debug entries' server lines at `at`, or stops asking.
    pub(super) fn request_debug(&self, at: Option<(Dim, [i32; 3])>) {
        if let Ok(mut want) = self.flags.debug_at.lock() {
            *want = at;
        }
    }
    /// The debug entries' server lines of the last tick.
    pub(super) fn debug(&self) -> super::debug_lines::ServerDebug {
        self.flags
            .debug
            .lock()
            .map(|d| d.clone())
            .unwrap_or_default()
    }
    /// `MinecraftServer.isCurrentlySaving`.
    pub fn is_saving(&self) -> bool {
        self.flags.saving.load(Ordering::Relaxed)
    }
    /// `IntegratedServer.tickServer` skips the world while the game is paused.
    pub fn set_paused(&self, paused: bool) {
        self.flags.paused.store(paused, Ordering::Relaxed);
    }
    /// The window's render and simulation distances and its Entity Distance
    /// option, for the server to follow.
    pub fn set_distances(&self, render: i32, simulation: i32, entity: f64) {
        self.flags.render_distance.store(render, Ordering::Relaxed);
        self.flags
            .simulation_distance
            .store(simulation, Ordering::Relaxed);
        self.flags
            .entity_distance
            .store(entity.to_bits(), Ordering::Relaxed);
    }
    /// `IntegratedServer.getPort` while `isPublished`.
    pub fn published(&self) -> Option<u16> {
        Some(self.flags.published.load(Ordering::Relaxed)).filter(|&p| p != 0)
    }
    /// `executeBlocking`: `None` once the server stopped or after `CALL_TIMEOUT` (the log tells which).
    pub(super) fn ask<T: Send + 'static>(
        &self,
        build: impl FnOnce(Reply<T>) -> OwnerRequest,
    ) -> Option<T> {
        if self.flags.stop.load(Ordering::Relaxed) || self.flags.finished.load(Ordering::Acquire) {
            return None;
        }
        let (tx, rx) = sync_channel(1);
        self.tasks.send(build(tx)).ok()?;
        let deadline = Instant::now() + CALL_TIMEOUT;
        loop {
            match rx.recv_timeout(Duration::from_millis(25)) {
                Ok(answer) => return Some(answer),
                Err(RecvTimeoutError::Disconnected) => return None,
                Err(RecvTimeoutError::Timeout) => {
                    if self.flags.stop.load(Ordering::Relaxed)
                        || self.flags.finished.load(Ordering::Acquire)
                    {
                        return None;
                    }
                    if Instant::now() >= deadline {
                        log::warn!(
                            "Sunrise: the server thread did not answer within {} s (running: {}, \
                             autosaving: {}, smoothed tick {:.1} ms)",
                            CALL_TIMEOUT.as_secs(),
                            !self.stopped(),
                            self.is_saving(),
                            self.smoothed_tick_ms()
                        );
                        return None;
                    }
                }
            }
        }
    }
    /// `execute`: send a request without waiting for it. False once the server has stopped.
    pub(super) fn tell(&self, request: OwnerRequest) -> bool {
        self.tasks.send(request).is_ok()
    }
    /// Tests only: `f` on the server thread between two ticks, its answer awaited as `ask` does.
    #[cfg(test)]
    pub fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Game<'_>) -> T + Send + 'static,
    ) -> Option<T> {
        self.ask(|reply| {
            OwnerRequest::InTest(Box::new(move |g| {
                let _ = reply.send(f(g));
            }))
        })
    }
    /// Tests only: `f` on the server thread between two ticks, not awaited.
    #[cfg(test)]
    pub fn post(&self, f: impl FnOnce(&mut Game<'_>) + Send + 'static) -> bool {
        self.tell(OwnerRequest::InTest(Box::new(f)))
    }
    /// An integrated server whose thread already finished, just before the window disconnects.
    #[cfg(test)]
    pub(in crate::game) fn stopped_for_test() -> Self {
        Self {
            flags: Arc::new(Flags::default()),
            thread: None,
            emergency_save: None,
            tasks: channel().0,
            world_name: "Test world".into(),
            save_folder: "test_world".into(),
            rule_registry: Arc::new(sunrise_client::gamerules::RuleRegistry::vanilla()),
        }
    }
    /// `IntegratedServer.halt(true)`: returns once the world is saved and let go.
    pub fn stop(mut self) -> Result<()> {
        self.halt()
    }
    /// Stop on the server thread; the window polls completion while drawing.
    pub fn request_stop(&self) {
        self.flags.stop.store(true, Ordering::Relaxed);
    }
    pub fn stopped(&self) -> bool {
        self.thread
            .as_ref()
            .is_none_or(|thread| thread.is_finished())
    }
    fn halt(&mut self) -> Result<()> {
        self.request_stop();
        self.emergency_save.take();
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| "the integrated server panicked")?;
        }
        match self
            .flags
            .failure
            .lock()
            .map_err(|_| "server status poisoned")?
            .as_ref()
        {
            Some(error) => Err(error.clone().into()),
            None => Ok(()),
        }
    }
}
impl Drop for Integrated {
    fn drop(&mut self) {
        if let Err(error) = self.halt() {
            log::error!("Sunrise: integrated server shutdown: {error}");
        }
    }
}

/// The server thread's side of the window's handle (`Game::serving`).
pub(super) struct Serving {
    flags: Arc<Flags>,
    tasks: Receiver<OwnerRequest>,
    /// The window's profile.
    pub(super) owner: u128,
    /// Open to LAN or closed from inside a player's command, while the hub
    /// is out of its place: applied after the tick.
    pending: Option<Publish>,
    /// Where the load focus is and how far the player's chunks are.
    load: LoadPhase,
}
/// The part of `LevelLoadListener` after `prepareLevels`: `PrepareSpawnTask`'s
/// `LOAD_PLAYER_CHUNKS`.
#[derive(Default)]
struct LoadPhase {
    /// `updateFocus`: the spawn chunk, once the stage started.
    focus: Option<(i32, i32)>,
    /// `finish(LOAD_PLAYER_CHUNKS)`.
    done: bool,
}
impl Serving {
    /// A server thread's handle with no thread behind it, published on `port` (0: not).
    #[cfg(test)]
    pub(super) fn fixture(port: u16) -> Self {
        Self::fixture_with_reports(port, Arc::default())
    }

    #[cfg(test)]
    pub(super) fn fixture_with_reports(port: u16, reports: Arc<Reports>) -> Self {
        let flags = Arc::new(Flags {
            reports,
            ..Flags::default()
        });
        flags.published.store(port, Ordering::Relaxed);
        let (_sender, tasks) = channel();
        Self {
            flags,
            tasks,
            owner: 1,
            pending: None,
            load: LoadPhase::default(),
        }
    }
    /// Send reports only to this server's owner, not to LAN clients.
    pub(super) fn reports(&self) -> &Reports {
        &self.flags.reports
    }

    /// The window reads this as `MinecraftServer.isCurrentlySaving`.
    pub(super) fn set_saving(&self, saving: bool) {
        self.flags.saving.store(saving, Ordering::Relaxed);
    }
}
enum Publish {
    Open(TcpListener, LanSettings),
    Close,
}

/// What the server thread hands the window once the world is open: its end
/// of the connection and the world's name, or why it failed.
type Ready = std::result::Result<(Pipe, String), String>;

/// An integrated server loading its world: the window keeps drawing its
/// loading screen and joins once the server is ready, as vanilla's client
/// does while its server prepares (`Minecraft.doWorldLoad`), instead of
/// waiting for it frozen for the whole load.
pub(super) struct Starting {
    pub(super) handle: Integrated,
    ready: Receiver<Ready>,
    player: String,
    /// The save's folder in `saves/`, for the world icon once joined.
    folder: String,
    /// `Minecraft.doWorldLoad`'s `newWorld`: the loading screen stays 500 ms
    /// once the level is ready.
    new_world: bool,
}

/// The world to open and the player who owns it.
struct Launch {
    folder: String,
    rule_registry: Arc<sunrise_client::gamerules::RuleRegistry>,
    name: Option<String>,
    seed: Option<i64>,
    creative: bool,
    create: Option<WorldOptions>,
    /// The window's profile: the only one let in until the world is open to
    /// the LAN, and the world's player.
    owner: u128,
    /// Its name, for the LAN's MOTD.
    player: String,
    /// The signed-in account's profile, fetched while the world loads.
    owner_profile: Option<Arc<OwnerProfile>>,
}

impl Game<'_> {
    /// Only the owner window changes its integrated server's command tree. A remote
    /// server keeps its own choice and permissions.
    pub(super) fn sync_pregen_option(&self) {
        let Some(server) = &self.integrated else {
            return;
        };
        let enabled = self.options.pregen || self.settings.extras.pregen;
        // Check on the server, after earlier updates have run, so a later toggle
        // wins. Do not block the window when an unrelated setting changes.
        server.queue_pregen_option(enabled);
    }

    /// `Minecraft.hasSingleplayerServer`: a world of this window's own, run
    /// in the frame or on its server thread.
    pub(super) fn has_singleplayer_server(&self) -> bool {
        self.remote.is_none() || self.integrated.is_some()
    }
    /// `Minecraft.isMultiplayerServer`: on another's server, or a world of one's own open to the
    /// network.
    pub(super) fn is_multiplayer_server(&self) -> bool {
        !self.has_singleplayer_server() || self.is_published()
    }
    /// `IntegratedServer.isPublished`: the world is open to the network.
    pub(super) fn is_published(&self) -> bool {
        self.lan.is_some()
            || self
                .integrated
                .as_ref()
                .is_some_and(|s| s.published().is_some())
    }
    /// `Minecraft.doWorldLoad`: start the server thread on the world; the
    /// window joins it through memory once it is ready (`poll_integrated_start`).
    pub(super) fn open_integrated(
        &mut self,
        folder: &str,
        name: Option<&str>,
        seed: Option<i64>,
        creative: bool,
        create: Option<&WorldOptions>,
    ) -> Result<()> {
        self.client_logging_out();
        sunrise_client::command::vanilla::set_sunrise_commands(
            self.options.pregen || self.settings.extras.pregen,
        );
        let auth = sunrise_client::auth::current();
        // Starling: the launcher's profile plays under its own name; else a name chosen in Skin
        // Customization goes before the account's.
        let player = match &auth {
            Some(account)
                if sunrise_client::accounts::in_game()
                    && sunrise_client::accounts::profiles().player().is_some() =>
            {
                account.name.clone()
            }
            Some(account) => std::env::current_dir()
                .ok()
                .and_then(|dir| sunrise_client::offline_name::chosen(&dir))
                .unwrap_or_else(|| account.name.clone()),
            None => self.options.player_name(),
        };
        let new_world = create.is_some();
        let rule_registry = create.map_or_else(
            || Arc::new(crate::game::api_values::game_rule_registry()),
            |options| options.rules.registry().clone(),
        );
        // `IntegratedServer`'s constructor: `setSingleplayerProfile(minecraft.getGameProfile())`
        // — the signed-in account's own UUID. Only an offline start names its player by the
        // offline profile. The owner is the world's player, so a world without
        // `singleplayer_uuid` (a Realms download, `renameAndDropPlayer`) finds the owner's
        // `players/data/<uuid>.dat` instead of starting a new player.
        let owner = auth
            .as_ref()
            .map_or_else(|| offline_uuid(&player), |a| a.uuid);
        let launch = Launch {
            rule_registry: rule_registry.clone(),
            folder: folder.to_owned(),
            name: name.map(str::to_owned),
            seed,
            creative,
            create: create.cloned(),
            owner,
            player: player.clone(),
            owner_profile: auth.as_ref().map(|a| {
                let id = a.uuid;
                Arc::new(OwnerProfile::new(move || {
                    sunrise_client::skins::fetch_own_properties(id)
                        .into_iter()
                        .map(
                            |(name, value, signature)| sunrise_core::profile::AccountProperty {
                                name,
                                value,
                                signature,
                            },
                        )
                        .collect()
                }))
            }),
        };
        let mut options = self.options.clone();
        // A server without a console: no local player, no window, no control socket.
        // Compatibility for gameplay checks outside startup/world loading.
        options.dedicated = Some(Vec::new());
        options.execution_role = ExecutionRole::IntegratedServer;
        options.control = None;
        options.benchmark = None;
        options.frames = None;
        options.world_frames = None;
        options.report = None;
        options.server = None;
        options.world = None;
        self.join_world_rest()?;
        // The window plays against the world's own biome table (its chunks
        // carry the packs' ids past the built-in list, `set_registries`
        // names them through it), as the server thread's copy does.
        let world = std::env::current_dir()?.join("saves").join(folder);
        let enabled_features = sunrise_client::save::Level::read(&world)
            .map(|l| l.enabled_features)
            .unwrap_or_default();
        self.world_biomes = Some(super::world_loading::packed_biomes(
            &world,
            &self.options.resources,
            self.res.biomes(),
            &enabled_features,
            &self.pack.additional_data,
        ));
        self.world_biomes_base = self.world_biomes.clone();
        let (res, pack, settings) = (self.res.clone(), self.pack.clone(), self.settings.clone());
        sunrise_client::parallel::set_setting(settings.sunrise.parallel_tick);
        let flags = Arc::new(Flags {
            reports: self.client_reports.clone(),
            ..Flags::default()
        });
        flags
            .render_distance
            .store(settings.render_distance, Ordering::Relaxed);
        flags
            .simulation_distance
            .store(settings.simulation_distance, Ordering::Relaxed);
        flags
            .entity_distance
            .store(settings.entity_distance.to_bits(), Ordering::Relaxed);
        let (ready_tx, ready) = channel();
        let (tasks, server_tasks) = channel();
        let server_flags = flags.clone();
        let run_server = move |_: &()| {
            let _finished = Finished(server_flags.clone());
            // MinecraftServer.spin's Thread.setPriority(8).
            sunrise_threads::favour_current();
            let result = serve(
                &res,
                pack,
                options,
                settings,
                launch,
                server_flags.clone(),
                server_tasks,
                &ready_tx,
            );
            if let Err(e) = result {
                log::error!("Sunrise: integrated server: {e}");
                *server_flags.failure.lock().unwrap() = Some(e.to_string());
                let _ = ready_tx.send(Err(e.to_string()));
            }
        };
        // Production installations are process-wide. A test installation is scoped
        // to its owning thread; the real server thread must inherit that snapshot.
        #[cfg(test)]
        let run_server = mod_api::testing::inherit_mods(run_server);
        let thread = std::thread::Builder::new()
            .name(sunrise_client::crash::INTEGRATED_SERVER_THREAD.into())
            .stack_size(server_thread_stack())
            .spawn(move || run_server(&()))?;
        let emergency_flags = flags.clone();
        let server_id = thread.thread().id();
        let emergency_save = sunrise_client::crash::on_emergency_save(move || {
            emergency_flags.emergency_save(server_id, Duration::from_secs(30));
        });
        let handle = Integrated {
            emergency_save: Some(emergency_save),
            flags,
            thread: Some(thread),
            tasks,
            world_name: String::new(),
            save_folder: folder.to_owned(),
            rule_registry,
        };
        self.integrated_starting = Some(Starting {
            handle,
            ready,
            player,
            folder: folder.to_owned(),
            new_world,
        });
        // Harnesses, control scripts and runs without a window go on with the world open.
        if self.window.is_none() || super::synchronous_startup(&self.options) {
            while self.integrated_starting.is_some() {
                self.poll_integrated_start()?;
                if self.integrated_starting.is_some() {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
        Ok(())
    }

    /// The starting server's answer, polled each frame: joined once it is ready.
    pub(super) fn poll_integrated_start(&mut self) -> Result<()> {
        if self.integrated_starting.is_none() {
            return Ok(());
        }
        self.tick_load_progress();
        let Some(starting) = &self.integrated_starting else {
            return Ok(());
        };
        let (pipe, world_name) = match starting.ready.try_recv() {
            Ok(Ok(ready)) => ready,
            Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(()),
            // FileFixerProgressScreen's Cancel: the upgrade stopped, back to the title.
            Ok(Err(_)) if starting.handle.upgrade().is_canceled() => {
                self.integrated_starting = None;
                self.perf.cancel_world_opening();
                return Ok(());
            }
            Ok(Err(e)) => {
                self.integrated_starting = None;
                return Err(e.into());
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.integrated_starting = None;
                return Err("the integrated server stopped".into());
            }
        };
        let Some(Starting {
            mut handle,
            player,
            folder,
            new_world,
            ..
        }) = self.integrated_starting.take()
        else {
            return Ok(());
        };
        handle.world_name = world_name;
        self.server_packs_world_pack(&folder);
        self.connect_local(pipe, &player)?;
        if let Some(r) = self.remote.as_mut() {
            // `new LevelLoadTracker(newWorld ? 500L : 0L)`.
            r.level_load = sunrise_client::level_loading::LevelLoadTracker::new(if new_world {
                sunrise_client::level_loading::LEVEL_LOAD_CLOSE_DELAY_MS
            } else {
                0
            });
        }
        // IntegratedServer.getWorldScreenshotFile: the client grabs the frame
        // the Select World list shows, and needs the save folder for it.
        self.world_icon = Some(WorldIcon {
            path: std::env::current_dir()?
                .join("saves")
                .join(&folder)
                .join("icon.png"),
            last_attempt: None,
        });
        self.integrated = Some(handle);
        Ok(())
    }

    /// `PrepareSpawnTask`'s `LOAD_PLAYER_CHUNKS`, reported to the window's
    /// loading screen: the focus moves to the spawn chunk when the stage
    /// starts, the 7x7 chunks around it count as they are ready, and the
    /// status map around the focus follows (`ChunkLoadStatusView.get`).
    pub(super) fn publish_load_view(&mut self) {
        let Some(serving) = self.serving.as_mut() else {
            return;
        };
        if serving.load.done {
            return;
        }
        let view = serving.flags.load.clone();
        let focus = match serving.load.focus {
            Some(focus) => focus,
            None => {
                let Some(spawn) = self.spawn else { return };
                let focus = ((spawn.x.floor() as i32) >> 4, (spawn.z.floor() as i32) >> 4);
                view.start(Stage::LoadPlayerChunks, PLAYER_CHUNKS);
                serving.load.focus = Some(focus);
                focus
            }
        };
        let ready = (-3..=3)
            .flat_map(|dz| (-3..=3).map(move |dx| (focus.0 + dx, focus.1 + dz)))
            .filter(|&(x, z)| self.streaming.is_lit(x, z))
            .count();
        view.update(Stage::LoadPlayerChunks, ready, PLAYER_CHUNKS);
        let diameter = STATUS_RADIUS * 2 + 1;
        view.set_status(
            (0..diameter)
                .flat_map(|x| (0..diameter).map(move |z| (x, z)))
                .map(|(x, z)| {
                    self.streaming.latest_status(
                        &self.world,
                        focus.0 + x - STATUS_RADIUS,
                        focus.1 + z - STATUS_RADIUS,
                    )
                })
                .collect(),
        );
        if ready == PLAYER_CHUNKS {
            view.finish(Stage::LoadPlayerChunks);
            if let Some(serving) = self.serving.as_mut() {
                serving.load.done = true;
            }
        }
    }

    /// `LevelLoadingScreen.tick`'s easing of the server's progress, every 50 ms
    /// of wall time (the starting server has no client tick yet).
    pub(super) fn tick_load_progress(&mut self) {
        let view = match (&self.integrated_starting, &self.integrated) {
            (Some(s), _) => s.handle.load_view().clone(),
            (None, Some(i)) => i.load_view().clone(),
            _ => return,
        };
        let now = Instant::now();
        let at = self.loading_progress_at.get_or_insert(now);
        let steps = (now.duration_since(*at).as_millis() / 50).min(20) as u32;
        *at += Duration::from_millis(u64::from(steps) * 50);
        if now.duration_since(*at) > Duration::from_secs(1) {
            *at = now;
        }
        for _ in 0..steps {
            self.loading_progress = sunrise_client::level_loading::smooth(
                self.loading_progress,
                view.server_progress(),
            );
        }
    }

    /// The server thread's loop (`MinecraftServer.runServer`): 20 ticks a
    /// second, the world held while the window is paused.
    fn run_integrated(
        &mut self,
        launch: Launch,
        flags: Arc<Flags>,
        tasks: Receiver<OwnerRequest>,
        ready: &Sender<Ready>,
        opening: Opening,
    ) -> Result<()> {
        let result = self.run_server_lifecycle(false, |game, startup_began| {
            *startup_began = true;
            game.run_integrated_inner(launch, flags, tasks, ready, opening)
        });
        self.finish_server_capture();
        result
    }

    fn run_integrated_inner(
        &mut self,
        launch: Launch,
        flags: Arc<Flags>,
        tasks: Receiver<OwnerRequest>,
        ready: &Sender<Ready>,
        opening: Opening,
    ) -> Result<()> {
        self.integrated_open(&launch, &flags, tasks, ready, opening)?;
        // The server thread's `server_started` site, before the tick loop.
        self.native_lifecycle_world_ready();
        if let (Some(script), Some(output)) = (
            std::env::var_os("SUNRISE_COMMAND_SCRIPT"),
            std::env::var_os("SUNRISE_COMMAND_OUTPUT"),
        ) {
            // The script owns the clock: connection setup and terrain loading
            // cannot add wall-clock-dependent world ticks to oracle output.
            return self.run_command_harness(script.as_ref(), output.as_ref());
        }
        let slow_ms = self
            .options
            .spikes
            .unwrap_or(super::dedicated::SLOW_TICK_MS);
        self.profile
            .server
            .get_or_insert_with(|| capture::ServerSide::new(slow_ms));
        self.integrated_tick_loop(&flags)
    }

    fn integrated_tick_loop(&mut self, flags: &Flags) -> Result<()> {
        let mut next = Instant::now();
        let mut was_paused = false;
        let mut failure = None;
        while !flags.stop.load(Ordering::Relaxed) {
            self.run_tasks();
            if self.agent.hold_server {
                // Keep servicing control requests so the fixture can release the barrier.
                // Unlike /tick freeze, no player's queued packets are applied here.
                self.wait_for_next_server_tick(&mut next, true);
                continue;
            }
            self.server_tick_deadline = Some(next + self.clock.tick_duration());
            let paused = flags.paused.load(Ordering::Relaxed);
            let ticked =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                    #[cfg(test)]
                    if flags.fail_tick.swap(false, Ordering::Relaxed) {
                        self.level.name = "saved after integrated tick panic".into();
                        self.experience.total = 37;
                        panic!("integrated tick fixture");
                    }
                    if paused && !was_paused {
                        // IntegratedServer.tickServer: the tick the game pauses saves
                        // everything (saveEverything, without flushing), as an
                        // autosave does.
                        log::info!("Saving and pausing game...");
                        self.save_game_with(false)?;
                    }
                    was_paused = paused;
                    if paused {
                        // Only the connections run.
                        self.tick_server_measured(true)?;
                    } else {
                        self.follow_distances(flags);
                        let cpu = sunrise_client::spans::thread_cpu();
                        let started = Instant::now();
                        self.tick_server_measured(false)?;
                        // MinecraftServer.tickServer's tick times, for /tick query.
                        self.clock.record(started.elapsed());
                        // Parallel Tick's Auto decides here, between ticks, how the next one runs.
                        let on_cpu = sunrise_client::spans::thread_cpu().zip(cpu);
                        sunrise_client::parallel::after_tick(
                            started.elapsed().as_secs_f32() * 1000.0,
                            on_cpu.map(|(end, start)| (end - start).as_secs_f32() * 1000.0),
                        );
                        self.debug_charts.log_tick(started, started.elapsed());
                        flags.smoothed_tick_ms.store(
                            self.debug_charts.smoothed_tick_ms.to_bits(),
                            Ordering::Relaxed,
                        );
                        if let (Some(terrain), Ok(mut terrains)) =
                            (self.end.locator(), flags.terrain.lock())
                        {
                            terrains.insert(self.dim.id(), terrain);
                        }
                    }
                    Ok(())
                }));
            match ticked {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    failure = Some(error);
                    break;
                }
                Err(payload) => {
                    failure = Some(
                        format!(
                            "the server thread panicked: {}",
                            super::dedicated::panic_message(payload.as_ref())
                        )
                        .into(),
                    );
                    break;
                }
            }
            self.apply_publish();
            self.publish_load_view();
            self.wait_for_next_server_tick(&mut next, paused);
        }
        if let Some(error) = failure {
            let finished = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.finish_server_capture();
            }));
            if let Err(payload) = finished {
                log::error!(
                    "Sunrise: the server capture could not be closed after the crash: {}",
                    super::dedicated::panic_message(payload.as_ref())
                );
            }
            let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.stop_after_crash(&error.to_string())
            }));
            match stopped {
                Ok(Ok(())) => {}
                Ok(Err(e)) => log::error!("Sunrise: crash shutdown: {e}"),
                Err(payload) => log::error!(
                    "Sunrise: crash shutdown panicked: {}",
                    super::dedicated::panic_message(payload.as_ref())
                ),
            }
            return Err(error);
        }
        Ok(())
    }

    /// After the level of `dim` ticked: the window's debug entries' server
    /// lines, when they want them from this level.
    pub(super) fn publish_server_debug(&self, dim: Dim) {
        let Some(serving) = &self.serving else {
            return;
        };
        let want = serving.flags.debug_at.lock().ok().and_then(|w| *w);
        if let Some((at, feet)) = want
            && at == dim
        {
            let lines = self.server_debug_lines(feet);
            if let Ok(mut debug) = serving.flags.debug.lock() {
                *debug = lines;
            }
        }
    }

    /// The integrated server's world open: `run_integrated`'s segment
    /// from `load_world` through `settle_spawn`, whole so its failures are
    /// one open failure (`WorldOpenFailed`, the engine's own error text).
    fn integrated_open(
        &mut self,
        launch: &Launch,
        flags: &Arc<Flags>,
        tasks: Receiver<OwnerRequest>,
        ready: &Sender<Ready>,
        opening: Opening,
    ) -> Result<()> {
        // The mods' server configs, read before the server starts.
        if let Ok(dir) = std::env::current_dir() {
            self.native_configs_server_start(&dir.join("saves").join(&launch.folder), false);
        }
        let prepared = self.prepare_early_game_rules(opening.world?)?;
        self.native_server_about_to_start(false);
        self.native_server_starting(false);
        self.native_lifecycle_open_started();
        // `load_world`, with the world `serve` started preparing.
        startup::time("server.wait_rest", || self.join_world_rest())?;
        self.perf.world_opening();
        let open_step = startup::step("server.open_world");
        self.finish_open_world(prepared)?;
        drop(open_step);
        if let Some(sync) = opening.sync {
            startup::time("server.sync_adopt", || sync.adopt(&self.pack_files()));
        }
        let saved = Box::new(self.level.clone());
        // IntegratedServer runs the server mods; the window the client mods.
        startup::time("server.start_mods", || self.start_mods());
        self.refresh_loop_context()?;
        // Math.max(2, options.renderDistance): past vanilla's 32 when the
        // window's render distance is (Sunrise's options go to 64).
        let view_distance = self.settings.render_distance.max(2);
        let config = HubConfig {
            allow_flight: true,
            default_game_mode: self.level.mode(),
            // Only the owner until it is opened to the LAN.
            guest_commands: false,
            force_game_mode: false,
            hardcore: self.level.hardcore,
            view_distance,
            simulation_distance: self.settings.simulation_distance.max(2),
            dimension: self.dim.id().to_owned(),
            levels: self.level_shapes,
            level_seeds: self.levels.runtime.seeds(),
            seed: self.level.seed,
            max_players: MAX_PLAYERS,
            // No player of its own: the hub runs without a host.
            integrated: true,
        };
        let owner = launch.owner;
        let published = flags.clone();
        let net = NetConfig {
            // No network until it is opened to the LAN; the memory connection
            // is not compressed (`Connection.isMemoryConnection`).
            listen: false,
            online: sunrise_client::lan::OnlineMode::lan(),
            gate: Some(Arc::new(move |address, uuid, _| {
                refused(
                    uuid == owner,
                    published.published.load(Ordering::Relaxed) != 0,
                    address == sunrise_client::lan::conn::MEMORY_ADDRESS,
                )
                .then(|| translatable("multiplayer.disconnect.not_whitelisted", &[]))
            })),
            levels: self.level_shapes,
            owner_profile: launch.owner_profile.clone(),
            // `IntegratedServer.getSingleplayerProfile`: the window's own player logs in
            // through memory as the account's profile, whatever the hello says.
            singleplayer_profile: Some((launch.player.clone(), owner)),
            // IntegratedServer.initServer: the MOTD names the owner and the world.
            ..NetConfig::lan(
                0,
                format!("{} - {}", launch.player, self.level.name),
                MAX_PLAYERS,
            )
        };
        let settings = LanSettings {
            port: 0,
            guest_commands: false,
            force_game_mode: false,
        };
        startup::time("server.lan_host", || self.lan_host(settings, net, config))?;
        let commands = self.allow_commands();
        let pipe = {
            let host = self
                .lan
                .as_mut()
                .ok_or("the integrated server did not start")?;
            // The owner is an operator when the world allows commands
            // (`IntegratedPlayerList`, `MinecraftServer.getProfilePermissions`).
            host.hub.ops.insert(owner, if commands { 4 } else { 0 });
            host.hub.singleplayer_owner = Some(owner);
            host.owner = Some(Owner {
                uuid: owner,
                saved: Some(saved),
            });
            host.hub.lan.connect_local()?
        };
        self.serving = Some(Serving {
            flags: flags.clone(),
            tasks,
            owner,
            pending: None,
            load: LoadPhase::default(),
        });
        // MinecraftServer.prepareLevels: the saved tickets' chunks load; the
        // player's chunks come after.
        let view = &flags.load;
        view.start(Stage::LoadInitialChunks, 0);
        startup::time("server.prepare_levels", || self.prepare_levels())?;
        view.finish(Stage::LoadInitialChunks);
        let _ = ready.send(Ok((pipe, self.level.name.clone())));
        startup::time("server.settle_spawn", || self.settle_spawn())
    }

    /// `MinecraftServer.stopServer`: the players are saved and sent away,
    /// then the world is saved.
    pub(super) fn stop_integrated(&mut self, crashed: bool, run_error: Option<&str>) -> Result<()> {
        // Abort background terrain generation/decoration on shutdown, matching vanilla behavior.
        self.streaming.abort_generation();
        self.levels.abort_parked_generation();
        // A failed open keeps its opening phase until the final save attempts finish.
        if run_error.is_none() || !self.native_lifecycle.is_opening() {
            self.native_lifecycle.begin_close();
        }
        self.native_configs_server_stop();
        self.native_players_leaving();
        // IntegratedServer.stopServer: teardownPublishedState.
        self.stop_serving();
        // PlayerList.removeAll: the owner and every guest leave the game.
        if !crashed {
            self.award_leave_game_to_all();
            self.save_leaving_guests();
        }
        if let Some(host) = self.lan.as_mut() {
            let conns: Vec<u32> = host.hub.guests.iter().map(|g| g.conn).collect();
            for conn in conns {
                host.hub.kick(
                    conn,
                    translatable("multiplayer.disconnect.server_shutdown", &[]),
                );
            }
            let has_remote_guests = host.hub.guests.iter().any(|g| g.conn != 0);
            if has_remote_guests {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        self.finish_server_save(crashed, run_error)
    }
}

/// What the world options screen shows of an integrated server.
#[derive(Clone, Copy, Debug)]
pub(super) struct ServerOptions {
    pub default_mode: i32,
    /// The owner's own game mode.
    pub personal_mode: i32,
    pub cheats: bool,
    pub difficulty: u8,
    pub hardcore: bool,
    pub difficulty_locked: bool,
    pub port: Option<u16>,
    pub guest_commands: bool,
    pub force: bool,
}

/// The server thread's side.
impl Game<'_> {
    /// IntegratedServer.teardownPublishedState, also reached when a shutdown
    /// callback unwinds before the normal stop path can retire the server.
    pub(super) fn stop_serving(&mut self) {
        if let Some(s) = self.serving.take() {
            s.flags.published.store(0, Ordering::Relaxed);
        }
    }

    /// Whether this game is an integrated server's.
    pub(super) fn serving(&self) -> bool {
        self.serving.is_some()
    }
    /// The window's agent runs a command as single player's console does
    /// (`as_console`, as without a server thread): as the world's owner, in
    /// the owner's level, at the owner permission level. The feedback goes to
    /// the owner's chat and is returned, with the command's outcome
    /// (`recording_command_outcome`; None when nothing ran).
    pub(super) fn owner_console_command(
        &mut self,
        command: &str,
    ) -> (Vec<String>, Option<(bool, i32)>) {
        let Some(owner) = self.serving.as_ref().map(|s| s.owner) else {
            return (Vec::new(), None);
        };
        let Some(host) = self.lan.as_ref() else {
            return (Vec::new(), None);
        };
        let mut out = Vec::new();
        let mut outcome = None;
        if let Some(g) = host.hub.guests.iter().find(|g| g.uuid == owner && g.placed) {
            let conn = g.conn;
            // Commands.performCommand runs in the player's level.
            let (back, theirs) = (self.dim, super::Dim::from_id(&g.dimension));
            let entered = theirs == back || self.enter_level(theirs).is_ok();
            if entered
                && let Some((lines, ran)) = self.with_guest_swap(conn, back, |game| {
                    let command = command.strip_prefix('/').unwrap_or(command).to_owned();
                    game.recording_command_outcome(|game| {
                        game.as_console(|g| {
                            g.chat_capture = Some(Vec::new());
                            // A command a native mod registered goes to the mods
                            // first, as the owner's own chat does (the console is
                            // level 4, so every registered usage is allowed).
                            if !g.perform_native_command(conn, 4, command.trim()) {
                                g.run_command(&command);
                            }
                            g.chat_capture.take().unwrap_or_default()
                        })
                    })
                })
            {
                outcome = ran;
                if let Some(host) = self.lan.as_mut() {
                    host.hub.sync_all_effects();
                }
                // A teleport into another level happens before its feedback is
                // sent, and what the command earned is announced before it too.
                self.run_level_teleports();
                self.announce_guest_advancements();
                for line in lines {
                    out.push(line.plain());
                    if let Some(host) = self.lan.as_ref() {
                        host.hub.send_to(
                            conn,
                            &sunrise_protocol::packets::play::Clientbound::SystemChat(
                                sunrise_protocol::packets::play::SystemChat {
                                    content: sunrise_client::lan::data::component_nbt(
                                        &line.component(),
                                    ),
                                    overlay: false,
                                },
                            ),
                        );
                    }
                }
                self.send_admin_feedback(Some(conn));
            }
            if theirs != back
                && let Err(e) = self.enter_level(back)
            {
                log::error!("could not reopen {}: {e}", back.id());
            }
        }
        (out, outcome)
    }

    /// Run `f` with the owner's level in place (the server holds one per
    /// dimension), then put back the level that was.
    pub(super) fn in_owner_level<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let owner = self.serving.as_ref().map(|s| s.owner);
        let dim = self.lan.as_ref().and_then(|h| {
            let g = h.hub.guests.iter().find(|g| Some(g.uuid) == owner)?;
            Some(super::Dim::from_id(&g.dimension))
        });
        let back = dim.and_then(|d| self.enter_command_level(d));
        let out = f(self);
        if let Some(back) = back {
            self.enter_command_level(back);
        }
        out
    }

    /// The window's requests, between two ticks.
    fn run_tasks(&mut self) {
        let requests: Vec<OwnerRequest> = match &self.serving {
            Some(s) => s.tasks.try_iter().collect(),
            None => return,
        };
        for request in requests {
            self.serve_owner_request(request);
            self.apply_publish();
        }
    }
    /// Whether anyone but a single-player owner can join this world: any hub of a client or
    /// a dedicated server, an integrated server's only once published to LAN.
    pub(super) fn open_to_others(&self) -> bool {
        self.lan.is_some() && (!self.serving() || self.published_port().is_some())
    }
    /// `IntegratedServer.getPort` while published.
    pub(super) fn published_port(&self) -> Option<u16> {
        let s = self.serving.as_ref()?;
        Some(s.flags.published.load(Ordering::Relaxed)).filter(|&p| p != 0)
    }
    /// `IntegratedServer.publishServer`: listen on `settings.port` (0 picks
    /// one) and announce the world. The socket is bound at once so a busy
    /// port fails here; the hub starts listening once it is at hand.
    pub(super) fn publish(&mut self, settings: LanSettings) -> std::result::Result<u16, String> {
        if self.options.developer_single_player {
            return Err(sunrise_client::lang::get(
                "sunrise.developer_offline.single_player",
            ));
        }
        let Some(s) = self.serving.as_mut() else {
            return Err("not an integrated server".into());
        };
        if s.flags.published.load(Ordering::Relaxed) != 0 {
            return Err("already published".into());
        }
        let listener = Lan::bind(None, settings.port).map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        log::info!("Published LAN server on port {port}");
        s.flags.published.store(port, Ordering::Relaxed);
        s.pending = Some(Publish::Open(listener, LanSettings { port, ..settings }));
        self.apply_publish();
        Ok(port)
    }
    /// `IntegratedServer.unpublishServer`: stop listening and send every
    /// player but the owner away. False when it was not published.
    pub(super) fn unpublish(&mut self) -> bool {
        let Some(s) = self.serving.as_mut() else {
            return false;
        };
        if s.flags.published.swap(0, Ordering::Relaxed) == 0 {
            return false;
        }
        s.pending = Some(Publish::Close);
        self.apply_publish();
        true
    }
    /// IntegratedServer.tickServer: the server follows the window's render
    /// and simulation distances when they change.
    fn follow_distances(&mut self, flags: &Flags) {
        // IntegratedServer.getScaledTrackingDistance reads the option each time.
        self.settings.entity_distance =
            f64::from_bits(flags.entity_distance.load(Ordering::Relaxed));
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let view = flags.render_distance.load(Ordering::Relaxed).max(2);
        let old = host.hub.config.view_distance;
        if view != old {
            log::info!("Changing view distance to {view}, from {old}");
            host.hub.set_view_distance(view);
            self.settings.render_distance = view;
        }
        let simulation = flags.simulation_distance.load(Ordering::Relaxed).max(2);
        let old = host.hub.config.simulation_distance;
        if simulation != old {
            log::info!("Changing simulation distance to {simulation}, from {old}");
            host.hub.set_simulation_distance(simulation);
            self.settings.simulation_distance = simulation;
        }
    }

    fn apply_publish(&mut self) {
        if self.lan.is_none() {
            return;
        }
        let Some(s) = self.serving.as_mut() else {
            return;
        };
        let (Some(pending), owner) = (s.pending.take(), s.owner) else {
            return;
        };
        let allow_commands = self.allow_commands();
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        match pending {
            Publish::Open(listener, settings) => {
                if let Err(e) = host.hub.lan.listen_on(listener, true) {
                    log::error!("Sunrise: Open to LAN: {e}");
                }
                host.settings = settings;
                host.hub.config.force_game_mode = settings.force_game_mode;
                // IntegratedServer.setGuestCommandAccess, then updatePermissions.
                host.hub.config.guest_commands = settings.guest_commands && allow_commands;
                host.hub.resend_commands();
            }
            Publish::Close => {
                host.hub.lan.stop_listening();
                log::info!(
                    "Unpublishing integrated server (was on port {})",
                    host.settings.port
                );
                let others: Vec<u32> = host
                    .hub
                    .guests
                    .iter()
                    .filter(|g| g.uuid != owner)
                    .map(|g| g.conn)
                    .collect();
                for conn in others {
                    host.hub.kick(
                        conn,
                        translatable("multiplayer.disconnect.server_shutdown", &[]),
                    );
                }
            }
        }
    }
    /// The world options screen's state, read on the server.
    pub(super) fn server_options(&self) -> ServerOptions {
        let owner = self.serving.as_ref().map(|s| s.owner);
        let host = self.lan.as_ref();
        let personal_mode = host
            .and_then(|h| h.hub.guests.iter().find(|g| Some(g.uuid) == owner))
            .map_or(self.level.mode(), |g| g.game_mode);
        let published = self.published_port();
        ServerOptions {
            default_mode: self.level.mode(),
            personal_mode,
            cheats: self.allow_commands(),
            difficulty: self.ui.difficulty,
            hardcore: self.level.hardcore,
            difficulty_locked: self.level.difficulty_locked,
            port: published,
            guest_commands: published.is_some() && host.is_some_and(|h| h.settings.guest_commands),
            force: published.is_none() || host.is_some_and(|h| h.settings.force_game_mode),
        }
    }
    /// `IntegratedServer.setPersonalGameType`: the owner's game mode.
    pub(super) fn set_owner_mode(&mut self, mode: i32) {
        let Some(owner) = self.serving.as_ref().map(|s| s.owner) else {
            return;
        };
        if let Some(host) = self.lan.as_mut()
            && let Some(conn) = host
                .hub
                .guests
                .iter()
                .find(|g| g.uuid == owner)
                .map(|g| g.conn)
        {
            host.hub
                .set_game_mode(conn, mode, &self.world, Some(self.collision));
        }
    }
    /// `IntegratedServer.setWorldAllowCommands`: the owner is an operator
    /// while the world allows commands; everyone's level is sent again.
    pub(super) fn owner_allow_commands(&mut self, allow: bool) {
        let Some(owner) = self.serving.as_ref().map(|s| s.owner) else {
            return;
        };
        let published = self.published_port().is_some();
        if let Some(host) = self.lan.as_mut() {
            let level = if allow { 4 } else { 0 };
            host.hub.ops.insert(owner, level);
            if published {
                host.hub.config.guest_commands = allow && host.settings.guest_commands;
            }
            host.hub.resend_commands();
        }
    }
}

/// The server thread: a game of its own over the shared resources.
#[allow(clippy::too_many_arguments)]
fn serve(
    res: &Arc<Resources>,
    pack: PackResources,
    options: Options,
    settings: Settings,
    launch: Launch,
    flags: Arc<Flags>,
    tasks: Receiver<OwnerRequest>,
    ready: &Sender<Ready>,
) -> Result<()> {
    // The world prepares, and the registries the server sends are read, on
    // helper threads while this one builds the server's own Game: none of
    // them reads what another makes (`integrated_open` takes them up).
    let world = WorldRequest::new(
        &launch.folder,
        launch.name.as_deref(),
        launch.seed,
        launch.creative,
        launch.create.as_ref(),
        launch.rule_registry.clone(),
    )
    // `IntegratedServer.setSingleplayerProfile`: the account this world belongs to
    // (an older world's offline owner moves to it at open).
    .and_then(|mut request| {
        request.chunk_reports = flags.reports.clone();
        WorldLoader::start(
            request.owned_by(&launch.player, launch.owner),
            res.clone(),
            pack.clone(),
            options.clone(),
            settings.clone(),
            flags.upgrade.clone(),
        )
    });
    let opening = Opening {
        world,
        sync: preload_sync_data(&res.core.item_data, res.shared_items()),
    };
    let mut server = match startup::time("server.game_new", || {
        server_game(res, pack, options, settings)
    }) {
        Ok(server) => server,
        Err(e) => {
            // The world's session lock is let go before the failure is told.
            if let Ok(world) = opening.world {
                drop(world.finish());
            }
            return Err(e);
        }
    };
    server.run_integrated(launch, flags, tasks, ready, opening)
}

/// What `serve` starts beside the server's own Game.
struct Opening {
    world: Result<WorldLoader>,
    sync: Option<SyncPreload>,
}

/// Component oracles supply their own synthetic levels and entity phases.
/// They still execute on a server Game, never the client's world.
pub(super) enum ComponentHarness {
    Dispenser(String),
    Sculk(String),
    /// The hot-path benchmark (`hot_paths`), which builds its own levels the same way.
    HotPaths(String),
}

impl Game<'_> {
    pub(super) fn run_component_harness(&self, fixture: ComponentHarness) -> Result<()> {
        let (res, pack, settings) = (self.res.clone(), self.pack.clone(), self.settings.clone());
        let mut options = self.options.clone();
        options.execution_role = ExecutionRole::IntegratedServer;
        options.dedicated = Some(Vec::new());
        let result = std::thread::Builder::new()
            .name(sunrise_client::crash::INTEGRATED_SERVER_THREAD.into())
            .stack_size(server_thread_stack())
            .spawn(move || {
                let run = || -> Result<()> {
                    let mut server = server_game(&res, pack, options, settings)?;
                    match fixture {
                        ComponentHarness::Dispenser(spec) => server.dispense_fixture(&spec),
                        ComponentHarness::Sculk(spec) => server.sculk_fixture(&spec),
                        ComponentHarness::HotPaths(output) => server.hot_paths(&output),
                    }
                };
                run().map_err(|e| e.to_string())
            })?
            .join()
            .map_err(|_| "the component harness server panicked")?;
        result.map_err(Into::into)
    }
}

fn server_game(
    res: &Arc<Resources>,
    pack: PackResources,
    options: Options,
    settings: Settings,
) -> Result<Game<'_>> {
    let gui = UiResources::server(&pack.source)?;
    let audio = Audio::silent(&pack.source);
    Game::new(res, pack, gui, options, settings, audio, None)
}

/// Who the integrated server turns away: anyone but the owner until it is opened to the LAN.
/// The owner joins only through memory. A host without a Microsoft account opens the world in
/// offline mode, where a
/// player's UUID comes from their name alone, so a LAN or friends player naming themselves after
/// the host would otherwise get the host's UUID, kick the host and take their operator rights and
/// player data (vanilla always authenticates LAN guests, so there it can't happen).
fn refused(owner: bool, published: bool, memory: bool) -> bool {
    if owner { !memory } else { !published }
}

impl Game<'_> {
    /// The server thread has already delivered its report: written to disk, or printed
    /// when it couldn't be written. The flag is set either way, since a dead server must
    /// not keep the game open. Join through the normal halt path, so a tick-loop crash
    /// finishes the server's own stop and save first, then exit without returning an
    /// error that would create another report. A panic inside a posted task happens
    /// outside the tick loop's catch, so the join doesn't promise a save there.
    pub(super) fn exit_after_server_crash(&mut self) {
        if !sunrise_client::crash::server_crashed() {
            return;
        }
        // A crash while the world is still loading leaves the server in `integrated_starting`.
        let starting = self
            .integrated_starting
            .take()
            .map(|starting| starting.handle);
        for mut server in self.integrated.take().into_iter().chain(starting) {
            if let Err(error) = server.halt() {
                log::error!("Sunrise: crashed integrated server: {error}");
            }
        }
        std::process::exit(101);
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use mod_api::prelude::*;

    #[test]
    #[ignore = "daily"]
    fn posted_native_panic_writes_one_server_report() {
        let scratch = tempfile::tempdir().unwrap();
        let home = scratch.path().to_path_buf();
        let previous = std::panic::take_hook();
        sunrise_client::crash::install(home.clone(), "client", || None);
        let (tasks, receiver) = channel::<OwnerRequest>();
        let mut server = Integrated {
            emergency_save: None,
            flags: Arc::new(Flags::default()),
            thread: None,
            tasks,
            world_name: String::new(),
            save_folder: String::new(),
            rule_registry: Arc::new(sunrise_client::gamerules::RuleRegistry::vanilla()),
        };
        assert!(server.post(|_| {
            let _guard = sunrise_client::crash::enter_native_mod(sunrise_client::crash::ModCall {
                id: "example:posted",
                hook: "server_tick",
                part: None,
            });
            panic!("posted integrated panic 3494");
        }));
        server.thread = Some(
            std::thread::Builder::new()
                .name(sunrise_client::crash::INTEGRATED_SERVER_THREAD.into())
                .stack_size(server_thread_stack())
                .spawn(move || {
                    crate::game::native_entity_refs::tests::with_game(|game| {
                        game.serve_owner_request(receiver.recv().unwrap());
                    });
                })
                .unwrap(),
        );
        assert_eq!(
            server.halt().unwrap_err().to_string(),
            "the integrated server panicked"
        );
        std::panic::set_hook(previous);
        let reports: Vec<_> = std::fs::read_dir(home.join("crash-reports"))
            .unwrap()
            .map(|p| p.unwrap().path())
            .collect();
        assert_eq!(reports.len(), 1);
        // `MinecraftServer.runServer` saves a server report even under a client install.
        let file = reports[0].file_name().unwrap().to_string_lossy();
        assert!(file.ends_with("-server.txt"), "{file}");
        let text = std::fs::read_to_string(&reports[0]).unwrap();
        assert!(text.contains("Description: Exception in server tick loop"));
        assert!(text.contains("Server thread"));
        assert!(text.contains("posted integrated panic 3494"));
        assert!(text.contains("in native mod example:posted, hook server_tick"));
    }

    /// The integrated server keeps its stack floor and honors a larger
    /// `RUST_MIN_STACK` override (#2430).
    #[test]
    fn integrated_server_stack_honors_its_floor_and_larger_overrides() {
        let host_stack = 8 * 1024 * 1024;
        assert_eq!(super::stack_for(None), host_stack);
        assert_eq!(
            super::stack_for(Some("1048576")),
            host_stack,
            "smaller values keep the integrated server floor"
        );
        assert_eq!(super::stack_for(Some("16777216")), 16 * 1024 * 1024);
        assert_eq!(
            super::stack_for(Some("lots")),
            host_stack,
            "an unparsed value is ignored"
        );
    }

    /// Observe both teardownPublishedState and the dropped server task queue.
    pub(in crate::game) fn install_serving(
        game: &mut Game<'_>,
        published: bool,
    ) -> impl Fn() -> bool + Send + Sync + 'static {
        let flags = Arc::new(Flags::default());
        flags
            .published
            .store(u16::from(published), Ordering::Relaxed);
        let (tasks, receiver) = channel();
        game.serving = Some(Serving {
            flags: flags.clone(),
            tasks: receiver,
            owner: 0,
            pending: None,
            load: LoadPhase::default(),
        });
        move || {
            // Pregen is off in tests, so a probe the server does read changes nothing.
            flags.published.load(Ordering::Relaxed) == 0
                && tasks.send(OwnerRequest::SetPregen(false)).is_err()
        }
    }

    #[test]
    #[ignore = "daily"]
    fn integrated_rules_screen_shares_server_session_registry() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let registry = Arc::new(sunrise_client::gamerules::RuleRegistry::with_categories(&[
                "example:machines".into(),
            ]));
            let request =
                WorldRequest::new("test", None, None, true, None, registry.clone()).unwrap();
            let (tasks, _receiver) = channel();
            game.integrated = Some(Integrated {
                emergency_save: None,
                flags: Arc::new(Flags::default()),
                thread: None,
                tasks,
                world_name: "test".into(),
                save_folder: "test".into(),
                rule_registry: registry.clone(),
            });
            assert!(Arc::ptr_eq(
                &request.rule_registry,
                &game.integrated.as_ref().unwrap().rule_registry
            ));
            assert!(game.test_open_remote_rules(Vec::new()));
            assert!(
                request
                    .rule_registry
                    .categories()
                    .contains(&"example:machines".to_owned())
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_state_weathering_reverts_after_integrated_session_stops() {
        use mod_api::HostBlockStates;
        use sunrise_world::level::{BlockMapValue, BlockMaps};

        super::super::native_entity_refs::tests::with_client_game(|window| {
            window.join_world_rest().unwrap();
            let state = |name| BlockStatesHost::block_state_id(window, name).unwrap();
            let copper = state("minecraft:copper_block");
            let exposed = state("minecraft:exposed_copper");
            let oxidized = state("minecraft:oxidized_copper");
            let next = || window.block_states().next_weathering_stage(copper);
            assert_eq!(next(), Some(exposed));

            for failed_open in [false, true] {
                let mut options = window.options.clone();
                options.dedicated = Some(Vec::new());
                options.execution_role = ExecutionRole::IntegratedServer;
                let mut server = server_game(
                    &window.res,
                    window.pack.clone(),
                    options,
                    window.settings.clone(),
                )
                .unwrap();
                server.save = window.save.clone();
                let result = server.run_server_lifecycle(false, |server, began| {
                    *began = true;
                    // Model publication at open_world_tables, before later steps can fail.
                    let maps = BlockMaps::from_values(
                        [(
                            "minecraft:copper_block",
                            BlockMapValue::Oxidizable("minecraft:oxidized_copper".into()),
                        )],
                        |_| 0,
                    );
                    server.res.behaviours().set_block_maps(Some(Arc::new(maps)));
                    assert_eq!(next(), Some(oxidized));
                    if failed_open {
                        Err("open failed after map publication".into())
                    } else {
                        server.world_adopted = true;
                        Ok(())
                    }
                });
                if failed_open {
                    assert_eq!(
                        result.unwrap_err().to_string(),
                        "open failed after map publication"
                    );
                } else {
                    result.unwrap();
                }
                // The window and its resource tables outlive both kinds of session.
                assert_eq!(next(), Some(exposed), "failed_open = {failed_open}");
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn forced_disconnect_keeps_wire_cache_until_integrated_server_stops() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_client_game};
        use sunrise_client::{
            lan::data::{SyncData, WorldTags},
            text::Text,
        };

        if run_fresh_cases(
            "game::integrated::tests::forced_disconnect_keeps_wire_cache_until_integrated_server_stops",
            "SUNRISE_INTEGRATED_DISCONNECT_SYNC_CHILD",
            &["disconnect"],
        ) {
            return;
        }
        with_client_game(|game| {
            for failure in [None, Some("test shutdown failure")] {
                let sync = Arc::new(SyncData::empty());
                super::super::lan::sync::install_wire_reload(
                    &game.options.resources,
                    &game.res.core.item_data,
                    &game.pack_files(),
                    Some(sync.clone()),
                    game.adv.tree.clone(),
                    WorldTags::new(game.res.shared_items(), &game.health_resources.damage),
                );
                let flags = Arc::new(Flags::default());
                let stopping = flags.clone();
                let (tasks, _waiting_server) = channel();
                let (observed, probe) = channel();
                game.integrated = Some(Integrated {
                    emergency_save: None,
                    flags,
                    thread: Some(std::thread::spawn(move || {
                        let deadline = Instant::now() + Duration::from_secs(5);
                        while !stopping.stop.load(Ordering::Relaxed) {
                            assert!(Instant::now() < deadline, "stop was never requested");
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        observed
                            .send(super::super::lan::sync::cached_reload_sync())
                            .unwrap();
                        *stopping.failure.lock().unwrap() = failure.map(str::to_owned);
                    })),
                    tasks,
                    world_name: "fixture".into(),
                    save_folder: "fixture".into(),
                    rule_registry: Arc::new(sunrise_client::gamerules::RuleRegistry::vanilla()),
                });
                let result = game.leave_server(Some(Text::literal("test disconnect")));
                assert_eq!(
                    result.err().map(|error| error.to_string()),
                    failure.map(str::to_owned)
                );
                assert!(
                    probe
                        .try_recv()
                        .unwrap()
                        .is_some_and(|seen| Arc::ptr_eq(&seen, &sync)),
                    "the cache must still hold the server's snapshot during shutdown"
                );
                assert!(super::super::lan::sync::cached_reload_sync().is_none());
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn client_session_identifies_only_its_own_integrated_server() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let _pipe = super::super::native_client_session::tests::logged_client(game);
            let (tasks, _waiting_server) = channel();
            game.integrated = Some(Integrated {
                emergency_save: None,
                flags: Arc::new(Flags::default()),
                thread: None,
                tasks,
                world_name: "First world".into(),
                save_folder: "first_world".into(),
                rule_registry: Arc::new(sunrise_client::gamerules::RuleRegistry::vanilla()),
            });
            let local = game.client_session().client_server().unwrap();
            assert!(local.integrated_local);
            assert_eq!(local.name, Some("First world"));
            assert_eq!(local.address, None);
            assert_eq!(local.save_folder, Some("first_world"));
            game.integrated.as_mut().unwrap().world_name = "Renamed world".into();
            let renamed = game.client_session().client_server().unwrap();
            assert_eq!(renamed.name, Some("Renamed world"));
            assert_eq!(renamed.save_folder, Some("first_world"));
            game.integrated.take();
            let guest = game.client_session().client_server().unwrap();
            assert!(!guest.integrated_local);
            assert_eq!(guest.name, None);
            assert_eq!(guest.save_folder, None);
            assert_eq!(guest.address, None);
        });
    }

    #[derive(Debug, PartialEq, Eq)]
    enum CheckpointEvent {
        LevelSaved,
        WorldSaved,
        LevelUnloaded,
        Closed(mod_api::WorldPhase, String),
    }

    struct CheckpointMod(Arc<Mutex<Vec<CheckpointEvent>>>);

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for CheckpointMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let kind = r
                .block_entity_type("counter")
                .blocks(&["crash_checkpoint:counter"])
                .build();
            r.block_entity_part(&kind, CheckpointCounter).unwrap();
        }

        fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
            event
                .register(
                    mod_api::Block::new("crash_checkpoint:counter", "minecraft:barrel")
                        .with_item(false),
                )
                .unwrap();
        }

        fn level_save(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::LevelSaveEvent<'_>,
        ) {
            if event.dimension == Dim::Overworld.id() {
                self.0.lock().unwrap().push(CheckpointEvent::LevelSaved);
            }
        }

        fn world_saved(&self, _: &mut dyn mod_api::ServerHost, _: &mut mod_api::WorldSavedEvent) {
            self.0.lock().unwrap().push(CheckpointEvent::WorldSaved);
        }

        fn level_unload(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::LevelUnloadEvent<'_>,
        ) {
            if event.dimension == Dim::Overworld.id() {
                self.0.lock().unwrap().push(CheckpointEvent::LevelUnloaded);
            }
        }

        fn world_lifecycle(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::WorldLifecycleEvent<'_>,
        ) {
            use mod_api::WorldPhase::{WorldCloseFailed, WorldCloseFinal};
            if matches!(event.phase, WorldCloseFailed | WorldCloseFinal) {
                self.0.lock().unwrap().push(CheckpointEvent::Closed(
                    event.phase,
                    event.error_text.to_owned(),
                ));
            }
        }
    }

    struct CheckpointCounter;
    #[mod_api::sunrise_part]
    impl mod_api::BlockPart for CheckpointCounter {}
    #[mod_api::sunrise_part]
    impl mod_api::BlockEntityPart for CheckpointCounter {
        fn block_entity_save_additional(
            &self,
            host: &dyn mod_api::LevelHost,
            event: &mut mod_api::BlockEntitySaveAdditionalEvent<'_>,
        ) {
            let entity = host
                .block_entity(event.pos)
                .expect("counter is still resident");
            let data = host
                .block_entity_data(entity)
                .expect("counter handle is live");
            event.tag.insert(
                "saved_progress".into(),
                data.get(&"progress".into()).unwrap().clone(),
            );
        }
    }

    /// Install the mod before building the fixture's frozen block and block entity registries.
    fn with_checkpoint_mod(test: impl FnOnce(&mut Game<'_>, &Mutex<Vec<CheckpointEvent>>)) {
        use crate::game::native_entity_refs::tests::{fixture_root, with_game_at};

        let events = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "crash_checkpoint",
            Box::new(CheckpointMod(events.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            crate::game::api_values::install_parts(&mods).unwrap();
            mod_api::__private::mark_game_thread();
            let resources =
                crate::game::mods::content_root(&fixture_root(), &Default::default()).unwrap();
            with_game_at(resources.prepared().to_path_buf(), |game| {
                test(game, &events)
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn integrated_tick_panic_saves_changed_world() {
        use crate::game::{
            native_entity_refs::tests::run_fresh_cases,
            native_living_refs::tests::{ZOMBIE_UUID, hurt_pig, send_to_the_nether, summon_mob},
        };
        use sunrise_client::{
            chunk::Chunk,
            containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields},
            save::{LevelShape, SaveDir, SessionLock},
        };
        use sunrise_core::nbt::Tag;

        const NAME: &str = "game::integrated::tests::integrated_tick_panic_saves_changed_world";
        // The mod's block and block entity declarations freeze process-wide registries.
        if run_fresh_cases(NAME, "SUNRISE_CRASH_CHECKPOINT_CHILD", &["crash"]) {
            return;
        }
        with_checkpoint_mod(|game, events| {
            let scratch = tempfile::tempdir().unwrap();
            let root = scratch.path();
            let open = || {
                SaveDir::new(
                    root.to_path_buf(),
                    LevelShape::OVERWORLD,
                    Some(Default::default()),
                )
            };
            game.world_root = root.to_path_buf();
            game.save = Arc::new(open());
            // Crash shutdown only saves an adopted world whose storage is owned.
            game.session_lock = Some(SessionLock::acquire(root).unwrap());
            game.world_adopted = true;
            game.native_level_opened(Dim::Overworld);
            game.native_lifecycle_world_ready();

            if !game.world.chunks.contains_key(&(0, 0)) {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            }
            let (victim, attacker) = hurt_pig(game);
            send_to_the_nether(game, attacker);
            let bystander = summon_mob(game, "minecraft:pig", "{NoAI:1b,UUID:[I;21,22,23,24]}");
            let victim_uuid = Tag::IntArray(game.persist.uuid_of(victim).unwrap().to_vec());
            let bystander_uuid = Tag::IntArray(game.persist.uuid_of(bystander).unwrap().to_vec());
            let slot = game.mobs.iter().position(|m| m.id == victim).unwrap();
            let ticks_since_hurt = 17;
            assert!(game.mobs[slot].ai.last_hurt_by_ref.is_none());
            game.mobs.update(slot, |m| {
                m.age = (m.ai.last_hurt_by_time + ticks_since_hurt) as u32;
            });
            // The victim joined first, but re-enters the bystander's section last.
            let at = game.mobs[slot].position;
            let outside = sunrise_core::geometry::Vec3::new(at.x, 50.0, at.z);
            for (from, to) in [(at, outside), (outside, at)] {
                game.mobs.update(slot, |m| m.position = to);
                game.entity_section_step(victim, [from.x, from.y, from.z], [to.x, to.y, to.z]);
            }
            let living = game.living(game.entity(victim).unwrap()).unwrap();
            assert_eq!(
                living.last_attacker(game).unwrap().level(game),
                game.level(Dim::Nether.id())
            );

            let block = game
                .pack
                .assets
                .registry
                .default_state("crash_checkpoint:counter")
                .unwrap();
            game.world.set(1, 64, 1, block).unwrap();
            game.world.block_entities.insert(
                (1, 64, 1),
                BlockEntity::Mod(Box::new(ModBlockEntity::new(
                    "crash_checkpoint:counter".into(),
                    ModBlockEntityFields {
                        nbt: [("progress".into(), Tag::Int(41))].into(),
                        ..Default::default()
                    },
                ))),
            );
            game.player = Some(
                sunrise_client::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::new(8.5, 64.0, 8.5),
                )
                .unwrap(),
            );
            // The owner's file as last saved, with an experience total the panicking tick's
            // state (37) differs from.
            game.level.xp_total = 11;
            game.level.write_player(root, game.res.items()).unwrap();
            let owner_file = std::fs::read_dir(root.join("players").join("data"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let owner_xp = || {
                let player = sunrise_client::save::read_dat(&owner_file)?;
                match player.get("XpTotal") {
                    Some(sunrise_core::nbt::Tag::Int(xp)) => Some(*xp),
                    _ => None,
                }
            };
            assert_eq!(owner_xp(), Some(11));
            let flags = Arc::new(Flags::default());
            flags.fail_tick.store(true, Ordering::Relaxed);
            flags.published.store(1234, Ordering::Relaxed);
            let (_sender, tasks) = channel();
            game.serving = Some(Serving {
                flags: flags.clone(),
                tasks,
                owner: 1,
                pending: None,
                load: LoadPhase::default(),
            });
            events.lock().unwrap().clear();
            let error = game.integrated_tick_loop(&flags).unwrap_err().to_string();
            assert!(error.contains("integrated tick fixture"));
            let saved = sunrise_client::save::Level::read(root).unwrap();
            assert_eq!(saved.name, "saved after integrated tick panic");
            // A fresh reader has no pending writes or captured payload to fall back to.
            let disk = open();
            let chunk = disk
                .raw_chunk(0, 0)
                .expect("crash checkpoint wrote the chunk");
            let Some(Tag::List(entities)) = chunk.tag.get("block_entities") else {
                panic!("crash checkpoint lost the block entities");
            };
            let counter = entities
                .iter()
                .find(|tag| tag.get("id") == Some(&Tag::String("crash_checkpoint:counter".into())))
                .expect("crash checkpoint lost the counter");
            assert_eq!(counter.get("saved_progress"), Some(&Tag::Int(41)));
            let entities = disk.load_entities(0, 0).unwrap();
            let ids: Vec<_> = entities.iter().map(|tag| tag.get("UUID")).collect();
            assert_eq!(ids, [Some(&bystander_uuid), Some(&victim_uuid)]);
            let victim = &entities[1];
            assert_eq!(
                victim.get("last_hurt_by_mob"),
                Some(&Tag::IntArray(ZOMBIE_UUID.to_vec()))
            );
            assert_eq!(
                victim.get("ticks_since_last_hurt_by_mob"),
                Some(&Tag::Int(ticks_since_hurt))
            );
            assert_eq!(
                *events.lock().unwrap(),
                [
                    CheckpointEvent::LevelSaved,
                    CheckpointEvent::WorldSaved,
                    CheckpointEvent::LevelUnloaded,
                    CheckpointEvent::Closed(mod_api::WorldPhase::WorldCloseFailed, error),
                ]
            );
            // No hub: players are not saved, so the owner's file keeps what it had.
            assert_eq!(owner_xp(), Some(11));
            assert_eq!(flags.published.load(Ordering::Relaxed), 0);
            assert!(game.session_lock.is_none());
        });
    }

    #[test]
    fn emergency_save_waits_for_finished_and_times_out() {
        let flags = Arc::new(Flags::default());
        let saving = flags.clone();
        let server = std::thread::spawn(move || {
            let _finished = Finished(saving.clone());
            while !saving.stop.load(Ordering::Relaxed) {
                std::thread::yield_now();
            }
        });
        let id = server.thread().id();
        assert!(flags.emergency_save(id, Duration::from_secs(2)));
        assert!(flags.finished.load(Ordering::Acquire));
        server.join().unwrap();
        assert!(flags.emergency_save(id, Duration::ZERO));
        let flags = Flags::default();
        let started = Instant::now();
        assert!(!flags.emergency_save(id, Duration::from_millis(20)));
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(flags.stop.load(Ordering::Relaxed));
        let flags = Flags::default();
        assert!(flags.emergency_save(std::thread::current().id(), Duration::ZERO));
        assert!(!flags.stop.load(Ordering::Relaxed));
    }

    #[test]
    fn pregen_toggle_back_is_queued_while_server_is_busy() {
        let (tasks, waiting_server) = channel();
        let server = Integrated {
            emergency_save: None,
            flags: Arc::new(Flags::default()),
            thread: None,
            tasks,
            world_name: String::new(),
            save_folder: String::new(),
            rule_registry: Arc::new(sunrise_client::gamerules::RuleRegistry::vanilla()),
        };
        // The receiver deliberately does not run either task between the updates. In particular,
        // the dispatcher can still show the old value when the owner toggles back.
        server.queue_pregen_option(true);
        server.queue_pregen_option(false);
        assert!(
            matches!(waiting_server.try_recv(), Ok(OwnerRequest::SetPregen(true))),
            "enable must reach the server"
        );
        assert!(
            matches!(
                waiting_server.try_recv(),
                Ok(OwnerRequest::SetPregen(false))
            ),
            "disable must follow it"
        );
        assert!(waiting_server.try_recv().is_err());
    }

    #[test]
    fn the_owner_joins_only_through_memory_and_lan_guests_only_when_published() {
        for published in [false, true] {
            assert!(
                !refused(true, published, true),
                "the owner joins through memory"
            );
            assert!(
                refused(true, published, false),
                "the owner's identity is reserved"
            );
            for memory in [false, true] {
                assert_eq!(refused(false, published, memory), !published);
            }
        }
    }

    #[test]
    fn shutdown_request_does_not_join_a_server_still_saving() {
        let flags = Arc::new(Flags::default());
        let (release, wait) = channel();
        let (tasks, _) = channel();
        let server = Integrated {
            emergency_save: None,
            flags: flags.clone(),
            thread: Some(std::thread::spawn(move || wait.recv().unwrap())),
            tasks,
            world_name: String::new(),
            save_folder: String::new(),
            rule_registry: Arc::new(sunrise_client::gamerules::RuleRegistry::vanilla()),
        };
        server.request_stop();
        assert!(flags.stop.load(Ordering::Relaxed));
        assert!(!server.stopped());
        release.send(()).unwrap();
        server.stop().unwrap();
    }

    #[test]
    fn shutdown_reports_a_failed_server_save() {
        let flags = Arc::new(Flags::default());
        let saving = flags.clone();
        let (tasks, _) = channel();
        let server = Integrated {
            emergency_save: None,
            flags,
            thread: Some(std::thread::spawn(move || {
                *saving.failure.lock().unwrap() = Some("disk write failed".into());
            })),
            tasks,
            world_name: String::new(),
            save_folder: String::new(),
            rule_registry: Arc::new(sunrise_client::gamerules::RuleRegistry::vanilla()),
        };
        assert_eq!(server.stop().unwrap_err().to_string(), "disk write failed");
    }
}
