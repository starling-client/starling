//! The registry repository (`horizon-rust/horizon-mods`, docs/modding/src/reference/registry-index.md):
//! one `mods/<id>.toml` per project, written by the submission form or by
//! hand and reviewed in a pull request, plus `revoked.toml`. Its CI turns
//! them into `index.json` with [`build`], reading every listed file, and
//! checks a pull request's changes with [`review`].
//!
//! What an entry can't say for itself (a mod's API, side, capabilities and dependencies,
//! a modpack's mods, a file's size) is read from the file, never typed.
pub use super::open;
use super::{
    File, Index, Kind, Nightly, NightlyBuild, NightlyMetadata, Project, Revoked, SCHEMA,
    SunriseRelease, Version, check_stable_tag, find_release, hex64, inspect, page as rules,
    stable_tag, valid_id,
};
use crate::manifest::{parse_version, revision};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

pub mod builds;
mod collections;
mod requirements;
pub use collections::{COLLECTIONS, CollectionEntry, review_collections};

/// The folder of entries in the registry repository.
pub const ENTRIES: &str = "mods";
/// The list of files taken down.
pub const REVOKED: &str = "revoked.toml";
/// What `cargo sunrise-mod publish` writes into a field it cannot fill from the manifest; an entry
/// that still holds it is refused ([`Entry::check_fields`]).
pub const TODO: &str = "TODO(publish)";
/// A project's submission or cache can contain its full release history.
const MAX_PROJECT_BYTES: u64 = 16 << 20;
/// A collection only holds a page and member ids, not a release history.
const MAX_COLLECTION_BYTES: u64 = 128 << 10;
const MAX_REVOKED_BYTES: u64 = 16 << 20;
/// The biggest icon, a PNG in `icons/`.
const MAX_ICON: u64 = crate::icon::MAX_ICON as u64;

/// `mods/<id>.toml`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub kind: Kind,
    /// The project's GitHub repository, `owner/name`. Its releases are its versions
    /// ([`cache`], `scripts/sync_releases.py` in the registry), and its
    /// [`PAGE_FILE`] may fill the page fields the entry leaves out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    // The page fields below may be left out when `repo` is set and the repository's page file
    // has them (the entry's own win, see [`Entry::resolved`]).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default)]
    pub authors: Vec<String>,
    /// GitHub logins allowed to publish updates.
    pub owners: Vec<String>,
    /// The owners' numeric GitHub IDs, which decide who they are: GitHub hands a renamed or deleted
    /// account's login to whoever claims it next, while an ID is never reused. The form records the
    /// lister's; the registry's backfill fills in older entries. When it is empty, an update is matched
    /// by login and always waits for a reviewer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub owner_ids: Vec<u64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub license: String,
    /// Defaults to the repository's address.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// `icons/<id>.png` in the registry repository, required before the project is listed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// 1 to 3 of the kind's categories ([`rules::categories`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    /// Free-form, lowercase, at most 10.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// At most 8, in order: `screenshots/<id>/<name>.png|.jpg` in the registry repository.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub screenshots: Vec<String>,
    /// The author's minisign public key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Source for the Nightly channel, separate from reviewed versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nightly: Option<NightlySource>,
    /// Hosted by the registry itself, newest first: the versions of before releases were read from
    /// the repository (and of Sunrise's own mods). Optional.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub versions: Vec<EntryVersion>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NightlySource {
    pub repo: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

pub(super) fn check_nightly_source(repo: &str, path: Option<&str>) -> Result<(), String> {
    if !valid_repo(repo) {
        return Err(format!(
            "nightly.repo {repo:?} must be owner/name on GitHub"
        ));
    }
    if let Some(path) = path
        && (path.contains(['\\', ':'])
            || path.chars().any(char::is_control)
            || path.split('/').any(|part| matches!(part, "" | "." | "..")))
    {
        return Err(format!(
            "nightly.path {path:?} must be a relative directory in the repository"
        ));
    }
    Ok(())
}

/// The page fields a project's repository may hold, in its [`PAGE_FILE`]. Only these: who owns the
/// project, its key and its kind are decided when it is listed.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Page {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
}

/// What the registry keeps of a repository between runs (`versions/<id>.json`, on the registry's
/// `versions` branch): the page last read, and every release version that passed its checks, as
/// the index lists it. Written by `cargo sunrise-mod registry release|page`, read by [`build`].
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Cache {
    #[serde(default)]
    pub page: Page,
    /// Newest first.
    #[serde(default)]
    pub versions: Vec<Version>,
}

/// The folder of [`Cache`] files, in the registry checkout.
pub const CACHE: &str = "versions";
/// The page file at the root of a project's repository (TOML, [`Page`]'s fields).
pub const PAGE_FILE: &str = "sunrise-registry.toml";
/// The longest changelog the index keeps of a release's notes.
const MAX_CHANGELOG: usize = 6000;

impl Cache {
    /// `<registry>/versions/<id>.json`; empty when there is none.
    pub fn read(registry: &Path, id: &str) -> Result<Cache, String> {
        if !valid_id(id) {
            return Err(format!("{id:?}: invalid project id for a release cache"));
        }
        let path = Path::new(CACHE).join(format!("{id}.json"));
        match read_regular(registry, &path, MAX_PROJECT_BYTES) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Cache::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }
}

