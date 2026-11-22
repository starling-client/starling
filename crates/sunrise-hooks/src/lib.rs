//! The game's extension points for code above the engine (docs/layers.md, "How a Sunrise crate
//! plugs in"). A platform or Sunrise crate implements [`Hooks`] and [`register`]s it before the
//! game starts; the engine calls every registered hook at the points below. This crate names no
//! type from above, so the engine can offer it; `sunrise-platform` re-exports it as
//! `sunrise_platform::hooks`, which is where the Sunrise layer uses it from.
//!
//! Registration happens once, before the game runs: the list is sealed the first time the engine
//! dispatches, and a hook registered after that is refused. With nothing registered a dispatch
//! is one load and an empty loop.
//!
//! A new hook is a new method with an empty default body, so existing implementations keep
//! compiling, and a dispatch function in [`engine`] that the engine calls at the place vanilla
//! does the matching thing.
//!
//! The platform registries (docs/layers.md, "Platform registries") have the same shape:
//! [`settings`] for the settings each crate keeps in its own lines, [`pages`] for its settings
//! pages, [`hud`] for what it draws on the HUD, [`lang`] for the lang files with its text,
//! [`mod_rows`] for the rows it adds to the Mods screen, [`title`] for the buttons it adds to
//! the title screen.
#![forbid(unsafe_code)]

pub mod connection;
pub mod containers;
pub mod hud;
pub mod lang;
pub mod look;
pub mod mod_rows;
pub mod pages;
pub mod perf;
mod registry;
pub mod settings;
pub mod sound;
pub mod title;
pub mod tunnel;

use registry::Registry;
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

/// How the client presents itself to the server it joins: its brand and the custom payloads it
/// sends in the configuration phase, after the brand, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientIdentity {
    /// The `minecraft:brand` (`ClientBrandRetriever.getClientModName`).
    pub brand: String,
    /// Configuration-phase custom payloads: channel and data.
    pub payloads: Vec<(String, Vec<u8>)>,
}

impl ClientIdentity {
    /// Vanilla's: the brand `vanilla`, no payloads.
    pub fn vanilla() -> Self {
        Self {
            brand: "vanilla".to_owned(),
            payloads: Vec::new(),
        }
    }
}

