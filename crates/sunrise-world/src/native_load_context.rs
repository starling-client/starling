//! Data-only inputs carried from an owning load boundary to native load hooks.
use mod_api::__private::BoundItemTags;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

/// The phase that owns this data load, selected before handing work to loaders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DataLoadOrigin {
    ServerDataLoad,
    ClientPreload,
}

/// Immutable metadata captured with the loaded item data. Later data loads
/// borrow the same registry snapshot instead of scanning it again.
pub struct LoadInputs {
    registries: BTreeMap<String, Arc<[String]>>,
    settings_root: PathBuf,
}

impl LoadInputs {
    pub fn new(registries: BTreeMap<String, Vec<String>>, settings_root: PathBuf) -> Self {
        Self {
            registries: registries
                .into_iter()
                .map(|(name, ids)| (name, ids.into()))
                .collect(),
            settings_root,
        }
    }
}

type SettingsTable = Arc<[(String, Arc<[(String, String)]>)]>;
type RegisteredSettingsTable = Arc<[(String, Arc<[(String, Option<String>)]>)]>;

/// A load's registry snapshot with the settings captured for its mods.
#[derive(Clone)]
struct Snapshot {
    inputs: Arc<LoadInputs>,
    settings: SettingsTable,
    /// Registered setting values captured by their known `(mod_id, key)` pairs
    /// at the owning load boundary without requiring directory enumeration. Each entry records whether the key was
    /// present and valid (`Some`) or absent/refused (`None`), so loop readers
    /// and live settings always observe the same single capture.
    registered_settings: RegisteredSettingsTable,
}

/// The settings one candidate generation reads, captured once: the generic per-mod values and
/// the registered `(mod_id, key)` values. Everything of that generation (reload-listener
/// registration, the data load, the loop readers and the live store) is built from the same
/// capture, so none of them can see a file another capture would not.
#[derive(Clone)]
pub struct SettingsCapture {
    settings: SettingsTable,
    registered_settings: RegisteredSettingsTable,
}

impl SettingsCapture {
    /// Captures generic settings for `mod_ids` and registered settings by their known
    /// `(mod_id, key)` pairs from `settings_root`.
    pub fn new<'a, 'b>(
        settings_root: &Path,
        mod_ids: impl IntoIterator<Item = &'a str>,
        registered_keys: impl IntoIterator<Item = (&'b str, &'b str)>,
    ) -> Self {
        Self {
            registered_settings: capture_registered_table(settings_root, registered_keys),
            settings: shared_settings(capture_settings(settings_root, mod_ids)),
        }
    }

    /// The initial load's capture over registration's already-captured, sorted settings:
    /// only `mod_ids` stay visible, and registered keys registration did not capture are
    /// read by known key.
    pub fn from_registration<'a, 'b>(
        settings_root: &Path,
        mod_ids: impl IntoIterator<Item = &'a str>,
        settings: Arc<[(String, Arc<[(String, String)]>)]>,
        registered_keys: impl IntoIterator<Item = (&'b str, &'b str)>,
    ) -> Self {
        let registered_settings =
            merge_registered_with_snapshot(settings_root, &settings, registered_keys);
        let ids: BTreeSet<_> = mod_ids.into_iter().collect();
        let settings = if settings.iter().all(|(id, _)| ids.contains(id.as_str())) {
            settings
        } else {
            settings
                .iter()
                .filter(|(id, _)| ids.contains(id.as_str()))
                .cloned()
                .collect()
        };
        Self {
            settings,
            registered_settings,
        }
    }

    /// A mod's setting as captured, with no file I/O.
    pub fn setting(&self, mod_id: &str, key: &str) -> Option<&str> {
        if let Ok(mod_at) = self
            .registered_settings
            .binary_search_by(|(known, _)| known.as_str().cmp(mod_id))
        {
            let values = &self.registered_settings[mod_at].1;
            if let Ok(at) = values.binary_search_by(|(known, _)| known.as_str().cmp(key)) {
                return values[at].1.as_deref();
            }
        }
        lookup_settings_table(&self.settings, mod_id, key)
    }
}

impl Snapshot {
    /// Captures settings for `mod_ids`; the registry inputs are shared, not scanned again.
    fn capture<'a>(inputs: Arc<LoadInputs>, mod_ids: impl IntoIterator<Item = &'a str>) -> Self {
        Self::capture_with_registered(inputs, mod_ids, std::iter::empty())
    }

    /// Captures generic settings for `mod_ids` and registered settings by
    /// their known `(mod_id, key)` pairs at the same owning load boundary.
    fn capture_with_registered<'a, 'b>(
        inputs: Arc<LoadInputs>,
        mod_ids: impl IntoIterator<Item = &'a str>,
        registered_keys: impl IntoIterator<Item = (&'b str, &'b str)>,
    ) -> Self {
        let capture = SettingsCapture::new(&inputs.settings_root, mod_ids, registered_keys);
        Self::from_capture(inputs, capture)
    }

    fn from_capture(inputs: Arc<LoadInputs>, capture: SettingsCapture) -> Self {
        Self {
            inputs,
            settings: capture.settings,
            registered_settings: capture.registered_settings,
        }
    }

    /// Builds a snapshot sharing registration's already-captured generic settings
    /// while filling any registered `(mod_id, key)` pairs missing from that bulk
    /// capture by known key with the same no-follow regular-file admission.
    fn from_settings_snapshot_with_registered<'a, 'b>(
        inputs: Arc<LoadInputs>,
        mod_ids: impl IntoIterator<Item = &'a str>,
        settings: SettingsTable,
        registered_keys: impl IntoIterator<Item = (&'b str, &'b str)>,
    ) -> Self {
        let capture = SettingsCapture::from_registration(
            &inputs.settings_root,
            mod_ids,
            settings,
            registered_keys,
        );
        Self::from_capture(inputs, capture)
    }

    fn registry(&self, name: &str) -> Option<&[String]> {
        self.inputs.registries.get(name).map(Arc::as_ref)
    }

    fn setting(&self, mod_id: &str, key: &str) -> Option<&str> {
        if let Ok(mod_at) = self
            .registered_settings
            .binary_search_by(|(known, _)| known.as_str().cmp(mod_id))
        {
            let values = &self.registered_settings[mod_at].1;
            if let Ok(at) = values.binary_search_by(|(known, _)| known.as_str().cmp(key)) {
                return values[at].1.as_deref();
            }
        }
        lookup_settings_table(&self.settings, mod_id, key)
    }
}

fn lookup_settings_table<'a>(
    settings: &'a [(String, Arc<[(String, String)]>)],
    mod_id: &str,
    key: &str,
) -> Option<&'a str> {
    let mod_at = settings
        .binary_search_by(|(known, _)| known.as_str().cmp(mod_id))
        .ok()?;
    let values = &settings[mod_at].1;
    let at = values
        .binary_search_by(|(known, _)| known.as_str().cmp(key))
        .ok()?;
    Some(values[at].1.as_str())
}

