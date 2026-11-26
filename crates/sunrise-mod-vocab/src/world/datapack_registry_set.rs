//! The entries of the datapack registries mods declared, as one world loads them:
//! a candidate built from the winning pack files and checked
//! as a whole, then published as a set and read directly by `(registry, entry)`. Nothing here
//! touches the world; the game builds a candidate before it commits any table and keeps the old
//! set when building fails.
use super::datapack_registry::{
    DatapackDecodeError, DatapackEntry, DatapackRegistryDeclaration, DatapackValue,
    VanillaValueDecoder, bind_vanilla_values, datapack_registry_directory, decode_datapack_entry,
    schema_has_vanilla_kind, valid_namespaced,
};
use std::collections::{BTreeMap, BTreeSet};

/// Why a local candidate was refused. A load error leaves the whole candidate unpublished;
/// a false load condition omits an entry without an error.
#[derive(Clone, Debug, PartialEq, Eq)]
// no wire: loading runs in the game; mods read published entries, never load failures
#[non_exhaustive]
pub enum DatapackLoadReason {
    /// The winning file could not be read. Carries the underlying I/O error message.
    ReadFile(String),
    /// The winning file is not UTF-8 text.
    NotUtf8,
    /// The entry's id (from its namespace and file name) is not a lowercase `namespace:path`.
    InvalidEntryId,
    /// The file did not decode against the registry's schema.
    Decode(DatapackDecodeError),
    /// A resolved tag has an invalid name or names an entry outside its accepted registry.
    InvalidTag {
        /// The tag's canonical name.
        tag: String,
        /// The invalid member, or empty when the tag name or registry is invalid.
        member: String,
    },
    /// A reference names an entry of a declared registry that no pack defines.
    UnresolvedReference {
        /// The referring field's path (`chain`, `drops[2].next`).
        field: String,
        /// The registry the field names.
        registry: String,
        /// The entry it names (`namespace:path`).
        target: String,
    },
}

/// A refused candidate: the registry, entry, winning pack and file, and the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
// no wire: loading runs in the game; mods read published entries, never load failures
// exhaustive: the game builds it with a literal and reports it
pub struct DatapackLoadError {
    /// The registry's key (`example:spell`).
    pub registry: String,
    /// The entry's id (`addon:fire`), as far as it was known.
    pub entry: String,
    /// The file the entry came from.
    pub file: String,
    /// The id of the pack whose version of `file` won (`file/override`); empty when no pack is
    /// involved, as for resolved tags or a snapshot a server sent.
    pub pack: String,
    /// What was wrong.
    pub reason: DatapackLoadReason,
}

/// Every entry of every declared registry, decoded and checked, not yet published.
/// Local candidates are built independently of network snapshot limits.
#[derive(Clone, Debug, PartialEq)]
// no wire: the game's candidate; a mod reads the published set through a level handle
// exhaustive: built by the local candidate builder or DatapackSnapshot::candidate
pub struct DatapackCandidate {
    pub(super) registries: BTreeMap<String, BTreeMap<String, DatapackEntry>>,
    pub(super) tags: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    /// Exactly the registries in a received snapshot; empty for a local candidate.
    pub(super) synchronized: BTreeSet<String>,
}

/// Inverse of `DatapackRegistryDeclaration::entry_file` for a registry directory.
/// Each matching directory interprets the physical file independently.
pub fn datapack_entry_id(directory: &str, file: &str) -> Option<String> {
    let (namespace, rest) = file.strip_prefix("data/")?.split_once('/')?;
    let path = rest
        .strip_prefix(directory)?
        .strip_prefix('/')?
        .strip_suffix(".json")?;
    (!path.is_empty()).then(|| format!("{namespace}:{path}"))
}

/// Local load stages, not yet eligible for publication or network projection.
#[derive(Default)]
// no wire: local staging retains pack provenance and does not cross a process boundary
// exhaustive: private fields; built through staging methods.
pub struct DatapackCandidateBuilder {
    registries: BTreeMap<String, BTreeMap<String, DatapackEntry>>,
    tags: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    packs: BTreeMap<String, BTreeMap<String, String>>,
}

impl DatapackCandidateBuilder {
    /// Add a registry's accepted entries with their winning pack ids and the tags
    /// already resolved against those entries by the registry's tag loader.
    pub fn stage_registry(
        &mut self,
        registry: &str,
        entries: BTreeMap<String, (DatapackEntry, String)>,
        tags: BTreeMap<String, Vec<String>>,
    ) {
        let (decoded, packs) = entries
            .into_iter()
            .map(|(id, (entry, pack))| ((id.clone(), entry), (id, pack)))
            .unzip();
        self.registries.insert(registry.to_owned(), decoded);
        self.packs.insert(registry.to_owned(), packs);
        if tags.is_empty() {
            self.tags.remove(registry);
        } else {
            self.tags.insert(registry.to_owned(), tags);
        }
    }