/// `owner/name` of a GitHub repository.
pub fn valid_repo(repo: &str) -> bool {
    let part = |p: &str, max: usize| {
        !p.is_empty()
            && p.len() <= max
            && p != "."
            && p != ".."
            && p.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    repo.split_once('/')
        .is_some_and(|(owner, name)| part(owner, 39) && part(name, 100))
}

impl Entry {
    fn source_url(&self) -> Cow<'_, str> {
        if self.source.is_empty()
            && let Some(repo) = &self.repo
        {
            return Cow::Owned(format!("https://github.com/{repo}"));
        }
        Cow::Borrowed(&self.source)
    }

    /// The file's name in the registry mirror. An explicit stable source must come from a
    /// canonical package asset on this entry's own GitHub release.
    pub fn source_file_name<'a>(&self, version: &'a EntryVersion) -> Result<&'a str, String> {
        if version.sunrise.is_none() {
            return Ok(&version.file);
        }
        self.stable_source_asset(version).map(|(_, _, asset)| asset)
    }

    /// The repository, tag and asset of a canonical stable source download on this project's release.
    pub fn stable_source_asset<'a>(
        &self,
        version: &'a EntryVersion,
    ) -> Result<(&'a str, &'a str, &'a str), String> {
        if version.sunrise.is_none() {
            return Err("a reviewed stable source row is required".into());
        }
        if !plain_version(&version.version) || semver::Version::parse(&version.version).is_err() {
            return Err(
                "a stable source must use the full SemVer version from its manifest".into(),
            );
        }
        let extension = self
            .kind
            .extension()
            .ok_or_else(|| format!("a {} has no stable source file", self.kind.name()))?;
        let name = format!("{}-{}.{extension}", self.id, version.version);
        let invalid = || {
            format!("file for a stable source must be a GitHub release asset URL ending in /{name}")
        };
        let (repo, tag, asset) = github_release_asset(&version.file).ok_or_else(invalid)?;
        if asset != name
            || !super::plain_name(asset)
            || !super::plain_name(tag)
            || !valid_repo(repo)
        {
            return Err(invalid());
        }
        let source_repo = github_repo(&self.source_url()).ok_or(
            "a stable source needs the project's GitHub repository in `source`, or in `repo` when `source` is omitted",
        )?;
        if !source_repo.eq_ignore_ascii_case(repo) {
            return Err(format!(
                "file must be hosted in the project's repository {source_repo}"
            ));
        }
        Ok((repo, tag, asset))
    }

    /// The entry with the repository's page filling what it leaves out.
    pub fn resolved(&self, page: &Page) -> Entry {
        let mut e = self.clone();
        let fill = |own: &mut String, other: &Option<String>| {
            if own.trim().is_empty() {
                *own = other.clone().unwrap_or_default().trim().to_owned();
            }
        };
        fill(&mut e.name, &page.name);
        fill(&mut e.summary, &page.summary);
        fill(&mut e.description, &page.description);
        fill(&mut e.license, &page.license);
        if e.authors.is_empty() {
            e.authors = page.authors.clone();
        }
        if let Cow::Owned(source) = self.source_url() {
            e.source = source;
        }
        e
    }

    /// Everything about the project but its versions, once the page is filled in.
    pub fn check_fields(&self) -> Vec<String> {
        self.check_page(self.nightly.is_some() && self.versions.is_empty() && self.repo.is_none())
    }

    /// Who the project is: checked for every entry, also one that publishes nothing yet and only
    /// holds its id.
    fn check_identity(&self) -> Vec<String> {
        let id = &self.id;
        let mut errors = Vec::new();
        if !valid_id(id) {
            errors.push(format!("{id:?} is not [a-z0-9_]{{1,64}}"));
        }
        if crate::manifest::RESERVED_IDS.contains(&id.as_str()) {
            errors.push(format!("{id:?} is reserved: pick another id"));
        }
        if let Err(e) = open(&self.kind) {
            errors.push(format!("{id}: {e}"));
        }
        if self.owners.is_empty() {
            errors.push(format!("{id}: needs at least one owner (a GitHub login)"));
        }
        for owner in &self.owners {
            if owner.trim().is_empty() {
                errors.push(format!("{id}: an owner is blank: give a GitHub login"));
            } else if owner.trim_start().starts_with(TODO) {
                errors.push(format!(
                    "{id}: the owner is still the {TODO} that `cargo sunrise-mod publish` wrote: replace it with your own text in mods/{id}.toml"
                ));
            }
        }
        if let Some(error) = owner_ids_error(&self.owners, &self.owner_ids) {
            errors.push(format!("{id}: {error}"));
        }
        errors
    }

    fn check_page(&self, nightly_only: bool) -> Vec<String> {
        let id = &self.id;
        let mut errors = self.check_identity();
        if self.repo.as_deref().is_some_and(|r| !valid_repo(r)) {
            errors.push(format!("{id}: repo must be owner/name on GitHub"));
        }
        if let Some(nightly) = &self.nightly
            && let Err(error) = check_nightly_source(&nightly.repo, nightly.path.as_deref())
        {
            errors.push(format!("{id}: {error}"));
        }
        for (field, value) in [("license", &self.license), ("source", &self.source)] {
            if !nightly_only && value.trim().is_empty() {
                errors.push(format!(
                    "{id}: has no {field} (in the entry, or in {PAGE_FILE} of its repo)"
                ));
            }
        }
        let unfinished = [
            ("name", self.name.as_str()),
            ("summary", &self.summary),
            ("description", &self.description),
            ("license", &self.license),
            ("source", &self.source),
        ]
        .into_iter()
        .chain(self.authors.iter().map(|a| ("author", a.as_str())))
        .chain(self.categories.iter().map(|c| ("category", c.as_str())))
        .chain(self.tags.iter().map(|t| ("tag", t.as_str())));
        for (field, value) in unfinished {
            if value.trim_start().starts_with(TODO) {
                errors.push(format!(
                    "{id}: the {field} is still the {TODO} that `cargo sunrise-mod publish` wrote: replace it with your own text in mods/{id}.toml"
                ));
            }
        }
        errors.extend(
            rules::check_title(&self.name, &self.summary)
                .into_iter()
                .map(|e| format!("{id}: {e}")),
        );
        for e in rules::check(&self.kind, &self.description, &self.categories, &self.tags) {
            errors.push(format!("{id}: {e}"));
        }
        if self.screenshots.len() > rules::MAX_SCREENSHOTS {
            errors.push(format!(
                "{id}: {} screenshots, at most {}",
                self.screenshots.len(),
                rules::MAX_SCREENSHOTS
            ));
        }
        if nightly_only {
            // Images are optional on a Nightly-only page, but must be local when supplied.
            errors.extend(
                self.icon
                    .as_deref()
                    .and_then(|icon| rules::check_icon_path(id, Some(icon)))
                    .into_iter()
                    .chain(rules::check_screenshot_paths(id, &self.screenshots))
                    .map(|problem| format!("{id}: {problem}")),
            );
        }
        errors
    }
}

fn owner_ids_error(owners: &[String], ids: &[u64]) -> Option<String> {
    // Once IDs are present, names and IDs must describe the same accounts.
    (!ids.is_empty() && ids.len() != owners.len()).then(|| format!(
        "owners lists {} login(s) but owner_ids {} ID(s): give each owner's numeric GitHub ID in owner_ids, in the same order (`gh api users/<login> --jq .id`)",
        owners.len(), ids.len()
    ))
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EntryVersion {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub builds: Vec<builds::BuildRow>,
    pub version: String,
    /// `YYYY-MM-DD`.
    pub published: String,
    /// The Minecraft version it is for.
    pub game: String,
    /// The stable Sunrise release tag it was made for. Absent only on legacy rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sunrise: Option<String>,
    /// A bare asset name, or the author's GitHub release URL when `sunrise` is set.
    pub file: String,
    /// Lowercase hex: pins the reviewed file.
    pub sha256: String,
    /// The `.minisig` text, when the project has a key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub changelog: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokedFile {
    #[serde(default)]
    revoked: Vec<Revoked>,
}

/// Where [`build`] reads and what it writes into the index.
pub struct Sources<'a> {
    /// A checkout of the registry repository.
    pub registry: &'a Path,
    /// Every listed file, named by its SHA-256 (CI downloads them there).
    pub files: &'a Path,
    /// Where the files are published: `<files>/<id>-<version>/<file>`
    /// ([`super::FILE_HOST`]; `file:///` for a local test registry).
    pub release: &'a str,
    /// Where the registry's site is (its icons): `https://.../sunrise-mods/`.
    pub site: &'a str,
    /// RFC 3339.
    pub generated: String,
    /// The stable releases CI supplied. `None` means no releases file was given.
    pub releases: Option<&'a [SunriseRelease]>,
    /// Nightly branch heads CI supplied, keyed by project ID.
    pub nightly: Option<&'a BTreeMap<String, NightlyMetadata>>,
    /// Required when any written version has prebuilt assets.
    pub binary_checks: Option<builds::Checks<'a>>,
}