fn shared_settings(settings: BTreeMap<String, BTreeMap<String, String>>) -> SettingsTable {
    settings
        .into_iter()
        .map(|(id, values)| (id, values.into_iter().collect()))
        .collect()
}

fn capture_registered_table<'a>(
    settings_root: &Path,
    registered_keys: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> RegisteredSettingsTable {
    let mut by_owner: BTreeMap<&'a str, BTreeSet<&'a str>> = BTreeMap::new();
    for (owner, key) in registered_keys {
        by_owner.entry(owner).or_default().insert(key);
    }
    if by_owner.is_empty() {
        return Arc::from([]);
    }
    let captured = capture_registered_settings_grouped(settings_root, &by_owner);
    by_owner
        .into_iter()
        .map(|(owner, keys)| {
            let owner_captured = captured.get(owner);
            let entries: Arc<[(String, Option<String>)]> = keys
                .into_iter()
                .map(|key| {
                    let value = owner_captured.and_then(|map| map.get(key)).cloned();
                    (key.to_owned(), value)
                })
                .collect();
            (owner.to_owned(), entries)
        })
        .collect()
}

fn merge_registered_with_snapshot<'a>(
    settings_root: &Path,
    settings: &[(String, Arc<[(String, String)]>)],
    registered_keys: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> RegisteredSettingsTable {
    let mut by_owner: BTreeMap<&'a str, BTreeSet<&'a str>> = BTreeMap::new();
    for (owner, key) in registered_keys {
        by_owner.entry(owner).or_default().insert(key);
    }
    if by_owner.is_empty() {
        return Arc::from([]);
    }
    let mut missing_by_owner: BTreeMap<&'a str, BTreeSet<&'a str>> = BTreeMap::new();
    for (&owner, keys) in &by_owner {
        for &key in keys {
            if lookup_settings_table(settings, owner, key).is_none() {
                missing_by_owner.entry(owner).or_default().insert(key);
            }
        }
    }
    let captured = if missing_by_owner.is_empty() {
        BTreeMap::new()
    } else {
        capture_registered_settings_grouped(settings_root, &missing_by_owner)
    };
    by_owner
        .into_iter()
        .map(|(owner, keys)| {
            let owner_captured = captured.get(owner);
            let entries: Arc<[(String, Option<String>)]> = keys
                .into_iter()
                .map(|key| {
                    let value = lookup_settings_table(settings, owner, key)
                        .map(str::to_owned)
                        .or_else(|| owner_captured.and_then(|map| map.get(key)).cloned());
                    (key.to_owned(), value)
                })
                .collect();
            (owner.to_owned(), entries)
        })
        .collect()
}

/// Origin and borrowed engine inputs for one owning load. This contains no
/// game, world handles or mutable registry access.
#[derive(Clone)]
pub struct LoadContext {
    origin: DataLoadOrigin,
    snapshot: Snapshot,
    fluid_tags: Option<Arc<[(String, Arc<[String]>)]>>,
}

impl LoadContext {
    /// Called at a load boundary, never by a reader of already-loaded data.
    /// Captures settings for the owning phase's loaded package IDs; reusing the
    /// registry inputs for another load still captures fresh settings.
    pub fn new<'a>(
        origin: DataLoadOrigin,
        inputs: Arc<LoadInputs>,
        mod_ids: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        Self::new_with_registered(origin, inputs, mod_ids, std::iter::empty())
    }

    /// Called at a client owning load boundary to capture both generic package
    /// settings and registered settings by their known `(mod_id, key)` pairs.
    pub fn new_with_registered<'a, 'b>(
        origin: DataLoadOrigin,
        inputs: Arc<LoadInputs>,
        mod_ids: impl IntoIterator<Item = &'a str>,
        registered_keys: impl IntoIterator<Item = (&'b str, &'b str)>,
    ) -> Self {
        Self {
            origin,
            snapshot: Snapshot::capture_with_registered(inputs, mod_ids, registered_keys),
            fluid_tags: None,
        }
    }

    /// The initial resource load over registration's already-captured, sorted
    /// settings. Only this load's admitted package IDs remain visible; values
    /// and (when the scope is unchanged) the outer snapshot are shared.
    pub fn from_settings_snapshot<'a>(
        origin: DataLoadOrigin,
        inputs: Arc<LoadInputs>,
        mod_ids: impl IntoIterator<Item = &'a str>,
        settings: Arc<[(String, Arc<[(String, String)]>)]>,
    ) -> Self {
        Self::from_settings_snapshot_with_registered(
            origin,
            inputs,
            mod_ids,
            settings,
            std::iter::empty(),
        )
    }

    /// The initial client resource load sharing registration's already-captured
    /// settings while ensuring every registered `(mod_id, key)` is captured by
    /// known key without generic snapshot key, enumeration, or aggregate budgets.
    pub fn from_settings_snapshot_with_registered<'a, 'b>(
        origin: DataLoadOrigin,
        inputs: Arc<LoadInputs>,
        mod_ids: impl IntoIterator<Item = &'a str>,
        settings: Arc<[(String, Arc<[(String, String)]>)]>,
        registered_keys: impl IntoIterator<Item = (&'b str, &'b str)>,
    ) -> Self {
        Self {
            origin,
            snapshot: Snapshot::from_settings_snapshot_with_registered(
                inputs,
                mod_ids,
                settings,
                registered_keys,
            ),
            fluid_tags: None,
        }
    }

    /// An owning load of `origin` over a settings capture taken earlier, so the load reads
    /// the values the candidate's other readers were given.
    pub fn with_capture(
        origin: DataLoadOrigin,
        inputs: Arc<LoadInputs>,
        capture: SettingsCapture,
    ) -> Self {
        Self {
            origin,
            snapshot: Snapshot::from_capture(inputs, capture),
            fluid_tags: None,
        }
    }

    /// A new owning load of `origin` over this load's registry inputs, with
    /// fresh settings and no recipe fluid tags.
    pub fn for_load<'a>(
        &self,
        origin: DataLoadOrigin,
        mod_ids: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        Self::new(origin, self.snapshot.inputs.clone(), mod_ids)
    }

    /// Attach the recipe pass's fluid tags (`data/<ns>/tags/fluid`), even if
    /// empty, while sharing this load's existing registry and settings snapshot.
    pub fn with_fluid_tags(&self, tags: HashMap<String, Vec<String>>) -> Self {
        Self {
            origin: self.origin,
            snapshot: self.snapshot.clone(),
            fluid_tags: Some(
                tags.into_iter()
                    .map(|(tag, fluids)| (tag, fluids.into()))
                    .collect(),
            ),
        }
    }

    /// Borrows each recipe fluid tag's members in source order. `None` means
    /// there is no recipe overlay; an empty overlay still returns `Some`.
    pub fn fluid_tags(&self) -> Option<&[(String, Arc<[String]>)]> {
        self.fluid_tags.as_deref()
    }

    /// Readers of this load's resource generation, with the same captured
    /// settings. Starting a distinct generation uses `loop_context` instead.
    pub fn captured_loop_context(&self, side: LoopSide) -> LoopContext {
        LoopContext {
            side,
            snapshot: self.snapshot.clone(),
            recipe_properties: Default::default(),
            item_tags: None,
        }
    }

    /// The context of a menu, recipe or player-list loop of `side`, over this
    /// load's registry inputs, with fresh settings. Built where the side's loop
    /// owner starts or reloads its resources, never by a reader.
    pub fn loop_context<'a>(
        &self,
        side: LoopSide,
        mod_ids: impl IntoIterator<Item = &'a str>,
    ) -> LoopContext {
        self.loop_context_with_registered(side, mod_ids, std::iter::empty())
    }

    /// The context of a client menu, recipe or player-list loop over this
    /// load's registry inputs, capturing generic package settings and
    /// registered settings by their known `(mod_id, key)` pairs.
    pub fn loop_context_with_registered<'a, 'b>(
        &self,
        side: LoopSide,
        mod_ids: impl IntoIterator<Item = &'a str>,
        registered_keys: impl IntoIterator<Item = (&'b str, &'b str)>,
    ) -> LoopContext {
        LoopContext {
            side,
            snapshot: Snapshot::capture_with_registered(
                self.snapshot.inputs.clone(),
                mod_ids,
                registered_keys,
            ),
            recipe_properties: Default::default(),
            item_tags: None,
        }
    }

    /// The context of a loop of `side` over this load's registry inputs and a settings capture
    /// taken earlier, not a new one.
    pub fn loop_context_with_capture(
        &self,
        side: LoopSide,
        capture: SettingsCapture,
    ) -> LoopContext {
        LoopContext {
            side,
            snapshot: Snapshot::from_capture(self.snapshot.inputs.clone(), capture),
            recipe_properties: Default::default(),
            item_tags: None,
        }
    }

    /// The context of the server's world generation, over this load's
    /// registry inputs, with fresh settings. Built where the server starts or
    /// reloads its resources, never by a reader.
    pub fn worldgen_context<'a>(
        &self,
        mod_ids: impl IntoIterator<Item = &'a str>,
    ) -> WorldgenContext {
        WorldgenContext {
            snapshot: Snapshot::capture(self.snapshot.inputs.clone(), mod_ids),
        }
    }

    pub fn origin(&self) -> DataLoadOrigin {
        self.origin
    }

    /// Names in registry ID order, from the load's registry snapshot.
    pub fn registry(&self, name: &str) -> Option<&[String]> {
        self.snapshot.registry(name)
    }

    /// A loaded mod's setting captured at this load boundary, with no file I/O.
    pub fn setting(&self, mod_id: &str, key: &str) -> Option<&str> {
        self.snapshot.setting(mod_id, key)
    }
}