    /// Finish the local candidate only once all registries are staged; references may be forward
    /// or cyclic. A refusal identifies the pack supplying the referring entry, not its target.
    pub fn finish(
        mut self,
        declarations: &[DatapackRegistryDeclaration],
    ) -> Result<DatapackCandidate, Box<DatapackLoadError>> {
        for declaration in declarations {
            self.registries.entry(declaration.key.clone()).or_default();
        }
        resolve_references(declarations, &self.registries).map_err(|mut error| {
            if let Some(pack) = self
                .packs
                .get_mut(&error.registry)
                .and_then(|entries| entries.remove(&error.entry))
            {
                error.pack = pack;
            }
            error
        })?;
        Ok(DatapackCandidate {
            registries: self.registries,
            tags: self.tags,
            synchronized: BTreeSet::new(),
        })
    }

    /// Decode already selected and admitted resources `(file, pack, bytes)`.
    fn from_winners<'a>(
        declarations: &[DatapackRegistryDeclaration],
        files: impl IntoIterator<Item = (&'a str, &'a str, &'a [u8])>,
        decoder: &mut dyn VanillaValueDecoder,
    ) -> Result<DatapackCandidate, Box<DatapackLoadError>> {
        let directories: Vec<_> = declarations
            .iter()
            .map(|declaration| datapack_registry_directory(&declaration.key))
            .collect();
        let mut staged = vec![BTreeMap::new(); declarations.len()];
        for (file, pack, bytes) in files {
            let mut text = None;
            for (index, directory) in directories.iter().enumerate() {
                let Some(entry) = directory
                    .as_deref()
                    .and_then(|directory| datapack_entry_id(directory, file))
                else {
                    continue;
                };
                let declaration = &declarations[index];
                let fail = |reason| {
                    Box::new(DatapackLoadError {
                        registry: declaration.key.clone(),
                        entry: entry.clone(),
                        file: file.to_owned(),
                        pack: pack.to_owned(),
                        reason,
                    })
                };
                if !valid_namespaced(&entry) {
                    return Err(fail(DatapackLoadReason::InvalidEntryId));
                }
                let text = match text {
                    Some(text) => text,
                    None => *text.insert(
                        std::str::from_utf8(bytes)
                            .map_err(|_| fail(DatapackLoadReason::NotUtf8))?,
                    ),
                };
                let decoded = decode_datapack_entry(declaration, text, decoder)
                    .map_err(|error| fail(DatapackLoadReason::Decode(error)))?;
                staged[index].insert(entry, (decoded, pack.to_owned()));
            }
        }
        let mut builder = Self::default();
        for (declaration, entries) in declarations.iter().zip(staged) {
            builder.stage_registry(&declaration.key, entries, BTreeMap::new());
        }
        builder.finish(declarations)
    }
}

impl DatapackCandidate {
    /// Decodes the winning file of every entry of every declared registry in `files`, using
    /// [`DatapackRegistryDeclaration::entry_file`]'s directory rule. Each file has versions in
    /// pack order, the last winning as `ResourceManager` hands them out. A physical file can
    /// match more than one registry directory. This constructor treats `neoforge:conditions`
    /// as metadata without testing it, and decodes `neoforge:value` when the wrapper supplies it.
    /// After a false condition, the caller omits the entire path rather than exposing a lower
    /// version. A condition error is not a false result.
    ///
    /// References resolve against the whole candidate, so a forward reference works and an
    /// absent target in a declared registry refuses everything. References to undeclared
    /// registries are not checked. Each version is its pack's id and its bytes, so a refusal
    /// names the pack. Local loading does not impose network byte or collection count limits.
    ///
    /// `decoder` decodes the vanilla value kinds (`IntProvider`, `BlockStateProvider`,
    /// `BlockPredicate`, `HolderSet`); pass `NoVanillaValueDecoder` when no schema uses one.
    pub fn build(
        declarations: &[DatapackRegistryDeclaration],
        files: &[(String, Vec<(String, Vec<u8>)>)],
        decoder: &mut dyn VanillaValueDecoder,
    ) -> Result<Self, Box<DatapackLoadError>> {
        DatapackCandidateBuilder::from_winners(
            declarations,
            files.iter().filter_map(|(file, versions)| {
                let (pack, bytes) = versions.last()?;
                Some((file.as_str(), pack.as_str(), bytes.as_slice()))
            }),
            decoder,
        )
    }

    /// The accepted entry names of a registry, sorted, before publication.
    pub fn entries(&self, registry: &str) -> Result<Vec<&str>, DatapackReadError> {
        Ok(self
            .registries
            .get(registry)
            .ok_or(DatapackReadError::UnknownRegistry)?
            .keys()
            .map(String::as_str)
            .collect())
    }