/// The wire row that discovery writes and the offline registry commands read.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseInput {
    pub tag: String,
    pub commit: String,
}

/// Read the workflow's release list, adding the line from each validated tag.
pub fn parse_releases(text: &str) -> Result<Vec<SunriseRelease>, String> {
    let inputs: Vec<ReleaseInput> = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let mut seen = std::collections::HashSet::new();
    let mut releases = Vec::with_capacity(inputs.len());
    for ReleaseInput { tag, commit } in &inputs {
        let (game, major) = stable_tag(tag)
            .ok_or_else(|| format!("{tag:?}: expected a stable release tag like 26.3-0.1.0"))?;
        let commit = commit.to_ascii_lowercase();
        if !revision(&commit) {
            return Err(format!("{tag}: commit {commit:?} must be 40 hex digits"));
        }
        if !seen.insert(tag) {
            return Err(format!("{tag}: release listed twice"));
        }
        releases.push(SunriseRelease {
            tag: tag.clone(),
            commit,
            line: format!("{game}-{major}"),
        });
    }
    Ok(releases)
}

/// Read the workflow's current game Nightly before using its commit as a directory name.
pub fn parse_nightly_build(text: &str) -> Result<NightlyBuild, String> {
    let mut build: NightlyBuild = serde_json::from_str(text).map_err(|e| e.to_string())?;
    build.commit.make_ascii_lowercase();
    if !revision(&build.commit) {
        return Err(format!(
            "Nightly commit {:?} must be 40 hex digits",
            build.commit
        ));
    }
    if !plain_date(&build.date) {
        return Err(format!("Nightly date {:?} is not YYYY-MM-DD", build.date));
    }
    Ok(build)
}

/// The release tag a version's file is published under.
pub fn tag(id: &str, version: &str) -> String {
    format!("{id}-{version}")
}

/// The registry checkout's declarations, before reading package files.
#[derive(Debug)]
pub struct Checkout {
    directory: PathBuf,
    pub entries: Vec<Entry>,
    pub collections: Vec<CollectionEntry>,
    pub revoked: Vec<Revoked>,
    caches: BTreeMap<String, Cache>,
}

impl Checkout {
    /// A project with neither reviewed versions nor Nightly source is not published yet.
    fn listed(&self) -> impl Iterator<Item = (&Entry, Option<&Cache>)> {
        self.entries.iter().filter_map(|entry| {
            let cache = self.caches.get(&entry.id);
            (entry.nightly.is_some() || has_versions(entry, cache)).then_some((entry, cache))
        })
    }

    /// The identity errors of the entries that publish nothing yet: [`Checkout::listed`] skips
    /// them, so their page is never checked.
    fn check_unlisted(&self) -> Vec<String> {
        let unlisted = |entry: &&Entry| {
            entry.nightly.is_none() && !has_versions(entry, self.caches.get(&entry.id))
        };
        self.entries
            .iter()
            .filter(unlisted)
            .flat_map(Entry::check_identity)
            .collect()
    }

    /// Validate release declarations and Nightly entries for a review without package files.
    pub fn check_declarations(&self, releases: Option<&[SunriseRelease]>) -> Vec<String> {
        let mut errors = self.check_unlisted();
        let empty = Cache::default();
        for (entry, cache) in self.listed() {
            let cache = cache.unwrap_or(&empty);
            if entry.nightly.is_some() {
                errors.extend(
                    entry
                        .resolved(&cache.page)
                        .check_page(!has_versions(entry, Some(cache))),
                );
            }
            for v in &entry.versions {
                if let Err(error) = v.release(releases).and_then(|_| entry.source_file_name(v)) {
                    errors.push(format!("{} {}: {error}", entry.id, v.version));
                }
            }
        }
        errors
    }

    /// The required `dependencies` (a package's facts) that no listed, not taken down mod version
    /// for `game` satisfies, so `cargo sunrise-mod publish` can name them before the pull request.
    /// Registry review, which reads every package, also checks sides and the dependencies' own
    /// requirements.
    pub fn unmet_dependencies(
        &self,
        game: &str,
        dependencies: &[super::Dependency],
    ) -> Vec<UnmetDependency> {
        dependencies
            .iter()
            .filter(|dependency| !dependency.optional)
            .filter_map(|dependency| {
                let id = &dependency.id;
                let listed: Vec<String> = self
                    .listed()
                    .filter(|(entry, _)| &entry.id == id && entry.kind.is_mod())
                    .flat_map(|(entry, cache)| {
                        let written = entry
                            .versions
                            .iter()
                            .map(|v| (&v.version, &v.game, &v.sha256));
                        let released = cache
                            .into_iter()
                            .flat_map(|c| &c.versions)
                            .map(|v| (&v.version, &v.game, &v.file.sha256));
                        written.chain(released)
                    })
                    .filter(|(_, g, sha256)| {
                        *g == game && !self.revoked.iter().any(|r| r.covers(id, sha256))
                    })
                    .map(|(version, _, _)| version.clone())
                    .collect();
                let met = semver::VersionReq::parse(&dependency.version).is_ok_and(|range| {
                    listed
                        .iter()
                        .any(|v| parse_version(v).is_ok_and(|v| range.matches(&v)))
                });
                (!met).then(|| UnmetDependency {
                    id: id.clone(),
                    range: dependency.version.clone(),
                    listed,
                })
            })
            .collect()
    }
}

/// A required dependency that no listed version satisfies ([`Checkout::unmet_dependencies`]).
#[derive(Debug, PartialEq)]
pub struct UnmetDependency {
    pub id: String,
    /// The range as the package declares it.
    pub range: String,
    /// The versions the registry lists for the same game, as written; empty when none is.
    pub listed: Vec<String>,
}

/// Reads `mods/*.toml`, `collections/*.toml`, `revoked.toml` and the repo-backed projects'
/// release caches, reporting all input errors.
pub fn load(registry: &Path) -> Result<Checkout, Vec<String>> {
    let mut errors = Vec::new();
    let entries = load_entries(
        registry,
        ENTRIES,
        false,
        MAX_PROJECT_BYTES,
        |e: &Entry| &e.id,
        &mut errors,
    );
    let collections = load_entries(
        registry,
        COLLECTIONS,
        true,
        MAX_COLLECTION_BYTES,
        |c: &CollectionEntry| &c.id,
        &mut errors,
    );
    let revoked = match read_regular(registry, Path::new(REVOKED), MAX_REVOKED_BYTES).and_then(utf8)
    {
        Ok(text) => match toml::from_str::<RevokedFile>(&text) {
            Ok(file) => file.revoked,
            Err(e) => {
                errors.push(format!("{REVOKED}: {e}"));
                Vec::new()
            }
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            errors.push(format!("{REVOKED}: {e}"));
            Vec::new()
        }
    };
    let mut caches = BTreeMap::new();
    for entry in entries.iter().filter(|e| e.repo.is_some()) {
        match Cache::read(registry, &entry.id) {
            Ok(cache) => {
                caches.insert(entry.id.clone(), cache);
            }
            Err(error) => errors.push(error),
        }
    }
    if errors.is_empty() {
        Ok(Checkout {
            directory: registry.to_owned(),
            entries,
            collections,
            revoked,
            caches,
        })
    } else {
        Err(errors)
    }
}

