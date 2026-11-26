# Config files and settings

A mod keeps its options and files in distinct places according to who edits them and how they are verified:

| | Config files | Settings | Verified package runtime files |
| --- | --- | --- | --- |
| Lives in | The game's `config/` folder (on a server, `serverconfig/` per world) | `sunrise-mods/.sunrise-mod-data/<mod id>/`, one file per key | `runtime/` inside the `.smod` package, listed in `[runtime]` with SHA-512 |
| Edited by | The player, in a text editor | The player, on the mod's page in **Options > Mods** | The mod author at packaging time (immutable verified assets) |
| You declare it with | `Registrar::register_config` | `Registrar::register_setting` (returns `SettingKey`) | `[runtime]` in `sunrise-mod.toml` |
| You read it in | The config's `ConfigListener` (`ConfigEvent::Loading`, `Reloading`, `Unloading`) or `std::fs` under `Host::config_dir()` | `host.setting(key)` and the typed `setting_*` reads | `RegistrationContext::runtime_files()` |
| NeoForge counterpart | `ModContainer.registerConfig`, `ModConfigEvent`, `FMLPaths.CONFIGDIR` | `ModConfigSpec` entries on the config screen | — |

## Config files

Register each managed config file from `Hooks::register` with
`Registrar::register_config(ConfigSpec::new(kind, default_text), listener)`.

- **Default naming and custom paths:** `ConfigSpec::new(kind, default_text)` uses the registering mod's ID
  from the registrar to build `<mod id>-<kind>.toml` (`client`, `common` or `server`). Call `.file(path)`
  with any `impl Into<PathBuf>` to register a different native path (such as `"mymod/machines.toml"`).
- **Uniqueness:** Registered filenames are unique by spelling across all mods and config kinds, matching
  FML's `ConfigTracker.trackConfig`. Registering a filename already claimed returns
  `Err(ConfigRegistrationError::Duplicate { file, owner })`, logs the conflict, and keeps the first
  declaration.
- **Owner-bound delivery:** The optional `Arc<ConfigListener>` is bound to that registration's owner and
  receives only that config's events (`ConfigEvent::Loading`, `Reloading`, `Unloading`) with
  `&ConfigSnapshot` (`mod_id`, `kind`, registered `file`, resolved on-disk `path`, and `text`). Pass `None`
  if the file needs no callback. A `Server` config loads before `server_about_to_start` from the world's
  `serverconfig/`, remains available through final level saves and unloads, and unloads after `server_stopped`.
  `Client` and `Common` configs load once and stay until the game quits.

```rust,ignore
use mod_api::{
    ConfigEvent, ConfigKind, ConfigSnapshot, ConfigSpec, Hooks, Registrar,
    RegistrationContext,
};
use std::sync::{Arc, RwLock};

struct MyMod {
    rate: Arc<RwLock<u32>>,
}

#[mod_api::sunrise_mod]
impl Hooks for MyMod {
    fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
        let rate = Arc::clone(&self.rate);
        let spec = ConfigSpec::new(ConfigKind::Server, "rate = 4\n")
            .file("mymod/machines.toml");
        if let Err(err) = r.register_config(
            spec,
            Some(Arc::new(
                move |event: ConfigEvent<'_>, snapshot: &ConfigSnapshot| {
                    match event {
                        ConfigEvent::Loading(_) | ConfigEvent::Reloading(_) => {
                            // `snapshot.file` is the registered path ("mymod/machines.toml");
                            // `snapshot.path` is the actual file on disk (e.g. `<world>/serverconfig/...`
                            // when overridden, or `<config_dir>/mymod/machines.toml`).
                            let parsed = parse_rate(&snapshot.text)?;
                            *rate.write().unwrap() = parsed;
                        }
                        ConfigEvent::Unloading(_) => {
                            *rate.write().unwrap() = 4;
                        }
                        _ => {}
                    }
                    Ok(())
                },
            )),
        ) {
            log::error!("failed to register config: {err}");
        }
    }
}
```