    /// Binds the engine's fully resolved tags atomically against accepted entries.
    /// An unknown registry, invalid tag name or missing member leaves the candidate unchanged.
    /// This checks membership, not the size of a future encoded snapshot.
    pub fn bind_tags(
        &mut self,
        registry: &str,
        tags: BTreeMap<String, Vec<String>>,
    ) -> Result<(), Box<DatapackLoadError>> {
        let fail = |reason| {
            Box::new(DatapackLoadError {
                registry: registry.to_owned(),
                entry: String::new(),
                file: String::new(),
                pack: String::new(),
                reason,
            })
        };
        let entries = self.registries.get(registry).ok_or_else(|| {
            fail(DatapackLoadReason::InvalidTag {
                tag: String::new(),
                member: String::new(),
            })
        })?;
        for (tag, members) in &tags {
            if !valid_namespaced(tag) {
                return Err(fail(DatapackLoadReason::InvalidTag {
                    tag: tag.clone(),
                    member: String::new(),
                }));
            }
            if let Some(member) = members.iter().find(|id| !entries.contains_key(*id)) {
                return Err(fail(DatapackLoadReason::InvalidTag {
                    tag: tag.clone(),
                    member: member.clone(),
                }));
            }
        }
        self.tags.insert(registry.to_owned(), tags);
        Ok(())
    }
}

/// Why a lookup failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DatapackReadError {
    /// No registry has this key or the requested level or its published set is absent.
    UnknownRegistry,
    /// The registry has no such entry (a reload removed it, or no pack defines it).
    MissingEntry,
    /// The phase does not allow world access, or there is no backend to answer the read.
    Unavailable,
}
impl crate::NoBackend for DatapackReadError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// Checks every reference to a declared registry against the whole `registries`, so a forward
/// reference works and an absent target refuses everything. A reference to a registry no
/// declaration covers (a vanilla one) is not checked.
pub(super) fn resolve_references(
    declarations: &[DatapackRegistryDeclaration],
    registries: &BTreeMap<String, BTreeMap<String, DatapackEntry>>,
) -> Result<(), Box<DatapackLoadError>> {
    for declaration in declarations {
        for (entry, decoded) in &registries[&declaration.key] {
            // Every reference, at any depth: a list's items, a map's keys and values, a record's fields.
            let mut found = Vec::new();
            for (field, (_, value)) in declaration.fields.iter().zip(&decoded.values) {
                super::datapack_registry::references(
                    &field.kind,
                    value,
                    &super::datapack_registry::joined("", &field.name),
                    &mut found,
                );
            }
            for (path, registry, target) in found {
                let Some(targets) = registries.get(registry) else {
                    continue;
                };
                if !targets.contains_key(target) {
                    return Err(Box::new(DatapackLoadError {
                        registry: declaration.key.clone(),
                        entry: entry.clone(),
                        file: declaration.entry_file(entry).unwrap_or_default(),
                        pack: String::new(),
                        reason: DatapackLoadReason::UnresolvedReference {
                            field: path,
                            registry: registry.to_owned(),
                            target: target.to_owned(),
                        },
                    }));
                }
            }
        }
    }
    Ok(())
}

/// The published entries of a world's datapack registries.
#[derive(Clone, Debug, PartialEq)]
// no wire: the game holds it; a mod reads through a level handle
// exhaustive: built only by [`DatapackRegistrySet::from_candidate`] and [`DatapackRegistrySet::received`]
pub struct DatapackRegistrySet {
    pub(super) registries: BTreeMap<String, BTreeMap<String, DatapackEntry>>,
    pub(super) tags: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    /// None on a server; on a client, exactly the registries in its accepted snapshot.
    pub(super) received_registries: Option<BTreeSet<String>>,
}

impl DatapackRegistrySet {
    /// Publishes `candidate` as a world's active registry set.
    pub fn from_candidate(candidate: DatapackCandidate) -> Self {
        Self {
            registries: candidate.registries,
            tags: candidate.tags,
            received_registries: None,
        }
    }

    /// Re-decode every vanilla value in this set against `decoder`, binding fresh per-load
    /// handles into `decoder`'s table. Used on clients after a snapshot arrives once their
    /// registries and tags are installed.
    pub(crate) fn rebind_vanilla_values(
        &self,
        declarations: &[DatapackRegistryDeclaration],
        decoder: &mut dyn VanillaValueDecoder,
    ) -> Result<Self, super::datapack_snapshot::DatapackSnapshotError> {
        if !declarations
            .iter()
            .any(|declaration| schema_has_vanilla_kind(&declaration.fields))
        {
            return Ok(self.clone());
        }
        let mut registries = self.registries.clone();
        for declaration in declarations {
            let Some(entries) = registries.get_mut(&declaration.key) else {
                continue;
            };
            for (id, entry) in entries.iter_mut() {
                for (field, (_, value)) in declaration.fields.iter().zip(entry.values.iter_mut()) {
                    bind_vanilla_values(&field.kind, value, &field.name, decoder).map_err(
                        |_| super::datapack_snapshot::DatapackSnapshotError::InvalidEntry {
                            registry: declaration.key.clone(),
                            entry: id.clone(),
                        },
                    )?;
                }
            }
        }
        Ok(Self {
            registries,
            tags: self.tags.clone(),
            received_registries: self.received_registries.clone(),
        })
    }