fn load_entries<T: DeserializeOwned>(
    registry: &Path,
    folder: &str,
    optional: bool,
    max_bytes: u64,
    id: impl Fn(&T) -> &str,
    errors: &mut Vec<String>,
) -> Vec<T> {
    let dir = registry.join(folder);
    let read = checkout_path(registry, Path::new(folder), true).and_then(std::fs::read_dir);
    let read = match read {
        Ok(read) => read,
        Err(error) if optional && error.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            errors.push(format!("{}: {error}", dir.display()));
            return Vec::new();
        }
    };
    let mut paths = Vec::new();
    for row in read {
        match row {
            Ok(row) => paths.push(row.file_name()),
            Err(error) => errors.push(format!("{}: {error}", dir.display())),
        }
    }
    paths.sort();
    let mut entries = Vec::new();
    for path in paths {
        let name = path.to_str().unwrap_or("");
        let Some(stem) = name.strip_suffix(".toml") else {
            errors.push(format!("{folder}/{name}: only <id>.toml files go here"));
            continue;
        };
        let parsed = read_regular(registry, &Path::new(folder).join(&path), max_bytes)
            .and_then(utf8)
            .map_err(|e| e.to_string())
            .and_then(|text| toml::from_str::<T>(&text).map_err(|e| e.to_string()));
        match parsed {
            Ok(entry) if id(&entry) == stem => entries.push(entry),
            Ok(entry) => errors.push(format!(
                "{folder}/{name}: its id is {:?}; the file must be {folder}/{}.toml",
                id(&entry),
                id(&entry)
            )),
            Err(e) => errors.push(format!("{folder}/{name}: {e}")),
        }
    }
    entries
}

/// Only descend through real directories under the trusted checkout root. The same policy
/// covers entry directories, caches, and images at any depth.
fn checkout_path(root: &Path, relative: &Path, directory: bool) -> io::Result<PathBuf> {
    let invalid_path = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "expected a relative checkout path",
        )
    };
    let mut path = root.to_owned();
    let mut components = relative.components().peekable();
    if components.peek().is_none() {
        return Err(invalid_path());
    }
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err(invalid_path());
        };
        // Push only a single relative name, not a prefix reinterpreted by the host filesystem.
        if !matches!(
            Path::new(name).components().next(),
            Some(Component::Normal(_))
        ) {
            return Err(invalid_path());
        }
        path.push(name);
        let meta = std::fs::symlink_metadata(&path)?;
        let want_dir = directory || components.peek().is_some();
        if (want_dir && !meta.is_dir()) || (!want_dir && !meta.is_file()) {
            let expected = if want_dir { "directory" } else { "file" };
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} is not a regular {expected}", path.display()),
            ));
        }
    }
    Ok(path)
}

fn read_regular(root: &Path, relative: &Path, max: u64) -> io::Result<Vec<u8>> {
    let path = checkout_path(root, relative, false)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("exceeds {max} byte limit"),
        ));
    }
    Ok(bytes)
}

fn utf8(bytes: Vec<u8>) -> io::Result<String> {
    String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// A version string that is also a plain release tag and file name part.
pub fn plain_version(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 64
        && v.as_bytes()[0].is_ascii_alphanumeric()
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
}

fn plain_date(d: &str) -> bool {
    let b = d.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// One project's entry and its files, as the index lists them.
pub fn project(entry: &Entry, src: &Sources) -> Result<Project, Vec<String>> {
    let cache = match &entry.repo {
        Some(_) => Cache::read(src.registry, &entry.id).map_err(|e| vec![e])?,
        None => Cache::default(),
    };
    project_with_cache(entry, &cache, src, &[])
}

fn has_versions(entry: &Entry, cache: Option<&Cache>) -> bool {
    !entry.versions.is_empty() || cache.is_some_and(|c| !c.versions.is_empty())
}

fn project_with_cache(
    entry: &Entry,
    cache: &Cache,
    src: &Sources,
    revoked: &[Revoked],
) -> Result<Project, Vec<String>> {
    let entry = &entry.resolved(&cache.page);
    let no_versions = !has_versions(entry, Some(cache));
    let nightly_only = no_versions && entry.nightly.is_some();
    let mut errors = entry.check_page(nightly_only);
    let id = &entry.id;
    if !valid_id(id) || crate::manifest::RESERVED_IDS.contains(&id.as_str()) {
        return Err(errors);
    }
    let icon = entry
        .icon
        .as_deref()
        .and_then(|icon| image(src, id, icon, "icons/", &mut errors));
    let screenshots: Vec<String> = entry
        .screenshots
        .iter()
        .filter_map(|s| image(src, id, s, &format!("screenshots/{id}/"), &mut errors))
        .collect();
    if no_versions && entry.nightly.is_none() {
        errors.push(format!("{id}: has no version"));
    }
    let mut versions = Vec::new();
    for v in &entry.versions {
        match checked_version(entry, v, src, revoked) {
            Ok(v) => versions.push(v),
            Err(e) => errors.push(format!("{id} {}: {e}", v.version)),
        }
    }
    // The versions read from the repository's releases passed their checks when they were read.
    // Newest first, the registry's own before them on a tie.
    versions.extend(cache.versions.iter().cloned());
    versions.sort_by(|a: &Version, b| b.published.cmp(&a.published));
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Project {
        id: id.clone(),
        kind: entry.kind.clone(),
        name: entry.name.clone(),
        summary: entry.summary.clone(),
        description: entry.description.clone(),
        authors: entry.authors.clone(),
        owners: entry.owners.clone(),
        license: entry.license.clone(),
        source: match &entry.nightly {
            Some(n) if entry.source.is_empty() => format!("https://github.com/{}", n.repo),
            _ => entry.source.clone(),
        },
        icon,
        categories: entry.categories.clone(),
        tags: entry.tags.clone(),
        screenshots,
        key: entry.key.as_ref().map(|k| k.trim().to_owned()),
        nightly: entry.nightly.as_ref().map(|n| Nightly {
            repo: n.repo.clone(),
            path: n.path.clone(),
            metadata: src
                .nightly
                .and_then(|m| m.get(id))
                .cloned()
                .unwrap_or_default(),
        }),
        versions,
        downloads: 0,
        stars: 0,
    })
}

/// An icon (`folder` = `icons/`) or a screenshot (`screenshots/<id>/`) as the index lists it: an
/// `https://` address as it is, or a file in the registry repository checked and served from the
/// registry's site. An icon's address ends in `?v=` and the start of its SHA-256, so a new icon
/// is a new address: the launcher keeps images by address for a week, and would go on showing
/// the old one.
fn image(
    src: &Sources,
    id: &str,
    path: &str,
    folder: &str,
    errors: &mut Vec<String>,
) -> Option<String> {
    if path.starts_with("https://") {
        return Some(path.to_owned());
    }
    let icon = folder == "icons/";
    // The page's own check refuses a path outside the project's folder; there is nothing to read.
    let name = path.strip_prefix(folder).filter(|n| super::plain_name(n))?;
    let max = if icon {
        MAX_ICON
    } else {
        rules::MAX_SCREENSHOT
    };
    let checked = read_regular(src.registry, Path::new(path), max)
        .map_err(|e| format!("{path}: {e}"))
        .and_then(|bytes| {
            if icon {
                rules::check_icon(&bytes).map_err(|e| format!("{path}: {e}"))?;
                Ok(format!("?v={}", &super::sha256(&bytes)[..12]))
            } else {
                rules::check_screenshot(name, &bytes).map(|()| String::new())
            }
        });
    match checked {
        Ok(version) => Some(format!("{}{path}{version}", src.site)),
        Err(e) => {
            errors.push(format!("{id}: {e}"));
            None
        }
    }
}

impl EntryVersion {
    /// The stable release this row names. A row that names one is never accepted unchecked: the
    /// registry needs the releases to compare its pin with. A row that names none has no release.
    fn release<'a>(
        &self,
        releases: Option<&'a [SunriseRelease]>,
    ) -> Result<Option<&'a SunriseRelease>, String> {
        let Some(tag) = &self.sunrise else {
            return Ok(None);
        };
        check_stable_tag(tag, &self.game)?;
        if releases.is_none() {
            return Err(format!(
                "sunrise {tag:?} cannot be checked without the Sunrise releases: supply them"
            ));
        }
        find_release(releases, Some(tag))
    }
}