/// The side a loop context serves, chosen where the context is built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoopSide {
    Server,
    Client,
}

/// Registries, settings and the side for hooks fired inside a menu, recipe or
/// player-list loop, where no game is at hand. It holds no game, world handle
/// or mutable registry access, and is built once per resource generation.
#[derive(Clone)]
pub struct LoopContext {
    side: LoopSide,
    snapshot: Snapshot,
    recipe_properties: Arc<HashMap<String, std::collections::HashSet<String>>>,
    item_tags: Option<Arc<BoundItemTags>>,
}

#[path = "native_load_context/recipe_properties.rs"]
mod recipe_properties;

impl LoopContext {
    /// Publish the API adapter's bound item tags at a load or synchronization boundary.
    /// `None` marks unavailable tags. Clones retain the previous generation.
    pub fn set_item_tags(&mut self, tags: Option<BoundItemTags>) {
        self.item_tags = tags.map(Arc::new);
    }

    /// Borrow the published item tags when building the API view of this loop.
    pub fn item_tags(&self) -> Option<&BoundItemTags> {
        self.item_tags.as_deref()
    }

    pub fn side(&self) -> LoopSide {
        self.side
    }

    /// The same resource generation's settings and registries for world generation.
    /// Sharing this snapshot performs no capture or diagnostic reporting.
    pub fn worldgen_context(&self) -> WorldgenContext {
        WorldgenContext {
            snapshot: self.snapshot.clone(),
        }
    }

    /// Names in registry ID order, from the registry snapshot it was built over.
    pub fn registry(&self, name: &str) -> Option<&[String]> {
        self.snapshot.registry(name)
    }

    /// A loaded mod's setting captured when this context was built, with no file I/O.
    pub fn setting(&self, mod_id: &str, key: &str) -> Option<&str> {
        self.snapshot.setting(mod_id, key)
    }

    /// The settings root captured at the owning load boundary.
    pub fn settings_root(&self) -> &Path {
        &self.snapshot.inputs.settings_root
    }
}

/// Capture regular UTF-8 files only, never following symlinks (even within the root).
/// Unreadable, non-regular, non-UTF-8-named, or invalid-UTF-8 entries are skipped and reported.
pub fn capture_settings<'a>(
    settings_root: &Path,
    mod_ids: impl IntoIterator<Item = &'a str>,
) -> BTreeMap<String, BTreeMap<String, String>> {
    capture_settings_with_diagnostics(settings_root, mod_ids, |diagnostics| {
        log::warn!("Bulk mod settings snapshot omissions at {settings_root:?}:\n{diagnostics}");
    })
}

/// Capture registered settings by their known `(mod_id, key)` pairs using the
/// same no-follow regular-file UTF-8 admission as `capture_settings`, without
/// directory enumeration.
pub fn capture_registered_settings<'a>(
    settings_root: &Path,
    keys: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut by_owner: BTreeMap<&'a str, BTreeSet<&'a str>> = BTreeMap::new();
    for (mod_id, key) in keys {
        by_owner.entry(mod_id).or_default().insert(key);
    }
    capture_registered_settings_grouped(settings_root, &by_owner)
}

fn capture_registered_settings_grouped(
    settings_root: &Path,
    by_owner: &BTreeMap<&str, BTreeSet<&str>>,
) -> BTreeMap<String, BTreeMap<String, String>> {
    capture_registered_settings_with_diagnostics(settings_root, by_owner, |diagnostics| {
        log::warn!("Registered mod settings omissions at {settings_root:?}:\n{diagnostics}");
    })
}

