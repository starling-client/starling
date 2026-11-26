//! The debug screen's entries (26.3 `DebugScreenEntries`, `DebugScreenEntryList`):
//! what each F3 line block and debug renderer is, whether it shows always,
//! only in the overlay or never, the two built-in profiles, and
//! `debug-profile.json` beside `options.txt`, which keeps them.
use mod_api::{
    __private::game::{CallbackOwner, debug_renderer_callback_type},
    Answered, ClientHost, DebugEntry, DebugGroup, DebugRenderer, Identifier,
};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::{any::TypeId, borrow::Cow};

/// `DebugScreenEntryStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    AlwaysOn,
    InOverlay,
    Never,
}

impl Status {
    /// `getSerializedName`.
    pub fn name(self) -> &'static str {
        match self {
            Status::AlwaysOn => "alwaysOn",
            Status::InOverlay => "inOverlay",
            Status::Never => "never",
        }
    }
    fn parse(name: &str) -> Option<Self> {
        [Status::AlwaysOn, Status::InOverlay, Status::Never]
            .into_iter()
            .find(|s| s.name() == name)
    }

    fn shown(self, overlay: bool) -> bool {
        match self {
            Status::AlwaysOn => true,
            Status::InOverlay => overlay,
            Status::Never => false,
        }
    }
}

/// `DebugScreenProfile`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Default,
    Performance,
}

impl Profile {
    pub fn name(self) -> &'static str {
        match self {
            Profile::Default => "default",
            Profile::Performance => "performance",
        }
    }
    pub fn translation_key(self) -> &'static str {
        match self {
            Profile::Default => "debug.options.profile.default",
            Profile::Performance => "debug.options.profile.performance",
        }
    }
    fn parse(name: &str) -> Option<Self> {
        [Profile::Default, Profile::Performance]
            .into_iter()
            .find(|p| p.name() == name)
    }
    /// `DebugScreenEntries.PROFILES`.
    fn statuses(self) -> &'static [(&'static str, Status)] {
        use Status::*;
        match self {
            Profile::Default => &[
                ("3d_crosshair", InOverlay),
                ("game_version", InOverlay),
                ("tps", InOverlay),
                ("fps", InOverlay),
                ("memory", InOverlay),
                ("system_specs", InOverlay),
                ("player_position", InOverlay),
                ("player_section_position", InOverlay),
                ("simple_performance_impactors", InOverlay),
            ],
            Profile::Performance => &[
                ("tps", InOverlay),
                ("fps", AlwaysOn),
                ("gpu_utilization", InOverlay),
                ("memory", InOverlay),
                ("simple_performance_impactors", InOverlay),
            ],
        }
    }
}

/// `DebugEntryCategory`: the screen's two sections, by `sortKey`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    /// `SCREEN_TEXT`, `debug.options.category.text`.
    Text,
    /// `RENDERER` (`DebugEntryNoop`), `debug.options.category.renderer`.
    Renderer,
}

impl Category {
    pub fn translation_key(self) -> &'static str {
        match self {
            Category::Text => "debug.options.category.text",
            Category::Renderer => "debug.options.category.renderer",
        }
    }
}

/// One registered entry: its vanilla path or full mod ID, its section,
/// and whether it shows while debug info is reduced (`isAllowed(true)`).
#[derive(Clone, Copy, Debug)]
pub struct Entry<'a> {
    pub path: &'a str,
    pub category: Category,
    pub reduced: bool,
}