    /// Whether generic reads may access this registry. Typed reads retain their existing
    /// empty defaults for omitted declarations; a received empty registry is still available.
    pub fn registry_available(&self, registry: &str) -> bool {
        self.received_registries.as_ref().map_or_else(
            || self.registries.contains_key(registry),
            |received| received.contains(registry),
        )
    }

    /// The entry `entry` of `registry` in this set.
    pub fn get(&self, registry: &str, entry: &str) -> Result<&DatapackEntry, DatapackReadError> {
        self.registries
            .get(registry)
            .ok_or(DatapackReadError::UnknownRegistry)?
            .get(entry)
            .ok_or(DatapackReadError::MissingEntry)
    }

    /// Borrow the entry IDs of `registry` in lexical order. The iterator borrows the set,
    /// not the lookup key. A known empty registry yields an empty iterator.
    pub fn entries<'set>(
        &'set self,
        registry: &str,
    ) -> Result<impl ExactSizeIterator<Item = &'set str> + use<'set>, DatapackReadError> {
        let entries = self
            .registries
            .get(registry)
            .ok_or(DatapackReadError::UnknownRegistry)?;
        Ok(entries.keys().map(String::as_str))
    }

    /// Whether the current set contains this entry.
    /// Unknown registries and missing entries both return false.
    pub fn contains_entry(&self, registry: &str, id: &str) -> bool {
        self.get(registry, id).is_ok()
    }

    /// Current merged-tag membership. None for an unknown registry or entry; absent tags are false.
    pub fn tag_contains(&self, registry: &str, entry: &str, tag: &str) -> Option<bool> {
        self.get(registry, entry).ok()?;
        Some(
            datapack_tag_members(self, registry, tag)
                .iter()
                .any(|member| member == entry),
        )
    }
}

/// Members already bound by the load that produced this registry set.
pub fn datapack_tag_members<'a>(
    set: &'a DatapackRegistrySet,
    registry: &str,
    tag: &str,
) -> &'a [String] {
    set.tags
        .get(registry)
        .and_then(|tags| tags.get(tag))
        .map(Vec::as_slice)
        .unwrap_or_default()
}

impl From<DatapackCandidate> for DatapackRegistrySet {
    fn from(candidate: DatapackCandidate) -> Self {
        Self::from_candidate(candidate)
    }
}

fn datapack_value_heap_bytes(value: &DatapackValue) -> usize {
    match value {
        DatapackValue::Bool(_)
        | DatapackValue::Int(_)
        | DatapackValue::Float(_)
        | DatapackValue::Absent => 0,
        DatapackValue::Text(text) => text.capacity(),
        DatapackValue::List(items) => items.iter().fold(
            items.capacity().saturating_mul(size_of::<DatapackValue>()),
            |bytes, item| bytes.saturating_add(datapack_value_heap_bytes(item)),
        ),
        DatapackValue::Map(pairs) | DatapackValue::Record(pairs) => pairs_heap_bytes(pairs),
        DatapackValue::Variant { id, fields } => {
            id.capacity().saturating_add(pairs_heap_bytes(fields))
        }
        DatapackValue::Vanilla(vanilla) => {
            use super::datapack_registry::DatapackVanillaValue;
            let inner = match vanilla.as_ref() {
                DatapackVanillaValue::IntProvider { json, .. }
                | DatapackVanillaValue::BlockStateProvider { json, .. }
                | DatapackVanillaValue::BlockPredicate { json, .. } => json.capacity(),
                DatapackVanillaValue::HolderSet { registry, json, .. } => {
                    registry.capacity().saturating_add(json.capacity())
                }
            };
            size_of::<DatapackVanillaValue>().saturating_add(inner)
        }
    }
}

fn pairs_heap_bytes(pairs: &Vec<(String, DatapackValue)>) -> usize {
    pairs.iter().fold(
        pairs
            .capacity()
            .saturating_mul(size_of::<(String, DatapackValue)>()),
        |bytes, (key, val)| {
            bytes
                .saturating_add(key.capacity())
                .saturating_add(datapack_value_heap_bytes(val))
        },
    )
}