fn capture_registered_settings_with_diagnostics(
    settings_root: &Path,
    by_owner: &BTreeMap<&str, BTreeSet<&str>>,
    report: impl FnOnce(&str),
) -> BTreeMap<String, BTreeMap<String, String>> {
    use std::fmt::Write;
    let mut diagnostics = String::new();
    let settings =
        capture_registered_settings_with_report(settings_root, by_owner, |mod_id, key, reason| {
            match key {
                Some(key) => writeln!(diagnostics, "  {mod_id:?}/{key:?}: {reason}"),
                None => writeln!(diagnostics, "  mod {mod_id:?}: {reason}"),
            }
            .expect("writing diagnostics to a String");
        });
    if !diagnostics.is_empty() {
        report(&diagnostics);
    }
    settings
}

fn is_plain_component(component: &str) -> bool {
    let mut components = Path::new(component).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
}

fn read_captured_value(mut file: fs::File) -> Result<String, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("read failed: {error}"))?;
    let value = String::from_utf8(bytes)
        .map_err(|error| format!("invalid UTF-8: {}", error.utf8_error()))?;
    Ok(value.trim().to_owned())
}

fn capture_registered_settings_with_report(
    settings_root: &Path,
    by_owner: &BTreeMap<&str, BTreeSet<&str>>,
    mut report: impl FnMut(&str, Option<&OsStr>, &dyn std::fmt::Display),
) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut settings = BTreeMap::new();
    for (&mod_id, owner_keys) in by_owner {
        if !is_plain_component(mod_id) {
            report(mod_id, None, &"unsafe package path");
            continue;
        }
        let directory = match open_settings_dir(settings_root, mod_id) {
            Ok(directory) => directory,
            Err(SnapshotError::Missing) => continue,
            Err(SnapshotError::Directory(error)) => {
                report(
                    mod_id,
                    None,
                    &format_args!("cannot inspect settings directory: {error}"),
                );
                continue;
            }
            Err(_) => continue,
        };
        let mut values = BTreeMap::new();
        for &key in owner_keys {
            if !is_plain_component(key) {
                report(mod_id, Some(OsStr::new(key)), &"unsafe setting key");
                continue;
            }
            let file = match open_regular_setting_file_optional(&directory, OsStr::new(key)) {
                Ok(Some(file)) => file,
                Ok(None) => continue,
                Err(error) => {
                    report(
                        mod_id,
                        Some(OsStr::new(key)),
                        &format_args!("unsafe or unreadable entry: {error}"),
                    );
                    continue;
                }
            };
            match read_captured_value(file) {
                Ok(value) => {
                    values.insert(key.to_owned(), value);
                }
                Err(error) => report(mod_id, Some(OsStr::new(key)), &error),
            }
        }
        if !values.is_empty() {
            settings.insert(mod_id.to_owned(), values);
        }
    }
    settings
}

fn capture_settings_with_diagnostics<'a>(
    settings_root: &Path,
    mod_ids: impl IntoIterator<Item = &'a str>,
    report: impl FnOnce(&str),
) -> BTreeMap<String, BTreeMap<String, String>> {
    use std::fmt::Write;
    let mut diagnostics = String::new();
    let settings = capture_settings_with_report(settings_root, mod_ids, |mod_id, key, reason| {
        match key {
            Some(key) => writeln!(diagnostics, "  {mod_id:?}/{key:?}: {reason}"),
            None => writeln!(diagnostics, "  mod {mod_id:?}: {reason}"),
        }
        .expect("writing diagnostics to a String");
    });
    if !diagnostics.is_empty() {
        report(&diagnostics);
    }
    settings
}

fn capture_settings_with_report<'a>(
    settings_root: &Path,
    mod_ids: impl IntoIterator<Item = &'a str>,
    mut report: impl FnMut(&str, Option<&OsStr>, &dyn std::fmt::Display),
) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut settings = BTreeMap::new();
    for mod_id in mod_ids
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
    {
        // Package ids are normally validated at load time; do not let this public
        // filesystem boundary interpret a path supplied by a caller.
        if !is_plain_component(mod_id) {
            report(mod_id, None, &"unsafe package path");
            continue;
        }
        let files = match snapshot_files(settings_root, mod_id) {
            Ok(files) => files,
            // A mod need not declare or save any settings. Absence of its
            // directory (or the settings root) does not omit a key.
            Err(SnapshotError::Missing) => continue,
            Err(SnapshotError::Directory(error)) => {
                report(
                    mod_id,
                    None,
                    &format_args!("cannot inspect settings directory: {error}"),
                );
                continue;
            }
            Err(SnapshotError::Enumeration { names, error }) => {
                for key in &names {
                    report(
                        mod_id,
                        Some(key),
                        &format_args!("enumeration failed: {error}"),
                    );
                }
                report(
                    mod_id,
                    None,
                    &format_args!("cannot finish enumerating settings: {error}"),
                );
                continue;
            }
        };
        let mut values = BTreeMap::new();
        for (key, file) in files {
            let key = match key.into_string() {
                Ok(key) => key,
                Err(name) => {
                    report(mod_id, Some(name.as_os_str()), &"non-UTF-8 setting name");
                    continue;
                }
            };
            let file = match file {
                Ok(file) => file,
                Err(error) => {
                    report(
                        mod_id,
                        Some(key.as_ref()),
                        &format_args!("unsafe or unreadable entry: {error}"),
                    );
                    continue;
                }
            };
            match read_captured_value(file) {
                Ok(value) => {
                    values.insert(key, value);
                }
                Err(error) => report(mod_id, Some(key.as_ref()), &error),
            }
        }
        settings.insert(mod_id.to_owned(), values);
    }
    settings
}

#[derive(Debug)]
enum SnapshotError {
    Missing,
    Directory(std::io::Error),
    Enumeration {
        names: Vec<OsString>,
        error: std::io::Error,
    },
}

impl From<std::io::Error> for SnapshotError {
    fn from(error: std::io::Error) -> Self {
        Self::Directory(error)
    }
}

// Only a missing root/package directory means no settings. A missing individual
// entry or a failed enumeration still reports the unreadable scope or key.
fn settings_directory<T>(result: std::io::Result<T>) -> Result<T, SnapshotError> {
    result.map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            SnapshotError::Missing
        } else {
            SnapshotError::Directory(error)
        }
    })
}

fn sorted_names(
    names: impl Iterator<Item = std::io::Result<OsString>>,
) -> Result<Vec<OsString>, SnapshotError> {
    let mut result = Vec::new();
    for name in names {
        let name = match name {
            Ok(name) => name,
            Err(error) => {
                return Err(SnapshotError::Enumeration {
                    names: result,
                    error,
                });
            }
        };
        result.push(name);
    }
    // The native encoding is a superset of UTF-8: valid keys retain their
    // existing byte order without revalidating UTF-8 on every comparison.
    result.sort_unstable_by(|a, b| a.as_encoded_bytes().cmp(b.as_encoded_bytes()));
    Ok(result)
}

