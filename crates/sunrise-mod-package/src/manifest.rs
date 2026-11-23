//! `sunrise-mod.toml`, a mod's manifest, and the `.smod` file around it.
//!
//! Native mixin schema adapted from horizon-mixins (Copyright (c) 2026 levzzz),
//! MIT licensed; see `HORIZON_MIXINS_LICENSE` and `THIRD_PARTY_NOTICES.md`.
mod hash;
mod jar;
pub mod loops;
pub mod mixin;
mod runtime_files;
pub use hash::sha512_of;
pub use jar::{
    JarConflict, JarInputsError, JarRef, JarRequirement, MissingJar, collect_jar_requirements,
    missing_jars,
};
pub use runtime_files::{RUNTIME_DIR, RuntimeData, RuntimeTable};

use mod_surface::uses::{ManifestFacts, USES_FILE};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, Visitor, value::MapAccessDeserializer},
};
use std::{
    collections::BTreeMap,
    fmt,
    io::Read,
    marker::PhantomData,
    path::{Path, PathBuf},
};

/// The manifest's file name in a `.smod` or a mod folder.
pub const MANIFEST: &str = "sunrise-mod.toml";
/// The largest uses table ([`USES_FILE`]) a package may carry: what the surface crate reads.
pub const MAX_USES: u64 = mod_surface::uses::MAX_USES_BYTES as u64;
/// The content manifest API this build loads.
pub const API: &str = "0.2";

fn api() -> String {
    API.to_owned()
}
/// Namespaces no mod may claim: vanilla's, the convention tags' (mods add
/// to those, docs/modding/src/how-to/content-mods.md) and the game's own.
const RESERVED_NAMESPACES: [&str; 3] = ["minecraft", "c", "sunrise"];
/// Ids no mod or pack may take: the mod site's own pages under `/mods/` (a project's page is
/// `/mods/<id>`), and the names of the pages below an id. The site's routes are checked against
/// this list on its side.
pub const RESERVED_IDS: [&str; 7] = [
    "account",
    "edit",
    "index",
    "mine",
    "policy",
    "registry-page",
    "submit",
];

/// A mod's saved settings, shared by native mods and modpack overrides.
pub const STORAGE_DIR: &str = ".sunrise-mod-data";

/// Maximum bytes in the complete manifest, including properties and dependency explanations.
pub const MAX_MANIFEST: u64 = 32 << 20;

/// The most source files (`Cargo.toml`, `Cargo.lock`, `build.rs`, `src/**`, the root notices) a
/// package selects: as many as its content files, so a port laid out one file per class fits.
pub const MAX_SOURCE_FILES: usize = crate::content::MAX_FILES;
/// The most bytes those source files take, unpacked.
pub const MAX_SOURCE_BYTES: u64 = 64 << 20;

/// A mod's version as semver, padding an omitted minor and patch (`1` is `1.0.0`, `1.2` is
/// `1.2.0`). Pre-release and build metadata are allowed.
pub fn parse_version(version: &str) -> Result<semver::Version, String> {
    // Only the `major.minor.patch` core is padded: the dots of a pre-release (`1-rc.1`) or build part
    // after a `-` or `+` belong to it.
    let (core, suffix) = version.split_at(version.find(['-', '+']).unwrap_or(version.len()));
    let padded = match core.matches('.').count() {
        0 => format!("{core}.0.0{suffix}"),
        1 => format!("{core}.0{suffix}"),
        _ => version.to_owned(),
    };
    semver::Version::parse(&padded).map_err(|_| {
        format!(
            "version {version:?} is not a semver version (MAJOR.MINOR.PATCH, e.g. 3.6.30+refabricated)"
        )
    })
}

/// The first build-metadata identifier of the placeholder a port gives a jar version it could not
/// read as semver (`0.0.0+unresolved.<the jar's text>`).
pub const UNRESOLVED_BUILD: &str = "unresolved";

/// Whether `version` is such a placeholder. Dependency resolution never lets one satisfy a
/// requirement, since semver matching ignores build metadata and `*` or `=0.0.0` would accept it;
/// it stays refused until someone sets the real version by hand. Any other `0.0.0` is a real version.
pub fn is_unresolved(version: &semver::Version) -> bool {
    version.major == 0
        && version.minor == 0
        && version.patch == 0
        && version.pre.is_empty()
        && version.build.split('.').next() == Some(UNRESOLVED_BUILD)
}

fn version_from_str<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<semver::Version, D::Error> {
    let version = String::deserialize(deserializer)?;
    parse_version(&version).map_err(serde::de::Error::custom)
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// `[a-z0-9_]`, 1 to 64 characters: the mod's folder, log and settings key.
    pub id: String,
    pub name: String,
    /// Semver, read once here: omitted minor and patch are padded ([`parse_version`]).
    #[serde(deserialize_with = "version_from_str")]
    pub version: semver::Version,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub authors: Vec<String>,
    /// The Mods screen's `License:` line (a name such as `MIT`); empty shows it as not given.
    #[serde(default)]
    pub license: String,
    /// Where the license text is: the screen makes its name a link.
    #[serde(default)]
    pub license_url: Option<String>,
    /// The Mods screen's `Credits:` line.
    #[serde(default)]
    pub credits: String,
    /// The mod's home page, the Mods screen's Homepage button.
    #[serde(default)]
    pub homepage: Option<String>,
    /// Where to report problems, the Mods screen's Issues button.
    #[serde(default)]
    pub issues: Option<String>,
    /// The content API line; the tool writes it into the packed package. Absent, it is [`API`].
    #[serde(default = "api")]
    pub api: String,
    #[serde(default)]
    pub side: Side,
    /// What the mod adds, as the Mods screen and the registry list it; the tool writes it into the
    /// packed package. Absent, it is [`Capability::derived`].
    #[serde(default = "Capability::derived")]
    pub capabilities: Vec<Capability>,
    /// Namespaces besides `id` that the mod's `assets/` and `data/` use, for
    /// a content mod whose files sit under other names than its id (Towns
    /// and Towers ships `kaisyn` and `towns_and_towers`). No two content mods
    /// that are on may share one (`content::clash`).
    #[serde(default)]
    pub namespaces: Vec<String>,
    /// Native runtime mod that serves code-side parts and registrations for
    /// this content package's namespaces; refused on `[native]` packages.
    #[serde(default)]
    pub served_by: Option<String>,
    /// Other native mods whose hooks must be available before this one.
    #[serde(default, deserialize_with = "policy_map")]
    pub dependencies: BTreeMap<String, Dependency>,
    /// Mods this one breaks with: the game refuses to start while one of them is loaded at a
    /// version the rule matches (NeoForge's `DependencyType.INCOMPATIBLE`).
    #[serde(default, deserialize_with = "policy_map")]
    pub incompatible: BTreeMap<String, Rule>,
    /// Mods this one works badly with: they load, with a warning (`DependencyType.DISCOURAGED`).
    #[serde(default, deserialize_with = "policy_map")]
    pub discouraged: BTreeMap<String, Rule>,
    /// Mods whose hooks run after this one's when they are loaded, without depending on them
    /// (`Ordering.BEFORE`).
    #[serde(default, deserialize_with = "load_before_ids")]
    pub load_before: Vec<String>,
    /// Original Java jars, layered in list order below this mod's own files.
    #[serde(default)]
    pub jar: Vec<JarRef>,
    /// Exact build pins and support modules for a code mod.
    pub native: Option<Native>,
    /// The data files of the package's `runtime/` folder, each with its SHA-512. Not pack data.
    #[serde(default)]
    pub runtime: Option<RuntimeTable>,
    /// Game internals used by this mod, fingerprinted against its pinned build.
    #[serde(default)]
    pub internals: Vec<Internal>,
    /// Free-form scalar values other mods read through `PackageInfo::property`, with no limit
    /// separate from the complete manifest's byte limit.
    #[serde(default)]
    pub properties: std::collections::BTreeMap<String, Property>,
}

/// One `[properties]` value: a string, integer, float or boolean.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(
    untagged,
    expecting = "expected a string, integer, float or boolean for [properties]"
)]
pub enum Property {
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(String),
}

/// A `[dependencies]` entry: a semver requirement or a detailed table.
#[derive(Clone, Debug, PartialEq)]
pub enum Dependency {
    Version(semver::VersionReq),
    Detailed(DependencyDetails),
}

impl<'de> Deserialize<'de> for Dependency {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        version_or_details(deserializer, Self::Version, Self::Detailed)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DependencyDetails {
    pub version: semver::VersionReq,
    #[serde(default)]
    pub optional: bool,
    /// The installation on which this dependency declaration applies.
    #[serde(default = "both_sides")]
    pub side: Side,
    /// A mod-provided explanation, bound only by the complete manifest's byte limit.
    #[serde(default)]
    pub reason: Option<String>,
    /// An HTTPS link with more information.
    #[serde(default, deserialize_with = "DependencyDetails::deserialize_url")]
    pub url: Option<String>,
    /// Dispatch ordering only; `none` does not remove a code link requirement.
    #[serde(default)]
    pub order: mod_vocab::DependencyOrder,
}

impl DependencyDetails {
    fn deserialize_url<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<String>, D::Error> {
        deserialize_https_url(deserializer)
    }
}

impl Dependency {
    /// The dependency's typed declaration, with bare-version defaults filled in.
    pub fn details(&self) -> DependencyDetails {
        match self {
            Self::Version(version) => DependencyDetails {
                version: version.clone(),
                optional: false,
                side: Side::Both,
                reason: None,
                url: None,
                order: mod_vocab::DependencyOrder::After,
            },
            Self::Detailed(details) => details.clone(),
        }
    }
}

/// An `[incompatible]` or `[discouraged]` entry: the versions of the other mod it matches
/// (`"<2"`), or a table that also gives the player a reason and a link.
#[derive(Clone, Debug, PartialEq)]
pub enum Rule {
    Version(semver::VersionReq),
    Detailed(RuleDetails),
}

impl<'de> Deserialize<'de> for Rule {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        version_or_details(deserializer, Self::Version, Self::Detailed)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RuleDetails {
    /// Every version when left out.
    #[serde(default = "any_version")]
    pub version: semver::VersionReq,
    /// Why, shown when the rule fails: bound only by the complete manifest's byte limit.
    #[serde(default)]
    pub reason: Option<String>,
    /// Where to read more, shown with the reason: an `https://` link.
    #[serde(default, deserialize_with = "RuleDetails::deserialize_url")]
    pub url: Option<String>,
    /// Where the rule holds: both sides, or only the client or the dedicated server.
    #[serde(default = "both_sides")]
    pub side: Side,
}

impl RuleDetails {
    fn deserialize_url<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<String>, D::Error> {
        deserialize_https_url(deserializer)
    }
}

fn deserialize_https_url<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    struct OptionalUrl;

    impl<'de> Visitor<'de> for OptionalUrl {
        type Value = Option<String>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an https:// URL string")
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D: Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> Result<Self::Value, D::Error> {
            let url = String::deserialize(deserializer)?;
            if !url.starts_with("https://") {
                return Err(de::Error::custom("url is not https://"));
            }
            Ok(Some(url))
        }
    }