const fn text(path: &'static str, reduced: bool) -> Entry<'static> {
    Entry {
        path,
        category: Category::Text,
        reduced,
    }
}
const fn renderer(path: &'static str) -> Entry<'static> {
    Entry {
        path,
        category: Category::Renderer,
        reduced: false,
    }
}

/// `DebugScreenEntries`' registrations, in their order.
pub const ENTRIES: [Entry<'static>; 46] = [
    text("game_version", true),
    text("fps", true),
    text("tps", true),
    text("memory", true),
    text("detailed_memory", true),
    text("system_specs", true),
    text("looking_at_block_state", false),
    text("looking_at_block_tags", false),
    text("looking_at_fluid_state", false),
    text("looking_at_fluid_tags", false),
    text("looking_at_entity", false),
    text("looking_at_entity_tags", false),
    text("chunk_render_stats", true),
    text("chunk_generation_stats", false),
    text("entity_render_stats", true),
    text("particle_render_stats", false),
    text("chunk_source_stats", true),
    text("player_position", false),
    text("player_section_position", true),
    text("player_speed", false),
    text("light_levels", false),
    text("heightmap", false),
    text("biome", false),
    text("local_difficulty", false),
    text("day_count", false),
    text("entity_spawn_counts", false),
    text("sound_mood", false),
    text("sound_cache", true),
    text("post_effects", false),
    renderer("entity_hitboxes"),
    renderer("chunk_borders"),
    renderer("3d_crosshair"),
    renderer("chunk_section_paths"),
    text("gpu_utilization", true),
    text("simple_performance_impactors", true),
    renderer("chunk_section_octree"),
    renderer("visualize_water_levels"),
    renderer("visualize_heightmap"),
    renderer("visualize_collision_boxes"),
    renderer("visualize_entity_supporting_blocks"),
    renderer("visualize_block_light_levels"),
    renderer("visualize_sky_light_levels"),
    renderer("visualize_solid_faces"),
    renderer("visualize_chunks_on_server"),
    renderer("visualize_sky_light_sections"),
    renderer("chunk_section_visibility"),
];

/// A startup declaration with the native mod identity supplied by dispatch.
pub struct RegisteredDebugEntry {
    owner: CallbackOwner,
    entry: DebugEntry,
}

/// A selected world renderer and the mod scope of its retained draw callback.
pub struct ActiveDebugRenderer {
    pub owner: CallbackOwner,
    renderer: Option<DebugRenderer>,
}

impl ActiveDebugRenderer {
    pub fn draw(
        &self,
        host: &dyn ClientHost,
        view: &mod_api::WorldRenderView<'_>,
        draw: &mut dyn mod_api::WorldDrawContext,
    ) {
        if self.owner.is_enabled() {
            let _ = self
                .owner
                .call("debug renderer", "debug_renderer_draw", || {
                    self.renderer
                        .as_ref()
                        .expect("active debug renderer")
                        .draw(host, view, draw);
                });
        }
    }
}

impl Drop for ActiveDebugRenderer {
    fn drop(&mut self) {
        self.owner.dispose(
            "debug renderer",
            "debug_renderer_drop",
            self.renderer.take(),
        );
    }
}

/// The entries native mods registered (`register_debug_entries`), kept once at start.
static REGISTERED: OnceLock<Arc<[RegisteredDebugEntry]>> = OnceLock::new();

/// Keeps the entries native mods registered, once at start. One whose id is not
/// `<namespace>:<path>` or duplicates a built-in or earlier entry is logged and left out.
pub fn register(entries: Vec<Answered<DebugEntry>>) {
    if REGISTERED.set(collect_registrations(entries)).is_err() {
        log::warn!("debug entries were registered already; these are left out");
    }
}

/// Share the startup declarations with a newly constructed game.
pub fn registered() -> Arc<[RegisteredDebugEntry]> {
    REGISTERED.get().cloned().unwrap_or_default()
}

/// Apply the startup registration checks without publishing the resulting declarations.
pub fn collect_registrations(entries: Vec<Answered<DebugEntry>>) -> Arc<[RegisteredDebugEntry]> {
    checked(entries.into_iter().map(|answer| RegisteredDebugEntry {
        owner: CallbackOwner::for_mod(answer.mod_id),
        entry: answer.answer,
    }))
    .into()
}

fn checked(entries: impl IntoIterator<Item = RegisteredDebugEntry>) -> Vec<RegisteredDebugEntry> {
    let mut kept: Vec<RegisteredDebugEntry> = Vec::new();
    for registered in entries {
        let id = registered.entry.id();
        if Identifier::parse_namespaced(id).is_none() {
            log::warn!(
                "{}: debug entry {id:?} must be <namespace>:<path>",
                registered.owner.id()
            );
        } else if ENTRIES
            .iter()
            .any(|entry| id.strip_prefix("minecraft:") == Some(entry.path))
            || kept.iter().any(|k| k.entry.id() == id)
        {
            log::warn!(
                "{}: debug entry {id} is registered twice; the first is kept",
                registered.owner.id()
            );
        } else {
            kept.push(registered);
        }
    }
    kept
}

/// How a registered text entry or renderer toggle appears in the debug options.
fn as_entry(registered: &RegisteredDebugEntry) -> Entry<'_> {
    let entry = &registered.entry;
    Entry {
        path: entry.id(),
        category: if entry.text_lines().is_some() {
            Category::Text
        } else {
            Category::Renderer
        },
        reduced: entry.allowed_in_reduced_debug_info(),
    }
}

/// Every entry: vanilla's, then the native mods' startup declarations.
fn all<'a>(registered: &'a [RegisteredDebugEntry]) -> impl Iterator<Item = Entry<'a>> {
    let vanilla: &[Entry<'a>] = &ENTRIES;
    vanilla
        .iter()
        .copied()
        .chain(registered.iter().map(as_entry))
}

/// `DebugScreenEntries.getEntry` by path or namespaced identifier.
fn entry<'a>(registered: &'a [RegisteredDebugEntry], path: &str) -> Option<Entry<'a>> {
    let bare = path.strip_prefix("minecraft:").unwrap_or(path);
    all(registered).find(|e| {
        let entry = e.path.strip_prefix("minecraft:").unwrap_or(e.path);
        entry == bare
    })
}