#[cfg(unix)]
fn open_settings_dir(root: &Path, mod_id: &str) -> Result<std::os::fd::OwnedFd, SnapshotError> {
    use rustix::fs::{Mode, OFlags, open, openat};
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let root = settings_directory(
        open(root, flags | OFlags::DIRECTORY, Mode::empty()).map_err(Into::into),
    )?;
    settings_directory(
        openat(&root, mod_id, flags | OFlags::DIRECTORY, Mode::empty()).map_err(Into::into),
    )
}

#[cfg(unix)]
fn open_regular_setting_file(
    directory: &std::os::fd::OwnedFd,
    key: &OsStr,
    after_open: impl FnOnce(&str),
) -> std::io::Result<fs::File> {
    use rustix::fs::{Mode, OFlags, openat};
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let file = fs::File::from(openat(directory, key, flags, Mode::empty())?);
    if let Some(key) = key.to_str() {
        after_open(key);
    }
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "entry is not a regular file",
        ));
    }
    Ok(file)
}

#[cfg(unix)]
fn open_regular_setting_file_optional(
    directory: &std::os::fd::OwnedFd,
    key: &OsStr,
) -> std::io::Result<Option<fs::File>> {
    match open_regular_setting_file(directory, key, |_| {}) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn snapshot_files(
    root: &Path,
    mod_id: &str,
) -> Result<impl Iterator<Item = (OsString, std::io::Result<fs::File>)>, SnapshotError> {
    snapshot_files_with_hook(root, mod_id, |_| {})
}

#[cfg(unix)]
fn snapshot_files_with_hook(
    root: &Path,
    mod_id: &str,
    after_open: impl Fn(&str),
) -> Result<impl Iterator<Item = (OsString, std::io::Result<fs::File>)>, SnapshotError> {
    use rustix::fs::Dir;
    use std::os::unix::ffi::OsStrExt;
    let directory = open_settings_dir(root, mod_id)?;
    let entries = Dir::read_from(&directory).map_err(std::io::Error::from)?;
    let names = sorted_names(entries.filter_map(|entry| {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => return Some(Err(error.into())),
        };
        let name = entry.file_name();
        if name.to_bytes() == b"." || name.to_bytes() == b".." {
            return None;
        }
        Some(Ok(OsStr::from_bytes(name.to_bytes()).to_owned()))
    }))?;
    Ok(names.into_iter().map(move |key| {
        let file = open_regular_setting_file(&directory, key.as_os_str(), &after_open);
        (key, file)
    }))
}

#[cfg(windows)]
fn open_settings_dir(root: &Path, mod_id: &str) -> Result<cap_std::fs::Dir, SnapshotError> {
    use cap_fs_ext::DirExt;
    use cap_std::fs::Dir;
    // Open the parent as a capability, then refuse a reparse point at the root
    // and package boundary as well as at each individual file boundary.
    let parent = root
        .parent()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let name = root
        .file_name()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let parent = settings_directory(Dir::open_ambient_dir(
        if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        },
        cap_std::ambient_authority(),
    ))?;
    let root = settings_directory(parent.open_dir_nofollow(name))?;
    settings_directory(root.open_dir_nofollow(mod_id))
}

#[cfg(windows)]
fn open_regular_setting_file(
    directory: &cap_std::fs::Dir,
    key: &OsStr,
) -> std::io::Result<fs::File> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    use cap_std::fs::{MetadataExt, OpenOptions};
    let metadata = directory.symlink_metadata(key)?;
    if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "entry is not a regular file or is a reparse point",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = directory.open_with(key, &options)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "entry is not a regular file or is a reparse point",
        ));
    }
    Ok(file.into_std())
}

#[cfg(windows)]
fn open_regular_setting_file_optional(
    directory: &cap_std::fs::Dir,
    key: &OsStr,
) -> std::io::Result<Option<fs::File>> {
    match open_regular_setting_file(directory, key) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
fn snapshot_files(
    root: &Path,
    mod_id: &str,
) -> Result<impl Iterator<Item = (OsString, std::io::Result<fs::File>)>, SnapshotError> {
    let directory = open_settings_dir(root, mod_id)?;
    let names = sorted_names(directory.entries()?.map(|entry| Ok(entry?.file_name())))?;
    Ok(names.into_iter().map(move |key| {
        let file = open_regular_setting_file(&directory, key.as_os_str());
        (key, file)
    }))
}

#[cfg(not(any(unix, windows)))]
struct UnsupportedSettingsDir;

#[cfg(not(any(unix, windows)))]
fn open_settings_dir(root: &Path, mod_id: &str) -> Result<UnsupportedSettingsDir, SnapshotError> {
    let _ = (root, mod_id);
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported).into())
}