/// Verify a written version without requiring a complete project page.
pub fn checked_version(
    entry: &Entry,
    v: &EntryVersion,
    src: &Sources,
    revoked: &[Revoked],
) -> Result<Version, String> {
    let release = v.release(src.releases)?;
    let name = entry.source_file_name(v)?;
    let url = format!("{}{}/{name}", src.release, tag(&entry.id, &v.version));
    version_at(
        entry,
        v,
        &src.files.join(&v.sha256),
        (name, url),
        release,
        Some(src),
        revoked,
    )
}

/// A version of `entry` whose file is at `path` and is downloaded from `url`.
fn version_at(
    entry: &Entry,
    v: &EntryVersion,
    path: &Path,
    (name, url): (&str, String),
    release: Option<&SunriseRelease>,
    sources: Option<&Sources<'_>>,
    revoked: &[Revoked],
) -> Result<Version, String> {
    if !plain_version(&v.version) {
        return Err("a version is letters, digits, '.', '-' and '_'".into());
    }
    if !plain_date(&v.published) {
        return Err(format!("published {:?} is not YYYY-MM-DD", v.published));
    }
    hex64(&v.sha256)?;
    let metadata =
        std::fs::symlink_metadata(path).map_err(|e| format!("its file {}: {e}", v.sha256))?;
    let size = metadata.len();
    if !metadata.is_file() || size == 0 || size > super::MAX_FILE {
        return Err(format!(
            "its file must be a regular file of 1 byte to {} MiB: supply the package named by its SHA-256",
            super::MAX_FILE >> 20
        ));
    }
    if super::sha256_file(path)? != v.sha256 {
        return Err("the file isn't the one its sha256 names".into());
    }
    match (&entry.key, &v.signature) {
        (Some(key), Some(sig)) => super::verify_signature(key, sig, path)?,
        (Some(_), None) => return Err("the project has a key: sign the file with it".into()),
        (None, Some(_)) => return Err("signed, but the project has no key".into()),
        (None, None) if v.sunrise.is_some() => {
            return Err(
                "a stable source version needs a signed package and the project's key".into(),
            );
        }
        (None, None) => {}
    }
    let source = if v.builds.is_empty() {
        None
    } else {
        if entry.kind != Kind::NativeMod {
            return Err("only a native mod may list builds".into());
        }
        let source = super::native::Bundle::read(path)?;
        crate::Content::read(path, &source.manifest)?;
        Some(source)
    };
    let facts = match &source {
        Some(source) => source.facts(),
        None => inspect(&entry.kind, path)?,
    };
    if facts.id.as_ref().is_some_and(|id| *id != entry.id) {
        return Err(format!(
            "the file's id is {:?}, not {:?}",
            facts.id.unwrap_or_default(),
            entry.id
        ));
    }
    if facts
        .version
        .as_ref()
        .is_some_and(|x| !super::same_version(&entry.kind, x, &v.version))
    {
        return Err(format!(
            "the file says version {:?}",
            facts.version.unwrap_or_default()
        ));
    }
    if facts.game.as_ref().is_some_and(|g| *g != v.game) {
        return Err(format!(
            "the file is for {:?}, the entry says {:?}",
            facts.game.unwrap_or_default(),
            v.game
        ));
    }
    if let Some(release) = release {
        facts.matches_release(release)?;
    }
    let native_binaries = match &source {
        Some(source) => {
            let src = sources.ok_or("no registry inputs for build assets")?;
            builds::verify(entry, v, source, release, src, revoked)?
        }
        None => Vec::new(),
    };
    Ok(Version {
        sunrise: v.sunrise.clone(),
        verdicts: BTreeMap::new(),
        native_binaries,
        native_binary_refusals: Vec::new(),
        version: v.version.clone(),
        published: v.published.clone(),
        changelog: v.changelog.clone(),
        api: facts.api,
        side: facts.side,
        capabilities: facts.capabilities.into(),
        game: v.game.clone(),
        dependencies: (!facts.dependencies.is_empty()).then_some(facts.dependencies),
        requires: facts.requires,
        file: File {
            url,
            name: name.to_owned(),
            size,
            sha256: v.sha256.clone(),
            signature: v.signature.clone(),
        },
    })
}

/// A release of a project's repository whose asset was downloaded.
pub struct Release<'a> {
    /// The release's tag: the version when the file does not say one.
    pub tag: &'a str,
    /// `YYYY-MM-DD`, the release's date.
    pub published: &'a str,
    /// The release notes.
    pub notes: &'a str,
    /// The asset's name.
    pub asset: &'a str,
    /// The Minecraft version, when the file names none.
    pub game: &'a str,
    /// The asset's `.minisig` text.
    pub signature: Option<&'a str>,
}

/// The version a release of `entry`'s repository is, once its asset at `path` passed the kind's
/// checks: the file's own manifest decides id, version and game (the tag and the default game fill
/// what it leaves out), the index lists the asset's address in the repository, and the SHA-256 is
/// computed here, never read from the release.
pub fn release_version(entry: &Entry, r: &Release, path: &Path) -> Result<Version, String> {
    let repo = entry
        .repo
        .as_deref()
        .filter(|r| valid_repo(r))
        .ok_or("the entry names no GitHub repository")?;
    let plain = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'+'))
    };
    if !plain(r.tag) || !super::plain_name(r.asset) {
        return Err(format!(
            "tag {:?} or file {:?} has characters the launcher cannot download: use letters, digits, '.', '-', '_' and '+'",
            r.tag, r.asset
        ));
    }
    let want = entry.kind.extension();
    if want.is_some_and(|e| !r.asset.ends_with(&format!(".{e}"))) {
        return Err(format!(
            "{:?} is not a .{} file",
            r.asset,
            want.unwrap_or("")
        ));
    }
    let facts = inspect(&entry.kind, path)?;
    let version = facts
        .version
        .clone()
        .unwrap_or_else(|| r.tag.trim_start_matches(['v', 'V']).to_owned());
    let mut notes: String = r.notes.trim().chars().take(MAX_CHANGELOG).collect();
    if r.notes.trim().chars().count() > MAX_CHANGELOG {
        notes.push_str("\n\n(Cut: the full notes are on the release.)");
    }
    let v = EntryVersion {
        sunrise: None,
        builds: Vec::new(),
        game: facts.game.clone().unwrap_or_else(|| r.game.to_owned()),
        file: r.asset.to_owned(),
        sha256: super::sha256_file(path)?,
        signature: r
            .signature
            .map(str::to_owned)
            .filter(|s| !s.trim().is_empty()),
        changelog: notes,
        published: r.published.to_owned(),
        version,
    };
    let url = format!(
        "https://github.com/{repo}/releases/download/{}/{}",
        r.tag, r.asset
    );
    version_at(entry, &v, path, (r.asset, url), None, None, &[])
}