### Phase, ordering and path resolution

Config files sit in the game's `config/` folder, beside `options.txt` (beside `server.properties` on a
dedicated server), as Fabric and NeoForge mods' do.

1. **Freeze order:** `Hooks::register` runs in installed dependency order, and each mod's `register_config`
   calls are retained in declaration order. Declarations freeze before any config file I/O runs.
2. **Initial load (`ConfigEvent::Loading`):** Each owning side loads its kinds in frozen declaration order:
   - Client window: `Client` and `Common` once, right after `client_setup` and before title, connection,
     input, render or tick callbacks.
   - Dedicated server: `Common` once and `Server` per world, before `server_about_to_start`.
   - Single-player integrated server: `Server` per world, before `server_about_to_start`.
   For a `Server` config, if `<world>/serverconfig/<file>` already exists on disk, that override path wins
   (`snapshot.path` points there and no missing base file is created). Otherwise the loader uses
   `<config_dir>/<file>`, creating parent folders and writing `default` only when that base file is missing.
   `ConfigEvent::Loading` lends an owner-bound [`ConfigContext`](../concepts/load-phases.md) for settings
   reads and `config_dir()`. It does not lend a game host, world handles or transactions.
3. **Reloading (`ConfigEvent::Reloading`):** Once a second on the owning side's tick, the game reads each
   tracked file's text. When the text differs from the last snapshot, it updates the snapshot and calls only
   that registration's listener with `ConfigEvent::Reloading` and the polling game's live `HostSide`.
   Files saved with unchanged text fire nothing. An I/O error (including a deleted or unreadable file) is
   logged with owner and path and keeps the last successfully read snapshot without substituting another
   path, retrying, or latching.
4. **Unloading (`ConfigEvent::Unloading`):** When a server stops, each tracked `Server` config is dropped
   and only its registration's listener receives `ConfigEvent::Unloading`, with a live `ServerHost` and
   the last snapshot. If dedicated world preparation fails after loading, its `Server` configs still
   unload. `Client` and `Common` configs stay until the process exits.
5. **Listener errors and panics:** Returning `Err(message)` from a listener logs the failure with owner,
   operation and `snapshot.path` on every invocation without disabling future deliveries. A panic disables
   only that registration's callback for the rest of the session and raises an error notice naming that mod
   without aborting server shutdown or affecting other mods.

## Native filesystem operations under `config/`