#[cfg(not(any(unix, windows)))]
fn open_regular_setting_file_optional(
    directory: &UnsupportedSettingsDir,
    key: &OsStr,
) -> std::io::Result<Option<fs::File>> {
    let _ = (directory, key);
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

#[cfg(not(any(unix, windows)))]
fn snapshot_files(
    root: &Path,
    mod_id: &str,
) -> Result<impl Iterator<Item = (OsString, std::io::Result<fs::File>)>, SnapshotError> {
    // Fail closed on targets without descriptor-relative no-follow traversal.
    let _ = (root, mod_id);
    Err::<std::iter::Empty<(OsString, std::io::Result<fs::File>)>, _>(
        std::io::Error::from(std::io::ErrorKind::Unsupported).into(),
    )
}

/// Registries and settings for hooks fired while a feature generates, on a
/// worldgen worker or the server thread, where no game is at hand. It holds no
/// game, world handle or mutable registry access, and is built once per
/// resource generation of the server.
pub struct WorldgenContext {
    snapshot: Snapshot,
}

impl WorldgenContext {
    /// Names in registry ID order, from the registry snapshot it was built over.
    pub fn registry(&self, name: &str) -> Option<&[String]> {
        self.snapshot.registry(name)
    }

    /// A loaded mod's setting captured when this context was built, with no file I/O.
    pub fn setting(&self, mod_id: &str, key: &str) -> Option<&str> {
        self.snapshot.setting(mod_id, key)
    }
}

/// Read the same per-key storage as the runtime settings service. No folder is
/// created, and unrelated mod data files are not scanned or loaded eagerly.
pub fn read_setting(directory: &Path, key: &str) -> Option<String> {
    fs::read_to_string(directory.join(key))
        .ok()
        .map(|value| value.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "snapshot-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(root.join("reader")).unwrap();
        root
    }

    #[test]
    fn one_capture_gives_every_reader_of_a_generation_the_same_value() {
        let root = root("shared-capture");
        fs::write(root.join("reader").join("count"), "3").unwrap();
        let capture = SettingsCapture::new(&root, ["reader"], [("reader", "count")]);
        // A file edited after the capture reaches nobody built from it.
        fs::write(root.join("reader").join("count"), "9").unwrap();
        let inputs = Arc::new(LoadInputs::new(BTreeMap::new(), root.clone()));
        let load = LoadContext::with_capture(
            DataLoadOrigin::ClientPreload,
            inputs.clone(),
            capture.clone(),
        );
        let looped = load.loop_context_with_capture(LoopSide::Client, capture.clone());
        assert_eq!(capture.setting("reader", "count"), Some("3"));
        assert_eq!(load.setting("reader", "count"), Some("3"));
        assert_eq!(looped.setting("reader", "count"), Some("3"));
        // A new capture is the one that sees the edit.
        let next = SettingsCapture::new(&root, ["reader"], [("reader", "count")]);
        assert_eq!(next.setting("reader", "count"), Some("9"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_and_empty_settings_do_not_report_omissions() {
        let root = root("empty");
        let captured = capture_settings_with_diagnostics(&root, ["reader", "absent"], |reason| {
            panic!("empty settings reported an omission: {reason}");
        });
        assert!(captured["reader"].is_empty());
        assert!(!captured.contains_key("absent"));
        fs::remove_dir_all(&root).unwrap();
        assert!(
            capture_settings_with_diagnostics(&root, ["reader"], |reason| {
                panic!("missing settings root reported an omission: {reason}");
            })
            .is_empty()
        );
        assert!(!root.exists());
    }

    #[test]
    fn failed_enumeration_retains_known_omitted_keys() {
        let names = [
            Ok(OsString::from("known")),
            Err(std::io::ErrorKind::PermissionDenied.into()),
        ];
        let error = sorted_names(names.into_iter()).unwrap_err();
        let SnapshotError::Enumeration { names, error } = error else {
            panic!("expected an enumeration failure");
        };
        assert_eq!(names, ["known"]);
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    #[cfg(unix)]
    fn unsafe_unreadable_and_non_utf8_entries_keep_their_names() {
        use std::os::unix::ffi::OsStringExt;
        let root = root("named-refusals");
        fs::write(root.join("reader/value"), "inside").unwrap();
        fs::create_dir(root.join("reader/directory")).unwrap();
        std::os::unix::fs::symlink("missing", root.join("reader/dangling")).unwrap();
        let mut refused = BTreeMap::new();
        let captured =
            capture_settings_with_report(&root, ["reader", "reader"], |mod_id, key, reason| {
                assert_eq!(mod_id, "reader");
                assert!(
                    refused
                        .insert(key.unwrap().to_owned(), reason.to_string())
                        .is_none()
                );
            });
        assert_eq!(captured["reader"].len(), 1);
        assert_eq!(refused.len(), 2);
        assert!(refused[OsStr::new("directory")].contains("not a regular file"));
        assert!(refused[OsStr::new("dangling")].starts_with("unsafe or unreadable entry:"));
        let invalid_key = OsString::from_vec(vec![0xff]);
        fs::write(root.join("reader").join(&invalid_key), "invalid key").unwrap();
        refused.clear();
        let captured = capture_settings_with_report(&root, ["reader"], |_, key, reason| {
            refused.insert(key.unwrap().to_owned(), reason.to_string());
        });
        assert_eq!(
            captured["reader"].get("value").map(String::as_str),
            Some("inside")
        );
        assert_eq!(captured["reader"].len(), 1);
        assert_eq!(refused.len(), 3);
        assert_eq!(refused[&invalid_key], "non-UTF-8 setting name");
        assert!(refused[OsStr::new("directory")].contains("not a regular file"));
        assert!(refused[OsStr::new("dangling")].starts_with("unsafe or unreadable entry:"));
        std::os::unix::fs::symlink(root.join("reader"), root.join("alias")).unwrap();
        let mut scopes = Vec::new();
        let captured = capture_settings_with_report(&root, ["alias"], |mod_id, key, reason| {
            assert_eq!(mod_id, "alias");
            assert!(key.is_none());
            scopes.push(reason.to_string());
        });
        assert!(captured.is_empty());
        assert_eq!(scopes.len(), 1);
        assert!(scopes[0].starts_with("cannot inspect settings directory:"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn published_loop_readers_share_one_immutable_settings_capture() {
        let root = root("published");
        fs::write(root.join("reader/value"), "before").unwrap();
        fs::write(root.join("reader/invalid"), [0xff]).unwrap();
        let load = LoadContext::new(
            DataLoadOrigin::ServerDataLoad,
            Arc::new(LoadInputs::new(BTreeMap::new(), root.clone())),
            [],
        );
        let mut diagnostics = Vec::new();
        let settings = capture_settings_with_diagnostics(&root, ["reader"], |report| {
            diagnostics.push(report.to_owned());
        });
        let game = LoopContext {
            side: LoopSide::Server,
            snapshot: Snapshot {
                inputs: load.snapshot.inputs.clone(),
                settings: shared_settings(settings),
                registered_settings: Arc::from([]),
            },
            recipe_properties: Default::default(),
            item_tags: None,
        };
        fs::write(root.join("reader/value"), "after").unwrap();
        fs::write(root.join("reader/later-invalid"), [0xff]).unwrap();
        let audio = game.clone();
        let worldgen = game.worldgen_context();
        assert!(Arc::ptr_eq(
            &game.snapshot.settings,
            &audio.snapshot.settings
        ));
        assert!(Arc::ptr_eq(
            &game.snapshot.settings,
            &worldgen.snapshot.settings
        ));
        std::thread::scope(|scope| {
            scope.spawn(|| {
                assert_eq!(worldgen.setting("reader", "value"), Some("before"));
                assert_eq!(worldgen.setting("reader", "invalid"), None);
            });
            scope.spawn(|| {
                assert_eq!(audio.setting("reader", "value"), Some("before"));
                assert_eq!(audio.setting("reader", "later-invalid"), None);
            });
        });
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].contains("\"reader\"/\"invalid\": invalid UTF-8:"));
        capture_settings_with_diagnostics(&root, ["reader"], |report| {
            diagnostics.push(report.to_owned());
        });
        assert_eq!(diagnostics.len(), 2);
        assert!(diagnostics[1].contains("\"reader\"/\"later-invalid\": invalid UTF-8:"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn initial_snapshot_is_shared_scoped_and_renewed_only_by_a_new_load() {
        let root = root("initial-publication");
        fs::create_dir(root.join("inactive")).unwrap();
        fs::write(root.join("reader/mode"), "before").unwrap();
        fs::write(root.join("inactive/mode"), "not admitted").unwrap();
        let settings = shared_settings(capture_settings_with_diagnostics(
            &root,
            ["reader", "inactive"],
            |reason| panic!("valid initial settings reported a refusal: {reason}"),
        ));
        let inputs = Arc::new(LoadInputs::new(BTreeMap::new(), root.clone()));
        let initial = LoadContext::from_settings_snapshot(
            DataLoadOrigin::ClientPreload,
            inputs.clone(),
            ["reader", "inactive"],
            settings.clone(),
        );
        assert!(Arc::ptr_eq(&settings, &initial.snapshot.settings));
        let admitted = LoadContext::from_settings_snapshot(
            DataLoadOrigin::ClientPreload,
            inputs,
            ["reader"],
            settings.clone(),
        );
        assert_eq!(admitted.setting("inactive", "mode"), None);
        assert!(Arc::ptr_eq(
            &settings[1].1,
            &admitted.snapshot.settings[0].1
        ));
        fs::write(root.join("reader/mode"), "after").unwrap();
        fs::write(root.join("reader/late"), "later key").unwrap();
        let loop_context = admitted.captured_loop_context(LoopSide::Client);
        let worldgen = loop_context.worldgen_context();
        assert!(Arc::ptr_eq(
            &admitted.snapshot.settings,
            &loop_context.snapshot.settings
        ));
        assert_eq!(loop_context.setting("reader", "mode"), Some("before"));
        assert_eq!(worldgen.setting("reader", "late"), None);
        let next = admitted.for_load(DataLoadOrigin::ClientPreload, ["reader"]);
        assert_eq!(next.setting("reader", "mode"), Some("after"));
        assert_eq!(next.setting("reader", "late"), Some("later key"));
        assert_eq!(loop_context.setting("reader", "mode"), Some("before"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn reviewer_capture_follows_external_symlink() {
        let root = root("symlinks");
        let data = root.join("data");
        fs::create_dir_all(data.join("reader")).unwrap();
        fs::write(root.join("secret"), "outside-token").unwrap();
        std::os::unix::fs::symlink(root.join("secret"), data.join("reader/token")).unwrap();
        fs::write(data.join("reader/normal"), "inside").unwrap();
        std::os::unix::fs::symlink("normal", data.join("reader/internal")).unwrap();
        std::os::unix::fs::symlink("missing", data.join("reader/dangling")).unwrap();
        std::os::unix::fs::symlink(root.join("reader"), data.join("alias")).unwrap();
        fs::write(root.join("reader/token"), "escaped").unwrap();
        let captured = capture_settings(&data, ["reader", "alias", "../reader"]);
        assert_eq!(captured["reader"].get("token"), None);
        assert_eq!(captured["reader"].get("internal"), None);
        assert_eq!(captured["reader"].get("dangling"), None);
        assert_eq!(captured["reader"]["normal"], "inside");
        assert!(!captured.contains_key("alias"));
        assert!(!captured.contains_key("../reader"));
        std::os::unix::fs::symlink(&data, root.join("linked-root")).unwrap();
        assert!(capture_settings(&root.join("linked-root"), ["reader"]).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    #[cfg(unix)]
    fn fifo_and_directory_do_not_block_or_become_settings() {
        let root = root("fifo");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(root.join("reader/pipe"))
                .status()
                .unwrap()
                .success()
        );
        fs::create_dir(root.join("reader/directory")).unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let input = root.clone();
        let worker =
            std::thread::spawn(move || send.send(capture_settings(&input, ["reader"])).unwrap());
        let captured = receive
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("snapshot blocked on FIFO");
        assert!(captured["reader"].is_empty());
        worker.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn large_files_and_many_keys_are_captured_in_full_and_invalid_utf8_is_skipped() {
        let root = root("uncapped");
        fs::create_dir(root.join("second")).unwrap();
        for i in 0..1030 {
            fs::write(root.join(format!("reader/key{i:04}")), "x").unwrap();
            fs::write(root.join(format!("second/key{:04}", 1029 - i)), "x").unwrap();
        }
        fs::write(root.join("reader/large"), vec![b'b'; 65_537]).unwrap();
        fs::write(root.join("reader/invalid"), [0xff]).unwrap();
        let mut refused = Vec::new();
        let captured =
            capture_settings_with_report(&root, ["second", "reader"], |mod_id, key, reason| {
                refused.push((
                    mod_id.to_owned(),
                    key.unwrap().to_owned(),
                    reason.to_string(),
                ));
            });
        assert_eq!(captured["second"].len(), 1030);
        assert_eq!(captured["reader"].len(), 1031);
        assert_eq!(captured["reader"]["key0000"], "x");
        assert_eq!(captured["reader"]["key1029"], "x");
        assert_eq!(captured["reader"]["large"].len(), 65_537);
        assert_eq!(captured["reader"].get("invalid"), None);
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0].0, "reader");
        assert_eq!(refused[0].1, "invalid");
        assert!(refused[0].2.starts_with("invalid UTF-8:"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    #[cfg(unix)]
    fn validates_open_handle_after_entry_replacement() {
        use std::io::Read;
        let root = root("replaced");
        let path = root.join("reader/value");
        fs::write(&path, "opened value").unwrap();
        let entries = snapshot_files_with_hook(&root, "reader", |_| {
            fs::remove_file(&path).unwrap();
            assert!(
                std::process::Command::new("mkfifo")
                    .arg(&path)
                    .status()
                    .unwrap()
                    .success()
            );
        })
        .unwrap();
        let (_, file) = entries.into_iter().next().unwrap();
        let mut file = file.unwrap();
        let mut value = String::new();
        file.read_to_string(&mut value).unwrap();
        assert_eq!(value, "opened value");

        // Now open the FIFO, then replace its path with a regular file. Path
        // metadata would wrongly accept it, but descriptor metadata must refuse.
        let entries = snapshot_files_with_hook(&root, "reader", |_| {
            fs::remove_file(&path).unwrap();
            fs::write(&path, "replacement").unwrap();
        })
        .unwrap();
        assert!(entries.into_iter().next().unwrap().1.is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn reviewer_symlink_replacement_keeps_opened_value() {
        use std::io::Read;
        let root = root("symlink-replaced");
        let path = root.join("reader/value");
        fs::write(&path, "opened").unwrap();
        fs::write(root.join("target"), "replacement").unwrap();
        let entries = snapshot_files_with_hook(&root, "reader", |_| {
            fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(root.join("target"), &path).unwrap();
        })
        .unwrap();
        let (_, file) = entries.into_iter().next().unwrap();
        let mut file = file.unwrap();
        let mut value = String::new();
        file.read_to_string(&mut value).unwrap();
        assert_eq!(value, "opened");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn settings_are_scoped_and_frozen_until_a_new_load() {
        let root = std::env::temp_dir().join(format!(
            "sunrise-load-settings-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let active = root.join("active");
        fs::create_dir_all(&active).unwrap();
        fs::create_dir_all(root.join("inactive")).unwrap();
        fs::write(active.join("mode"), "  before\n").unwrap();
        fs::write(active.join("empty"), " \n").unwrap();
        fs::write(active.join("invalid"), [0xff]).unwrap();
        fs::write(root.join("inactive/mode"), "not loaded").unwrap();
        let inputs = Arc::new(LoadInputs::new(BTreeMap::new(), root.clone()));
        let before = LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs.clone(), ["active"]);

        // Change the files before the first lookup, not just between reads.
        fs::write(active.join("mode"), "  after\n").unwrap();
        fs::write(active.join("late"), "new key").unwrap();
        assert_eq!(before.setting("active", "mode"), Some("before"));
        assert_eq!(before.setting("active", "empty"), Some(""));
        assert_eq!(before.setting("active", "invalid"), None);
        assert_eq!(before.setting("active", "late"), None);
        assert_eq!(before.setting("inactive", "mode"), None);
        assert_eq!(read_setting(&active, "mode").as_deref(), Some("after"));

        let after = LoadContext::new(DataLoadOrigin::ClientPreload, inputs.clone(), ["active"]);
        assert_eq!(after.setting("active", "mode"), Some("after"));
        assert_eq!(after.setting("active", "late"), Some("new key"));
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(before.setting("active", "mode"), Some("before"));
        assert_eq!(after.setting("active", "mode"), Some("after"));
        let missing = LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs, ["active"]);
        assert_eq!(missing.setting("active", "mode"), None);
        assert!(!root.exists());
    }

    #[test]
    fn a_loop_context_shares_the_registries_and_captures_its_own_settings() {
        let root = std::env::temp_dir().join(format!(
            "sunrise-loop-settings-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let mod_dir = root.join("active");
        fs::create_dir_all(&mod_dir).unwrap();
        fs::write(mod_dir.join("mode"), "one").unwrap();
        let registries = BTreeMap::from([("item".to_owned(), vec!["a:b".to_owned()])]);
        let inputs = Arc::new(LoadInputs::new(registries, root.clone()));
        let load = LoadContext::new(DataLoadOrigin::ClientPreload, inputs, ["active"]);

        let client = load.loop_context(LoopSide::Client, ["active"]);
        fs::write(mod_dir.join("mode"), "two").unwrap();
        let server = load.loop_context(LoopSide::Server, ["active"]);

        assert_eq!(client.side(), LoopSide::Client);
        assert_eq!(server.side(), LoopSide::Server);
        assert_eq!(client.registry("item"), Some(&["a:b".to_owned()][..]));
        assert_eq!(server.registry("item"), client.registry("item"));
        assert_eq!(client.setting("active", "mode"), Some("one"));
        assert_eq!(server.setting("active", "mode"), Some("two"));
        assert_eq!(load.setting("active", "mode"), Some("one"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_worldgen_context_shares_the_registries_and_captures_its_own_settings() {
        let root = std::env::temp_dir().join(format!(
            "sunrise-worldgen-settings-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let mod_dir = root.join("active");
        fs::create_dir_all(&mod_dir).unwrap();
        fs::write(mod_dir.join("ground"), "podzol").unwrap();
        let registries = BTreeMap::from([("block".to_owned(), vec!["a:b".to_owned()])]);
        let inputs = Arc::new(LoadInputs::new(registries, root.clone()));
        let load = LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs, ["active"]);
        fs::write(mod_dir.join("ground"), "mud").unwrap();

        let worldgen = load.worldgen_context(["active"]);
        fs::write(mod_dir.join("ground"), "moss").unwrap();

        assert_eq!(worldgen.registry("block"), Some(&["a:b".to_owned()][..]));
        assert_eq!(worldgen.setting("active", "ground"), Some("mud"));
        assert_eq!(worldgen.setting("inactive", "ground"), None);
        assert_eq!(load.setting("active", "ground"), Some("podzol"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn registered_settings_capture_by_known_keys_shares_the_load_snapshot() {
        let root = root("registered-known-keys");
        for key in ["first", "last"] {
            fs::write(root.join("reader").join(key), "true").unwrap();
        }
        fs::create_dir(root.join("reader/dir_key")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            fs::write(root.join("secret"), "outside").unwrap();
            std::os::unix::fs::symlink(root.join("secret"), root.join("reader/link_key")).unwrap();
            // An unrelated non-UTF-8 sibling does not affect a known-key capture.
            fs::write(
                root.join("reader").join(OsString::from_vec(vec![0xff])),
                "junk",
            )
            .unwrap();
        }

        let registered = [
            ("reader", "first"),
            ("reader", "last"),
            ("reader", "missing_key"),
            ("reader", "dir_key"),
            #[cfg(unix)]
            ("reader", "link_key"),
        ];

        // Direct per-owner capture (page refresh) reads known keys and refuses links/dirs.
        let single = capture_registered_settings(&root, registered.iter().copied());
        assert_eq!(single["reader"].len(), 2);
        assert_eq!(
            single["reader"].get("last").map(String::as_str),
            Some("true")
        );
        assert_eq!(single["reader"].get("missing_key"), None);
        assert_eq!(single["reader"].get("dir_key"), None);
        #[cfg(unix)]
        assert_eq!(single["reader"].get("link_key"), None);

        // Full owning load + loop context capture shares the same known-key admission across reloads.
        let inputs = Arc::new(LoadInputs::new(BTreeMap::new(), root.clone()));
        let load = LoadContext::new_with_registered(
            DataLoadOrigin::ClientPreload,
            inputs.clone(),
            ["reader"],
            registered.iter().copied(),
        );
        let loop_ctx = load.captured_loop_context(LoopSide::Client);
        assert_eq!(loop_ctx.setting("reader", "first"), Some("true"));
        assert_eq!(loop_ctx.setting("reader", "last"), Some("true"));
        assert_eq!(loop_ctx.setting("reader", "missing_key"), None);
        assert_eq!(loop_ctx.setting("reader", "dir_key"), None);
        #[cfg(unix)]
        assert_eq!(loop_ctx.setting("reader", "link_key"), None);

        // Startup snapshot sharing keeps registration values already captured while still
        // picking up registered keys omitted by the generic registration snapshot.
        let registration_snapshot = shared_settings(BTreeMap::from([(
            "reader".to_owned(),
            BTreeMap::from([("first".to_owned(), "false".to_owned())]),
        )]));
        let startup = LoadContext::from_settings_snapshot_with_registered(
            DataLoadOrigin::ClientPreload,
            inputs,
            ["reader"],
            registration_snapshot,
            registered.iter().copied(),
        );
        assert_eq!(startup.setting("reader", "first"), Some("false"));
        assert_eq!(startup.setting("reader", "last"), Some("true"));

        // A new loop context reads edits without mutating the published one.
        fs::write(root.join("reader/last"), "false").unwrap();
        let reloaded = load.loop_context_with_registered(
            LoopSide::Client,
            ["reader"],
            registered.iter().copied(),
        );
        assert_eq!(reloaded.setting("reader", "last"), Some("false"));
        assert_eq!(loop_ctx.setting("reader", "last"), Some("true"));

        fs::remove_dir_all(&root).unwrap();
    }
}