/// A project's page file as it would fill `entry` (which names the repository): refused when it
/// has keys other than [`Page`]'s, or leaves the page incomplete.
pub fn page(entry: &Entry, text: &str) -> Result<Page, String> {
    let page: Page = toml::from_str(text).map_err(|e| format!("{PAGE_FILE}: {e}"))?;
    let errors = entry.resolved(&page).check_fields();
    if errors.is_empty() {
        Ok(page)
    } else {
        Err(errors.join("; "))
    }
}

impl Entry {
    /// The entry as `mods/<id>.toml` holds it (the submission form writes it).
    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string(self).map_err(|e| e.to_string())
    }
}

/// The index of the whole registry: every entry and file checked, then the
/// index's own rules ([`Index::check`]: names, a modpack's mods present).
pub fn build(src: &Sources) -> Result<Index, Vec<String>> {
    let checkout = load(src.registry)?;
    let mut errors = Vec::new();
    let empty = Cache::default();
    for (entry, cache) in checkout.listed() {
        let cache = cache.unwrap_or(&empty);
        if !has_versions(entry, Some(cache)) {
            continue;
        }
        // A listed project's page is complete (registry v2's rollout, step 4).
        let e = entry.resolved(&cache.page);
        errors.extend(
            rules::check_required(
                &e.id,
                &e.description,
                &e.categories,
                e.icon.as_deref(),
                &e.screenshots,
            )
            .into_iter()
            .map(|problem| format!("{}: {problem}", e.id)),
        );
    }
    let index = match candidate(&checkout, src) {
        Ok(index) if errors.is_empty() => index,
        Ok(_) => return Err(errors),
        Err(e) => {
            errors.extend(e);
            return Err(errors);
        }
    };
    let local = src.release.starts_with("file:///");
    index.check_with(local).map_err(|e| vec![e])?;
    Ok(index)
}