/// Whether any declared registry schema contains a vanilla value kind (`IntProvider`,
/// `BlockStateProvider`, `BlockPredicate`, or `HolderSet`).
pub fn declarations_use_vanilla_values(declarations: &[DatapackRegistryDeclaration]) -> bool {
    declarations
        .iter()
        .any(|declaration| schema_has_vanilla_kind(&declaration.fields))
}

/// Re-decode every vanilla value in `set` against `decoder`, binding fresh per-load handles.
pub fn rebind_datapack_vanilla_values(
    set: &DatapackRegistrySet,
    declarations: &[DatapackRegistryDeclaration],
    decoder: &mut dyn VanillaValueDecoder,
) -> Result<DatapackRegistrySet, super::datapack_snapshot::DatapackSnapshotError> {
    set.rebind_vanilla_values(declarations, decoder)
}

/// Charge a synchronized set in the ordered configuration handoff. It is retained by an Arc.
pub fn queued_datapack_set_bytes(set: &DatapackRegistrySet) -> usize {
    use sunrise_mod_runtime::byte_channel::btree_node_bytes;
    let arc = size_of::<DatapackRegistrySet>()
        + 2 * size_of::<usize>()
        + 2 * (align_of::<DatapackRegistrySet>().max(align_of::<usize>()) - 1);
    let registries = set.registries.iter().fold(
        btree_node_bytes::<String, BTreeMap<String, DatapackEntry>>(set.registries.len()),
        |bytes, (key, entries)| {
            let bytes = bytes
                .saturating_add(key.capacity())
                .saturating_add(btree_node_bytes::<String, DatapackEntry>(entries.len()));
            entries.iter().fold(bytes, |bytes, (id, entry)| {
                let bytes = bytes.saturating_add(id.capacity()).saturating_add(
                    entry
                        .values
                        .capacity()
                        .saturating_mul(size_of::<(String, DatapackValue)>()),
                );
                entry.values.iter().fold(bytes, |bytes, (field, value)| {
                    bytes
                        .saturating_add(field.capacity())
                        .saturating_add(datapack_value_heap_bytes(value))
                })
            })
        },
    );
    let tags = set.tags.iter().fold(
        btree_node_bytes::<String, BTreeMap<String, Vec<String>>>(set.tags.len()),
        |bytes, (key, tags)| {
            let bytes = bytes
                .saturating_add(key.capacity())
                .saturating_add(btree_node_bytes::<String, Vec<String>>(tags.len()));
            tags.iter().fold(bytes, |bytes, (key, members)| {
                members.iter().fold(
                    bytes
                        .saturating_add(key.capacity())
                        .saturating_add(members.capacity().saturating_mul(size_of::<String>())),
                    |bytes, member| bytes.saturating_add(member.capacity()),
                )
            })
        },
    );
    let received = set.received_registries.as_ref().map_or(0, |keys| {
        keys.iter()
            .fold(btree_node_bytes::<String, ()>(keys.len()), |bytes, key| {
                bytes.saturating_add(key.capacity())
            })
    });
    arc.saturating_add(registries)
        .saturating_add(tags)
        .saturating_add(received)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::datapack_registry::{
        DatapackField, DatapackFieldKind, DatapackRegistrySync, DatapackValue,
        NoVanillaValueDecoder,
    };

    /// `example:spell`: `{power: integer 0..100, next: reference to example:spell = example:fire}`.
    fn spell() -> DatapackRegistryDeclaration {
        DatapackRegistryDeclaration {
            mod_id: "example".into(),
            key: "example:spell".into(),
            fields: vec![
                DatapackField::required("power", DatapackFieldKind::Int { min: 0, max: 100 }),
                DatapackField::with_default(
                    "next",
                    DatapackFieldKind::Reference {
                        registry: "example:spell".into(),
                    },
                    DatapackValue::Text("example:fire".into()),
                ),
            ],
            sync: DatapackRegistrySync::ServerOnly,
        }
    }

    /// A file whose version `n` is in the pack `pack/<n>`.
    fn file(path: &str, versions: &[&str]) -> (String, Vec<(String, Vec<u8>)>) {
        (
            path.to_owned(),
            versions
                .iter()
                .enumerate()
                .map(|(n, v)| (format!("pack/{n}"), v.as_bytes().to_vec()))
                .collect(),
        )
    }

    fn power(set: &DatapackRegistrySet, entry: &str) -> DatapackValue {
        set.get("example:spell", entry)
            .unwrap()
            .get("power")
            .unwrap()
            .clone()
    }

    fn candidate(files: &[(String, Vec<(String, Vec<u8>)>)]) -> DatapackCandidate {
        DatapackCandidate::build(&[spell()], files, &mut NoVanillaValueDecoder).unwrap()
    }

    /// Base fire power 3, a higher pack's 7: one entry, the winner's value.
    #[test]
    fn the_winning_pack_file_is_the_one_entry() {
        let files = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 3}"#, r#"{"power": 7}"#],
        )];
        let set = DatapackRegistrySet::from_candidate(candidate(&files));
        assert_eq!(power(&set, "example:fire"), DatapackValue::Int(7));
        assert_eq!(
            set.entries("example:spell").unwrap().collect::<Vec<_>>(),
            ["example:fire"]
        );
        assert!(set.contains_entry("example:spell", "example:fire"));
        assert!(!set.contains_entry("example:spell", "example:missing"));
        assert!(!set.contains_entry("example:unknown", "example:fire"));
    }

    /// An entry in another mod's namespace sits in that namespace's folder; nested paths work;
    /// files of unrelated registries and other folders are not entries.
    #[test]
    fn entries_come_from_their_namespaced_folder_only() {
        let files = [
            file("data/example/example/spell/fire.json", &[r#"{"power": 3}"#]),
            file("data/addon/example/spell/ice.json", &[r#"{"power": 4}"#]),
            file(
                "data/addon/example/spell/nested/hail.json",
                &[r#"{"power": 5}"#, r#"{"power": 7}"#],
            ),
            file("data/example/example/other/fire.json", &[r#"{"power": 1}"#]),
            file(
                "data/example/example/spellbook/fire.json",
                &[r#"{"power": 1}"#],
            ),
            file("data/example/spell/fire.json", &[r#"{"power": 1}"#]),
            file("data/addon/example/spell/ice.txt", &[r#"{"power": 1}"#]),
            file(
                "data/addon/minecraft/example/spell/nested/old.json",
                &[r#"{"power": 1}"#],
            ),
        ];
        let declarations = [
            spell(),
            DatapackRegistryDeclaration {
                key: "minecraft:example/spell/nested".into(),
                ..spell()
            },
        ];
        let set = DatapackRegistrySet::from_candidate(
            DatapackCandidate::build(&declarations, &files, &mut NoVanillaValueDecoder).unwrap(),
        );
        let registry = String::from("example:spell");
        let ids = set.entries(&registry).unwrap();
        drop(registry); // The iterator borrows only the published set.
        assert_eq!(
            ids.collect::<Vec<_>>(),
            ["addon:ice", "addon:nested/hail", "example:fire"]
        );
        assert_eq!(
            set.entries("minecraft:example/spell/nested")
                .unwrap()
                .collect::<Vec<_>>(),
            ["addon:hail"]
        );
        // Each registry has its own entry ID for the same physical winner.
        assert_eq!(power(&set, "addon:nested/hail"), DatapackValue::Int(7));
        assert_eq!(
            set.get("minecraft:example/spell/nested", "addon:hail")
                .unwrap()
                .get("power"),
            Some(&DatapackValue::Int(7))
        );
        assert!(matches!(
            set.entries("example:nothing"),
            Err(DatapackReadError::UnknownRegistry)
        ));
    }

    /// Publishing a new candidate reflects updated values and removed entries while leaving
    /// the prior immutable set untouched.
    #[test]
    fn a_new_candidate_updates_and_removes_entries() {
        let files = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 7}"#],
        )];
        let one = DatapackRegistrySet::from_candidate(candidate(&files));
        let nine = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 7}"#, r#"{"power": 9}"#],
        )];
        let two = DatapackRegistrySet::from_candidate(candidate(&nine));
        assert_eq!(power(&two, "example:fire"), DatapackValue::Int(9));
        assert_eq!(power(&one, "example:fire"), DatapackValue::Int(7));
        // Remove fire.
        let empty = DatapackRegistrySet::from_candidate(candidate(&[]));
        assert_eq!(
            empty.get("example:spell", "example:fire"),
            Err(DatapackReadError::MissingEntry)
        );
    }

    /// Invalid winning data (power 101, malformed JSON) fails the candidate with the file and
    /// field, and does not fall back to the lower-priority version; the old set is untouched.
    #[test]
    fn invalid_winning_data_refuses_the_candidate_and_keeps_the_old_set() {
        let good = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 7}"#],
        )];
        let old = DatapackRegistrySet::from_candidate(candidate(&good));
        let bad = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 7}"#, r#"{"power": 101}"#],
        )];
        assert_eq!(
            DatapackCandidate::build(&[spell()], &bad, &mut NoVanillaValueDecoder),
            Err(Box::new(DatapackLoadError {
                registry: "example:spell".into(),
                entry: "example:fire".into(),
                file: "data/example/example/spell/fire.json".into(),
                pack: "pack/1".into(),
                reason: DatapackLoadReason::Decode(DatapackDecodeError::OutOfRange {
                    field: "power".into()
                }),
            }))
        );
        let malformed = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 7}"#, "{"],
        )];
        let refused = DatapackCandidate::build(&[spell()], &malformed, &mut NoVanillaValueDecoder)
            .unwrap_err();
        assert!(matches!(
            refused.reason,
            DatapackLoadReason::Decode(DatapackDecodeError::Malformed(_))
        ));
        assert_eq!(power(&old, "example:fire"), DatapackValue::Int(7));
        let next = DatapackRegistrySet::from_candidate(candidate(&good));
        assert_eq!(power(&next, "example:fire"), DatapackValue::Int(7));
    }

    /// A forward reference resolves after the candidate completes; an absent target refuses it
    /// all.
    #[test]
    fn references_resolve_against_the_whole_candidate() {
        let forward = [
            file(
                "data/example/example/spell/fire.json",
                &[r#"{"power": 4, "next": "addon:ice"}"#],
            ),
            file("data/addon/example/spell/ice.json", &[r#"{"power": 2}"#]),
        ];
        // `ice` leaves `next` at its default, `example:fire`, which exists: a cycle, which
        // references by name allow.
        let set = DatapackRegistrySet::from_candidate(candidate(&forward));
        assert_eq!(power(&set, "example:fire"), DatapackValue::Int(4));
        let missing = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 4}"#, r#"{"power": 7, "next": "addon:ice"}"#],
        )];
        assert_eq!(
            DatapackCandidate::build(&[spell()], &missing, &mut NoVanillaValueDecoder),
            Err(Box::new(DatapackLoadError {
                registry: "example:spell".into(),
                entry: "example:fire".into(),
                file: "data/example/example/spell/fire.json".into(),
                pack: "pack/1".into(),
                reason: DatapackLoadReason::UnresolvedReference {
                    field: "next".into(),
                    registry: "example:spell".into(),
                    target: "addon:ice".into(),
                },
            }))
        );
    }

    /// A reference to a registry no declaration covers (a vanilla one) is not checked here.
    #[test]
    fn a_reference_to_an_undeclared_registry_is_left_to_its_owner() {
        let mut declaration = spell();
        declaration.fields[1].kind = DatapackFieldKind::Reference {
            registry: "minecraft:block".into(),
        };
        declaration.fields[1].default = Some(DatapackValue::Text("minecraft:stone".into()));
        let files = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 1, "next": "minecraft:nothing"}"#],
        )];
        assert!(
            DatapackCandidate::build(&[declaration], &files, &mut NoVanillaValueDecoder).is_ok()
        );
    }

    /// An entry whose id is not a lowercase `namespace:path`, or whose file is not UTF-8, refuses
    /// the candidate.
    #[test]
    fn an_unusable_entry_file_refuses_the_candidate() {
        let reason = |files: &[(String, Vec<(String, Vec<u8>)>)]| {
            DatapackCandidate::build(&[spell()], files, &mut NoVanillaValueDecoder)
                .unwrap_err()
                .reason
        };
        assert_eq!(
            reason(&[file(
                "data/example/example/spell/Fire.json",
                &[r#"{"power": 1}"#]
            )]),
            DatapackLoadReason::InvalidEntryId
        );
        assert_eq!(
            reason(&[(
                "data/example/example/spell/fire.json".to_owned(),
                vec![("pack/0".to_owned(), vec![0xff, 0xfe])]
            )]),
            DatapackLoadReason::NotUtf8
        );
    }

    /// With nothing declared, no file is read and the set is empty.
    #[test]
    fn nothing_declared_decodes_nothing() {
        let files = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 101}"#],
        )];
        let candidate = DatapackCandidate::build(&[], &files, &mut NoVanillaValueDecoder).unwrap();
        let set = DatapackRegistrySet::from_candidate(candidate);
        assert!(matches!(
            set.entries("example:spell"),
            Err(DatapackReadError::UnknownRegistry)
        ));
    }

    #[test]
    fn local_candidates_load_without_snapshot_limits() {
        let mut declarations = vec![spell()];
        declarations.extend((0..256).map(|n| DatapackRegistryDeclaration {
            key: format!("example:empty_{n}"),
            fields: Vec::new(),
            ..spell()
        }));
        let text = format!(r#"{{"power": 7{}}}"#, " ".repeat(1 << 20));
        let files = [file("data/example/example/spell/fire.json", &[&text])];
        let candidate =
            DatapackCandidate::build(&declarations, &files, &mut NoVanillaValueDecoder).unwrap();
        let set = DatapackRegistrySet::from_candidate(candidate);
        assert_eq!(power(&set, "example:fire"), DatapackValue::Int(7));
        assert!(set.registry_available("example:empty_255"));
        assert!(set.entries("example:empty_255").unwrap().next().is_none());
    }

    #[test]
    fn a_nested_value_fails_the_flat_entry_schema() {
        let deep = format!(r#"{{"power": {}7{}}}"#, "[".repeat(64), "]".repeat(64));
        let files = [file("data/example/example/spell/fire.json", &[&deep])];
        let error =
            DatapackCandidate::build(&[spell()], &files, &mut NoVanillaValueDecoder).unwrap_err();
        assert!(matches!(
            error.reason,
            DatapackLoadReason::Decode(DatapackDecodeError::WrongType { field, .. })
                if field == "power"
        ));
    }

    /// `example:book`: a list of chapter records, each with an optional `next` reference to a
    /// spell, and a map of spells by reference key.
    fn book() -> DatapackRegistryDeclaration {
        let spell_ref = || DatapackFieldKind::Reference {
            registry: "example:spell".into(),
        };
        DatapackRegistryDeclaration {
            mod_id: "example".into(),
            key: "example:book".into(),
            fields: vec![
                DatapackField::required(
                    "chapters",
                    DatapackFieldKind::List {
                        of: Box::new(DatapackFieldKind::Record {
                            fields: vec![
                                DatapackField::required(
                                    "pages",
                                    DatapackFieldKind::Int { min: 1, max: 99 },
                                ),
                                DatapackField::optional("spell", spell_ref()),
                            ],
                        }),
                        min_len: 0,
                        max_len: None,
                    },
                ),
                DatapackField::with_default(
                    "costs",
                    DatapackFieldKind::Map {
                        key: crate::world::datapack_registry::DatapackKeyKind::Reference {
                            registry: "example:spell".into(),
                        },
                        value: Box::new(DatapackFieldKind::Int { min: 0, max: 9 }),
                    },
                    DatapackValue::Map(Vec::new()),
                ),
            ],
            sync: DatapackRegistrySync::Required,
        }
    }

    fn chapter_pages(set: &DatapackRegistrySet, entry: &str, chapter: usize) -> Option<i64> {
        set.get("example:book", entry)
            .unwrap()
            .get("chapters")?
            .as_list()?[chapter]
            .get("pages")?
            .as_int()
    }

    /// A nested reference, in a list's record or a map's key, resolves against the whole
    /// candidate; one to a missing entry refuses the candidate and names its path.
    #[test]
    fn nested_references_resolve_and_a_missing_one_names_its_path() {
        let spells = file("data/example/example/spell/fire.json", &[r#"{"power": 1}"#]);
        let good = [
            spells.clone(),
            file(
                "data/example/example/book/tome.json",
                &[
                    r#"{"chapters": [{"pages": 3}, {"pages": 5, "spell": "example:fire"}],
                      "costs": {"example:fire": 2}}"#,
                ],
            ),
        ];
        let set = DatapackRegistrySet::from_candidate(
            DatapackCandidate::build(&[spell(), book()], &good, &mut NoVanillaValueDecoder)
                .unwrap(),
        );
        assert_eq!(chapter_pages(&set, "example:tome", 1), Some(5));
        let refused = |text: &str| {
            let files = [
                spells.clone(),
                file("data/example/example/book/tome.json", &[text]),
            ];
            match *DatapackCandidate::build(&[spell(), book()], &files, &mut NoVanillaValueDecoder)
                .unwrap_err()
            {
                DatapackLoadError {
                    reason: DatapackLoadReason::UnresolvedReference { field, target, .. },
                    ..
                } => (field, target),
                other => panic!("{other:?}"),
            }
        };
        assert_eq!(
            refused(r#"{"chapters": [{"pages": 3}, {"pages": 5, "spell": "example:ice"}]}"#),
            ("chapters[1].spell".to_owned(), "example:ice".to_owned())
        );
        assert_eq!(
            refused(r#"{"chapters": [], "costs": {"example:ice": 2}}"#),
            ("costs.example:ice".to_owned(), "example:ice".to_owned())
        );
    }

    /// A higher pack's file replaces a nested entry whole on reload.
    #[test]
    fn a_reload_replaces_a_nested_entry() {
        let one = [file(
            "data/example/example/book/tome.json",
            &[r#"{"chapters": [{"pages": 3}]}"#],
        )];
        let first = DatapackRegistrySet::from_candidate(
            DatapackCandidate::build(&[book()], &one, &mut NoVanillaValueDecoder).unwrap(),
        );
        assert_eq!(chapter_pages(&first, "example:tome", 0), Some(3));
        let two = [file(
            "data/example/example/book/tome.json",
            &[
                r#"{"chapters": [{"pages": 3}]}"#,
                r#"{"chapters": [{"pages": 8}, {"pages": 9}]}"#,
            ],
        )];
        // A reload publishes a set built from the new candidate.
        let second = DatapackRegistrySet::from_candidate(
            DatapackCandidate::build(&[book()], &two, &mut NoVanillaValueDecoder).unwrap(),
        );
        assert_eq!(chapter_pages(&second, "example:tome", 0), Some(8));
        assert_eq!(chapter_pages(&second, "example:tome", 1), Some(9));
    }
}

#[cfg(feature = "testing")]
#[path = "datapack_registry_fixture_support.rs"]
mod fixture_support;