    deserializer.deserialize_option(OptionalUrl)
}

// Choose the form by its TOML type so mistakes in a detailed table retain their own errors.
fn version_or_details<'de, D, T, V>(
    deserializer: D,
    version: fn(semver::VersionReq) -> V,
    details: fn(T) -> V,
) -> Result<V, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct VersionOrDetails<T, V> {
        version: fn(semver::VersionReq) -> V,
        details: fn(T) -> V,
    }

    impl<'de, T: Deserialize<'de>, V> Visitor<'de> for VersionOrDetails<T, V> {
        type Value = V;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a semver requirement string or a dependency policy table")
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<V, E> {
            value.parse().map(self.version).map_err(E::custom)
        }

        fn visit_map<M: de::MapAccess<'de>>(self, map: M) -> Result<V, M::Error> {
            T::deserialize(MapAccessDeserializer::new(map)).map(self.details)
        }
    }

    deserializer.deserialize_any(VersionOrDetails { version, details })
}

/// Validate an ID while Serde still has the key or array item's span.
struct DependencyId;

impl<'de> de::DeserializeSeed<'de> for DependencyId {
    type Value = String;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<String, D::Error> {
        deserializer.deserialize_str(self)
    }
}

impl Visitor<'_> for DependencyId {
    type Value = String;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a dependency mod ID")
    }

    fn visit_str<E: de::Error>(self, id: &str) -> Result<String, E> {
        if !valid_mod_id(id) {
            return Err(E::custom(format!("invalid dependency id {id:?}")));
        }
        Ok(id.to_owned())
    }
}

fn policy_map<'de, D, T>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct PolicyMap<T>(PhantomData<T>);

    impl<'de, T: Deserialize<'de>> Visitor<'de> for PolicyMap<T> {
        type Value = BTreeMap<String, T>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a table keyed by dependency mod IDs")
        }

        fn visit_map<M: de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
            let mut entries = BTreeMap::new();
            while let Some(id) = map.next_key_seed(DependencyId)? {
                entries.insert(id, map.next_value()?);
            }
            Ok(entries)
        }
    }

    deserializer.deserialize_map(PolicyMap(PhantomData))
}

fn load_before_ids<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    struct LoadBeforeIds;

    impl<'de> Visitor<'de> for LoadBeforeIds {
        type Value = Vec<String>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a list of dependency mod IDs")
        }

        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut ids = Vec::new();
            while let Some(id) = seq.next_element_seed(DependencyId)? {
                ids.push(id);
            }
            Ok(ids)
        }
    }

    deserializer.deserialize_seq(LoadBeforeIds)
}

fn both_sides() -> Side {
    Side::Both
}

fn any_version() -> semver::VersionReq {
    semver::VersionReq::STAR
}

impl Rule {
    /// The versions, reason, link and side, the bare form's as defaults.
    pub fn details(&self) -> RuleDetails {
        match self {
            Self::Version(version) => RuleDetails {
                version: version.clone(),
                reason: None,
                url: None,
                side: Side::Both,
            },
            Self::Detailed(details) => details.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Native {
    pub game: String,
    pub sunrise: String,
    /// The horizon-mixins revision, when the mod pins one itself: absent, it is the one the pinned
    /// SDK names (see [`Native::mixins_or`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mixins: Option<String>,
    /// Whether this native mod opts in to serving content packages that name it in `served_by`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub serves_content: bool,
    /// Source-level edits of game functions, native only: the one form that compiles game code.
    #[serde(default)]
    pub mixin: Vec<mixin::Mixin>,
    #[serde(default)]
    pub module: Vec<Module>,
    /// Accessor and invoker bridges, forwarded as horizon-mixins' `[[bridge]]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bridge: Vec<mixin::Forwarded>,
    /// State fields added to game types, forwarded as horizon-mixins' `[[state]]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub state: Vec<mixin::Forwarded>,
    /// Slots of published game loops, native only (`mod_api::loops`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub r#loop: Vec<loops::Loop>,
}

impl Native {
    /// The horizon-mixins revision this mod builds with: its own `mixins` pin, else `sdk`, the
    /// revision the pinned SDK names (`sdk.json`'s `mixins`).
    pub fn mixins_or<'a>(&'a self, sdk: &'a str) -> &'a str {
        self.mixins.as_deref().unwrap_or(sdk)
    }
}

/// One declared game internal, as listed in the pinned game's `internals.json`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Internal {
    pub path: String,
    /// The catalog's member kind, required when several kinds share the path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Optional pinning: the catalog fingerprint `internals add` writes. Absent, the build takes
    /// it from the pinned game's catalog, which `[native] sunrise` fixes exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

/// A support module merged into a game crate, forwarded as horizon-mixins'
/// `[[module]]`: self-contained Rust items, emitted into `file` as
/// `mod name`, for helpers that need more than one function.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Module {
    pub id: String,
    /// The game source file that receives the module, as the pin indexes it.
    pub file: String,
    /// The module's Rust name under the destination file's top level.
    pub name: String,
    /// The payload's file in this package, under `src/`.
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
}

pub use mod_vocab::Side;

/// What a content mod adds; the player sees it before turning the mod on.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    /// New blocks, items and mob effects from the mod's `data/` (API 0.2).
    Registries,
}
impl Capability {
    /// What a package's capabilities are when its manifest does not say: the one label every
    /// package has carried (the Mods screen and the registry list read it), so every reader sees
    /// the same value as before.
    pub fn derived() -> Vec<Capability> {
        vec![Capability::Registries]
    }
    /// What the player reads before turning the mod on: "May <this>".
    pub fn describe(self) -> &'static str {
        match self {
            Capability::Registries => {
                "add blocks, items, mob effects and entity types to your worlds"
            }
        }
    }
}
impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Capability::Registries => f.write_str("registries"),
        }
    }
}

/// Whether a source pin is an exact lowercase Git object id.
pub fn revision(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) fn valid_mod_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && !reserved_mod_id(id)
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn reserved_mod_id(id: &str) -> bool {
    matches!(id, "con" | "prn" | "aux" | "nul")
        || (id.len() == 4
            && (id.starts_with("com") || id.starts_with("lpt"))
            && matches!(id.as_bytes()[3], b'1'..=b'9'))
}

fn rust_ident(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !name.as_bytes()[0].is_ascii_digit()
}

impl Internal {
    fn check_all(internals: &[Self]) -> Result<(), String> {
        let mut paths = std::collections::HashSet::new();
        for internal in internals {
            if internal.path.is_empty()
                || internal.hash.as_ref().is_some_and(String::is_empty)
                || internal.kind.as_ref().is_some_and(|kind| kind.is_empty())
                || !paths.insert((&internal.path, &internal.kind))
            {
                return Err(format!(
                    "{MANIFEST}: empty internal path, kind or hash, or duplicate internal path and kind"
                ));
            }
        }
        Ok(())
    }
}

/// A manifest's TOML error in the tool's words: no caret box, the missing field with a line to add,
/// an unknown field with the nearest valid key when one is a few edits away.
fn toml_error(text: &str, e: &toml::de::Error) -> String {
    let message = e.message();
    let at = e.span().map_or_else(String::new, |span| {
        let before = &text[..text.floor_char_boundary(span.start.min(text.len()))];
        let line = before.bytes().filter(|&byte| byte == b'\n').count() + 1;
        let column = before
            .rsplit('\n')
            .next()
            .unwrap_or_default()
            .chars()
            .count()
            + 1;
        format!(" (line {line}, column {column})")
    });
    let quoted = |after: &str| -> Option<String> {
        let rest = message.strip_prefix(after)?;
        Some(rest.split('`').next()?.to_owned())
    };
    if let Some(field) = quoted("missing field `") {
        let root_table = e.span().is_none_or(|span| span.is_empty());
        let at = if root_table { "" } else { at.as_str() };
        let example = match (root_table, field.as_str()) {
            (true, "id") => "id = \"my_mod\"".to_owned(),
            (true, "name") => "name = \"My Mod\"".to_owned(),
            (true, "version") => "version = \"0.1.0\"".to_owned(),
            (_, "game") => "game = \"26.3\"".to_owned(),
            (_, "sunrise") => "sunrise = \"<the 40-character game build commit>\"".to_owned(),
            (_, other) => format!("{other} = …"),
        };
        return format!("{MANIFEST}: the field {field} is missing{at}; add {example}");
    }
    if let Some(field) = quoted("unknown field `") {
        let nearest = message
            .split("expected ")
            .nth(1)
            .into_iter()
            .flat_map(|list| list.split('`').skip(1).step_by(2))
            .map(|key| (edit_distance(&field, key), key))
            .min()
            .filter(|(distance, key)| *distance <= (key.len() / 3).clamp(1, 2));
        let hint = nearest.map_or(String::new(), |(_, key)| format!("; did you mean `{key}`?"));
        return format!("{MANIFEST}: {message}{at}{hint}");
    }
    format!("{MANIFEST}: {message}{at}")
}

/// The number of single-character edits between two keys.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (diagonal + usize::from(x != *y))
                .min(above + 1)
                .min(row[j] + 1);
            diagonal = above;
        }
    }
    row[b.len()]
}

impl From<&Manifest> for ManifestFacts {
    fn from(manifest: &Manifest) -> Self {
        Self {
            names: manifest
                .capabilities
                .iter()
                .map(ToString::to_string)
                .collect(),
        }
    }
}

impl Manifest {
    /// Read a manifest from an opened file or package entry, under the published byte limit.
    pub fn read(reader: impl Read) -> Result<Self, String> {
        let mut bytes = Vec::new();
        reader
            .take(MAX_MANIFEST + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("{MANIFEST}: {e}"))?;
        Self::check_size(bytes.len())?;
        let text = std::str::from_utf8(&bytes).map_err(|e| format!("{MANIFEST}: {e}"))?;
        Self::parse_toml(text)
    }

    /// Parse and validate a complete `sunrise-mod.toml`, under [`MAX_MANIFEST`].
    pub fn parse(text: &str) -> Result<Self, String> {
        Self::check_size(text.len())?;
        Self::parse_toml(text)
    }

    fn check_size(bytes: usize) -> Result<(), String> {
        if bytes as u64 > MAX_MANIFEST {
            return Err(format!("{MANIFEST}: over {} MiB", MAX_MANIFEST >> 20));
        }
        Ok(())
    }