For files outside the managed config lifecycle, `Host::config_dir()` lends `&std::path::Path` for the game's
`config/` folder (NeoForge's `FMLPaths.CONFIGDIR`). `ConfigContext::config_dir()` and
`ClientSetupContext::config_dir()` lend the same path; lending it performs no I/O. During registration,
use `cx.instance_dir()?.to_path_buf()?.join("config")` in an `Option`-returning helper.

Use `std::fs` directly on paths joined from `host.config_dir()`:

```rust,ignore
use std::fs;

let root = host.config_dir().join("mymod");
let file = root.join("presets.toml");

// Create parent folders and write
fs::create_dir_all(&root)?;
fs::write(&file, b"enabled = true\n")?;

// Read
let bytes = fs::read(&file)?;

// List a directory
for entry in fs::read_dir(&root)? {
    let entry = entry?;
    let _name = entry.file_name();
}

// Inspect metadata
let meta = fs::metadata(&file)?;
let _len = meta.len();

// Remove a file
fs::remove_file(&file)?;
```

## Bulk data

Keep map tiles, waypoints, seeds and exports out of settings and config files, using `std::fs` under the
host's real directories:

- **Instance-wide bulk data (both sides):** `Host::instance_dir()` returns the instance folder as `PathBuf`.
  Store instance-wide files under `host.instance_dir().join("sunrise-mod-data").join("mymod")` (for example,
  keying client-only subfolders by server or save and dimension). Create parent folders with
  `std::fs::create_dir_all` before writing.
- **World-scoped bulk data (server only):** `ServerHost::world_dir()` returns `Option<PathBuf>` for the open
  save folder (`None` when no world is open). Store save-bound files under
  `world_dir.join("sunrise-mod-data").join("mymod")`; their contents are copied, backed up and deleted with
  the save.

The game never scans `sunrise-mod-data/` as settings or config.

## Settings

Register each setting from `Hooks::register` with `Registrar::register_setting(SettingSpec::…)` to receive an
owner-bound `SettingKey`: a key (`a-z`, `0-9`, `_`, `-` and `.`, not starting with `.`), translation keys for its
label and tooltip, and its kind with the default. A setting the spec check refuses, or a key you registered
already, comes back as the error. On the client the settings appear on your mod's page in
**Options > Mods**, opened from its **Config** button.

- Read a setting with `host.setting(key)` or the typed `setting_bool`, `setting_choice`, `setting_f64`
  and `setting_color` on any `Host`; each reads the live value on the side it runs. These calls take
  the `SettingKey` minted at registration and return `Result<_, SettingAccessError>`.
- Group settings under headings with `Registrar::categorize_setting(key, category)` after registering
  them: settings sharing a category (a translation key) are drawn under one heading, in registration
  order, on the mod's page. Settings without a category render as today. A key the calling mod never
  registered, or an empty category, comes back as the error.
- Update a setting on either host with `host.set_setting(key, value) -> Result<bool, SettingAccessError>`.
  The game validates it, persists it atomically, updates that side's live store and any client Options > Mods
  row, and synchronously tells its owner through `setting_changed`. The hook takes a read-only `&dyn Host` and can
  read the new value, but cannot write: make a follow-up write in the callback that made the first one. Writing the same value does not notify; range values compare by bits, so `-0.0` and
  `+0.0` are distinct, matching vanilla's `OptionInstance.set`.
- Options > Mods uses the same write path. Publishing a settings reload, or reopening a mod's settings page
  after an external edit, also notifies the owner of changed values.
- Settings writes made in a config listener’s `Reloading` or `Unloading` callback are live: the value and its notification
  stand even if the writing listener later panics. Only that listener is disabled; later listeners still run.

Keep the keys returned by your registration. There is no live raw mod-id read/write path. `ConfigContext`,
`ReloadContext` and `ClientReloadListenerContext` allow typed reads without giving out a mutable host.

The settings folder is settings only, not bulk storage: ordinary files there are read as setting keys and
subfolders or unsafe entries still warn during snapshots. Keep bulk files under `Host::instance_dir()` or
`ServerHost::world_dir()` instead; their files are not included in settings snapshots.

### Your own config screen

To show your own screen instead of the settings page, register a config screen factory from `Hooks::register`
with `Registrar::register_config_screen(Arc::new(|host: &dyn ClientHost| true))`, as NeoForge mods register an
`IConfigScreenFactory`. **Config** then shows for your mod even with no settings. Pressing it calls the factory:
prepare what your screen shows and return `true`, and the game opens your mod's own screen (as
[`mod_screen_open`](client/screens.md#opening-a-mod-screen-and-synchronous-transitions) does, so `screen_opening`
can still cancel or replace it) over the Mods screen. Closing it, with Escape or `mod_screen_close`, returns to
the Mods screen with your mod still selected. Return `false` to stay on the Mods screen. A mod registers one
factory; a second comes back as the error. Config opens nothing before the game has finished starting.

## Verified package runtime files

Do not mix player-editable configs or settings with immutable data shipped inside your `.smod` package.
Files declared under `[runtime]` in `sunrise-mod.toml` are SHA-512 verified at package load and read
through `RegistrationContext::runtime_files()` (see [package contents](native-mods/03-data-files.md#package-contents)).
They are never written by the game, never scanned as settings or configs, and stay separate from
`Host::config_dir()`, `Host::instance_dir()` and `ServerHost::world_dir()`.