/// What a crate above the engine hears from the game. Every method has an empty default:
/// implement only those you need. Hooks run on the game's thread (the render thread) unless a
/// method says otherwise; they must return quickly and must not panic.
pub trait Hooks: Sync {
    /// The game started for someone playing it: once, after the log and the crash handler, before
    /// the first frame. Not for a dedicated server, a benchmark, an offscreen run or a run that
    /// quits by itself or that sunrise-ctl drives (those start in a source checkout).
    fn started(&self, _game: &Started<'_>) {}

    /// The world this window plays was opened to LAN or to friends (`true`), or no longer is
    /// (`false`). Called when it changes, not every frame.
    fn published(&self, _open: bool) {}

    /// The game is ending normally: its window closed and the world is saved. Not after a crash.
    fn shutdown(&self) {}

    /// The resolver of `friend:` addresses this crate offers ([`tunnel`]); the first hook that
    /// offers one is used. Asked on the network thread when joining and on the game thread when
    /// an address is shown, never per frame.
    fn tunnel(&self) -> Option<&dyn tunnel::Resolver> {
        None
    }

    /// Opens and shares saved performance captures ([`perf`]); the first hook that offers one is used.
    fn captures(&self) -> Option<&dyn perf::Captures> {
        None
    }

    /// What this crate does on a container screen ([`containers`]); the engine dispatches only to
    /// hooks that offer one, and builds nothing when none does.
    fn containers(&self) -> Option<&dyn containers::ContainerHooks> {
        None
    }

    /// Once a rendered frame is handed off for submission: how long it took
    /// ([`perf::FrameTimes`]), with the [`seat`] of the `Game` that drew it.
    fn frame_timed(&self, _times: &perf::FrameTimes) {}

    /// A game window opened: once, before the first frame, for every run that has a window,
    /// automated ones included ([`Client::automated`]); never for a dedicated server.
    fn client_started(&self, _client: &Client) {}

    /// What the player is doing, for a status shown outside the game (Discord's): called when it
    /// changes, not every frame.
    fn activity(&self, _now: Activity) {}

    /// Where a game started without its prepared files (run by hand, not from a launcher) sends
    /// the player for them: the screen's text and its button. The first hook that answers is
    /// used; with none, the screen says how to prepare the files and opens no page. Asked once,
    /// before that screen's window opens.
    fn get_the_game(&self) -> Option<GetTheGame> {
        None
    }

    /// How this client presents itself to the server it is joining: the brand it reports
    /// (`ClientBrandRetriever.getClientModName`, the `minecraft:brand` payload) and the custom
    /// payloads it sends in the configuration phase after it. `server` is the session target before
    /// SRV or DNS resolution: the trimmed address for a direct join, the supplied `host:port` for a
    /// transfer, the sender's IP and advertised port for an announced LAN world, or the `friend:`
    /// target for a friend join. The hook runs once on the network thread during initial
    /// configuration (a transfer starts a new session). The first answer wins; with none it sends
    /// [`ClientIdentity::vanilla`]. A local join bypasses the hook and uses that identity directly.
    fn client_identity(&self, _server: &str) -> Option<ClientIdentity> {
        None
    }

    /// The window starts a connection to `server` over the network, each attempt; never for its
    /// own server thread (single player, a splitscreen player joining it). Each such connection
    /// then closes in exactly one [`Hooks::left`] or [`Hooks::ended`] ([`connection`]).
    fn connecting(&self, _server: &connection::Server) {}

    /// The server the window is connecting to let the player in; at most once a connection.
    fn joined(&self) {}

    /// The window's connection closed without being lost: the player disconnected, the game
    /// closed, or the server transferred the window elsewhere.
    fn left(&self, _how: connection::Left) {}

    /// The window's connection ended without the player leaving: lost, refused or kicked, or
    /// dropped by this client ([`connection::By`]). The Disconnected screen shows next, and the
    /// engine asks [`Hooks::retry`].
    fn ended(&self, _end: &connection::Ended<'_>) {}

    /// Whether to connect again: asked once right after [`Hooks::ended`], and the engine acts on
    /// that answer; after [`connection::Retry::Wait`], asked again once a frame while the
    /// Disconnected screen still offers it (the player can cancel or leave the screen). The
    /// first hook that answers is used; with none the screen offers nothing, as vanilla's.
    fn retry(&self) -> Option<connection::Retry> {
        None
    }
}

/// What [`Hooks::get_the_game`] answers. The texts are translation keys of the game's own strings
/// (`assets/sunrise/lang`); a `%s` in a paragraph is [`address`](Self::address) without its
/// scheme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GetTheGame {
    /// The page the button opens.
    pub address: &'static str,
    pub title: &'static str,
    pub paragraphs: &'static [&'static str],
    /// The button's label.
    pub button: &'static str,
}

/// What [`Hooks::client_started`] gets.
#[non_exhaustive]
pub struct Client {
    /// A run nobody plays: offscreen, a benchmark, a run that quits by itself after some frames.
    pub automated: bool,
}

impl Client {
    pub fn new(automated: bool) -> Self {
        Self { automated }
    }
}

/// What the player is doing ([`Hooks::activity`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Activity {
    /// On the title screen or another menu outside a world.
    Menus,
    World {
        play: Play,
        dimension: Dimension,
        mode: GameMode,
    },
}

/// How the player is in a world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Play {
    Singleplayer,
    /// A server or someone else's LAN world.
    Multiplayer,
    /// Their world is open to LAN or friends; `players` counts the host, `max` is how many may
    /// join.
    Hosting {
        players: usize,
        max: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dimension {
    Overworld,
    Nether,
    End,
    /// A custom dimension, with no private identifier published.
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameMode {
    Survival,
    Creative,
    Adventure,
    Spectator,
}

/// What [`Hooks::started`] gets.
#[non_exhaustive]
pub struct Started<'a> {
    /// The build: the release tag (`0.2.31`), or `dev <crate version>` for a build from source.
    pub version: &'a str,
    /// The player's game folder: servers.dat, options.txt and saves (the working directory unless
    /// `--game-dir` names another).
    pub game_dir: &'a std::path::Path,
}

impl<'a> Started<'a> {
    pub fn new(version: &'a str, game_dir: &'a std::path::Path) -> Self {
        Self { version, game_dir }
    }
}

static HOOKS: Registry<dyn Hooks> = Registry::new();

/// Why [`register`] refused a hook.
#[derive(Debug, PartialEq, Eq)]
pub struct TooLate;

impl std::fmt::Display for TooLate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the game already started: hooks register before it runs")
    }
}

impl std::error::Error for TooLate {}

/// Adds `hooks` to those the game calls. Call it before the game starts (from the executable's
/// `main`, before it runs the game); once the engine has dispatched, the list is fixed and this
/// returns [`TooLate`]. Hooks are called in the order they registered.
pub fn register(hooks: &'static dyn Hooks) -> Result<(), TooLate> {
    HOOKS.register(hooks)
}

