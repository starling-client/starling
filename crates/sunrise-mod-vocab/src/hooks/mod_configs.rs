//! The `mod_configs` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The config files native mods keep (NeoForge's `registerConfig`):
    /// push yours, each of a kind, with the text written to `config/` when
    /// the file is missing. Asked by the window when the game starts (its
    /// `Client` and `Common` configs), by a dedicated server when it starts
    /// (`Common` and `Server`) and by a server before each world it opens
    /// (`Server`); a game reads only its own kinds, so push them all every
    /// time. Receives a [`ConfigContext`] with settings reads and the scoped
    /// `ConfigFiles` service. A file outside `config/`, or one already registered, is skipped.
    #[hook(
        receiver = "associated",
        context = "config",
        dispatch = "flat",
        side = "both",
        contract = "collect",
        out = "configs",
        neoforge = "ModContainer.registerConfig",
        version = 2
    )]
    fn register_configs(configs: &mut Vec<mod_api::Config>);
    /// A registered config was read (`ModConfigEvent.Loading`): `config`
    /// says whose, which file and its text, which the mod parses (TOML by
    /// convention). Every mod hears every config, so check `mod_id`. Fired
    /// where the config's kind loads (`register_configs`), a `Server`
    /// one before `server_about_to_start`. Receives a [`ConfigContext`] with
    /// settings reads and the scoped `ConfigFiles` service. Client and Common
    /// load once on their owning side, Server once per world.
    #[hook(
        receiver = "associated",
        context = "config",
        dispatch = "flat",
        side = "both",
        contract = "listen",
        neoforge = "ModConfigEvent.Loading",
        cancellable = false,
        version = 2
    )]
    fn config_loading(config: &ConfigFile);
    /// A loaded config's file changed on disk (`ModConfigEvent.Reloading`):
    /// its new text. The game looks at the files' modification times once a
    /// second, on the window and on the server's tick; a file saved with the
    /// same text, removed or unreadable fires nothing and keeps its last
    /// text. Receives the polling game's [`HostSide`]; world queries return
    /// `None` if no world is open. A panic disables only the offending mod’s
    /// reload callback; later listeners still run. Live writes outside a journal stand.
    #[hook(
        dispatch = "flat",
        side = "both",
        contract = "listen",
        neoforge = "ModConfigEvent.Reloading",
        cancellable = false,
        version = 4
    )]
    fn config_reloading(config: &ConfigFile);
    /// A `Server` config is dropped as its server stops
    /// (`ModConfigEvent.Unloading`), after `server_stopping`; the next
    /// world reads it again. `Client` and `Common` configs stay until the
    /// game quits. Receives the stopping game's [`ServerHost`], before the
    /// final save and teardown. A panic disables only that mod’s unload callback,
    /// not other listeners or the rest of teardown. Live writes outside a journal stand.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "ModConfigEvent.Unloading",
        cancellable = false,
        version = 4
    )]
    fn config_unloading(config: &ConfigFile);
}