    fn parse_toml(text: &str) -> Result<Self, String> {
        let m: Manifest = toml::from_str(text).map_err(|e| toml_error(text, &e))?;
        Internal::check_all(&m.internals)?;
        if !m.internals.is_empty() && m.native.is_none() {
            return Err(format!("{MANIFEST}: internals need a [native] mod"));
        }
        if RESERVED_NAMESPACES.contains(&m.id.as_str()) {
            return Err(format!(
                "{MANIFEST}: id {:?} is a reserved namespace, not a mod's to claim",
                m.id
            ));
        }
        if m.runtime.is_some() && m.native.is_none() {
            return Err(format!(
                "{MANIFEST}: a content mod has no code, so no [runtime]"
            ));
        }
        if RESERVED_IDS.contains(&m.id.as_str()) {
            return Err(format!(
                "{MANIFEST}: id {:?} is a page of the mod site, so no mod can be published under it: pick another id",
                m.id
            ));
        }
        if reserved_mod_id(&m.id) {
            return Err(format!(
                "{MANIFEST}: id {:?} is a reserved Windows device name",
                m.id
            ));
        }
        if !valid_mod_id(&m.id) {
            return Err(format!(
                "{MANIFEST}: id {:?} is not [a-z0-9_]{{1,64}}",
                m.id
            ));
        }
        if let Some(host) = &m.served_by {
            if m.native.is_some() {
                return Err(format!(
                    "{MANIFEST}: a [native] mod cannot declare served_by"
                ));
            }
            if RESERVED_NAMESPACES.contains(&host.as_str()) {
                return Err(format!(
                    "{MANIFEST}: served_by {host:?} is a reserved namespace, not a mod id"
                ));
            }
            if RESERVED_IDS.contains(&host.as_str()) {
                return Err(format!(
                    "{MANIFEST}: served_by {host:?} is a page of the mod site, not a mod id"
                ));
            }
            if reserved_mod_id(host) {
                return Err(format!(
                    "{MANIFEST}: served_by {host:?} is a reserved Windows device name"
                ));
            }
            if !valid_mod_id(host) {
                return Err(format!(
                    "{MANIFEST}: served_by {host:?} is not [a-z0-9_]{{1,64}}"
                ));
            }
            if host == &m.id {
                return Err(format!(
                    "{MANIFEST}: served_by cannot name this package's own id {host:?}"
                ));
            }
        }
        mixin::Mixin::check_all(
            m.native
                .as_ref()
                .map(|n| n.mixin.as_slice())
                .unwrap_or_default(),
            &m.id,
        )
        .map_err(|e| format!("{MANIFEST}: {e}"))?;
        loops::Loop::check_all(
            m.native
                .as_ref()
                .map(|n| n.r#loop.as_slice())
                .unwrap_or_default(),
            &m.id,
        )
        .map_err(|e| format!("{MANIFEST}: {e}"))?;
        if let Some(entry) = m
            .native
            .as_ref()
            .map(|n| n.mixin.as_slice())
            .unwrap_or_default()
            .iter()
            .find(|entry| {
                m.dependencies.keys().any(|id| {
                    entry.target.split("::").next() == Some(id.replace('-', "_").as_str())
                })
            })
        {
            return Err(format!(
                "{MANIFEST}: {}",
                mixin::MixinError::Invalid {
                    id: format!("{}:{}", m.id, entry.id),
                    field: "target",
                    fix: "a mixin targets a game function",
                }
            ));
        }
        if let Some(native) = &m.native {
            // Native source edits share one id space.
            let mut ids = std::collections::HashSet::new();
            for id in native
                .mixin
                .iter()
                .map(|entry| &entry.id)
                .chain(native.bridge.iter().chain(&native.state).map(|row| &row.id))
                .chain(native.r#loop.iter().map(|row| &row.id))
            {
                if id.trim().is_empty() || !ids.insert(id) {
                    return Err(format!(
                        "{MANIFEST}: empty or duplicate mixin, bridge, state or loop id"
                    ));
                }
            }
            // A bridge's id becomes part of its accessor's name: `-` reads as `_`, and case is
            // dropped, so the id is that alphabet and no two ids may give one name.
            let mut accessors = std::collections::HashMap::new();
            for bridge in &native.bridge {
                let id = &bridge.id;
                if !id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                {
                    return Err(format!(
                        "{MANIFEST}: E_MIXIN_INVALID: {}:{id}: invalid id; a bridge id is ASCII letters, digits, `_` and `-`",
                        m.id
                    ));
                }
                if let Some(other) = accessors.insert(id.replace('-', "_").to_ascii_lowercase(), id)
                {
                    return Err(format!(
                        "{MANIFEST}: E_MIXIN_INVALID: {}:{id}: invalid id; it gives the same accessor name as bridge `{other}`",
                        m.id
                    ));
                }
            }
            for module in &native.module {
                if module.id.trim().is_empty() || !ids.insert(&module.id) {
                    return Err(format!("{MANIFEST}: empty or duplicate module id"));
                }
                if !plain_path(&module.file) {
                    return Err(format!("{MANIFEST}: module file must be a plain path"));
                }
                if !rust_ident(&module.name) {
                    return Err(format!("{MANIFEST}: module name must be an identifier"));
                }
                if !plain_path(&module.source) || !source_file(&module.source) {
                    return Err(format!(
                        "{MANIFEST}: module source must be a file under src/"
                    ));
                }
                if let Some(visibility) = &module.visibility
                    && !matches!(visibility.as_str(), "pub" | "pub(crate)")
                {
                    return Err(format!(
                        "{MANIFEST}: module visibility must be pub or pub(crate)"
                    ));
                }
            }
            if native.game.is_empty()
                || !revision(&native.sunrise)
                || native.mixins.as_deref().is_some_and(|m| !revision(m))
            {
                return Err(format!(
                    "{MANIFEST}: native requires game and exact revisions"
                ));
            }
        }
        if m.api != API {
            return Err(format!(
                "unsupported content API {}; this Sunrise reads {API}: delete the api line, the tool writes it",
                m.api
            ));
        }
        if m.capabilities != [Capability::Registries] {
            let names = |list: &[Capability]| {
                format!(
                    "[{}]",
                    list.iter()
                        .map(|c| format!("\"{c}\""))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            return Err(format!(
                "{MANIFEST}: capabilities = {} but the tool writes {}; delete the capabilities line",
                names(&m.capabilities),
                names(&Capability::derived())
            ));
        }
        for (field, url) in [
            ("license_url", &m.license_url),
            ("homepage", &m.homepage),
            ("issues", &m.issues),
        ] {
            if let Some(url) = url
                && (url.len() > 2048
                    || !(url.starts_with("https://") || url.starts_with("http://")))
            {
                return Err(format!(
                    "{MANIFEST}: {field} is an http(s) link of at most 2048 characters"
                ));
            }
        }
        for (i, ns) in m.namespaces.iter().enumerate() {
            // Vanilla's namespace characters, at most as long as an id.
            let chars_ok = (1..=64).contains(&ns.len())
                && ns
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b));
            if !chars_ok {
                return Err(format!(
                    "{MANIFEST}: namespace {ns:?} is not [a-z0-9_.-]{{1,64}}"
                ));
            }
            if RESERVED_NAMESPACES.contains(&ns.as_str()) {
                return Err(format!(
                    "{MANIFEST}: namespace {ns} is not a mod's to claim (only to add to its tags)"
                ));
            }
            if *ns == m.id || m.namespaces[..i].contains(ns) {
                return Err(format!("{MANIFEST}: namespace {ns} is listed twice"));
            }
        }
        Ok(m)
    }
    /// The namespaces the mod's content owns: its id, then `namespaces`.
    pub fn owned_namespaces(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.id.as_str()).chain(self.namespaces.iter().map(String::as_str))
    }
    /// The refusal of a `side = "server"` mod whose code declares registry content (one of
    /// `declared`, each its registry kind and id, in a namespace it owns), naming the mod, the
    /// kind and the id: every client must have that content.
    pub fn server_only_declaration<'a>(
        &self,
        declared: impl Iterator<Item = (&'a str, &'a str)>,
    ) -> Option<String> {
        self.server_only_declaration_for(&[self], declared)
            .map(|refusal| refusal.to_string())
    }
    /// Refuses any client-required declaration in `declared` when either `self` or the package
    /// in `suppliers` supplying the declaration's namespace is `side = "server"`.
    pub fn server_only_declaration_for<'a>(
        &self,
        suppliers: &[&Manifest],
        mut declared: impl Iterator<Item = (&'a str, &'a str)>,
    ) -> Option<ServerOnlyDeclaration> {
        let (supplier, kind, id) = declared.find_map(|(kind, id)| {
            let ns = id.split_once(':').map_or("", |(ns, _)| ns);
            let supplier = suppliers
                .iter()
                .copied()
                .find(|m| m.owned_namespaces().any(|owned| owned == ns))?;
            (self.side == Side::Server || supplier.side == Side::Server)
                .then_some((supplier, kind, id))
        })?;
        Some(ServerOnlyDeclaration {
            declarer: self.id.clone(),
            supplier: supplier.id.clone(),
            kind: kind.to_owned(),
            id: id.to_owned(),
        })
    }
}

/// Refusal when code declares client-required content for a `side = "server"` declarer or
/// served namespace supplier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerOnlyDeclaration {
    pub declarer: String,
    pub supplier: String,
    pub kind: String,
    pub id: String,
}

impl std::fmt::Display for ServerOnlyDeclaration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.declarer == self.supplier {
            write!(
                f,
                "{}: a side = \"server\" mod declares the {} {}, which every client must have; \
                 declare side = \"both\"",
                self.declarer, self.kind, self.id
            )
        } else {
            write!(
                f,
                "{}: declares the {} {} in side = \"server\" package {}'s namespace, which every \
                 client must have; declare side = \"both\"",
                self.declarer, self.kind, self.id, self.supplier
            )
        }
    }
}

impl std::error::Error for ServerOnlyDeclaration {}

pub use mod_source::package::{NOT_PACKAGED, source_file};

pub(crate) fn plain_path(name: &str) -> bool {
    !name.starts_with('/')
        && !name.contains(['\\', ':'])
        && !name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
}

/// A mod as found on disk.
pub struct Package {
    pub manifest: Manifest,
    pub icon: Option<crate::icon::Icon>,
}
impl Package {
    /// The kind of this validated package, independent of its install folder.
    pub fn is_native(&self) -> bool {
        self.manifest.native.is_some()
    }

    /// Read a flat `.smod` or project folder, without executing native code.
    pub fn read(path: &Path) -> Result<Self, String> {
        let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        let mut icon = None;
        let mut has_cargo = false;
        let mut has_source = false;
        let manifest = if metadata.is_dir() {
            for old in [
                "sunrise-native.json",
                "horizon-mixin.toml",
                "native",
                "rust",
            ] {
                if std::fs::symlink_metadata(path.join(old)).is_ok() {
                    return Err(format!("{old}: unsupported package path"));
                }
            }
            if std::fs::symlink_metadata(path.join("Cargo.toml")).is_ok() {
                check_folder_file(path, "Cargo.toml")?;
                has_cargo = true;
            }
            if let Ok(source) = std::fs::symlink_metadata(path.join("src")) {
                if !source.is_dir() {
                    return Err("src: unsupported package path".into());
                }
                has_source = true;
            }
            let icon_path = path.join("icon.png");
            match std::fs::symlink_metadata(&icon_path) {
                Ok(metadata) => {
                    check_folder_file(path, "icon.png")?;
                    if metadata.len() > crate::icon::MAX_ICON as u64 {
                        return Err("icon.png exceeds 256 KiB".into());
                    }
                    let mut bytes = Vec::new();
                    std::fs::File::open(&icon_path)
                        .map_err(|e| format!("icon.png: {e}"))?
                        .take(crate::icon::MAX_ICON as u64 + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|e| format!("icon.png: {e}"))?;
                    icon = Some(crate::icon::Icon::parse(bytes)?);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("icon.png: {e}")),
            }
            check_folder_file(path, MANIFEST)?;
            for (name, _) in crate::content::folder_files(path)? {
                check_folder_file(path, &name)?;
            }
            let file =
                std::fs::File::open(path.join(MANIFEST)).map_err(|e| format!("{MANIFEST}: {e}"))?;
            Manifest::read(file)?
        } else {
            if !metadata.is_file() {
                return Err(format!(
                    "{}: unsupported file type in a content mod",
                    path.display()
                ));
            }
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
            return Self::read_zip(&mut zip);
        };
        let package = Self::from_parts(manifest, icon, has_cargo, has_source)?;
        package.check_runtime_layout(&crate::content::runtime_folder_files(path)?)?;
        Ok(package)
    }