/// The registered hooks, sealing the list on the first call.
fn all() -> &'static [&'static dyn Hooks] {
    HOOKS.all()
}

thread_local! {
    static SEAT: Cell<usize> = const { Cell::new(0) };
}

/// The local-instance index of the `Game` whose call runs on this thread: 0 for the
/// process's first `Game`, and outside any `Game` call.
pub fn seat() -> usize {
    SEAT.get()
}

/// The engine's side: where it calls the hooks. Only the engine calls these.
pub mod engine {
    use super::*;

    /// Engine-only: sets this thread's [`seat`] for a `Game` call.
    pub fn set_seat(seat: usize) {
        SEAT.set(seat);
    }

    /// Seals every list (hooks and the registries); their first use does it anyway. Call where
    /// registration ends.
    pub fn seal() {
        all();
        crate::settings::all();
        crate::pages::engine::all();
        crate::hud::engine::all();
        crate::lang::all();
        crate::mod_rows::engine::all();
        crate::containers::engine::any();
        crate::title::engine::all();
    }

    /// [`Hooks::started`].
    pub fn started(game: &Started<'_>) {
        for hooks in all() {
            hooks.started(game);
        }
    }

    /// Whether the world was open to LAN or friends at the last call.
    static PUBLISHED: AtomicBool = AtomicBool::new(false);

    /// [`Hooks::published`], when `open` differs from the last call's. Cheap enough to call
    /// every frame.
    pub fn published(open: bool) {
        let hooks = all();
        if !hooks.is_empty() && PUBLISHED.swap(open, Ordering::Relaxed) != open {
            for h in hooks {
                h.published(open);
            }
        }
    }

    /// [`Hooks::client_started`].
    pub fn client_started(client: &Client) {
        for hooks in all() {
            hooks.client_started(client);
        }
    }

    /// The activity at the last call (`None` before the first).
    static ACTIVITY: std::sync::Mutex<Option<Activity>> = std::sync::Mutex::new(None);

    /// [`Hooks::activity`], when `now` differs from the last call's. Cheap enough to call every
    /// frame: with no hook registered it only loads the empty list.
    pub fn activity(now: Activity) {
        let hooks = all();
        if hooks.is_empty() {
            return;
        }
        let mut last = ACTIVITY.lock().unwrap_or_else(|e| e.into_inner());
        if last.replace(now) != Some(now) {
            drop(last);
            for h in hooks {
                h.activity(now);
            }
        }
    }

    /// [`Hooks::shutdown`].
    pub fn shutdown() {
        for hooks in all() {
            hooks.shutdown();
        }
    }