/// `DebugOptionsScreen.OptionList.updateSearch`: the entries whose path
/// contains `search`, by category then identifier.
pub fn search<'a>(registered: &'a [RegisteredDebugEntry], search: &str) -> Vec<Entry<'a>> {
    let mut entries: Vec<_> = all(registered)
        .filter(|e| e.path.contains(search))
        .collect();
    entries.sort_by(|a, b| a.category.cmp(&b.category).then(a.path.cmp(b.path)));
    entries
}

/// Drawn lines of visible native text entries with their `DebugScreenDisplayer` group key, in
/// registration order. Renderer toggles, hidden entries and callbacks that append nothing do not
/// contribute a group.
pub fn mod_lines<'a>(
    entries: &'a [RegisteredDebugEntry],
    host: &'a dyn ClientHost,
    selection: &'a DebugEntries,
    overlay: bool,
    reduced: bool,
) -> impl Iterator<Item = (Cow<'static, str>, Vec<String>)> + 'a {
    entries.iter().filter_map(move |registered| {
        let entry = &registered.entry;
        let (group, append_lines) = entry.text_lines()?;
        // The startup registry already validated the full ID; no catalog lookup is needed.
        if !entry.is_allowed(reduced)
            || !selection
                .statuses
                .get(entry.id())
                .is_some_and(|status| status.shown(overlay))
        {
            return None;
        }
        if !registered.owner.is_enabled() {
            return None;
        }
        let mut lines = Vec::new();
        registered
            .owner
            .call("debug entry", "debug_entry_lines", || {
                append_lines(host, &mut lines);
            })
            .ok()?;
        if lines.is_empty() {
            return None;
        }
        // Vanilla's groups are `minecraft:` identifiers; the overlay keys them by path.
        let key = match group {
            DebugGroup::Position => Cow::Borrowed("position"),
            DebugGroup::Light => Cow::Borrowed("light"),
            DebugGroup::Memory => Cow::Borrowed("memory"),
            DebugGroup::LookingAtBlock => Cow::Borrowed("looking_at_block"),
            DebugGroup::LookingAtEntity => Cow::Borrowed("looking_at_entity"),
            DebugGroup::LookingAtFluid => Cow::Borrowed("looking_at_fluid"),
            DebugGroup::ChunkGeneration => Cow::Borrowed("chunk_generation"),
            DebugGroup::Named(id) if id.namespace() == "minecraft" => {
                Cow::Owned(id.path().to_owned())
            }
            DebugGroup::Named(id) => Cow::Owned(id.to_string()),
        };
        Some((key, lines))
    })
}

/// `DebugScreenEntryList`: every entry's status (by full identifier, as the
/// file keeps them) and the profile they came from if unchanged. Whether the
/// F3 overlay is open (`isOverlayVisible`) is the game's `debug`.
#[derive(Clone, Debug)]
pub struct DebugEntries {
    statuses: BTreeMap<String, Status>,
    profile: Option<Profile>,
    file: Option<PathBuf>,
    /// Monotonic `rebuildCurrentList` revision, including unchanged selections.
    version: u64,
    renderer_owners: HashMap<(&'static str, TypeId, usize), CallbackOwner>,
}

impl Default for DebugEntries {
    fn default() -> Self {
        let mut entries = Self {
            statuses: BTreeMap::new(),
            profile: None,
            file: None,
            version: 0,
            renderer_owners: HashMap::new(),
        };
        entries.reset_to_profile(Profile::Default);
        entries
    }
}

/// `Identifier.CODEC`: a bare path is in the `minecraft` namespace, a native mod's id is its own.
fn identifier(path: &str) -> Cow<'_, str> {
    if path.contains(':') {
        Cow::Borrowed(path)
    } else {
        Cow::Owned(format!("minecraft:{path}"))
    }
}

impl DebugEntries {
    /// `DebugScreenEntryList.load` from `debug-profile.json` in `dir`: the
    /// default profile without a file; a file that does not parse is reset
    /// to it and saved again.
    pub fn load(dir: &Path) -> Self {
        let file = dir.join("debug-profile.json");
        let mut entries = Self {
            file: Some(file.clone()),
            ..Self::default()
        };
        let Ok(text) = std::fs::read_to_string(&file) else {
            return entries;
        };
        match Self::parse(&text) {
            Some((Some(profile), _)) => entries.reset_to_profile(profile),
            Some((None, custom)) => {
                entries.statuses = custom;
                entries.profile = None;
            }
            None => {
                log::error!(
                    "Couldn't read debug profile file {}, resetting to default",
                    file.display()
                );
                entries.save();
            }
        }
        entries
    }
    /// `SerializedOptions.CODEC`: an optional `profile` name and an optional
    /// `custom` map of identifiers to statuses.
    fn parse(text: &str) -> Option<(Option<Profile>, BTreeMap<String, Status>)> {
        let mut json: serde_json::Value = serde_json::from_str(text).ok()?;
        upgrade(&mut json);
        let object = json.as_object()?;
        let profile = match object.get("profile") {
            Some(name) => Some(Profile::parse(name.as_str()?)?),
            None => None,
        };
        let mut custom = BTreeMap::new();
        if let Some(map) = object.get("custom") {
            for (key, status) in map.as_object()? {
                // Identifier.CODEC: a bare path is in the minecraft namespace.
                let key = if key.contains(':') {
                    key.clone()
                } else {
                    identifier(key).into_owned()
                };
                custom.insert(key, Status::parse(status.as_str()?)?);
            }
        }
        Some((profile, custom))
    }
    /// `DebugScreenEntryList.save`: the profile's name while it is unchanged,
    /// else every status, with the data version the codec wraps it in.
    pub fn save(&self) {
        let Some(file) = &self.file else {
            return;
        };
        let mut json = serde_json::Map::new();
        json.insert(
            "DataVersion".into(),
            sunrise_core::version::DATA_VERSION.into(),
        );
        match self.profile {
            Some(profile) => {
                json.insert("profile".into(), profile.name().into());
            }
            None => {
                let custom: serde_json::Map<String, serde_json::Value> = self
                    .statuses
                    .iter()
                    .map(|(k, s)| (k.clone(), s.name().into()))
                    .collect();
                json.insert("custom".into(), custom.into());
            }
        }
        if let Err(e) = std::fs::write(file, serde_json::Value::Object(json).to_string()) {
            log::error!("Failed to save debug profile file {}: {e}", file.display());
        }
    }
    fn reset_to_profile(&mut self, profile: Profile) {
        self.profile = Some(profile);
        self.statuses = profile
            .statuses()
            .iter()
            .map(|(path, status)| (identifier(path).into_owned(), *status))
            .collect();
    }
    /// `loadProfile` then `save` (the screen's profile buttons).
    pub fn load_profile(&mut self, profile: Profile) {
        self.reset_to_profile(profile);
        self.invalidate();
        self.save();
    }
    pub fn is_using_profile(&self, profile: Profile) -> bool {
        self.profile == Some(profile)
    }
    /// `getStatus`: never for an entry the statuses leave out. The debug renderers ask every
    /// frame, so no identifier is built for the lookup; a path with a namespace (a native mod's entry) is
    /// matched as it is.
    pub fn status(&self, path: &str) -> Status {
        self.statuses
            .iter()
            .find(|(id, _)| id.as_str() == path || id.strip_prefix("minecraft:") == Some(path))
            .map_or(Status::Never, |(_, status)| *status)
    }
    /// `setStatus`: the statuses are custom from now on, and saved.
    pub fn set_status(&mut self, path: &str, status: Status) {
        self.profile = None;
        self.statuses.insert(identifier(path).into_owned(), status);
        self.invalidate();
        self.save();
    }
    /// `toggleStatus` (F3+B, F3+G) with the overlay open or not: whether the
    /// entry is on afterwards.
    pub fn toggle_status(&mut self, path: &str, overlay: bool) -> bool {
        let (status, on) = match self.statuses.get(identifier(path).as_ref()) {
            Some(Status::AlwaysOn) => (Status::Never, false),
            Some(Status::InOverlay) if overlay => (Status::Never, false),
            Some(Status::Never) if overlay => (Status::InOverlay, true),
            _ => (Status::AlwaysOn, true),
        };
        self.set_status(path, status);
        on
    }
    /// `isCurrentlyEnabled` (`rebuildCurrentList`): on always, or in the
    /// `overlay` while it is open, and allowed with the debug info `reduced`.
    pub fn enabled(
        &self,
        registered: &[RegisteredDebugEntry],
        path: &str,
        overlay: bool,
        reduced: bool,
    ) -> bool {
        self.status(path).shown(overlay)
            && entry(registered, path).is_some_and(|e| e.reduced || !reduced)
    }
    /// `DebugScreenEntryList.rebuildCurrentList`: invalidate even if the enabled set is unchanged.
    pub fn invalidate(&mut self) {
        self.version += 1;
    }
    /// `getCurrentlyEnabledVersion`: every rebuild, not a fingerprint of the final selection.
    pub fn enabled_version(&self) -> u64 {
        self.version
    }
    /// `DebugRenderer.refreshRendererList`: retain submitted callbacks in dispatch order.
    /// A refresh does not re-enable a callback that already panicked in this session.
    pub fn active_renderers(
        &mut self,
        renderers: Vec<Answered<DebugRenderer>>,
    ) -> Vec<ActiveDebugRenderer> {
        let mut seen = HashMap::<(&'static str, TypeId), usize>::new();
        renderers
            .into_iter()
            .map(|renderer| {
                let kind = debug_renderer_callback_type(&renderer.answer);
                let ordinal = seen.entry((renderer.mod_id, kind)).or_default();
                let owner = self
                    .renderer_owners
                    .entry((renderer.mod_id, kind, *ordinal))
                    .or_insert_with(|| CallbackOwner::for_mod(renderer.mod_id))
                    .clone();
                *ordinal += 1;
                ActiveDebugRenderer {
                    owner,
                    renderer: Some(renderer.answer),
                }
            })
            .collect()
    }
    /// `getCurrentlyEnabled().isEmpty()` negated: something to draw. Asked every frame, F3 closed
    /// or not, so it walks the few statuses kept rather than every entry.
    pub fn any_enabled(
        &self,
        registered: &[RegisteredDebugEntry],
        overlay: bool,
        reduced: bool,
    ) -> bool {
        self.statuses
            .keys()
            .any(|path| self.enabled(registered, path, overlay, reduced))
    }
}

/// `DataFixTypes.DEBUG_PROFILE.wrapCodec(..., 4649)`: a file without a
/// `DataVersion` is from 4649, the version the file first had.
const FIRST_VERSION: i64 = 4649;

/// The `DataFixers` rules for `References.DEBUG_PROFILE`, each run when the
/// file is older than its schema, on the `custom` map.
fn upgrade(json: &mut serde_json::Value) {
    let version = json
        .get("DataVersion")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(FIRST_VERSION);
    let Some(custom) = json.get_mut("custom").and_then(|c| c.as_object_mut()) else {
        return;
    };
    // 4650 DebugProfileOverlayReferenceFix: the status `inF3` is `inOverlay`.
    if version < 4650 {
        for status in custom.values_mut() {
            if status.as_str() == Some("inF3") {
                *status = "inOverlay".into();
            }
        }
    }
    // 4763 DebugProfileLookingAtSplitFix: the targeted block and fluid
    // entries became their `_state` halves; a key already there wins.
    if version < 4763 {
        for (old, new) in [
            (
                "minecraft:looking_at_block",
                "minecraft:looking_at_block_state",
            ),
            (
                "minecraft:looking_at_fluid",
                "minecraft:looking_at_fluid_state",
            ),
        ] {
            if let Some(status) = custom.remove(old) {
                custom.entry(new).or_insert(status);
            }
        }
    }
    // 5001 DebugProfileRenameEntryFix: `post_effect` is `post_effects`.
    if version < 5001
        && let Some(status) = custom.remove("minecraft:post_effect")
    {
        custom.insert("minecraft:post_effects".into(), status);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{content::crash::current_native_mod, testing::FakeHost};

    #[test]
    fn toggling_follows_the_overlay() {
        let mut e = DebugEntries::default();
        // Not in the default profile: on always while F3 is closed.
        assert!(e.toggle_status("entity_hitboxes", false));
        assert_eq!(e.status("entity_hitboxes"), Status::AlwaysOn);
        assert!(!e.toggle_status("entity_hitboxes", false));
        assert_eq!(e.status("entity_hitboxes"), Status::Never);
        // With F3 open, a hidden entry goes in the overlay only.
        assert!(e.toggle_status("chunk_borders", true));
        assert_eq!(e.status("chunk_borders"), Status::AlwaysOn);
        assert!(!e.toggle_status("chunk_borders", true));
        assert_eq!(e.status("chunk_borders"), Status::Never);
        assert!(e.toggle_status("chunk_borders", true));
        assert_eq!(e.status("chunk_borders"), Status::InOverlay);
        assert!(!e.toggle_status("chunk_borders", true));
        assert!(!e.is_using_profile(Profile::Default));
    }

    #[test]
    fn enabled_needs_the_overlay_and_full_debug_info() {
        let mut e = DebugEntries::default();
        assert!(!e.any_enabled(&[], false, false));
        assert!(e.enabled(&[], "player_position", true, false));
        // Reduced debug info hides the position, not the section.
        assert!(!e.enabled(&[], "player_position", true, true));
        assert!(e.enabled(&[], "player_section_position", true, true));
        e.load_profile(Profile::Performance);
        assert!(e.enabled(&[], "fps", false, false));
        assert!(!e.enabled(&[], "memory", false, false));
    }

    #[test]
    fn the_file_keeps_a_profile_or_the_statuses() {
        let dir =
            std::env::temp_dir().join(format!("sunrise-debug-profile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut e = DebugEntries::load(&dir);
        assert!(e.is_using_profile(Profile::Default));
        e.load_profile(Profile::Performance);
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("debug-profile.json")).unwrap())
                .unwrap();
        assert_eq!(saved["profile"], "performance");
        assert!(DebugEntries::load(&dir).is_using_profile(Profile::Performance));
        e.set_status("biome", Status::AlwaysOn);
        let loaded = DebugEntries::load(&dir);
        assert_eq!(loaded.status("biome"), Status::AlwaysOn);
        assert_eq!(loaded.status("fps"), Status::AlwaysOn);
        assert!(!loaded.is_using_profile(Profile::Performance));
        // A broken file is reset to the default profile.
        std::fs::write(dir.join("debug-profile.json"), "{\"profile\":\"nope\"}").unwrap();
        assert!(DebugEntries::load(&dir).is_using_profile(Profile::Default));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn old_files_are_upgraded_by_their_data_version() {
        // No DataVersion: every fix runs.
        let (_, custom) = DebugEntries::parse(
            r#"{"custom":{"minecraft:fps":"inF3","minecraft:looking_at_block":"alwaysOn",
            "minecraft:looking_at_fluid":"inF3","minecraft:looking_at_fluid_state":"never",
            "minecraft:post_effect":"alwaysOn"}}"#,
        )
        .unwrap();
        assert_eq!(custom["minecraft:fps"], Status::InOverlay);
        assert_eq!(custom["minecraft:looking_at_block_state"], Status::AlwaysOn);
        assert_eq!(custom["minecraft:looking_at_fluid_state"], Status::Never);
        assert_eq!(custom["minecraft:post_effects"], Status::AlwaysOn);
        assert!(!custom.contains_key("minecraft:looking_at_block"));
        assert!(!custom.contains_key("minecraft:post_effect"));
        // From 4763 on, only the post effects rename is left.
        let (_, custom) = DebugEntries::parse(
            r#"{"DataVersion":4763,"custom":{"minecraft:looking_at_block":"alwaysOn",
            "minecraft:post_effect":"alwaysOn"}}"#,
        )
        .unwrap();
        assert_eq!(custom["minecraft:looking_at_block"], Status::AlwaysOn);
        assert_eq!(custom["minecraft:post_effects"], Status::AlwaysOn);
        // `inF3` is no status any more once the file is past 4650.
        assert!(DebugEntries::parse(r#"{"DataVersion":4650,"custom":{"fps":"inF3"}}"#).is_none());
    }

    #[test]
    fn the_list_sorts_text_before_renderers() {
        let all = search(&[], "");
        assert_eq!(all.len(), 46);
        assert_eq!(all[0].path, "biome");
        let first_renderer = all
            .iter()
            .position(|e| e.category == Category::Renderer)
            .unwrap();
        assert_eq!(all[first_renderer].path, "3d_crosshair");
        assert!(
            all[first_renderer..]
                .iter()
                .all(|e| e.category == Category::Renderer)
        );
        assert_eq!(search(&[], "chunk_b").len(), 1);
    }

    #[test]
    fn a_native_mods_entries_keep_their_full_id_in_the_file() {
        let mut e = DebugEntries::default();
        e.set_status("mymod:tick_cost", Status::AlwaysOn);
        assert_eq!(e.status("mymod:tick_cost"), Status::AlwaysOn);
        assert_eq!(e.status("tick_cost"), Status::Never);
    }

    #[test]
    fn a_native_mods_entries_allow_custom_minecraft_ids_and_reject_malformed_or_duplicates() {
        let kept = checked([
            RegisteredDebugEntry {
                owner: CallbackOwner::for_mod("mymod"),
                entry: DebugEntry::text("mymod:tick_cost".into(), DebugGroup::Memory, |_, _| {}),
            },
            RegisteredDebugEntry {
                owner: CallbackOwner::for_mod("mymod"),
                entry: DebugEntry::renderer("minecraft:fps".into()),
            },
            RegisteredDebugEntry {
                owner: CallbackOwner::for_mod("mymod"),
                entry: DebugEntry::text(
                    "minecraft:custom_stat".into(),
                    DebugGroup::Memory,
                    |_, _| {},
                ),
            },
            RegisteredDebugEntry {
                owner: CallbackOwner::for_mod("other"),
                entry: DebugEntry::renderer("minecraft:custom_stat".into()),
            },
            RegisteredDebugEntry {
                owner: CallbackOwner::for_mod("mymod"),
                entry: DebugEntry::renderer("tick_cost".into()),
            },
            RegisteredDebugEntry {
                owner: CallbackOwner::for_mod("mymod"),
                entry: DebugEntry::renderer("mymod:Bad".into()),
            },
            RegisteredDebugEntry {
                owner: CallbackOwner::for_mod("other"),
                entry: DebugEntry::renderer("mymod:tick_cost".into()),
            },
            RegisteredDebugEntry {
                owner: CallbackOwner::for_mod("other"),
                entry: DebugEntry::renderer("other:queue".into()),
            },
        ]);
        let ids: Vec<&str> = kept.iter().map(|e| e.entry.id()).collect();
        assert_eq!(
            ids,
            ["mymod:tick_cost", "minecraft:custom_stat", "other:queue"]
        );
        assert_eq!(kept[0].owner.id(), "mymod");
        assert_eq!(kept[1].owner.id(), "mymod");
        assert_eq!(
            kept[0].entry.text_lines().map(|(group, _)| group),
            Some(&DebugGroup::Memory)
        );
        let mut selection = DebugEntries::default();
        selection.set_status("minecraft:custom_stat", Status::AlwaysOn);
        assert!(selection.enabled(&kept, "minecraft:custom_stat", false, false));
        assert!(selection.any_enabled(&kept, false, false));
    }

    #[test]
    fn custom_minecraft_entries_and_reduced_debug_info_opt_in_resolve() {
        let extra = [
            RegisteredDebugEntry {
                owner: CallbackOwner::for_mod("mymod"),
                entry: DebugEntry::text(
                    "minecraft:custom_stat".into(),
                    DebugGroup::Memory,
                    |_, _| {},
                )
                .with_reduced_debug_info(true),
            },
            RegisteredDebugEntry {
                owner: CallbackOwner::for_mod("mymod"),
                entry: DebugEntry::renderer("mymod:hidden_when_reduced".into()),
            },
        ];

        // Vanilla entries resolve by both bare path and full minecraft: ID.
        assert_eq!(entry(&extra, "fps").map(|e| e.path), Some("fps"));
        assert_eq!(entry(&extra, "minecraft:fps").map(|e| e.path), Some("fps"));
        // Custom minecraft:* mod entry resolves by full ID and bare path, with reduced = true.
        let custom = entry(&extra, "minecraft:custom_stat").unwrap();
        assert_eq!(custom.path, "minecraft:custom_stat");
        assert!(custom.reduced);
        assert_eq!(
            entry(&extra, "custom_stat").map(|e| e.path),
            Some("minecraft:custom_stat")
        );

        let mut selection = DebugEntries::default();
        selection.set_status("minecraft:custom_stat", Status::AlwaysOn);
        selection.set_status("mymod:hidden_when_reduced", Status::AlwaysOn);

        // Under reducedDebugInfo, the opted-in custom minecraft:* entry stays enabled while the
        // default mod entry is hidden.
        assert!(selection.enabled(&extra, "minecraft:custom_stat", false, true));
        assert!(selection.enabled(&extra, "custom_stat", false, true));
        assert!(selection.any_enabled(&extra, false, true));
        assert!(selection.enabled(&extra, "mymod:hidden_when_reduced", false, false));
        assert!(!selection.enabled(&extra, "mymod:hidden_when_reduced", false, true));
    }

    #[test]
    fn a_native_mods_lines_join_their_group_and_only_enabled_entries_are_asked() {
        mod_platform::testing::with_mods(Vec::new().into(), || {
            let cost = String::from("Tick cost: 3 ms");
            let entries = [
                RegisteredDebugEntry {
                    owner: CallbackOwner::for_mod("inspect"),
                    entry: DebugEntry::text(
                        "metrics:tick_cost".into(),
                        DebugGroup::Memory,
                        move |_, lines| {
                            assert_eq!(current_native_mod(), Some("inspect"));
                            lines.push(cost.clone());
                        },
                    ),
                },
                RegisteredDebugEntry {
                    owner: CallbackOwner::for_mod("metrics"),
                    entry: DebugEntry::text(
                        "metrics:queue".into(),
                        DebugGroup::Position,
                        |_, _| panic!("the disabled entry must not be called"),
                    ),
                },
            ];
            let host = FakeHost::new();
            let mut selection = DebugEntries::default();
            selection.set_status("metrics:tick_cost", Status::InOverlay);
            assert!(
                mod_lines(&entries, &host, &selection, false, false)
                    .next()
                    .is_none()
            );
            assert!(
                mod_lines(&entries, &host, &selection, true, true)
                    .next()
                    .is_none()
            );

            let groups: Vec<_> = mod_lines(&entries, &host, &selection, true, false).collect();
            assert_eq!(
                groups,
                [("memory".into(), vec!["Tick cost: 3 ms".to_owned()])]
            );
            assert_eq!(current_native_mod(), None);
        });
    }

    #[test]
    fn a_named_group_keeps_its_lines_together_and_minecraft_names_join_vanillas() {
        mod_platform::testing::with_mods(Vec::new().into(), || {
            let entry = |id: &str, group: &str, line: &'static str| RegisteredDebugEntry {
                owner: CallbackOwner::for_mod("mymod"),
                entry: DebugEntry::text(
                    id.into(),
                    match group {
                        "memory" => DebugGroup::Memory,
                        named => DebugGroup::Named(Identifier::parse_namespaced(named).unwrap()),
                    },
                    move |_, lines| lines.push(line.into()),
                ),
            };
            let entries = [
                entry("mymod:queue", "mymod:pipes", "Queue: 3"),
                entry("mymod:heap", "minecraft:memory", "Heap: 12 MiB"),
                entry("mymod:flow", "mymod:pipes", "Flow: 40/t"),
                entry("mymod:cache", "memory", "Cache: 2 MiB"),
            ];
            let mut selection = DebugEntries::default();
            for e in &entries {
                selection.set_status(e.entry.id(), Status::AlwaysOn);
            }
            let mut displayer = crate::debug_overlay::Displayer::default();
            displayer.group("memory", ["Mem: 25% 1046/4096MiB".to_owned()]);
            for (group, lines) in mod_lines(&entries, &FakeHost::new(), &selection, true, false) {
                displayer.group(group, lines);
            }
            let (left, right) = displayer.columns(&[]);
            assert_eq!(
                left,
                ["Mem: 25% 1046/4096MiB", "Heap: 12 MiB", "Cache: 2 MiB", ""]
            );
            assert_eq!(right, ["Queue: 3", "Flow: 40/t", ""]);
        });
    }

    #[test]
    fn a_panicking_debug_entry_is_skipped_and_the_next_mod_keeps_drawing() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct Empty;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Empty {}
        mod_platform::testing::with_mods(
            vec![
                mod_api::ModEntry::new("debug_panic", Box::new(Empty)),
                mod_api::ModEntry::new("debug_healthy", Box::new(Empty)),
            ]
            .into(),
            || {
                let runs = std::sync::Arc::new(AtomicUsize::new(0));
                let bad_runs = runs.clone();
                let entries = [
                    RegisteredDebugEntry {
                        owner: CallbackOwner::new("debug_panic").unwrap(),
                        entry: DebugEntry::text(
                            "debug_panic:text".into(),
                            DebugGroup::Memory,
                            move |_, lines| {
                                bad_runs.fetch_add(1, Ordering::Relaxed);
                                lines.push("must not be shown".into());
                                panic!("broken debug entry");
                            },
                        ),
                    },
                    RegisteredDebugEntry {
                        owner: CallbackOwner::new("debug_healthy").unwrap(),
                        entry: DebugEntry::text(
                            "debug_healthy:text".into(),
                            DebugGroup::Position,
                            |_, lines| {
                                lines.push(current_native_mod().unwrap_or("wrong scope").into());
                            },
                        ),
                    },
                ];
                let mut selection = DebugEntries::default();
                selection.set_status("debug_panic:text", Status::AlwaysOn);
                selection.set_status("debug_healthy:text", Status::AlwaysOn);
                let host = FakeHost::new();
                for _ in 0..2 {
                    assert_eq!(
                        mod_lines(&entries, &host, &selection, true, false).collect::<Vec<_>>(),
                        [("position".into(), vec!["debug_healthy".to_owned()]),]
                    );
                }
                assert_eq!(runs.load(Ordering::Relaxed), 1);
                assert_eq!(current_native_mod(), None);
            },
        );
    }

    #[test]
    fn a_native_mods_renderer_entry_is_listed_as_a_renderer_and_has_no_lines() {
        let entry = RegisteredDebugEntry {
            owner: CallbackOwner::for_mod("mymod"),
            entry: DebugEntry::renderer("mymod:pipes".into()),
        };
        assert_eq!(as_entry(&entry).category, Category::Renderer);
        assert!(!as_entry(&entry).reduced);
        let mut selection = DebugEntries::default();
        selection.set_status("mymod:pipes", Status::AlwaysOn);
        assert!(
            mod_lines(&[entry], &FakeHost::new(), &selection, true, false)
                .next()
                .is_none()
        );
    }

    #[test]
    fn the_enabled_version_changes_with_every_rebuild_of_the_list() {
        let mut e = DebugEntries::default();
        let first = e.enabled_version();
        assert_eq!(e.status("mymod:pipes"), Status::Never);
        assert_eq!(e.enabled_version(), first);

        e.invalidate();
        assert_eq!(e.enabled_version(), first + 1);
        e.invalidate();
        assert_eq!(e.enabled_version(), first + 2);

        e.set_status("mymod:pipes", Status::AlwaysOn);
        let set = e.enabled_version();
        assert_ne!(set, first);

        e.load_profile(Profile::Performance);
        assert_ne!(e.enabled_version(), set);
    }

    #[test]
    fn an_entry_with_no_lines_joins_no_group() {
        mod_platform::testing::with_mods(Vec::new().into(), || {
            let entries = [RegisteredDebugEntry {
                owner: CallbackOwner::for_mod("mymod"),
                entry: DebugEntry::text("mymod:quiet".into(), DebugGroup::Light, |_, _| {}),
            }];
            let mut selection = DebugEntries::default();
            selection.set_status("mymod:quiet", Status::AlwaysOn);
            assert!(
                mod_lines(&entries, &FakeHost::new(), &selection, false, false)
                    .next()
                    .is_none()
            );
        });
    }
}