    /// Read and validate an archive, including all selected source and resource entries.
    pub fn read_zip<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>) -> Result<Self, String> {
        let mut icon = None;
        let mut has_cargo = false;
        let mut has_source = false;
        let mut entries = Vec::new();
        let mut uses = false;
        let mut runtime = Vec::new();
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
            let name = entry.name().map_err(|e| e.to_string())?.into_owned();
            let mode = entry.unix_mode().unwrap_or_default();
            entries.push((name.clone(), entry.size(), entry.is_dir()));
            let first = name.split('/').next().unwrap_or("");
            if matches!(
                first,
                "sunrise-native.json" | "horizon-mixin.toml" | "native" | "rust"
            ) {
                return Err(format!("{name}: unsupported package path"));
            }
            has_source |= first == "src";
            if entry.is_dir() && matches!(mode & 0o170000, 0 | 0o040000) {
                continue;
            }
            has_cargo |= name == "Cargo.toml";
            if let Some(below) = name
                .strip_prefix(RUNTIME_DIR)
                .and_then(|n| n.strip_prefix('/'))
            {
                // Any file type, but a file: its bytes are checked against `[runtime]` below.
                if !matches!(mode & 0o170000, 0 | 0o100000) {
                    return Err(format!("{name}: unsupported file type"));
                }
                runtime.push((below.to_owned(), entry.size()));
                continue;
            }
            if name == USES_FILE && matches!(mode & 0o170000, 0 | 0o100000) {
                if uses {
                    return Err(format!("duplicate {USES_FILE}"));
                }
                if entry.size() > MAX_USES {
                    return Err(format!("{USES_FILE} exceeds {} MiB", MAX_USES >> 20));
                }
                uses = true;
                continue;
            }
            check_file(&name, mode, || Ok(&mut entry)).map_err(|e| e.to_string())?;
            if name == "icon.png" {
                if icon.is_some() {
                    return Err("duplicate icon.png".into());
                }
                if entry.size() > crate::icon::MAX_ICON as u64 {
                    return Err("icon.png exceeds 256 KiB".into());
                }
                let mut bytes = Vec::new();
                drop(entry);
                zip.by_index(i)
                    .map_err(|e| e.to_string())?
                    .take(crate::icon::MAX_ICON as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| format!("icon.png: {e}"))?;
                icon = Some(crate::icon::Icon::parse(bytes)?);
            }
        }
        let entry = zip
            .by_name(MANIFEST)
            .map_err(|e| format!("{MANIFEST}: {e}"))?;
        let package = Self::from_parts(Manifest::read(entry)?, icon, has_cargo, has_source)?;
        if uses && !package.is_native() {
            return Err(format!("a content mod has no code, so no {USES_FILE}"));
        }
        package.check_runtime_layout(&runtime)?;
        if package.is_native() {
            let mut names = std::collections::HashSet::new();
            let mut source_count = 0usize;
            let mut source_bytes = 0u64;
            let mut resource_count = 0usize;
            let mut resource_bytes = 0u64;
            for (name, size, directory) in entries {
                let path = if directory {
                    name.trim_end_matches('/')
                } else {
                    &name
                };
                if !plain_path(path) || !names.insert(name.clone()) {
                    return Err(format!("invalid or duplicate package path: {name}"));
                }
                if !directory {
                    // Starling: a package adopted from Horizon's registry keeps the SHA-256 of the file it
                    // came from (`horizon::ORIGIN`), which names its library in the mod store.
                    if name != MANIFEST
                        && name != USES_FILE
                        && name != "icon.png"
                        && name != crate::horizon::ORIGIN
                        && !name.starts_with("runtime/")
                        && !name.starts_with("assets/")
                        && !name.starts_with("data/")
                        && !source_file(&name)
                    {
                        return Err(format!("unexpected package path: {name}"));
                    }
                    if source_file(&name) {
                        source_count += 1;
                        source_bytes = source_bytes
                            .checked_add(size)
                            .ok_or("native package too large")?;
                    } else if name.starts_with("assets/") || name.starts_with("data/") {
                        crate::content::check_path(&name)?;
                        crate::content::check_namespace(&name, &package.manifest)?;
                        if size > crate::content::MAX_FILE {
                            return Err(format!("{name} exceeds the content file size limit"));
                        }
                        resource_count += 1;
                        resource_bytes = resource_bytes
                            .checked_add(size)
                            .ok_or("content too large")?;
                    }
                }
            }
            if source_count > MAX_SOURCE_FILES || source_bytes > MAX_SOURCE_BYTES {
                return Err("native package exceeds its entry or size limit".into());
            }
            if resource_count > crate::content::MAX_FILES
                || resource_bytes > crate::content::MAX_TOTAL
            {
                return Err("content exceeds its entry or size limit".into());
            }
        }
        Ok(package)
    }

    /// Refuse a `runtime/` folder that is not the one `[runtime]` lists: an extra, a missing or an
    /// oversized file, each named. `found` is each file below `runtime/` and its size.
    fn check_runtime_layout(&self, found: &[(String, u64)]) -> Result<(), String> {
        if !found.is_empty() && !self.is_native() {
            return Err("a content mod has no code, so no runtime/ folder".into());
        }
        RuntimeTable::check_layout(self.manifest.runtime.as_ref(), found)
    }

    /// The package's runtime files, read from `path` (a folder or a `.smod`) and each checked
    /// against its `[runtime]` SHA-512: the one place their bytes are hashed. A mismatch is
    /// refused by name. `None` for a package with no `[runtime]`.
    pub fn read_runtime(&self, path: &Path) -> Result<Option<RuntimeData>, String> {
        let Some(table) = &self.manifest.runtime else {
            return Ok(None);
        };
        let mut files = Vec::new();
        let runtime_dir = path.is_dir().then(|| path.join(RUNTIME_DIR));
        let mut zip = if runtime_dir.is_some() {
            None
        } else {
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            Some(zip::ZipArchive::new(file).map_err(|e| e.to_string())?)
        };
        for (name, _) in table.files() {
            let entry = format!("{RUNTIME_DIR}/{name}");
            let mut bytes = Vec::new();
            let path = runtime_dir.as_ref().map(|dir| dir.join(name));
            let read = if let Some(path) = &path {
                std::fs::File::open(path)
                    .map_err(|e| format!("{entry}: {e}"))?
                    .read_to_end(&mut bytes)
            } else {
                zip.as_mut()
                    .expect("an archive")
                    .by_name(&entry)
                    .map_err(|e| format!("{entry}: {e}"))?
                    .read_to_end(&mut bytes)
            };
            read.map_err(|e| format!("{entry}: {e}"))?;
            table.check(name, &bytes)?;
            files.push((
                name.to_owned(),
                runtime_files::RuntimeFile {
                    bytes: bytes.into_boxed_slice(),
                    path,
                },
            ));
        }
        Ok(Some(RuntimeData::new(files, table.digest())))
    }

    fn from_parts(
        manifest: Manifest,
        icon: Option<crate::icon::Icon>,
        has_cargo: bool,
        has_source: bool,
    ) -> Result<Self, String> {
        if has_cargo != manifest.native.is_some() || has_source != has_cargo {
            return Err("[native] and Rust source must be declared together".into());
        }
        Ok(Package { manifest, icon })
    }
    /// Select and validate a flat project's source and resource files deterministically. Any
    /// other file in the folder (a `module.toml`, `tools/generate.rs`, `.cargo/config.toml`,
    /// `rust-toolchain.toml`) is left out, and no folder but `src/` is read for source.
    pub fn directory_files(
        &self,
        root: &Path,
    ) -> Result<std::collections::BTreeMap<String, Vec<u8>>, String> {
        let content = crate::Content::read(root, &self.manifest)?;
        let mut files = std::collections::BTreeMap::new();
        let reader = content.reader();
        for name in content.files.iter() {
            files.insert(name.clone(), reader.read(name)?);
        }
        if let Some(icon) = &self.icon {
            files.insert("icon.png".to_string(), icon.bytes().to_vec());
        }
        if let Some(runtime) = self.read_runtime(root)? {
            for name in runtime.names() {
                let (bytes, _) = runtime.file(name).expect("a listed file");
                files.insert(format!("{RUNTIME_DIR}/{name}"), bytes.to_vec());
            }
        }
        if self.is_native() {
            let (mut source_bytes, mut source_count) = (0u64, 0usize);
            for name in mod_source::package::source_files(root).map_err(|e| e.to_string())? {
                check_folder_file(root, &name)?;
                let path = root.join(&name);
                let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
                source_bytes = source_bytes
                    .checked_add(size)
                    .ok_or("native package too large")?;
                source_count += 1;
                if source_bytes > MAX_SOURCE_BYTES || source_count > MAX_SOURCE_FILES {
                    return Err("native package exceeds its entry or size limit".into());
                }
                files.insert(name, std::fs::read(path).map_err(|e| e.to_string())?);
            }
        }
        files.insert(
            MANIFEST.to_owned(),
            std::fs::read(root.join(MANIFEST)).map_err(|e| e.to_string())?,
        );
        Ok(files)
    }

    /// Pack a flat project, selecting its source and resource files deterministically.
    pub fn pack_directory(root: &Path) -> Result<Vec<u8>, String> {
        Self::pack_directory_with(root, None, None)
    }

    /// The bytes of entry `name` in the `.smod` at `path`, refused past `most` bytes; `None` when
    /// it has no such entry.
    pub fn entry(path: &Path, name: &str, most: u64) -> Result<Option<Vec<u8>>, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", path.display()))?;
        let entry = match zip.by_name(name) {
            Ok(entry) => entry,
            Err(zip::result::ZipError::FileNotFound) => return Ok(None),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let mut bytes = Vec::new();
        entry
            .take(most + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("{name}: {e}"))?;
        if bytes.len() as u64 > most {
            return Err(format!("{name}: over {most} bytes"));
        }
        Ok(Some(bytes))
    }

    /// [`Package::pack_directory`], carrying `uses` as the package's [`USES_FILE`] table when given.
    pub fn pack_directory_with(
        root: &Path,
        uses: Option<&[u8]>,
        manifest: Option<String>,
    ) -> Result<Vec<u8>, String> {
        let package = Self::read(root)?;
        let mut files = package.directory_files(root)?;
        if let Some(uses) = uses {
            files.insert(USES_FILE.to_owned(), uses.to_vec());
        }
        let selected = files.remove(MANIFEST).expect("selected manifest");
        let text = match manifest {
            Some(text) => text,
            None => String::from_utf8(selected).map_err(|e| e.to_string())?,
        };
        let files: Vec<_> = files
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        Self::write_files(&text, &files)
    }

    /// The `.smod` bytes for `manifest_text` and selected project files.
    pub fn write_files(manifest_text: &str, files: &[(&str, &[u8])]) -> Result<Vec<u8>, String> {
        use std::io::Write;
        let manifest = Manifest::parse(manifest_text)?;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        let err = |e: zip::result::ZipError| e.to_string();
        let mut put = |name: &str, bytes: &[u8]| -> Result<(), String> {
            if minisign_secret_key(bytes) {
                return Err(format!("{name}: a Minisign private key cannot be packaged"));
            }
            zip.start_file(name, options).map_err(err)?;
            zip.write_all(bytes).map_err(|e| e.to_string())
        };
        put(MANIFEST, manifest_text.as_bytes())?;
        let code = manifest.native.is_some();
        for (name, bytes) in files {
            let source = code && source_file(name) && plain_path(name);
            if let Some(below) = name.strip_prefix("runtime/") {
                let table = manifest
                    .runtime
                    .as_ref()
                    .ok_or(format!("{name}: not listed in [runtime]"))?;
                table.check(below, bytes)?;
                put(name, bytes)?;
                continue;
            }
            if *name == USES_FILE && code {
                if bytes.len() as u64 > MAX_USES {
                    return Err(format!("{USES_FILE} exceeds {} MiB", MAX_USES >> 20));
                }
                put(name, bytes)?;
                continue;
            }
            if *name != "icon.png" && !source {
                crate::content::check_path(name)?;
            }
            check_file(name, 0, || Ok(*bytes)).map_err(|e| e.to_string())?;
            put(name, bytes)?;
        }
        let bytes = zip.finish().map_err(err)?.into_inner();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).map_err(err)?;
        Self::read_zip(&mut archive)?;
        Ok(bytes)
    }
}