    /// [`Hooks::captures`]: the first registered hook's.
    pub fn captures() -> Option<&'static dyn perf::Captures> {
        all().iter().find_map(|hooks| hooks.captures())
    }

    /// [`Hooks::frame_timed`].
    pub fn frame_timed(times: &perf::FrameTimes) {
        for hooks in all() {
            hooks.frame_timed(times);
        }
    }

    /// [`Hooks::tunnel`]: the first registered hook's resolver.
    pub fn tunnel() -> Option<&'static dyn tunnel::Resolver> {
        all().iter().find_map(|hooks| hooks.tunnel())
    }

    /// [`Hooks::get_the_game`]: the first registered hook's answer.
    pub fn get_the_game() -> Option<GetTheGame> {
        all().iter().find_map(|hooks| hooks.get_the_game())
    }

    /// [`Hooks::client_identity`]: the first answer, or vanilla's.
    pub fn client_identity(server: &str) -> ClientIdentity {
        all()
            .iter()
            .find_map(|h| h.client_identity(server))
            .unwrap_or_else(ClientIdentity::vanilla)
    }

    /// [`Hooks::connecting`].
    pub fn connecting(server: &connection::Server) {
        for hooks in all() {
            hooks.connecting(server);
        }
    }

    /// [`Hooks::joined`].
    pub fn joined() {
        for hooks in all() {
            hooks.joined();
        }
    }

    /// [`Hooks::left`].
    pub fn left(how: connection::Left) {
        for hooks in all() {
            hooks.left(how);
        }
    }

    /// [`Hooks::ended`].
    pub fn ended(end: &connection::Ended<'_>) {
        for hooks in all() {
            hooks.ended(end);
        }
    }

    /// [`Hooks::retry`]: the first registered hook's answer.
    pub fn retry() -> Option<connection::Retry> {
        all().iter().find_map(|hooks| hooks.retry())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    /// Records what it heard. One process-wide list, as the game has: this test is the only
    /// one that registers or dispatches.
    struct Recorder(StdMutex<Vec<String>>);

    impl Hooks for Recorder {
        fn started(&self, game: &Started<'_>) {
            self.0
                .lock()
                .unwrap()
                .push(format!("started {}", game.version));
        }
        fn published(&self, open: bool) {
            self.0.lock().unwrap().push(format!("published {open}"));
        }
        fn shutdown(&self) {
            self.0.lock().unwrap().push("shutdown".into());
        }
        fn activity(&self, now: Activity) {
            self.0.lock().unwrap().push(format!("{now:?}"));
        }
        fn connecting(&self, server: &connection::Server) {
            let line = format!("connecting {}", server.address);
            self.0.lock().unwrap().push(line);
        }
        fn joined(&self) {
            self.0.lock().unwrap().push("joined".into());
        }
        fn left(&self, left: connection::Left) {
            self.0.lock().unwrap().push(format!("left {left:?}"));
        }
        fn ended(&self, end: &connection::Ended<'_>) {
            self.0.lock().unwrap().push(format!("ended {}", end.reason));
        }
        fn retry(&self) -> Option<connection::Retry> {
            Some(connection::Retry::Wait("in 5 s".into()))
        }
        fn get_the_game(&self) -> Option<GetTheGame> {
            Some(GetTheGame {
                address: "https://example.org",
                title: "recorder.title",
                paragraphs: &[],
                button: "recorder.button",
            })
        }
        fn client_identity(&self, server: &str) -> Option<ClientIdentity> {
            (server == "first.example").then(|| ClientIdentity {
                brand: "first".to_owned(),
                payloads: vec![("recorder:hello".to_owned(), vec![1, 2])],
            })
        }
    }

    /// Implements nothing: the defaults do nothing.
    struct Silent;
    impl Hooks for Silent {}

    /// Registered after the recorder: answers for two servers, one the recorder answers too.
    struct Later;
    impl Hooks for Later {
        fn retry(&self) -> Option<connection::Retry> {
            Some(connection::Retry::Now(connection::Server::new(
                "b", "P", false,
            )))
        }
        fn client_identity(&self, server: &str) -> Option<ClientIdentity> {
            matches!(server, "first.example" | "second.example").then(|| ClientIdentity {
                brand: "later".to_owned(),
                payloads: Vec::new(),
            })
        }
    }

    static RECORDER: Recorder = Recorder(StdMutex::new(Vec::new()));

    #[test]
    fn registered_hooks_hear_the_game_and_late_ones_are_refused() {
        register(&RECORDER).unwrap();
        register(&Silent).unwrap();
        register(&Later).unwrap();
        engine::started(&Started::new("0.2.31", std::path::Path::new("game")));
        // Changes only: the same state every frame is said once.
        for open in [false, true, true, true, false, false] {
            engine::published(open);
        }
        let world = Activity::World {
            play: Play::Singleplayer,
            dimension: Dimension::Nether,
            mode: GameMode::Creative,
        };
        for now in [Activity::Menus, Activity::Menus, world, world] {
            engine::activity(now);
        }
        engine::shutdown();
        engine::connecting(&connection::Server::new("a", "P", false));
        engine::joined();
        engine::left(connection::Left::Quit);
        engine::connecting(&connection::Server::new("b", "P", false));
        engine::ended(&connection::Ended::new(
            "Timed out",
            None,
            connection::By::Server,
        ));
        let wait = connection::Retry::Wait("in 5 s".into());
        assert_eq!(engine::retry(), Some(wait));
        assert_eq!(
            engine::get_the_game().map(|g| g.address),
            Some("https://example.org")
        );
        // The first answer wins, a later hook answers what the first leaves, none gives vanilla.
        let first = engine::client_identity("first.example");
        assert_eq!(first.brand, "first");
        assert_eq!(first.payloads, [("recorder:hello".to_owned(), vec![1, 2])]);
        assert_eq!(engine::client_identity("second.example").brand, "later");
        assert_eq!(
            engine::client_identity("other.example"),
            ClientIdentity::vanilla()
        );
        assert_eq!(
            *RECORDER.0.lock().unwrap(),
            [
                "started 0.2.31",
                "published true",
                "published false",
                "Menus",
                "World { play: Singleplayer, dimension: Nether, mode: Creative }",
                "shutdown",
                "connecting a",
                "joined",
                "left Quit",
                "connecting b",
                "ended Timed out",
            ]
        );
        assert!(
            !containers::engine::any(),
            "no registrant offers containers()"
        );
        assert_eq!(register(&Silent), Err(TooLate));
    }
}