/// Assemble the entries from their package files, before checking links between projects.
/// Review can still report the impact of a takedown on a candidate that fails [`Index::check`].
pub fn candidate(checkout: &Checkout, src: &Sources) -> Result<Index, Vec<String>> {
    let mut projects = Vec::new();
    let (collections, mut errors) = checkout.collection_pages(src);
    errors.extend(checkout.check_unlisted());
    let empty = Cache::default();
    for (entry, cache) in checkout.listed() {
        match project_with_cache(entry, cache.unwrap_or(&empty), src, &checkout.revoked) {
            Ok(mut p) => {
                // A takedown of the whole project covers its unreviewed source as well as its
                // files; one that names a hash covers that file alone.
                let taken_down = |r: &Revoked| r.id == p.id && r.sha256.is_none();
                if checkout.revoked.iter().any(taken_down) {
                    p.nightly = None;
                }
                if p.nightly.is_some() || !p.versions.is_empty() {
                    projects.push(p);
                }
            }
            Err(e) => errors.extend(e),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    projects.sort_by(|a, b| a.id.cmp(&b.id));
    let mut index = Index {
        schema: SCHEMA,
        generated: src.generated.clone(),
        projects,
        releases: src.releases.map(<[_]>::to_vec),
        nightly_build: None,
        unjudged: Vec::new(),
        collections,
        revoked: checkout.revoked.clone(),
        left_out: Vec::new(),
    };
    index.prune_native_binaries();
    Ok(index)
}

/// What GitHub counts for the registry, gathered by its publish workflow (this crate makes no
/// network calls): each release asset's `download_count` by its download address, and each
/// source repository's `stargazers_count` by `owner/repo`.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Counts {
    #[serde(default)]
    pub downloads: BTreeMap<String, u64>,
    #[serde(default)]
    pub stars: BTreeMap<String, u64>,
}

/// Sets each project's `downloads`, its files' counts summed (every version's file and native
/// binaries), and its `stars`, its GitHub `source` repository's. A file or a repository the
/// counts don't name counts 0.
pub fn count(index: &mut Index, counts: &Counts) {
    for project in &mut index.projects {
        project.downloads = project
            .versions
            .iter()
            .flat_map(|v| std::iter::once(&v.file).chain(v.native_binaries.iter().map(|b| &b.file)))
            .filter_map(|file| counts.downloads.get(&file.url))
            .sum();
        project.stars = github_repo(&project.source)
            .and_then(|repo| {
                counts
                    .stars
                    .iter()
                    .find_map(|(name, stars)| name.eq_ignore_ascii_case(&repo).then_some(*stars))
            })
            .unwrap_or(0);
    }
}

/// `owner/repo` of a `https://github.com/<owner>/<repo>[/...]` address (GitHub compares them
/// without case).
fn github_repo(source: &str) -> Option<String> {
    let mut parts = source.strip_prefix("https://github.com/")?.split('/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    (!owner.is_empty() && !repo.is_empty()).then(|| format!("{owner}/{repo}"))
}

/// The repository, tag and asset of a GitHub release download URL.
fn github_release_asset(url: &str) -> Option<(&str, &str, &str)> {
    let (repo, rest) = url
        .strip_prefix("https://github.com/")?
        .split_once("/releases/download/")?;
    let (tag, asset) = rest.rsplit_once('/')?;
    Some((repo, tag, asset))
}

/// Projects and collections use the same ownership test in registry review.
fn owned_by(owners: &[String], author: &str) -> bool {
    owners
        .iter()
        .any(|owner| owner.eq_ignore_ascii_case(author))
}

/// What a pull request changes, for its reviewers.
#[derive(Debug, Default, PartialEq)]
pub struct Review {
    /// Changes the registry refuses whoever reviews them.
    pub errors: Vec<String>,
    /// Findings that do not block the registry review.
    pub warnings: Vec<String>,
    /// What reviewers should look at: new projects, new owners or keys, a
    /// change by someone who isn't an owner.
    pub notes: Vec<String>,
    /// Changes to collection membership or presentation.
    pub collection_reviews: Vec<ReviewChange>,
    /// Changes to a project's Nightly source declaration.
    pub nightly_reviews: Vec<ReviewChange>,
    /// New or changed binary build declarations. Routine is conditional on a passing check.
    pub build_reviews: Vec<ReviewChange>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Routine,
    ReviewRequired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReviewChange {
    pub id: String,
    pub status: ReviewStatus,
    pub reason: String,
}

/// Reject a change to a published project's kind.
pub fn check_kind(id: &str, old: &Kind, new: &Kind) -> Result<(), String> {
    if old != new {
        return Err(format!("{id}: its kind can't change"));
    }
    Ok(())
}

/// A `mod` or `native-mod` listed in the `library` category has nothing to show: it is exempt from
/// the screenshot requirement.
fn is_library(entry: &Entry) -> bool {
    matches!(entry.kind, Kind::Mod | Kind::NativeMod)
        && entry.categories.iter().any(|c| c == "library")
}

/// Whether a project needs a screenshot: it is new (`base` is `None`), or `head` adds a version to
/// a listing, unless it is a library mod. An existing listing with no screenshots that gains no
/// version stays valid. Collections are not entries, so they never need one.
fn screenshot_required(base: Option<&Entry>, head: &Entry) -> bool {
    if is_library(head) || (head.versions.is_empty() && head.repo.is_none()) {
        return false;
    }
    base.is_none_or(|old| {
        head.versions
            .iter()
            .any(|w| old.versions.iter().all(|v| v.version != w.version))
    })
}

/// The one screenshot rule, for `registry check`, `registry review` and `publish`: every project of
/// `head` with a new listing or version, other than library mods, must list at least one valid
/// PNG or JPEG (`screenshots/<id>/<name>.png|.jpg`) in the `registry` checkout. An address elsewhere
/// does not count.
pub fn check_screenshot_rule(base: &[Entry], head: &[Entry], registry: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    for new in head {
        let old = base.iter().find(|e| e.id == new.id);
        if !screenshot_required(old, new) {
            continue;
        }
        let id = &new.id;
        let what = if old.is_none() {
            "a new project"
        } else {
            "a new version"
        };
        let folder = format!("screenshots/{id}/");
        let mut problems = Vec::new();
        let mut found = false;
        for path in &new.screenshots {
            let named = path.strip_prefix(&folder).is_some_and(super::plain_name);
            if !named {
                problems.push(format!(
                    "{path:?} is not a file {folder}<name>.png|.jpg in the registry"
                ));
                continue;
            }
            match read_regular(registry, Path::new(path), rules::MAX_SCREENSHOT) {
                Err(e) => problems.push(format!("{path}: {e}")),
                Ok(bytes) if bytes.is_empty() => {
                    problems.push(format!("{path}: the file is empty"))
                }
                Ok(bytes) => match rules::check_screenshot(path, &bytes) {
                    Ok(()) => found = true,
                    Err(e) => problems.push(e),
                },
            }
        }
        if !found {
            let detail = if problems.is_empty() {
                String::new()
            } else {
                format!(" ({})", problems.join("; "))
            };
            errors.push(format!(
                "{id}: {what} needs at least one screenshot: add an image as `{folder}<name>.png|.jpg` to the registry and list it in `screenshots`{detail} (a mod in the `library` category is exempt)"
            ));
        }
    }
    errors
}

/// Review the registry state on both sides of a pull request. A routine build append must
/// be the only change, including to collections, takedowns and other project data.
pub fn review_checkout(base: &Checkout, head: &Checkout, author: &str) -> Review {
    let mut review = review_with_revoked(&base.entries, &head.entries, &head.revoked, author);
    review.collection_reviews = review_collections(base, head, author);
    if review
        .build_reviews
        .iter()
        .any(|change| change.status == ReviewStatus::Routine)
        && (!review.collection_reviews.is_empty()
            || base.revoked != head.revoked
            || base.caches != head.caches
            || base.entries.iter().any(|entry| {
                entry
                    .icon
                    .iter()
                    .map(|name| (name, MAX_ICON))
                    .chain(
                        entry
                            .screenshots
                            .iter()
                            .map(|name| (name, rules::MAX_SCREENSHOT)),
                    )
                    .any(|(name, limit)| {
                        read_regular(&base.directory, Path::new(name), limit).ok()
                            != read_regular(&head.directory, Path::new(name), limit).ok()
                    })
            }))
    {
        for change in &mut review.build_reviews {
            if change.status == ReviewStatus::Routine {
                change.status = ReviewStatus::ReviewRequired;
                change.reason = "the checkout also changes other registry data".into();
            }
        }
    }
    review
}

/// Compares the entries of a pull request (`head`) with `main`'s (`base`):
/// new and changed version rows must name a stable Sunrise release. A published
/// file never changes, a project's kind never changes, and changes by someone
/// who isn't an owner, or to owners and keys, are pointed out.
pub fn review(base: &[Entry], head: &[Entry], author: &str) -> Review {
    review_with_revoked(base, head, &[], author)
}

/// [`review`], where a version may leave its entry if it never named a stable
/// Sunrise release or `revoked` (the pull request's `revoked.toml`) takes its
/// file down by hash. A project whose signing key was lost can drop the versions
/// its new key can't verify once those files are taken down.
pub fn review_with_revoked(
    base: &[Entry],
    head: &[Entry],
    revoked: &[Revoked],
    author: &str,
) -> Review {
    let mut r = Review::default();
    let proposed: BTreeMap<_, _> = head
        .iter()
        .map(|entry| (entry.id.as_str(), entry))
        .collect();
    let one_project_changed = base.len() == head.len()
        && base
            .iter()
            .filter(|entry| proposed.get(entry.id.as_str()).copied() != Some(*entry))
            .take(2)
            .count()
            == 1;
    for old in base {
        let Some(&new) = proposed.get(old.id.as_str()) else {
            r.errors.push(format!(
                "{}: removed; take its files down in {REVOKED} instead",
                old.id
            ));
            continue;
        };
        if new == old {
            continue;
        }
        let owner = owned_by(&old.owners, author);
        if old.nightly != new.nightly {
            let mut without_change = new.clone();
            without_change.nightly.clone_from(&old.nightly);
            let (status, reason) = if owner && new.nightly.is_some() && without_change == *old {
                (
                    ReviewStatus::Routine,
                    "an owner changed only the Nightly source",
                )
            } else {
                (
                    ReviewStatus::ReviewRequired,
                    "not an owner-only Nightly source update",
                )
            };
            r.nightly_reviews.push(ReviewChange {
                id: new.id.clone(),
                status,
                reason: reason.into(),
            });
        }
        if !owner {
            r.notes.push(format!(
                "{}: changed by {author}, who isn't one of its owners ({})",
                old.id,
                old.owners.join(", ")
            ));
        }
        if let Err(error) = check_kind(&old.id, &old.kind, &new.kind) {
            r.errors.push(error);
        }
        let mut without_builds = None;
        let mut only_appended = true;
        for v in &old.versions {
            let taken_down = || {
                revoked
                    .iter()
                    .any(|t| t.id == old.id && t.sha256.as_deref() == Some(v.sha256.as_str()))
            };
            match new
                .versions
                .iter()
                .enumerate()
                .find(|(_, w)| w.version == v.version)
            {
                None if taken_down() => r.notes.push(format!(
                    "{} {}: leaves the entry, taken down in {REVOKED}",
                    old.id, v.version
                )),
                None if v.sunrise.is_none() => r.notes.push(format!(
                    "{} {}: leaves the entry, it never named a stable release",
                    old.id, v.version
                )),
                None => r.errors.push(format!(
                    "{} {}: removed; take it down in {REVOKED} instead",
                    old.id, v.version
                )),
                Some((position, w)) => {
                    if w.sha256 != v.sha256
                        || w.file != v.file
                        || w.signature != v.signature
                        || w.game != v.game
                    {
                        r.errors.push(format!(
                            "{} {}: a published file never changes: publish a new version",
                            old.id, v.version
                        ));
                    }
                    if v.sunrise.is_some() && w.sunrise.is_some() && w.sunrise != v.sunrise {
                        r.errors.push(format!(
                            "{} {}: the stable release a published version names never changes: publish a new version",
                            old.id, v.version
                        ));
                    }
                    if w.builds != v.builds {
                        if !w.builds.starts_with(&v.builds) {
                            only_appended = false;
                            r.errors.push(format!(
                                "{} {}: published builds are append-only",
                                old.id, v.version
                            ));
                        }
                        without_builds.get_or_insert_with(|| new.clone()).versions[position]
                            .builds
                            .clone_from(&v.builds);
                    }
                }
            }
        }
        if let Some(without_builds) = without_builds {
            let (status, reason) = if owner && only_appended && without_builds == *old {
                if one_project_changed {
                    (
                        ReviewStatus::Routine,
                        "an owner only appended builds to a published version",
                    )
                } else {
                    (
                        ReviewStatus::ReviewRequired,
                        "the checkout also changes other projects",
                    )
                }
            } else {
                (
                    ReviewStatus::ReviewRequired,
                    "not an owner-only append of stable builds",
                )
            };
            r.build_reviews.push(ReviewChange {
                id: old.id.clone(),
                status,
                reason: reason.into(),
            });
        }
        if new.repo != old.repo {
            r.notes.push(format!(
                "{}: its repository {} -> {}: new releases will come from the second; versions already read stay",
                old.id,
                old.repo.as_deref().unwrap_or("(none)"),
                new.repo.as_deref().unwrap_or("(none)")
            ));
        }
        if new.owners != old.owners {
            r.notes.push(format!(
                "{}: owners {} -> {}",
                old.id,
                old.owners.join(", "),
                new.owners.join(", ")
            ));
        }
        if new.owner_ids != old.owner_ids {
            let ids = |ids: &[u64]| match ids {
                [] => "(none)".to_owned(),
                ids => ids
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
            };
            r.notes.push(format!(
                "{}: owner IDs {} -> {}: check each is the account its login names",
                old.id,
                ids(&old.owner_ids),
                ids(&new.owner_ids)
            ));
        }
        if new.key != old.key {
            r.notes.push(format!(
                "{}: its signing key changes; updates signed with the old one stop working",
                old.id
            ));
        }
        let added = new.versions.len().saturating_sub(old.versions.len());
        if added > 0 {
            r.notes.push(format!("{}: {added} new version(s)", old.id));
        }
    }
    for new in head {
        let old = base.iter().find(|e| e.id == new.id);
        if old != Some(new) {
            // Only unchanged legacy rows may omit the release. Match each base row at most once.
            let mut legacy: Vec<_> = old
                .into_iter()
                .flat_map(|e| &e.versions)
                .filter(|v| v.sunrise.is_none())
                .collect();
            for v in new.versions.iter().filter(|v| v.sunrise.is_none()) {
                if let Some(i) = legacy.iter().position(|&w| w == v) {
                    legacy.swap_remove(i);
                } else {
                    r.errors.push(format!(
                        "{} {}: a new or changed version row needs sunrise = \"<stable release tag>\"",
                        new.id, v.version
                    ));
                }
            }
        }
        // Every version is a submission, checked and reviewed: the registry no longer follows a
        // repository's releases (entries from before keep theirs until they are pinned).
        if new.repo.is_some() && old.is_none_or(|e| e.repo.is_none()) {
            r.errors.push(format!(
                "{}: the registry doesn't follow repositories any more: list each version's file (the form, or [[versions]])",
                new.id
            ));
        }
        if old.is_none() {
            if new.nightly.is_some() {
                let (status, reason) = if new.versions.is_empty()
                    && new.repo.is_none()
                    && owned_by(&new.owners, author)
                {
                    (
                        ReviewStatus::Routine,
                        "the author owns this new Nightly-only project",
                    )
                } else {
                    (
                        ReviewStatus::ReviewRequired,
                        "a new Nightly project must be source-only and owned by the author to be routine",
                    )
                };
                r.nightly_reviews.push(ReviewChange {
                    id: new.id.clone(),
                    status,
                    reason: reason.into(),
                });
            }
            r.notes.push(format!(
                "{}: a new {} by {}{}",
                new.id,
                new.kind.name(),
                new.owners.join(", "),
                new.repo.as_deref().map_or(String::new(), |repo| format!(
                    "; its releases come from https://github.com/{repo} with no further review: check the repository is the author\'s and read its source"
                ))
            ));
        }
    }
    r
}

/// Newly added takedowns (changing a reason or date doesn't take down a new file).
pub fn new_takedowns<'a>(
    base: &'a [Revoked],
    head: &'a [Revoked],
) -> impl Iterator<Item = &'a Revoked> {
    head.iter().filter(|r| {
        !base
            .iter()
            .any(|old| old.id == r.id && old.sha256 == r.sha256)
    })
}

/// Notes for the required ranges each newly revoked file satisfied. A hash-specific takedown
/// can remove the version from its entry in the same submission, so look in the base as well.
pub fn takedown_notes(base: &[Entry], previous: &[Revoked], head: &Index) -> Vec<String> {
    let mut notes = std::collections::BTreeSet::new();
    for r in new_takedowns(previous, &head.revoked) {
        for collection in &head.collections {
            if collection.members.iter().any(|member| member.id == r.id) {
                notes.insert(format!(
                    "collection {} lists {}, which has a new takedown",
                    collection.id, r.id
                ));
            }
        }
        let old = base
            .iter()
            .filter(|p| p.id == r.id)
            .flat_map(|p| p.versions.iter().map(|v| (&v.version, &v.sha256)));
        let new = head
            .project(&r.id)
            .into_iter()
            .flat_map(|p| p.versions.iter().map(|v| (&v.version, &v.file.sha256)));
        let affected: Vec<_> = old
            .chain(new)
            .filter(|(_, hash)| {
                r.sha256
                    .as_ref()
                    .is_none_or(|h| h.eq_ignore_ascii_case(hash))
            })
            .filter_map(|(v, _)| crate::manifest::parse_version(v).ok())
            .collect();
        for p in &head.projects {
            for v in &p.versions {
                for dep in v
                    .dependencies
                    .iter()
                    .flatten()
                    .filter(|d| !d.optional && d.id == r.id)
                {
                    if dep
                        .range(&p.id, &v.version)
                        .is_ok_and(|range| affected.iter().any(|n| range.matches(n)))
                    {
                        notes.insert(format!(
                            "{} {} requires {} {}: this takedown removes a matching file; check that a usable version remains",
                            p.id, v.version, dep.id, dep.version
                        ));
                    }
                }
            }
        }
    }
    notes.into_iter().collect()
}

#[cfg(test)]
mod tests;