/// Recognize a Minisign secret-key record by its framing, without decrypting or copying the secret.
/// The 158-byte payload starts with Ed25519, a scrypt or empty KDF, and a BLAKE2b checksum tag.
fn minisign_secret_key(bytes: &[u8]) -> bool {
    use base64::{Engine, engine::general_purpose::STANDARD};
    const KEY_BYTES: usize = 158;
    let header = |bytes: &[u8]| matches!(bytes, b"EdScB2" | b"Ed\0\0B2");
    if bytes.len() >= KEY_BYTES && header(&bytes[..6]) {
        return true;
    }
    // The text format has an arbitrary comment line, then the base64 payload. The comment
    // is not authentication: a renamed file or a different comment still carries the key.
    let Some(line) = bytes.splitn(3, |&byte| byte == b'\n').nth(1) else {
        return false;
    };
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.len() < KEY_BYTES.div_ceil(3) * 4 || !line.len().is_multiple_of(4) {
        return false;
    }
    let mut prefix = [0; 6];
    if STANDARD.decode_slice(&line[..8], &mut prefix).is_err() || !header(&prefix) {
        return false;
    }
    let encoded = line
        .strip_suffix(b"==")
        .or_else(|| line.strip_suffix(b"="))
        .unwrap_or(line);
    encoded.len() * 3 / 4 >= KEY_BYTES
        && encoded[8..]
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/'))
}

/// A content entry that cannot be safely loaded as data.
#[derive(Debug)]
pub enum ContentFileError {
    UnsupportedType(String),
    Executable(String),
    Read {
        name: String,
        source: std::io::Error,
    },
}

impl std::fmt::Display for ContentFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedType(name) => {
                write!(f, "{name}: unsupported file type in a content mod")
            }
            Self::Executable(name) => {
                write!(f, "{name}: unsupported executable file in a content mod")
            }
            Self::Read { name, source } => write!(f, "{name}: {source}"),
        }
    }
}

impl std::error::Error for ContentFileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// The shared content-file boundary. Refuse type/mode before opening a file,
/// then inspect its header even when its name has no executable extension.
pub(crate) fn check_file<R: Read>(
    name: &str,
    mode: u32,
    open: impl FnOnce() -> std::io::Result<R>,
) -> Result<(), ContentFileError> {
    if !matches!(mode & 0o170000, 0 | 0o100000) {
        return Err(ContentFileError::UnsupportedType(name.into()));
    }
    let extension = name
        .rsplit(['/', '\\'])
        .next()
        .and_then(|n| n.rsplit_once('.'));
    let executable = extension.is_some_and(|(_, ext)| {
        [
            "exe", "dll", "so", "dylib", "class", "jar", "com", "bat", "cmd", "ps1", "wasm",
        ]
        .iter()
        .any(|kind| ext.eq_ignore_ascii_case(kind))
    });
    if executable || mode & 0o111 != 0 {
        return Err(ContentFileError::Executable(name.into()));
    }
    let error = |source| ContentFileError::Read {
        name: name.into(),
        source,
    };
    let mut file = open().map_err(error)?;
    let mut header = [0; 4];
    let mut read = 0;
    while read < header.len() {
        let size = file.read(&mut header[read..]).map_err(error)?;
        if size == 0 {
            break;
        }
        read += size;
    }
    let executable = matches!(
        &header,
        b"\x7fELF"
            | b"\0asm"
            | b"\xfe\xed\xfa\xce"
            | b"\xce\xfa\xed\xfe"
            | b"\xfe\xed\xfa\xcf"
            | b"\xcf\xfa\xed\xfe"
            | b"\xca\xfe\xba\xbe"
            | b"\xbe\xba\xfe\xca"
    ) || header.starts_with(b"MZ")
        || (header.starts_with(b"#!") && header[2] != b'[');
    if executable {
        return Err(ContentFileError::Executable(name.into()));
    }
    Ok(())
}

fn check_folder_file(root: &Path, name: &str) -> Result<(), String> {
    let path = root.join(name);
    let metadata = std::fs::symlink_metadata(&path).map_err(|e| format!("{name}: {e}"))?;
    let mode = if metadata.is_file() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            metadata.mode()
        }
        #[cfg(not(unix))]
        {
            0
        }
    } else {
        0o120000
    };
    check_file(name, mode, || std::fs::File::open(&path)).map_err(|e| e.to_string())
}

/// The `.smod` files and mod folders in `dir`, by name.
pub fn find(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "smod") || p.join(MANIFEST).is_file())
        .collect();
    found.sort();
    found
}

#[cfg(test)]
#[path = "manifest/runtime_files_tests.rs"]
mod runtime_files_tests;

#[cfg(test)]
mod tests {
    /// A project folder packs its source set and leaves every other file out, so the archive
    /// reads back as a valid package.
    #[test]
    fn a_native_project_folder_packs_its_source_set_and_leaves_other_files_out() {
        let folder = tempfile::tempdir().unwrap();
        let write = |name: &str, text: &str| {
            let path = folder.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            MANIFEST,
            &format!(
                "id='butterflies'\nname='Butterflies'\nversion='0.1.0'\napi='0.2'\n\
                 capabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'\nmixins='{}'\n",
                "a".repeat(40),
                "b".repeat(40)
            ),
        );
        write(
            "Cargo.toml",
            "[package]\nname='butterflies'\nversion='0.1.0'\n",
        );
        write("src/lib.rs", "pub fn hooks() {}");
        write("module.toml", "[module]\nname='butterflies'\n");
        write("sources/wings.toml", "colour='blue'\n");
        write("tools/generate.rs", "fn main() {}");
        write(
            ".cargo/config.toml",
            "[source.crates-io]\nreplace-with='local'\n",
        );
        write("rust-toolchain.toml", "[toolchain]\nchannel='nightly'\n");
        // A folder beside the crate is never read, so a name packing could not encode is no bar.
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let junk = folder.path().join("node_modules");
            std::fs::create_dir_all(&junk).unwrap();
            std::fs::write(junk.join(std::ffi::OsStr::from_bytes(b"\xff.rs")), "").unwrap();
            std::fs::write(
                folder
                    .path()
                    .join(std::ffi::OsStr::from_bytes(b"\xff.toml")),
                "",
            )
            .unwrap();
            // On Unix a `\` is part of a file name, never a separator.
            std::fs::write(folder.path().join("src").join("a\\b.rs"), "").unwrap();
            std::fs::write(
                folder
                    .path()
                    .join("src")
                    .join(std::ffi::OsStr::from_bytes(b"\xff.rs")),
                "",
            )
            .unwrap();
        }

        let bytes = Package::pack_directory(folder.path()).unwrap();
        let archive = folder.path().join("butterflies.smod");
        std::fs::write(&archive, &bytes).unwrap();
        let mut names: Vec<_> = zip::ZipArchive::new(std::io::Cursor::new(&bytes))
            .unwrap()
            .file_names()
            .map(|name| name.unwrap().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["Cargo.toml", "src/lib.rs", MANIFEST]);
        assert!(Package::read(&archive).unwrap().is_native());
    }

    /// A short version with a pre-release or build part is padded before it, not after it.
    #[test]
    fn a_short_version_with_a_pre_release_is_padded_in_its_core() {
        for (short, full) in [
            ("1", "1.0.0"),
            ("1.2", "1.2.0"),
            ("1.2.3", "1.2.3"),
            ("1.2-beta", "1.2.0-beta"),
            ("1-rc.1", "1.0.0-rc.1"),
            ("1.2-rc.1", "1.2.0-rc.1"),
            ("1.2.3-rc.1", "1.2.3-rc.1"),
            ("1.2+build.5", "1.2.0+build.5"),
            ("1.0-SNAPSHOT", "1.0.0-SNAPSHOT"),
        ] {
            assert_eq!(
                parse_version(short).unwrap(),
                semver::Version::parse(full).unwrap(),
                "{short}"
            );
        }
        assert!(parse_version("").is_err());
        assert!(parse_version("one.two").is_err());
    }

    use super::*;

    #[test]
    fn a_version_that_is_not_semver_refuses_the_manifest_when_read() {
        let with = |v: &str| {
            Manifest::parse(&EXAMPLE.replace("version = \"0.1.0\"", &format!("version = '{v}'")))
        };
        let error = with("v1 beta!").unwrap_err();
        assert!(
            error.contains(
                "version \"v1 beta!\" is not a semver version (MAJOR.MINOR.PATCH, e.g. 3.6.30+refabricated)"
            ),
            "{error}"
        );
        assert!(with("").is_err());
    }

    #[test]
    fn an_omitted_minor_and_patch_are_padded() {
        let with = |v: &str| {
            Manifest::parse(&EXAMPLE.replace("version = \"0.1.0\"", &format!("version = '{v}'")))
                .unwrap()
                .version
                .to_string()
        };
        assert_eq!(with("1"), "1.0.0");
        assert_eq!(with("1.2"), "1.2.0");
        assert_eq!(with("3.6.30+refabricated"), "3.6.30+refabricated");
    }

    #[test]
    fn windows_device_mod_ids_are_refused() {
        for id in ["con", "prn", "aux", "nul", "com1", "com9", "lpt1", "lpt9"] {
            assert!(
                Manifest::parse(&EXAMPLE.replace("ruby", id))
                    .unwrap_err()
                    .contains("reserved Windows device name"),
                "{id}"
            );
        }
        for id in ["console", "com0", "com10", "lpt0", "con_tool"] {
            assert!(
                Manifest::parse(&EXAMPLE.replace("ruby", id)).is_ok(),
                "{id}"
            );
        }
    }

    const EXAMPLE: &str = r#"
id = "ruby"
name = "Ruby"
version = "0.1.0"
api = "0.2"
capabilities = ["registries"]
"#;

    #[test]
    fn api_capabilities_and_mixins_are_optional_and_an_explicit_other_value_is_refused() {
        let bare = "id = \"ruby\"\nname = \"Ruby\"\nversion = \"0.1.0\"\n";
        let manifest = Manifest::parse(bare).unwrap();
        assert_eq!(manifest.api, API);
        assert_eq!(manifest.capabilities, [Capability::Registries]);
        let error = Manifest::parse(&format!("{bare}capabilities = []\n")).unwrap_err();
        assert!(
            error.contains("[]") && error.contains("[\"registries\"]"),
            "{error}"
        );
        let native = format!(
            "{bare}[native]\ngame = \"26.3\"\nsunrise = \"{}\"\n",
            "a".repeat(40)
        );
        let native = Manifest::parse(&native).unwrap().native.unwrap();
        assert_eq!(native.mixins_or("sdk"), "sdk");
        let pinned = Native {
            mixins: Some("b".repeat(40)),
            ..native
        };
        assert_eq!(pinned.mixins_or("sdk"), "b".repeat(40));
    }

    #[test]
    fn a_missing_or_misspelled_field_is_named_with_the_line_to_write() {
        let missing = Manifest::parse(&EXAMPLE.replace("version = \"0.1.0\"\n", "")).unwrap_err();
        assert!(missing.contains("version = \"0.1.0\""), "{missing}");
        let typo = Manifest::parse(&EXAMPLE.replace("version =", "vrsion =")).unwrap_err();
        assert!(typo.contains("unknown field `vrsion`"), "{typo}");
        assert!(typo.contains("did you mean `version`?"), "{typo}");
        let far = Manifest::parse(&format!("{EXAMPLE}zzzzzzzz = 1\n")).unwrap_err();
        assert!(!far.contains("did you mean"), "{far}");
    }

    #[test]
    fn unknown_top_level_array_of_tables_is_refused() {
        let error = Manifest::parse(&format!(
            "{EXAMPLE}\n[[inject]]\nhook = 'player_logged_in'\n"
        ))
        .unwrap_err();
        assert!(error.contains("unknown field `inject`"), "{error}");
    }

    #[test]
    fn refuses_bad_ids_and_retired_manifests() {
        let bad_id = EXAMPLE.replace("ruby", "Ruby!");
        assert!(Manifest::parse(&bad_id).unwrap_err().contains("is not"));
        for api in ["0.1", "0.3", "0.4"] {
            let text = EXAMPLE.replace("api = \"0.2\"", &format!("api = \"{api}\""));
            assert!(
                Manifest::parse(&text)
                    .unwrap_err()
                    .contains("unsupported content API")
            );
        }
        for caps in ["[]", "[\"hud\"]", "[\"registries\", \"registries\"]"] {
            let text = EXAMPLE.replace("[\"registries\"]", caps);
            assert!(Manifest::parse(&text).is_err());
        }
        for field in ["network = true", "experimental = []", "side = \"local\""] {
            assert!(Manifest::parse(&format!("{EXAMPLE}{field}\n")).is_err());
        }
        assert_eq!(Manifest::parse(EXAMPLE).unwrap().side, Side::Client);
        assert_eq!(
            Manifest::parse(&format!("{EXAMPLE}side = \"both\"\n"))
                .unwrap()
                .side,
            Side::Both
        );
        let server = Manifest::parse(&format!("{EXAMPLE}side = \"server\"\n")).unwrap();
        assert_eq!(server.side, Side::Server);
        let id = &server.id;
        assert_eq!(
            server.server_only_declaration(
                [
                    ("block", "minecraft:stone"),
                    ("item", &format!("{id}:marker")[..])
                ]
                .into_iter()
            ),
            Some(format!(
                "{id}: a side = \"server\" mod declares the item {id}:marker, which every client \
                 must have; declare side = \"both\""
            ))
        );
        assert_eq!(
            server.server_only_declaration([("block", "minecraft:stone")].into_iter()),
            None
        );
    }

    #[test]
    fn native_manifest_requires_exact_pins_and_known_fields() {
        let native = format!(
            "[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        Manifest::parse(&format!("{EXAMPLE}{native}")).unwrap();
        assert!(
            Manifest::parse(&format!(
                "{EXAMPLE}{}",
                native.replace(&"a".repeat(40), "main")
            ))
            .is_err()
        );
        for unsupported in ["unexpected = true", "[[native.post_build]]\nbin = 'bake'"] {
            assert!(Manifest::parse(&format!("{EXAMPLE}{native}{unsupported}\n")).is_err());
        }
    }

    #[test]
    fn native_unknown_fields_are_refused_at_the_owning_boundary() {
        let pin = "a".repeat(40);
        let base = format!("{EXAMPLE}[native]\ngame='26.3'\nsunrise='{pin}'\nmixins='{pin}'\n");
        let mut accepted = Vec::new();
        for field in [
            "injection=[]",
            "[[native.injection]]\nid='tick'\ntarget='game::tick'\nkind='HEAD'\nraw='tick'",
            "internals=[]",
            "[[native.internals]]\npath='Game'\nhash='h'",
            "unexpected=true",
        ] {
            let text = format!("{base}{field}\n");
            match Manifest::parse(&text) {
                Ok(_) => accepted.push(field),
                Err(error) => assert!(error.contains("unknown field"), "{error}"),
            }
            // The same owning serde boundary applies outside Manifest::parse.
            if toml::from_str::<Native>(text.split("[native]\n").nth(1).unwrap()).is_ok() {
                accepted.push(field);
            }
        }
        assert!(
            accepted.is_empty(),
            "accepted unknown native fields: {accepted:?}"
        );
    }

    #[test]
    fn internal_kinds_distinguish_members_and_round_trip() {
        let base = format!(
            "{EXAMPLE}[native]\ngame='26.3'\nsunrise='{}'\nmixins='{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let declaration = "internals=[{path='Round::listening',kind='field',hash='1234'}, {path='Round::listening',kind='method',hash='5678'}]\n";
        let manifest =
            Manifest::parse(&base.replace("[native]", &format!("{declaration}[native]"))).unwrap();
        assert_eq!(manifest.internals[1].kind.as_deref(), Some("method"));
        let row = toml::to_string(&manifest.internals[1]).unwrap();
        assert_eq!(
            toml::from_str::<Internal>(&row).unwrap(),
            manifest.internals[1]
        );
        let native = manifest.native.unwrap();
        let text = toml::to_string(&native).unwrap();
        assert_eq!(toml::from_str::<Native>(&text).unwrap(), native);
        for kind in ["field", ""] {
            let invalid = declaration.replace("kind='method'", &format!("kind='{kind}'"));
            assert!(
                Manifest::parse(&base.replace("[native]", &format!("{invalid}[native]"))).is_err()
            );
        }
        let untyped = Internal {
            path: "Game".into(),
            kind: None,
            hash: Some("1234".into()),
        };
        assert!(!toml::to_string(&untyped).unwrap().contains("kind"));
    }

    #[test]
    fn native_modules_are_validated() {
        let native = format!(
            "[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let module = "[[native.module]]\nid = 'runtime'\nfile = 'crates/sunrise-client/src/game.rs'\nname = 'runtime'\nsource = 'src/runtime.rs'\n";
        let manifest = Manifest::parse(&format!("{EXAMPLE}{native}{module}")).unwrap();
        let module = &manifest.native.as_ref().unwrap().module[0];
        assert_eq!(module.name, "runtime");
        assert_eq!(module.visibility, None);
        // Support modules still forward whole to horizon-mixins.
        let forwarded = toml::to_string(manifest.native.as_ref().unwrap()).unwrap();
        assert!(forwarded.contains("[[module]]"));
        assert!(forwarded.contains("id = \"runtime\""));
        let valid = "[[native.module]]\nid = 'runtime'\nfile = 'crates/sunrise-client/src/game.rs'\nname = 'runtime'\nsource = 'src/runtime.rs'\n";
        for bad in [
            // escapes the package's src/
            valid.replace("source = 'src/runtime.rs'", "source = '../escape.rs'"),
            // unknown field
            format!("{valid}order = 1\n"),
            // duplicate module id
            format!("{valid}{valid}"),
            format!(
                "{valid}[[native.module]]\nid='runtime'\nfile='game.rs'\nname='other'\nsource='src/other.rs'\n"
            ),
            // not an identifier
            valid.replace("name = 'runtime'", "name = 'run time'"),
            // file must stay a plain path
            valid.replace(
                "file = 'crates/sunrise-client/src/game.rs'",
                "file = '/abs.rs'",
            ),
            // horizon-mixins accepts exactly pub and pub(crate)
            valid.to_owned() + "visibility = 'public'\n",
        ] {
            assert!(
                Manifest::parse(&format!("{EXAMPLE}{native}{bad}")).is_err(),
                "accepted: {bad}"
            );
        }
        let public = valid.to_owned() + "visibility = 'pub(crate)'\n";
        assert!(Manifest::parse(&format!("{EXAMPLE}{native}{public}")).is_ok());
    }

    #[test]
    fn properties_keep_their_scalar_types() {
        let text =
            format!("{EXAMPLE}[properties]\ntier = 3\nratio = 0.5\nlabel = 'x'\nfast = true\n");
        let manifest = Manifest::parse(&text).unwrap();
        assert_eq!(manifest.properties["tier"], Property::Integer(3));
        assert_eq!(manifest.properties["ratio"], Property::Float(0.5));
        assert_eq!(manifest.properties["label"], Property::String("x".into()));
        assert_eq!(manifest.properties["fast"], Property::Boolean(true));
    }

    #[test]
    fn property_values_and_key_count_share_the_whole_manifest_limit() {
        let value = "a".repeat(9 * 1024);
        let mut text = format!("{EXAMPLE}[properties]\ntext = '{value}'\n");
        use std::fmt::Write;
        for key in 0..65 {
            writeln!(text, "k{key} = {key}").unwrap();
        }
        let manifest = Manifest::parse(&text).unwrap();
        assert_eq!(manifest.properties.len(), 66);
        assert_eq!(manifest.properties["k64"], Property::Integer(64));
        assert_eq!(manifest.properties["text"], Property::String(value));
    }

    #[test]
    fn manifest_parse_rejects_input_over_the_published_byte_limit() {
        let text = " ".repeat(32 * 1024 * 1024 + 1);
        assert_eq!(
            Manifest::parse(&text).unwrap_err(),
            "sunrise-mod.toml: over 32 MiB"
        );
    }

    #[test]
    fn manifest_read_refuses_oversized_input_before_decoding_utf8() {
        let start = format!("{EXAMPLE}#");
        let whitespace = std::io::repeat(b' ').take((32 << 20) - start.len() as u64);
        // The bounded reader sees only the first byte of this character beyond the limit.
        let reader = start.as_bytes().chain(whitespace).chain("é\n".as_bytes());
        assert_eq!(
            Manifest::read(reader).unwrap_err(),
            "sunrise-mod.toml: over 32 MiB"
        );
    }

    #[test]
    fn dependencies_are_typed_for_content_and_code_mods() {
        let deps =
            "[dependencies]\ncreate = '^6.0'\naddon = { version = '^1.2', optional = true }\n";
        let content = Manifest::parse(&format!("{EXAMPLE}{deps}")).unwrap();
        assert_eq!(content.dependencies.len(), 2);
        let native = format!(
            "[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let text = format!("{EXAMPLE}{deps}{native}");
        let manifest = Manifest::parse(&text).unwrap();
        assert!(
            matches!(&manifest.dependencies["addon"], Dependency::Detailed(details) if details.optional)
        );
        assert!(matches!(
            &manifest.dependencies["create"],
            Dependency::Version(_)
        ));
        for bad in [
            text.replace("^6.0", "not semver"),
            text.replace("create =", "\"not-valid\" ="),
            text.replace("optional = true", "unexpected = true"),
        ] {
            assert!(Manifest::parse(&bad).is_err());
        }
    }

    #[test]
    fn a_code_mod_carries_one_bounded_uses_table_and_a_content_mod_none() {
        let native = format!(
            "{EXAMPLE}[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let cargo = b"[package]\nname = 'ruby'\n".as_slice();
        let lib = b"pub fn f() {}\n".as_slice();
        let table = br#"{"schema":1,"made_for":"x","entries":[]}"#.as_slice();
        let bytes = Package::write_files(
            &native,
            &[
                ("Cargo.toml", cargo),
                ("src/lib.rs", lib),
                (USES_FILE, table),
            ],
        )
        .unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        Package::read_zip(&mut zip).unwrap();
        let mut read = Vec::new();
        zip.by_name(USES_FILE)
            .unwrap()
            .read_to_end(&mut read)
            .unwrap();
        assert_eq!(read, table);
        let large = vec![b' '; MAX_USES as usize + 1];
        assert!(
            Package::write_files(
                &native,
                &[
                    ("Cargo.toml", cargo),
                    ("src/lib.rs", lib),
                    (USES_FILE, &large)
                ],
            )
            .unwrap_err()
            .contains(USES_FILE)
        );
        // A content mod has no code to scan, so no table either, written or read.
        assert!(Package::write_files(EXAMPLE, &[(USES_FILE, table)]).is_err());
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, bytes) in [(MANIFEST, EXAMPLE.as_bytes()), (USES_FILE, table)] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        let mut archive = zip::ZipArchive::new(zip.finish().unwrap()).unwrap();
        let error = Package::read_zip(&mut archive)
            .err()
            .expect("a content package's uses.json is refused");
        assert!(error.contains(USES_FILE), "{error}");
    }

    #[test]
    fn flat_native_archive_round_trips_through_shared_writer() {
        let manifest = format!(
            "{EXAMPLE}[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let files: &[(&str, &[u8])] = &[
            (
                "Cargo.toml",
                b"[package]\nname = 'other'\nversion = '2.0.0'\n",
            ),
            ("src/lib.rs", b"pub fn hooks() {}"),
        ];
        let bytes = Package::write_files(&manifest, files).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let package = Package::read_zip(&mut zip).unwrap();
        assert_eq!(package.manifest.id, "ruby");
        assert_eq!(package.manifest.version.to_string(), "0.1.0");
        assert_eq!(package.manifest.native.unwrap().game, "26.3");
        assert!(
            Package::write_files(&manifest, &[("Cargo.toml", b""), ("src/../bad.rs", b"")])
                .is_err()
        );
        assert!(
            Package::write_files(
                &manifest,
                &[
                    ("Cargo.toml", b""),
                    ("src/lib.rs", b""),
                    ("src/lib.rs", b"")
                ]
            )
            .is_err()
        );
    }

    #[test]
    fn flat_project_packs_only_selected_safe_files() {
        let root = std::env::temp_dir().join(format!("sunrise-flat-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join(".git/hooks")).unwrap();
        std::fs::create_dir_all(root.join("target/generated")).unwrap();
        let manifest = format!(
            "{EXAMPLE}[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        std::fs::write(root.join(MANIFEST), manifest).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = 'other'\nversion = '2.0.0'\n",
        )
        .unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn hooks() {}\n").unwrap();
        let registry_lock = "version = 4\n[[package]]\nname = 'itoa'\nversion = '1.0.18'\n";
        std::fs::write(root.join("Cargo.lock"), registry_lock).unwrap();
        std::fs::write(root.join(".git/hooks/pre-commit"), "#!/bin/sh\n").unwrap();
        std::fs::write(root.join("target/generated/bad.rs"), "#!/bin/sh\n").unwrap();
        // The SDK the mod tool fetched: the API's own crate, which is never the mod's.
        let sdk = root.join(".sunrise/sdk/api/crates/sunrise-mod-api/src");
        std::fs::create_dir_all(&sdk).unwrap();
        std::fs::write(sdk.join("lib.rs"), "pub fn api() {}\n").unwrap();
        std::fs::write(root.join(".sunrise/sdk/sdk.json"), "{}").unwrap();
        let bytes = Package::pack_directory(&root).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(Package::read_zip(&mut zip).unwrap().manifest.id, "ruby");
        assert!(zip.by_name("Cargo.toml").is_ok());
        assert!(zip.by_name("src/lib.rs").is_ok());
        let mut packed_lock = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("Cargo.lock").unwrap(), &mut packed_lock)
            .unwrap();
        assert_eq!(packed_lock, registry_lock);
        assert!(zip.by_name(".git/hooks/pre-commit").is_err());
        assert!(zip.by_name("target/generated/bad.rs").is_err());
        assert!(
            zip.file_names_raw()
                .all(|name| !name.starts_with(b".sunrise")),
            "{:?}",
            zip.file_names_raw()
                .map(String::from_utf8_lossy)
                .collect::<Vec<_>>()
        );
        assert!(!source_file(
            ".sunrise/sdk/api/crates/sunrise-mod-api/src/lib.rs"
        ));
        std::fs::write(root.join("src/lib.rs"), b"MZ executable").unwrap();
        assert!(
            Package::pack_directory(&root)
                .unwrap_err()
                .contains("unsupported executable")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A port written one file per class keeps more than 511 source files, far under the byte cap.
    #[test]
    fn a_native_project_of_many_source_files_packs_and_reads() {
        let root =
            std::env::temp_dir().join(format!("sunrise-many-sources-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src/classes")).unwrap();
        let manifest = format!(
            "{EXAMPLE}[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        std::fs::write(root.join(MANIFEST), manifest).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = 'ruby'\n").unwrap();
        std::fs::write(root.join("src/lib.rs"), "mod classes;\n").unwrap();
        for i in 0..700 {
            std::fs::write(root.join(format!("src/classes/c{i}.rs")), "pub struct C;\n").unwrap();
        }
        let bytes = Package::pack_directory(&root).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(Package::read_zip(&mut zip).unwrap().manifest.id, "ruby");
        assert!(zip.by_name("src/classes/c699.rs").is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn package_kind_requires_matching_source_and_native_declaration() {
        let root =
            std::env::temp_dir().join(format!("sunrise-package-kind-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(MANIFEST), EXAMPLE).unwrap();
        assert!(Package::read(&root).is_ok());
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = 'unrelated'\nversion = '7.0.0'\n",
        )
        .unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        assert!(Package::read(&root).is_err());
        let native = format!(
            "[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        std::fs::write(root.join(MANIFEST), format!("{EXAMPLE}{native}")).unwrap();
        assert_eq!(Package::read(&root).unwrap().manifest.id, "ruby");
        std::fs::create_dir(root.join("native")).unwrap();
        assert!(Package::read(&root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn checks_extra_namespaces() {
        let with = |list: &str| Manifest::parse(&format!("{EXAMPLE}namespaces = [{list}]\n"));
        let m = with("\"kaisyn\", \"towns_and_towers\"").unwrap();
        let owned: Vec<&str> = m.owned_namespaces().collect();
        assert_eq!(owned, ["ruby", "kaisyn", "towns_and_towers"]);
        for reserved in ["minecraft", "c", "sunrise"] {
            let e = with(&format!("\"{reserved}\"")).unwrap_err();
            assert!(e.contains("not a mod's to claim"), "{e}");
        }
        assert!(with("\"ruby\"").unwrap_err().contains("twice"));
        assert!(with("\"a\", \"a\"").unwrap_err().contains("twice"));
        assert!(with("\"Kaisyn\"").unwrap_err().contains("is not"));
        assert!(with("\"\"").unwrap_err().contains("is not"));
    }

    #[test]
    fn checks_served_by_and_native_serves_content() {
        let default = Manifest::parse(EXAMPLE).unwrap();
        assert_eq!(default.served_by, None);

        let served = Manifest::parse(&format!("{EXAMPLE}served_by = \"horizon_java\"\n")).unwrap();
        assert_eq!(served.served_by.as_deref(), Some("horizon_java"));

        let native_header = format!(
            "[native]\ngame = \"26.3\"\nsunrise = \"{}\"\n",
            "a".repeat(40)
        );
        let native_default = Manifest::parse(&format!("{EXAMPLE}{native_header}")).unwrap();
        assert!(!native_default.native.as_ref().unwrap().serves_content);
        assert!(
            !toml::to_string(native_default.native.as_ref().unwrap())
                .unwrap()
                .contains("serves_content")
        );

        let native_host =
            Manifest::parse(&format!("{EXAMPLE}{native_header}serves_content = true\n")).unwrap();
        assert!(native_host.native.as_ref().unwrap().serves_content);

        let native_served_by = Manifest::parse(&format!(
            "{EXAMPLE}served_by = \"horizon_java\"\n{native_header}"
        ))
        .unwrap_err();
        assert!(native_served_by.contains("served_by"), "{native_served_by}");

        let self_ref = Manifest::parse(&format!("{EXAMPLE}served_by = \"ruby\"\n")).unwrap_err();
        assert!(
            self_ref.contains("served_by") && self_ref.contains("ruby"),
            "{self_ref}"
        );

        for invalid_id in RESERVED_NAMESPACES
            .into_iter()
            .chain(["account", "index", "con", "com1", "", "Bad_Id", "has-dash"])
        {
            let err =
                Manifest::parse(&format!("{EXAMPLE}served_by = \"{invalid_id}\"\n")).unwrap_err();
            assert!(err.contains("served_by"), "{invalid_id}: {err}");
            if !invalid_id.is_empty() {
                assert!(err.contains(invalid_id), "{invalid_id}: {err}");
            }
        }

        let server_host = Manifest::parse(&format!("{EXAMPLE}side = \"server\"\n")).unwrap();
        let both_host = Manifest::parse(&format!("{EXAMPLE}side = \"both\"\n")).unwrap();
        let server_runes = Manifest::parse(
            "id = \"runes\"\nname = \"Runes\"\nversion = \"1.0.0\"\napi = \"0.2\"\n\
             side = \"server\"\nserved_by = \"ruby\"\ncapabilities = [\"registries\"]\n",
        )
        .unwrap();
        let both_runes = Manifest::parse(
            "id = \"runes\"\nname = \"Runes\"\nversion = \"1.0.0\"\napi = \"0.2\"\n\
             side = \"both\"\nserved_by = \"ruby\"\ncapabilities = [\"registries\"]\n",
        )
        .unwrap();
        assert_eq!(
            server_host.server_only_declaration_for(
                &[&server_host, &server_runes],
                [("block", "minecraft:stone"), ("block", "runes:altar")].into_iter(),
            ),
            Some(ServerOnlyDeclaration {
                declarer: "ruby".into(),
                supplier: "runes".into(),
                kind: "block".into(),
                id: "runes:altar".into(),
            })
        );
        // A side = "both" host declaring client-required content in a side = "server" served
        // package's namespace must also be refused, while its own or side = "both" namespaces pass.
        assert_eq!(
            both_host.server_only_declaration_for(
                &[&both_host, &server_runes],
                [("block", "ruby:gear"), ("block", "runes:altar")].into_iter(),
            ),
            Some(ServerOnlyDeclaration {
                declarer: "ruby".into(),
                supplier: "runes".into(),
                kind: "block".into(),
                id: "runes:altar".into(),
            })
        );
        assert_eq!(
            both_host.server_only_declaration_for(
                &[&both_host, &both_runes],
                [("block", "ruby:gear"), ("block", "runes:altar")].into_iter(),
            ),
            None
        );
        assert_eq!(
            server_host.server_only_declaration_for(
                &[&server_host, &server_runes],
                [("block", "other:altar")].into_iter(),
            ),
            None
        );
    }

    #[test]
    fn reserved_namespaces_cannot_be_the_mod_id() {
        for id in RESERVED_NAMESPACES {
            let manifest = EXAMPLE.replace("id = \"ruby\"", &format!("id = {id:?}"));
            let error = Manifest::parse(&manifest).unwrap_err();
            assert!(
                error.contains(MANIFEST)
                    && error.contains(id)
                    && error.contains("reserved namespace"),
                "{error}"
            );
        }
    }

    #[test]
    fn a_page_of_the_mod_site_cannot_be_the_mod_id() {
        for id in ["mine", "account", "policy", "submit", "index", "edit"] {
            let manifest = EXAMPLE.replace("id = \"ruby\"", &format!("id = {id:?}"));
            let error = Manifest::parse(&manifest).unwrap_err();
            assert!(
                error.contains(MANIFEST) && error.contains(id) && error.contains("mod site"),
                "{error}"
            );
        }
    }

    #[test]
    fn the_mods_screen_fields_are_optional_and_links_must_be_http() {
        let m = Manifest::parse(EXAMPLE).unwrap();
        assert!(m.license.is_empty() && m.credits.is_empty());
        assert!(m.license_url.is_none() && m.homepage.is_none() && m.issues.is_none());
        let full = format!(
            "{EXAMPLE}license = \"MIT\"
credits = \"Alex\"
homepage = \"https://example.org\"
issues = \"http://example.org/issues\"
"
        );
        let m = Manifest::parse(&full).unwrap();
        assert_eq!((m.license.as_str(), m.credits.as_str()), ("MIT", "Alex"));
        assert_eq!(m.homepage.as_deref(), Some("https://example.org"));
        for field in ["license_url", "homepage", "issues"] {
            let e = Manifest::parse(&format!(
                "{EXAMPLE}{field} = \"file:///etc/passwd\"
"
            ))
            .unwrap_err();
            assert!(e.contains(field), "{e}");
        }
    }

    #[test]
    fn content_round_trips_and_executable_files_are_refused() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("sunrise-mods-{}", std::process::id()));
        let dir = root.join("folder");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST), EXAMPLE).unwrap();
        let archive = root.join("ruby.smod");
        std::fs::write(&archive, Package::write_files(EXAMPLE, &[]).unwrap()).unwrap();
        assert_eq!(Package::read(&archive).unwrap().manifest.id, "ruby");
        assert_eq!(Package::read(&dir).unwrap().manifest.id, "ruby");
        let cases: &[(&str, &[u8])] = &[
            ("mod.exe", b""),
            ("./mod.exe", b"MZ"),
            ("MOD.EXE", b""),
            ("sub/mod.exe", b""),
            ("assets/ruby/CODE.ExE", b""),
            ("assets/ruby/CODE.WaSm", b""),
            ("assets/ruby/model.bin", b"\0asm"),
            ("assets/ruby/native.bin", b"\x7fELF"),
            ("assets/ruby/windows.bin", b"MZ"),
            ("assets/ruby/mac.bin", b"\xcf\xfa\xed\xfe"),
            ("assets/ruby/script", b"#!/bin/sh"),
            ("assets/ruby/program.EXE", b""),
        ];
        for (name, bytes) in cases {
            let file = dir.join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, bytes).unwrap();
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file(MANIFEST, options).unwrap();
            zip.write_all(EXAMPLE.as_bytes()).unwrap();
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
            zip.finish().unwrap();
            let error = Package::read(&archive).err().unwrap();
            assert!(
                error.contains("unsupported executable file"),
                "{name}: {error}"
            );
            if name.starts_with("assets/") {
                let error = Package::read(&dir).err().unwrap();
                assert!(
                    error.contains("unsupported executable file"),
                    "{name}: {error}"
                );
            } else {
                assert!(Package::read(&dir).is_ok(), "unpackaged {name}");
            }
            std::fs::remove_file(file).unwrap();
        }
        for (name, bytes) in cases.iter().filter(|(name, _)| name.starts_with("assets/")) {
            assert!(
                Package::write_files(EXAMPLE, &[(name, bytes)]).is_err(),
                "{name}"
            );
        }
        // Native bundle sources can start with Rust inner attributes, not a script shebang.
        assert!(
            Package::write_files(
                EXAMPLE,
                &[("assets/ruby/source.txt", b"#![allow(dead_code)]")]
            )
            .is_ok()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn project_folder_loads_without_packaging_git_hooks() {
        let root = std::env::temp_dir().join(format!("sunrise-mod-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(root.join(MANIFEST), EXAMPLE).unwrap();
        std::fs::create_dir_all(root.join(".git/hooks")).unwrap();
        std::fs::write(root.join(".git/hooks/pre-commit"), b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                root.join(".git/hooks/pre-commit"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        std::fs::create_dir_all(root.join("assets/ruby")).unwrap();
        std::fs::write(root.join("assets/ruby/icon.png"), b"image").unwrap();
        assert_eq!(Package::read(&root).unwrap().manifest.id, "ruby");
        std::fs::write(root.join("assets/ruby/icon.png"), b"MZ").unwrap();
        assert!(
            Package::read(&root)
                .err()
                .unwrap()
                .contains("unsupported executable")
        );
        std::fs::remove_file(root.join("assets/ruby/icon.png")).unwrap();
        std::fs::create_dir_all(root.join("assets/minecraft/lang")).unwrap();
        std::fs::write(root.join("assets/minecraft/lang/en_us.json"), "{}").unwrap();
        let package = Package::read(&root).unwrap();
        assert!(crate::Content::read(&root, &package.manifest).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn structure_nbt_is_data_but_not_an_executable_exception() {
        let name = "data/ruby/structure/hut.nbt";
        // An uncompressed, named NBT root containing an empty compound.
        let nbt = [10u8, 0, 0, 0];
        assert!(check_file(name, 0o100644, || Ok(std::io::Cursor::new(nbt))).is_ok());
        assert!(matches!(
            check_file(name, 0o100755, || Ok(std::io::Cursor::new(nbt))),
            Err(ContentFileError::Executable(_))
        ));
        for header in [b"\x7fELF", b"MZ!!", b"\xcf\xfa\xed\xfe"] {
            assert!(matches!(
                check_file(name, 0o100644, || Ok(std::io::Cursor::new(header))),
                Err(ContentFileError::Executable(_))
            ));
        }
    }

    #[test]
    fn archive_links_and_executable_modes_are_refused() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("sunrise-mods-kinds-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("ruby.smod");
        for link in [false, true] {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
            // Made by a Unix tool on every OS: the writer's default on Windows is DOS attributes,
            // which hold no mode to refuse.
            let options = zip::write::SimpleFileOptions::default().system(zip::System::Unix);
            zip.start_file(MANIFEST, options).unwrap();
            zip.write_all(EXAMPLE.as_bytes()).unwrap();
            if link {
                zip.add_symlink("assets/ruby/link", "missing", options)
                    .unwrap();
            } else {
                zip.start_file("assets/ruby/program", options.unix_permissions(0o755))
                    .unwrap();
                zip.write_all(b"ordinary bytes").unwrap();
            }
            zip.finish().unwrap();
            let error = Package::read(&path).err().unwrap();
            assert!(
                error.contains(if link {
                    "unsupported file type"
                } else {
                    "unsupported executable"
                }),
                "{error}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn folder_links_and_executable_modes_are_refused() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root =
            std::env::temp_dir().join(format!("sunrise-mods-folder-kinds-{}", std::process::id()));
        let dir = root.join("folder");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST), EXAMPLE).unwrap();
        std::fs::create_dir_all(dir.join("assets/ruby")).unwrap();
        let file = dir.join("assets/ruby/program");
        std::fs::write(&file, b"ordinary bytes").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = Package::read(&dir).err().unwrap();
        assert!(error.contains("unsupported executable"), "{error}");
        std::fs::remove_file(&file).unwrap();
        for name in ["assets/ruby/mod.exe", "assets/ruby/nested/link"] {
            let link = dir.join(name);
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            symlink("missing", &link).unwrap();
            let error = Package::read(&dir).err().unwrap();
            assert!(error.contains("unsupported file type"), "{error}");
            std::fs::remove_file(link).unwrap();
        }
        std::fs::remove_dir_all(dir.join("assets")).unwrap();
        symlink("missing", dir.join("assets")).unwrap();
        assert!(
            Package::read(&dir)
                .err()
                .unwrap()
                .contains("unsupported file type")
        );
        let link = root.join("linked-folder");
        symlink(&dir, &link).unwrap();
        let error = Package::read(&link).err().unwrap();
        assert!(error.contains("unsupported file type"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }
}
