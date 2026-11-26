//! Updates and rollbacks (docs/server.md, section 7) for a `sunrise-server` installed by install.sh or
//! install.ps1: each release in `<dir>/releases/<tag>`, `<dir>/current` pointing at the one in use, and
//! `<dir>/previous` naming the one before it. An update downloads the newest build of the server's channel from
//! GitHub ([`newest`]: on a stable release the newest stable release of its line, otherwise the current Nightly),
//! checks it against the SHA-256 in its signed SHA256SUMS (docs/security.md), unpacks it beside the others under its
//! tag (a Nightly as `nightly-<date>-<commit>`) and switches
//! `current`; the supervisor then restarts the game on it and switches back by itself if the new game doesn't come
//! up.

use crate::{api::Client, backup, engine, supervisor};
use serde_json::{Value, json};
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use sunrise_platform::proxy;
use sunrise_server_launch::{
    self as launch, Install,
    config::{self, Config},
};

const REPO: &str = sunrise_release_sign::REPOSITORY;
/// The release archives for this computer, most specific first: on Linux the server build (no ALSA or
/// udev), else the full game (releases before the server build).
fn assets_for(os: &str, arch: &str, musl: bool) -> &'static [&'static str] {
    // No musl archive is published: a musl build is built from source, and a glibc update would replace this
    // supervisor with a binary that cannot even start to roll back on a musl system.
    if musl {
        return &[];
    }
    match (os, arch) {
        ("linux", "x86_64") => &[
            "sunrise-server-linux-x86_64.tar.gz",
            "sunrise-linux-x86_64.tar.gz",
        ],
        ("macos", "aarch64") => &["sunrise-macos-arm64.zip"],
        ("windows", "x86_64") => &["sunrise-windows-x64.zip"],
        _ => &[],
    }
}

static ASSETS: std::sync::LazyLock<&'static [&'static str]> = std::sync::LazyLock::new(|| {
    assets_for(
        std::env::consts::OS,
        std::env::consts::ARCH,
        cfg!(target_env = "musl"),
    )
});

fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

/// Changes to an installed release, owned by the supervisor's updater.
pub trait UpdateInstall {
    fn switch(&self, tag: &str) -> Result<(), String>;
    fn switch_back(&self, from: &str, previous: Option<&str>) -> Result<(), String>;
    fn prune(&self, keep: u32) -> (Vec<String>, Vec<String>);
}

impl UpdateInstall for Install {
    /// Points `current` at the release `tag`, and remembers the one it pointed at as `previous`.
    fn switch(&self, tag: &str) -> Result<(), String> {
        let target = self.release(tag);
        if !target.join(exe("sunrise-server")).is_file() {
            return Err(format!("{} holds no sunrise-server", target.display()));
        }
        if let Some(old) = self.current().filter(|old| old != tag) {
            std::fs::write(self.root.join("previous"), format!("{old}\n"))
                .map_err(|e| format!("{}: {e}", self.root.join("previous").display()))?;
        }
        point(&self.root.join("current"), &target)
            .map_err(|e| format!("switching current to {tag}: {e}"))
    }

    /// Undoes a [`UpdateInstall::switch`]: `current` back on `from` and `previous` as it was before, so a later
    /// rollback doesn't go to the release that failed.
    fn switch_back(&self, from: &str, previous: Option<&str>) -> Result<(), String> {
        point(&self.root.join("current"), &self.release(from))
            .map_err(|e| format!("switching current back to {from}: {e}"))?;
        let file = self.root.join("previous");
        match previous {
            Some(tag) => std::fs::write(&file, format!("{tag}\n")),
            None => std::fs::remove_file(&file).or_else(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(e)
                }
            }),
        }
        .map_err(|e| format!("{}: {e}", file.display()))
    }

    /// Deletes old releases after a successful update: it keeps the one `current` points at, the one
    /// `previous` names (where `rollback` goes), the one this program runs from, and the `keep` newest of
    /// the others. Returns the releases deleted, and what couldn't be (a program still running from it on
    /// Windows), which stays for the next update to try again.
    fn prune(&self, keep: u32) -> (Vec<String>, Vec<String>) {
        let releases = self.root.join("releases");
        let running = std::env::current_exe()
            .ok()
            .and_then(|e| std::fs::canonicalize(e).ok());
        let Some(current) = self.current() else {
            // `current` points nowhere: nothing is known to be safe to delete.
            return (Vec::new(), Vec::new());
        };
        let previous = self.previous();
        let mut others: Vec<String> = std::fs::read_dir(&releases)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            // `.<tag>.unpacking` is an update in progress.
            .filter(|tag| {
                !tag.starts_with('.') && *tag != current && previous.as_deref() != Some(tag)
            })
            .filter(|tag| {
                let dir = std::fs::canonicalize(releases.join(tag)).ok();
                !running
                    .as_ref()
                    .zip(dir)
                    .is_some_and(|(exe, dir)| exe.starts_with(dir))
            })
            .collect();
        // Newest first; names that aren't release tags (archives installed by hand) by name.
        others.sort_by(|a, b| {
            if newer(a, b) {
                std::cmp::Ordering::Less
            } else if newer(b, a) {
                std::cmp::Ordering::Greater
            } else {
                b.cmp(a)
            }
        });
        let (mut removed, mut failed) = (Vec::new(), Vec::new());
        for tag in others.into_iter().skip(keep as usize) {
            match std::fs::remove_dir_all(releases.join(&tag)) {
                Ok(()) => removed.push(tag),
                Err(e) => failed.push(format!("{tag} ({e})")),
            }
        }
        (removed, failed)
    }
}

/// What [`UpdateInstall::prune`] did, for the log: empty when it deleted nothing.
pub fn pruned(removed: &[String], failed: &[String]) -> String {
    let mut parts = Vec::new();
    if !removed.is_empty() {
        parts.push(format!("Deleted old releases: {}", removed.join(", ")));
    }
    if !failed.is_empty() {
        parts.push(format!(
            "Couldn't delete old releases: {}",
            failed.join(", ")
        ));
    }
    parts.join(". ")
}

/// Makes `link` point at the folder `target`, replacing what it pointed at.
#[cfg(unix)]
fn point(link: &Path, target: &Path) -> std::io::Result<()> {
    // A new link renamed over the old one: `current` always points at a whole release.
    let fresh = link.with_extension("switching");
    let _ = std::fs::remove_file(&fresh);
    std::os::unix::fs::symlink(target, &fresh)?;
    std::fs::rename(&fresh, link)
}

/// Makes `link` a junction to `target` (install.ps1's links: they need no admin rights). The new
/// junction is made first, beside it, so a failure leaves `current` as it was; it then takes the old
/// one's place, and if that last step fails the old junction is put back.
#[cfg(windows)]
fn point(link: &Path, target: &Path) -> std::io::Result<()> {
    let fresh = link.with_extension("switching");
    if fresh.symlink_metadata().is_ok() {
        std::fs::remove_dir(&fresh)?;
    }
    junction(&fresh, target)?;
    let old = std::fs::read_link(link).ok();
    if link.symlink_metadata().is_ok() {
        // A junction: removing it removes the link, never the release it points at.
        if let Err(e) = std::fs::remove_dir(link) {
            let _ = std::fs::remove_dir(&fresh);
            return Err(e);
        }
    }
    std::fs::rename(&fresh, link).inspect_err(|_| {
        let _ = std::fs::remove_dir(&fresh);
        if let Some(old) = &old {
            let _ = junction(link, old);
        }
    })
}

/// `mklink /J link target` through cmd, with both paths quoted as cmd reads them: unquoted, a folder
/// named with `&` or `^` (and no space, so std doesn't quote it) would split the command. `%` and `"`
/// keep a meaning inside quotes, so paths holding them are refused.
#[cfg(windows)]
fn junction(link: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    let quoted = |p: &Path| -> std::io::Result<String> {
        let text = p.to_str().ok_or_else(|| {
            std::io::Error::other(format!("{}: not a path cmd can take", p.display()))
        })?;
        if text.contains(['%', '"']) {
            return Err(std::io::Error::other(format!(
                "{text}: a folder whose path holds % or \" can't be linked; move the server elsewhere"
            )));
        }
        Ok(format!("\"{text}\""))
    };
    let line = mklink_line(&quoted(link)?, &quoted(target)?);
    let status = crate::process::command("cmd")
        .raw_arg(line)
        .stdout(std::process::Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "mklink /J failed ({status})"
        )))
    }
}

/// cmd's arguments for a junction, both paths already quoted: `/D` skips AutoRun, and `/S /C "..."`
/// makes cmd take the outer quotes off and run the rest as written.
#[cfg_attr(not(windows), allow(dead_code))]
fn mklink_line(link: &str, target: &str) -> String {
    format!("/D /S /C \"mklink /J {link} {target}\"")
}

/// A release on GitHub: its name here and the archive for this computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// The release's tag, or for a Nightly the version it installs as (`nightly-2026-10-28-0c02067`).
    pub tag: String,
    pub asset: String,
    pub url: String,
    pub sha256: String,
}

/// GitHub's API, or a release asset: the body of `url`.
fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    proxy::get(
        url,
        &[
            ("Accept", "application/vnd.github+json"),
            ("User-Agent", "sunrise-server"),
        ],
        None,
    )
    .map_err(|e| match e {
        e if e.status() == Some(404) => "404".to_string(),
        e => format!("GitHub didn't answer: {e}"),
    })?
    .into_reader()
    .take(64 << 20)
    .read_to_end(&mut body)
    .map_err(|e| format!("GitHub's answer: {e}"))?;
    Ok(body)
}

/// A release's address in GitHub's API.
fn api(tag: &str) -> String {
    format!("https://api.github.com/repos/{REPO}/releases/tags/{tag}")
}

/// The release a server running `current` updates to, with the SHA-256 its signed checksums give
/// for this computer's archive. A server on a stable release follows its line: the newest published
/// stable release of the same Minecraft version and Sunrise major (`26.3-0.x`), never a Nightly,
/// whose mod API may differ (docs/release-flow.md "Release branches"). Any other server follows
/// the Nightly ([`release`]).
pub fn newest(current: &str) -> Result<Release, String> {
    if stable(current).is_none() {
        return release(None);
    }
    newest_on_line(
        current,
        fetch,
        &sunrise_release_sign::trusted_keys_for(REPO),
    )
}

/// [`newest`] for a server on the stable release `current`: the stable tags of its line, newest
/// first, and the first of them with a published (not pre-release) release. A tag with no release
/// yet (one a release reserved before publishing) is passed over.
fn newest_on_line(
    current: &str,
    fetch: impl Fn(&str) -> Result<Vec<u8>, String>,
    keys: &[sunrise_release_sign::PublicKey],
) -> Result<Release, String> {
    let (game, [major, ..]) = stable(current).ok_or("not a stable release")?;
    let target = current
        .split_once('-')
        .map_or(current, |(target, _)| target);
    let line = format!("{target}-{major}.");
    let refs: Vec<Value> = serde_json::from_slice(&fetch(&format!(
        "https://api.github.com/repos/{REPO}/git/matching-refs/tags/{line}"
    ))?)
    .map_err(|e| format!("GitHub's tags: {e}"))?;
    let mut tags: Vec<&str> = refs
        .iter()
        .filter_map(|entry| entry["ref"].as_str()?.strip_prefix("refs/tags/"))
        .filter(|tag| stable(tag).is_some_and(|(g, [m, ..])| g == game && m == major))
        .collect();
    tags.sort_by_key(|tag| std::cmp::Reverse(tag_order(tag)));
    for tag in tags {
        let release: Value = match fetch(&api(tag)) {
            Err(e) if e == "404" => continue,
            found => {
                serde_json::from_slice(&found?).map_err(|e| format!("GitHub's answer: {e}"))?
            }
        };
        // Verified below for the tag it names: it must be the tag selected, or another signed release could
        // stand in for it and move the server off its line.
        if release["tag_name"] != tag {
            return Err(format!("GitHub's release {tag} names another tag"));
        }
        if release["prerelease"] != false || release["draft"] != false {
            continue;
        }
        return pick(&release, &fetch, keys);
    }
    Err(format!("no published release of {line}x on GitHub"))
}

/// The current Nightly (or `tag`: a release, `nightly` or a dated `nightly-<date>`), from GitHub's
/// API, with the SHA-256 its signed checksums give for this computer's archive. With no Nightly
/// published, GitHub's latest release.
pub fn release(tag: Option<&str>) -> Result<Release, String> {
    let text = match tag {
        Some(tag) => fetch(&api(tag)),
        None => match fetch(&api(sunrise_release_sign::NIGHTLY)) {
            Err(e) if e == "404" => fetch(&format!(
                "https://api.github.com/repos/{REPO}/releases/latest"
            )),
            found => found,
        },
    }
    .map_err(|e| match e.as_str() {
        "404" => format!("no release {} on GitHub", tag.unwrap_or("")),
        _ => e,
    })?;
    let json = serde_json::from_slice(&text).map_err(|e| format!("GitHub's answer: {e}"))?;
    pick(&json, fetch, &sunrise_release_sign::trusted_keys_for(REPO))
}

/// The archive for this computer in a release's JSON, with the SHA-256 the release's SHA256SUMS
/// gives for it, once its signature checks with one of `keys` (docs/security.md). A release from
/// before signing gets the SHA-256 pinned for it (legacy-releases.txt); any other release that
/// isn't signed, or not by Sunrise's key, is refused.
fn pick(
    release: &Value,
    fetch: impl Fn(&str) -> Result<Vec<u8>, String>,
    keys: &[sunrise_release_sign::PublicKey],
) -> Result<Release, String> {
    pick_for_assets(release, fetch, keys, &ASSETS)
}

fn pick_for_assets(
    release: &Value,
    fetch: impl Fn(&str) -> Result<Vec<u8>, String>,
    keys: &[sunrise_release_sign::PublicKey],
    candidates: &[&str],
) -> Result<Release, String> {
    let tag = release["tag_name"]
        .as_str()
        .ok_or("GitHub's answer has no tag")?;
    let assets = release["assets"].as_array().cloned().unwrap_or_default();
    let asset = candidates
        .iter()
        .find_map(|name| assets.iter().find(|a| a["name"] == *name))
        .ok_or_else(|| {
            format!(
                "release {tag} has no archive for this computer ({})",
                candidates.join(", ")
            )
        })?;
    let name = asset["name"].as_str().unwrap_or_default();
    // A release from before signing: only the files pinned for it at the time (a rollback).
    if let Some(hex) = sunrise_release_sign::legacy_digest_for(REPO, tag, name) {
        return Ok(Release {
            tag: tag.to_owned(),
            asset: name.to_owned(),
            url: asset["browser_download_url"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            sha256: hex.to_owned(),
        });
    }
    let attached = |file: &str| -> Result<Option<Vec<u8>>, String> {
        match assets.iter().find(|a| a["name"] == file) {
            Some(a) => fetch(a["browser_download_url"].as_str().unwrap_or_default())
                .map(Some)
                .map_err(|e| format!("{file} of {tag}: {e}")),
            None => Ok(None),
        }
    };
    let sums = attached(sunrise_release_sign::SUMS)?;
    let text = |bytes: Vec<u8>| String::from_utf8_lossy(&bytes).into_owned();
    // A Nightly installs under its date and commit, which its title names (strictly: nothing signs
    // GitHub's publication time), and its SHA256SUMS.v2.sig must sign the checksums for exactly that
    // version, so an older Nightly can't be served as a newer one.
    let (tag, sha256) = if sunrise_release_sign::is_nightly(tag) {
        let title = release["name"].as_str().unwrap_or_default();
        let version = sunrise_release_sign::build_title_version(sunrise_release_sign::Channel::Nightly, title).ok_or_else(|| {
            format!("the Nightly's title {title:?} doesn't name its date and commit, so it can't be checked")
        })?;
        let signature = attached(sunrise_release_sign::SIGNATURE_V2)?.map(text);
        let sha256 = sunrise_release_sign::build_digest(
            sunrise_release_sign::Channel::Nightly,
            &version,
            name,
            sums.as_deref(),
            signature.as_deref(),
            keys,
        )?;
        (version, sha256)
    } else {
        let signature = attached(sunrise_release_sign::SIGNATURE)?.map(text);
        let sha256 = sunrise_release_sign::signed_digest(
            tag,
            name,
            sums.as_deref(),
            signature.as_deref(),
            keys,
        )?;
        (tag.to_owned(), sha256)
    };
    Ok(Release {
        tag,
        asset: name.to_owned(),
        url: asset["browser_download_url"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        sha256,
    })
}

/// Where a release tag stands among Sunrise's releases (docs/versioning.md). Releases before 0.2 were
/// tagged `vYYYY.MM.DD[.N]` (any `v` and numbers: `Dated`), compared number by number; from 0.2 they are
/// `vMAJOR.MINOR.PATCH[-PRE]` (`Semver`), and every one of those is newer than every dated one, so
/// v0.2.0 follows v2026.10.19.15. A pre-release (`v0.3.0-rc.1`) comes before its release, as in semver.
/// Stable releases (`26.3-0.1.0`) follow those, by Minecraft version and then semver. Nightlies
/// (`nightly-<date>-<commit>`) follow every release, by date: a server never compares a Nightly with
/// a stable release to update, since each follows its own channel ([`newest`]).
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum TagOrder {
    Dated(Vec<u64>),
    Semver([u64; 3], Prerelease),
    Stable(Vec<u64>, [u64; 3]),
    Nightly(String),
}

/// A stable release's tag, `<Minecraft version>-<semver>` (`26.3-0.1.0`), as
/// `release_train.stable_version` reads it: the Minecraft version's two or three numbers, and the
/// release's major, minor and patch without leading zeros, pre-release or build suffix.
fn stable(tag: &str) -> Option<(Vec<u64>, [u64; 3])> {
    let digits = |n: &str| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit());
    let (target, version) = tag.split_once('-')?;
    let game = target
        .split('.')
        .map(|n| digits(n).then(|| n.parse().ok()).flatten())
        .collect::<Option<Vec<u64>>>()?;
    let version = version
        .split('.')
        .map(|n| {
            (digits(n) && (n == "0" || !n.starts_with('0')))
                .then(|| n.parse().ok())
                .flatten()
        })
        .collect::<Option<Vec<u64>>>()?;
    match (game.len(), version.as_slice()) {
        (2 | 3, &[major, minor, patch]) => Some((game, [major, minor, patch])),
        _ => None,
    }
}

/// A pre-release's place: (is a release, its identifiers). A release has none and follows every
/// pre-release of its version; identifiers compare one by one, numbers before words, and a shorter
/// list first when one starts the other.
type Prerelease = (bool, Vec<Identifier>);

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Identifier {
    Number(u64),
    Word(String),
}

/// Semver's majors stay below this; a date tag's first number is a year.
const FIRST_YEAR: u64 = 1000;

fn tag_order(tag: &str) -> Option<TagOrder> {
    if let Some(date) = sunrise_release_sign::nightly_date(tag) {
        return Some(TagOrder::Nightly(date.to_owned()));
    }
    if let Some((game, version)) = stable(tag) {
        return Some(TagOrder::Stable(game, version));
    }
    let rest = tag.strip_prefix('v')?;
    let (core, pre) = match rest.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (rest, None),
    };
    let numbers: Vec<u64> = core
        .split('.')
        .map(|n| n.parse().ok())
        .collect::<Option<_>>()?;
    match (numbers.as_slice(), pre) {
        (&[major, minor, patch], pre) if major < FIRST_YEAR => {
            let pre = match pre {
                None => (true, Vec::new()),
                Some(pre) => (
                    false,
                    pre.split('.')
                        .map(|id| {
                            if id.is_empty() {
                                None
                            } else {
                                Some(id.parse().map_or_else(
                                    |_| Identifier::Word(id.to_owned()),
                                    Identifier::Number,
                                ))
                            }
                        })
                        .collect::<Option<_>>()?,
                ),
            };
            Some(TagOrder::Semver([major, minor, patch], pre))
        }
        (_, None) => Some(TagOrder::Dated(numbers)),
        (_, Some(_)) => None,
    }
}

/// Whether the release `tag` is newer than `than` ([`TagOrder`]). A tag that isn't a release (an
/// archive installed by hand) is never newer than one, and every release is newer than it.
pub fn newer(tag: &str, than: &str) -> bool {
    match (tag_order(tag), tag_order(than)) {
        (Some(a), Some(b)) => a > b,
        (Some(_), None) => true,
        _ => false,
    }
}

/// Whether `tag`, the newest build on GitHub, is an update for a server running `current`: a
/// different build that isn't older. Two Nightlies of one day are equal by [`newer`], so the one
/// on GitHub is taken.
pub fn update_available(tag: &str, current: &str) -> bool {
    tag != current && tag_order(tag).is_some() && !newer(current, tag)
}

/// Downloads a release's archive into `<root>/downloads/` and checks its SHA-256; the archive's path.
pub fn download(install: &Install, release: &Release) -> Result<PathBuf, String> {
    use sha2::Digest;
    let folder = install.root.join("downloads");
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let path = folder.join(&release.asset);
    let part = folder.join(format!("{}.part", release.asset));
    let result = (|| {
        let mut reader = proxy::get(&release.url, &[("User-Agent", "sunrise-server")], None)
            .map_err(|e| format!("downloading {}: {e}", release.asset))?
            .into_reader();
        let mut file =
            std::fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
        let mut hasher = sha2::Sha256::new();
        let mut buffer = vec![0; 1 << 16];
        loop {
            let n = reader
                .read(&mut buffer)
                .map_err(|e| format!("downloading {}: {e}", release.asset))?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
            std::io::Write::write_all(&mut file, &buffer[..n]).map_err(|e| e.to_string())?;
        }
        let got: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if got != release.sha256 {
            return Err(format!(
                "{} doesn't match its SHA-256 (got {got}, expected {}): not installed",
                release.asset, release.sha256
            ));
        }
        std::fs::rename(&part, &path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result.map(|()| path)
}

/// Unpacks a release archive into `<root>/releases/<tag>`, with its `.local/sunrise-client` linked to the
/// install's prepared game files (which updates keep), as the installers do. It doesn't switch to it.
pub fn unpack(install: &Install, archive: &Path, tag: &str) -> Result<PathBuf, String> {
    if tag.is_empty() || tag.contains(['/', '\\']) || tag.starts_with('.') {
        return Err(format!("{tag:?} isn't a release name"));
    }
    let target = install.release(tag);
    if install.current().as_deref() == Some(tag) {
        return Err(format!("{tag} is the release in use"));
    }
    let scratch = install
        .root
        .join("releases")
        .join(format!(".{tag}.unpacking"));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    let result = (|| {
        let name = archive.to_string_lossy();
        if name.ends_with(".zip") {
            let file =
                std::fs::File::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
            zip::ZipArchive::new(file)
                .and_then(|mut z| z.extract(&scratch))
                .map_err(|e| format!("{}: {e}", archive.display()))?;
        } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            // tar is on every Linux and macOS, and on Windows 10 and later. The files are the
            // user's, not the archive's owners: as root in a user namespace (an unprivileged LXC
            // or rootless container) taking those fails, and as plain root they'd be strangers'.
            let status = crate::process::command("tar")
                .arg("--no-same-owner")
                .arg("-xzf")
                .arg(archive)
                .arg("-C")
                .arg(&scratch)
                .status()
                .map_err(|e| format!("tar: {e}"))?;
            if !status.success() {
                return Err(format!(
                    "tar couldn't unpack {} ({status})",
                    archive.display()
                ));
            }
        } else {
            return Err(format!(
                "{} is neither a .tar.gz nor a .zip",
                archive.display()
            ));
        }
        let source = find_program(&scratch, &exe("sunrise-server"))
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .ok_or_else(|| format!("{} holds no sunrise-server", archive.display()))?;
        #[cfg(unix)]
        for name in [
            "sunrise-server",
            "sunrise-engine",
            "sunrise-client",
            "sunrise-prepare",
        ] {
            use std::os::unix::fs::PermissionsExt;
            let path = source.join(name);
            if path.is_file() {
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
            }
        }
        if target.exists() {
            std::fs::remove_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
        }
        std::fs::rename(&source, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        let game_files = install.root.join("game-files/sunrise-client");
        std::fs::create_dir_all(&game_files).map_err(|e| e.to_string())?;
        let local = target.join(".local");
        std::fs::create_dir_all(&local).map_err(|e| e.to_string())?;
        let link = local.join("sunrise-client");
        if link.symlink_metadata().is_err() {
            point(&link, &game_files).map_err(|e| format!("{}: {e}", link.display()))?;
        }
        Ok(target.clone())
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

/// The first file called `name` under `dir`, breadth first.
fn find_program(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut queue = std::collections::VecDeque::from([dir.to_path_buf()]);
    while let Some(folder) = queue.pop_front() {
        let mut entries: Vec<_> = std::fs::read_dir(&folder).ok()?.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let kind = entry.file_type().ok()?;
            if kind.is_file() && entry.file_name() == name {
                return Some(path);
            }
            if kind.is_dir() {
                queue.push_back(path);
            }
        }
    }
    None
}

/// A backup name for the world before a switch to `tag` (backup names take letters, digits, - and _).
pub fn backup_name(tag: &str) -> String {
    let clean: String = tag
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(48)
        .collect();
    format!("before-{clean}")
}

/// `update [--check] [--version TAG] [--archive FILE --version TAG] [--timeout SECONDS]`.
pub fn command(dir: &Path, args: &[String]) -> Result<(), String> {
    let mut check = false;
    let mut as_json = false;
    let mut version = None;
    let mut archive = None;
    let mut timeout = crate::api::UPDATE_TIMEOUT;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--check" => check = true,
            "--json" => as_json = true,
            "--version" => version = Some(value()?),
            "--archive" => archive = Some(PathBuf::from(value()?)),
            "--timeout" => timeout = seconds(&value()?)?,
            other => return Err(format!("update: unknown option {other}")),
        }
    }
    let install = Install::required()?;
    let current = install
        .current()
        .ok_or("the installer's `current` points at no release")?;
    let (tag, archive, downloaded) = match archive {
        Some(_) if check => return Err("--check looks at GitHub's releases: drop --archive".into()),
        Some(archive) => {
            let tag = version
                .ok_or("--archive needs --version NAME: the release name to install it as")?;
            (tag, archive, false)
        }
        None => {
            let release = match &version {
                Some(tag) => release(Some(tag))?,
                None => newest(&current)?,
            };
            if check && as_json {
                println!(
                    "{}",
                    json!({
                        "installed": current,
                        "newest": release.tag,
                        "available": update_available(&release.tag, &current),
                        "previous": install.previous(),
                    })
                );
                return Ok(());
            }
            if check {
                println!("{}", check_line(version.as_deref(), &release.tag, &current));
                return Ok(());
            }
            if version.is_none() && !update_available(&release.tag, &current) {
                println!("Already on the newest release ({current}).");
                return Ok(());
            }
            if newer(&current, &release.tag) {
                // Asked for by --version: a rollback is allowed, never done without saying so.
                println!(
                    "{} is older than {current}, which this server runs: installing it as a rollback.",
                    release.tag
                );
            }
            println!("Downloading Sunrise {} ({})...", release.tag, release.asset);
            let archive = download(&install, &release)?;
            (release.tag, archive, true)
        }
    };
    if tag == current {
        println!("Already on {tag}.");
        return Ok(());
    }
    let unpacked = unpack(&install, &archive, &tag);
    if downloaded {
        let _ = std::fs::remove_file(&archive);
    }
    unpacked?;
    switch_to(dir, &install, &tag, timeout)
}

/// What `update --check` says about `newest` for a server running `current`. With `--version`
/// (`asked`), the command it suggests names the tag asked for, not the version that tag installs
/// as: `nightly` resolves to `nightly-<date>-<commit>`, which is no tag on GitHub.
fn check_line(asked: Option<&str>, newest: &str, current: &str) -> String {
    match asked {
        Some(asked) if newest != current => format!(
            "Sunrise {newest} is on GitHub; this server runs {current}. `sunrise-server update --version {asked}` \
             installs it."
        ),
        _ if update_available(newest, current) => format!(
            "Sunrise {newest} is out; this server runs {current}. `sunrise-server update` installs it."
        ),
        _ => format!("Up to date: {current} (the newest release is {newest})."),
    }
}

/// `rollback [--timeout SECONDS]`: back to the release used before the current one.
pub fn rollback(dir: &Path, args: &[String]) -> Result<(), String> {
    let timeout = match args {
        [] => crate::api::UPDATE_TIMEOUT,
        [flag, value] if flag == "--timeout" => seconds(value)?,
        _ => return Err("rollback takes only --timeout SECONDS".into()),
    };
    let install = Install::required()?;
    let previous = install.previous().ok_or(
        "no release to go back to: this install hasn't been updated, or the one before is gone",
    )?;
    switch_to(dir, &install, &previous, timeout)
}

fn seconds(text: &str) -> Result<Duration, String> {
    text.parse()
        .ok()
        .filter(|&s| s > 0 && s <= crate::api::MAX_UPDATE_TIMEOUT.as_secs())
        .map(Duration::from_secs)
        .ok_or_else(|| {
            format!(
                "--timeout needs a number of seconds up to {} (got {text:?})",
                crate::api::MAX_UPDATE_TIMEOUT.as_secs()
            )
        })
}

/// Switches the install to the release `tag`. A running server restarts on it through its supervisor, which
/// goes back by itself if the game isn't up within `timeout`; a stopped one is switched and its game checked.
/// Either way the world is backed up first.
fn switch_to(dir: &Path, install: &Install, tag: &str, timeout: Duration) -> Result<(), String> {
    let Some(client) = Client::open(dir)? else {
        return switch_stopped(dir, install, tag);
    };
    let finished = |status: &Value| status["update"]["finished"].as_u64().unwrap_or(0);
    let before = finished(&client.call("GET", "/api/v1/status", None)?);
    client
        .call(
            "POST",
            "/api/v1/update",
            Some(&json!({ "tag": tag, "timeout": timeout.as_secs() })),
        )
        .map_err(|e| {
            if e == "no such endpoint" {
                "the running server is older than updates: stop it, then run `sunrise-server update` again".into()
            } else {
                e
            }
        })?;
    print!("Restarting the server on {tag}");
    // The backup, the stop, the start and, if it fails, the way back.
    let deadline = Instant::now() + timeout + Duration::from_secs(3600);
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let status = client.call("GET", "/api/v1/status", None)?;
        if finished(&status) > before {
            let update = &status["update"];
            let last = update["last"].as_str().unwrap_or_default().to_owned();
            if update["ok"] == true {
                println!("\n{last}");
                return Ok(());
            }
            // Rolled back (or stayed): say so once the game is up again on the release it went back to.
            let mut phase = status["phase"].as_str().unwrap_or_default().to_owned();
            let until = Instant::now() + crate::api::UPDATE_TIMEOUT;
            while phase != "running" && phase != "stopping" && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(500));
                print!(".");
                let _ = std::io::Write::flush(&mut std::io::stdout());
                phase = client.call("GET", "/api/v1/status", None)?["phase"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
            }
            println!();
            return Err(if phase == "running" {
                format!("{last}. The game is up again.")
            } else {
                last
            });
        }
        if Instant::now() > deadline {
            println!();
            return Err(
                "no word from the server about the update; `sunrise-server logs` shows where it is"
                    .into(),
            );
        }
        print!(".");
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }
}

/// The switch for a stopped server (or outside any server folder): the world backed up, `current` moved,
/// and the new game checked to load on this system, else switched back.
fn switch_stopped(dir: &Path, install: &Install, tag: &str) -> Result<(), String> {
    let from = install
        .current()
        .ok_or("the installer's `current` points at no release")?;
    let server = dir.join(config::FILE).is_file();
    let _lock = if server {
        Some(supervisor::lock(dir)?)
    } else {
        None
    };
    let config = if server {
        Config::load(dir)?
    } else {
        Config::default()
    };
    let backed_up = if server {
        let file = backup::file_name(std::time::SystemTime::now(), Some(&backup_name(tag)))?;
        backup::create(dir, &config, &file)?;
        format!(" The world is backed up in {}/{file}.", backup::DIR)
    } else {
        String::new()
    };
    let previous = install.previous();
    install.switch(tag)?;
    if let Err(e) =
        launch::program_in(&config, dir, Some(install)).and_then(|p| engine::check_starts(&p))
    {
        install.switch_back(&from, previous.as_deref())?;
        return Err(format!("stayed on {from}: {e}"));
    }
    println!(
        "Switched from {from} to {tag}.{backed_up}{} `sunrise-server rollback` goes back to {from}.",
        if server {
            " The server is stopped: `sunrise-server start` runs the new version."
        } else {
            ""
        }
    );
    let (removed, failed) = install.prune(config.updates.keep);
    let pruned = pruned(&removed, &failed);
    if !pruned.is_empty() {
        println!("{pruned}.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_update_timeout_past_a_day_is_refused() {
        assert_eq!(seconds("90").unwrap(), Duration::from_secs(90));
        assert!(seconds("86400").is_ok());
        assert!(seconds("86401").is_err());
        assert!(seconds("18446744073709551615").is_err());
        assert!(seconds("0").is_err());
    }
    use serde_json::json;

    #[test]
    fn musl_update_refuses_glibc_before_downloading() {
        let release = json!({"tag_name": "nightly", "assets": [
            {"name": "sunrise-server-linux-x86_64.tar.gz", "browser_download_url": "server"},
            {"name": "sunrise-linux-x86_64.tar.gz", "browser_download_url": "client"}
        ]});
        let error = pick_for_assets(
            &release,
            |_| panic!("unsupported updates must not download an archive or signatures"),
            &[],
            assets_for("linux", "x86_64", true),
        )
        .unwrap_err();
        assert!(error.contains("no archive for this computer"));
    }

    #[test]
    fn updates_choose_only_the_three_published_platforms() {
        assert_eq!(
            assets_for("windows", "x86_64", false),
            &["sunrise-windows-x64.zip"]
        );
        assert_eq!(
            assets_for("macos", "aarch64", false),
            &["sunrise-macos-arm64.zip"]
        );
        assert!(assets_for("windows", "aarch64", false).is_empty());
        assert!(assets_for("macos", "x86_64", false).is_empty());
    }

    #[cfg(unix)]
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sunrise-server-update-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// An install as install.sh leaves it, with releases `tags`, the first in use.
    #[cfg(unix)]
    fn installed(name: &str, tags: &[&str]) -> Install {
        // Canonical, as `around` sees it (macOS's temporary folder is behind a link).
        let root = std::fs::canonicalize(scratch(name)).unwrap();
        for tag in tags {
            let release = root.join("releases").join(tag);
            std::fs::create_dir_all(&release).unwrap();
            std::fs::write(release.join("sunrise-server"), tag).unwrap();
        }
        std::os::unix::fs::symlink(root.join("releases").join(tags[0]), root.join("current"))
            .unwrap();
        Install { root }
    }

    /// Real junctions, in a folder whose name cmd would split on unquoted (`&`, no space so std doesn't
    /// quote it): `current` moves from one release to the other and nothing else changes.
    #[cfg(windows)]
    #[test]
    fn junctions_switch_in_a_folder_cmd_would_split() {
        let root = std::env::temp_dir().join(format!(
            "sunrise-server-junction-mc&co-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        for tag in ["v1", "v2", "v%3"] {
            let release = root.join("releases").join(tag);
            std::fs::create_dir_all(&release).unwrap();
            std::fs::write(release.join(exe("sunrise-server")), tag).unwrap();
        }
        let install = Install { root: root.clone() };
        let running = root.join("current").join(exe("sunrise-server"));
        install.switch("v1").unwrap();
        assert_eq!(std::fs::read_to_string(&running).unwrap(), "v1");
        install.switch("v2").unwrap();
        assert_eq!(std::fs::read_to_string(&running).unwrap(), "v2");
        assert!(root.join("current.switching").symlink_metadata().is_err());
        // Replacing the junction never touched the release it pointed at.
        let v1 = root.join("releases").join("v1").join(exe("sunrise-server"));
        assert_eq!(std::fs::read_to_string(v1).unwrap(), "v1");
        // `%` keeps a meaning inside cmd's quotes: refused, and `current` stays where it was.
        let refused = install.switch("v%3").unwrap_err();
        assert!(refused.contains('%'), "{refused}");
        assert_eq!(std::fs::read_to_string(&running).unwrap(), "v2");
        assert!(root.join("current.switching").symlink_metadata().is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_junctions_paths_are_quoted_for_cmd() {
        assert_eq!(
            mklink_line(r#""D:\mc&co\current""#, r#""D:\mc&co\releases\v0.2.95""#),
            r#"/D /S /C "mklink /J "D:\mc&co\current" "D:\mc&co\releases\v0.2.95"""#
        );
    }

    #[test]
    fn tags_compare_by_number() {
        assert!(newer("v2026.10.19", "v2026.10.18.27"));
        assert!(newer("v2026.10.18.27", "v2026.10.18.9"));
        assert!(newer("v2026.10.19.1", "v2026.10.19"));
        assert!(!newer("v2026.10.19", "v2026.10.19"));
        assert!(!newer("v2026.10.18", "v2026.10.19"));
        // An archive installed by hand: any release is newer, and it is never newer than one.
        assert!(newer("v2026.10.19", "local"));
        assert!(!newer("local", "v2026.10.19"));
    }

    #[test]
    fn semver_tags_follow_every_date_tag() {
        // 0.2 follows the last date tags, whatever their numbers.
        assert!(newer("v0.2.0", "v2026.10.19.15"));
        assert!(newer("v0.2.0", "v2026.10.21"));
        assert!(!newer("v2026.10.19.15", "v0.2.0"));
        assert!(!newer("v2099.12.31.99", "v0.2.0"));
        // Within 0.2: patch, minor, and numbers compared as numbers.
        assert!(newer("v0.2.1", "v0.2.0"));
        assert!(!newer("v0.2.0", "v0.2.1"));
        assert!(newer("v0.2.10", "v0.2.9"));
        assert!(newer("v0.3.0", "v0.2.57"));
        assert!(newer("v1.0.0", "v0.99.0"));
        assert!(!newer("v0.2.0", "v0.2.0"));
        // A semver tag is a release: an archive installed by hand is older.
        assert!(newer("v0.2.0", "local"));
        assert!(!newer("local", "v0.2.0"));
    }

    #[test]
    fn prereleases_come_before_their_release() {
        assert!(newer("v0.3.0", "v0.3.0-rc.1"));
        assert!(newer("v0.3.0-rc.1", "v0.2.9"));
        assert!(newer("v0.3.0-beta.2", "v0.3.0-beta.1"));
        assert!(newer("v0.3.0-beta.11", "v0.3.0-beta.2"));
        assert!(newer("v0.3.0-rc.1", "v0.3.0-beta.9"));
        assert!(newer("v0.3.0-beta.1", "v0.3.0-beta"));
        // Numbers come before words; any pre-release still follows every date tag.
        assert!(newer("v0.3.0-alpha", "v0.3.0-1"));
        assert!(newer("v0.2.1-beta.1", "v2026.10.21"));
        assert!(!newer("v0.2.1-beta.1", "v0.2.1"));
        // Not tags: a date tag with a suffix, an empty identifier.
        assert!(newer("v0.2.0", "v2026.10.21-x"));
        assert!(newer("v0.2.0", "v0.3.0-"));
        assert!(newer("v0.2.0", "v0.3.0-rc..1"));
    }

    #[test]
    fn nightlies_follow_every_release() {
        let today = "nightly-2026-10-28-0c02067";
        assert!(newer(today, "v0.2.95"));
        assert!(newer(today, "v2026.10.21"));
        assert!(newer(today, "nightly-2026-10-27-1a2b3c4"));
        assert!(!newer("v0.2.95", today));
        assert!(newer(today, "local"));
        // Two builds of one day: neither is newer, but the one on GitHub is the update.
        let later = "nightly-2026-10-28-9b772b2";
        assert!(!newer(later, today) && !newer(today, later));
        assert!(update_available(later, today));
        assert!(update_available(today, "v0.2.95"));
        assert!(!update_available(today, today));
        assert!(!update_available("v0.2.95", today));
        assert!(!update_available("nightly-2026-10-27-1a2b3c4", today));
        assert!(update_available("v0.2.96", "v0.2.95"));
        assert!(!update_available("v0.2.94", "v0.2.95"));
    }

    #[test]
    fn a_nightly_is_picked_with_its_version_signature_and_named_by_its_build() {
        use sunrise_release_sign::{SigningKey, sha256_hex, sums_file};
        let Some(first) = ASSETS.first() else { return };
        let asset = |name: &str| json!({ "name": name, "browser_download_url": format!("https://example.invalid/{name}") });
        let (key, _) = SigningKey::generate().unwrap();
        let sums = sums_file(&[(first.to_string(), sha256_hex(b"server"))]);
        // SHA256SUMS.sig signed for `channel`, SHA256SUMS.v2.sig for `v2`.
        let serving = |channel: &str, v2: &str| {
            let sums = sums.clone();
            let (one, two) = (
                key.sign(channel, sums.as_bytes()),
                key.sign(v2, sums.as_bytes()),
            );
            move |url: &str| -> Result<Vec<u8>, String> {
                match url.rsplit('/').next() {
                    Some("SHA256SUMS") => Ok(sums.clone().into_bytes()),
                    Some("SHA256SUMS.sig") => Ok(one.clone().into_bytes()),
                    Some("SHA256SUMS.v2.sig") => Ok(two.clone().into_bytes()),
                    _ => Err("404".into()),
                }
            }
        };
        const TONIGHT: &str = "nightly-2026-10-28-0c02067";
        let nightly = json!({
            "tag_name": "nightly", "name": "Nightly 2026-10-28 · 0c02067",
            "published_at": "2026-10-28T20:40:00Z",
            "assets": [asset(first), asset("SHA256SUMS"), asset("SHA256SUMS.sig"), asset("SHA256SUMS.v2.sig")],
        });
        let picked = pick(&nightly, serving("nightly", TONIGHT), &[key.public()]).unwrap();
        assert_eq!(picked.tag, TONIGHT);
        assert_eq!(picked.sha256, sha256_hex(b"server"));
        // An older Nightly's files under tonight's title: refused.
        let old = serving("nightly", "nightly-2026-10-21-aaaaaaa");
        assert!(pick(&nightly, old, &[key.public()]).is_err());
        // Only the channel's signature (a Nightly from before v2): refused.
        let mut v1_only = nightly.clone();
        v1_only["assets"] = json!([asset(first), asset("SHA256SUMS"), asset("SHA256SUMS.sig")]);
        let err = pick(&v1_only, serving("nightly", TONIGHT), &[key.public()]).unwrap_err();
        assert!(err.contains("SHA256SUMS.v2.sig"), "{err}");
        // A title that doesn't name its date and commit: refused, never dated by `published_at`.
        let mut untitled = nightly.clone();
        untitled["name"] = json!("Nightly");
        let err = pick(&untitled, serving("nightly", TONIGHT), &[key.public()]).unwrap_err();
        assert!(err.contains("doesn't name"), "{err}");
        // A release's signed list served as the Nightly: refused.
        assert!(pick(&nightly, serving("v0.2.95", "v0.2.95"), &[key.public()]).is_err());
        // A Nightly's signed list served as a release: refused.
        let mut release = nightly.clone();
        release["tag_name"] = json!("v0.2.95");
        assert!(pick(&release, serving(TONIGHT, TONIGHT), &[key.public()]).is_err());
        // A genuine Dev build (its title, its own valid signatures) served as the Nightly, or under
        // its own tag: refused, so no server updates to a Dev build.
        const DEV: &str = "dev-2026-10-28-0c02067";
        let mut dev = nightly.clone();
        dev["name"] = json!("Dev 2026-10-28 · 0c02067");
        assert!(pick(&dev, serving("dev", DEV), &[key.public()]).is_err());
        dev["tag_name"] = json!(DEV);
        assert!(pick(&dev, serving("dev", DEV), &[key.public()]).is_err());
    }

    #[test]
    fn stable_tags_compare_within_their_line() {
        assert!(newer("26.3-0.1.1", "26.3-0.1.0"));
        assert!(newer("26.3-0.10.0", "26.3-0.9.3"));
        assert!(newer("26.3-1.0.0", "26.3-0.99.0"));
        assert!(newer("26.3.1-0.1.0", "26.3-2.0.0"));
        assert!(update_available("26.3-0.2.0", "26.3-0.1.0"));
        assert!(!update_available("26.3-0.1.0", "26.3-0.2.0"));
        assert!(!update_available("26.3-0.1.0", "26.3-0.1.0"));
        // Not stable tags: no release to compare.
        for tag in [
            "26.3-0.1",
            "26.3-01.1.0",
            "26-0.1.0",
            "26.3-0.1.0-rc.1",
            "26.3.1.2-0.1.0",
            "a.3-0.1.0",
        ] {
            assert!(stable(tag).is_none(), "{tag}");
        }
    }

    /// A server on 26.3-0.1.0 updates to the newest published release of 26.3-0.x: never a tag
    /// with no release yet, never another major, never the Nightly.
    #[test]
    fn a_stable_server_follows_its_line() {
        use sunrise_release_sign::{SigningKey, sha256_hex, sums_file};
        let Some(first) = ASSETS.first() else { return };
        let (key, _) = SigningKey::generate().unwrap();
        let sums = sums_file(&[(first.to_string(), sha256_hex(b"server"))]);
        let signed = key.sign("26.3-0.2.0", sums.as_bytes());
        let asset = |name: &str| json!({ "name": name, "browser_download_url": format!("https://example.invalid/{name}") });
        let refs = json!([
            {"ref": "refs/tags/26.3-0.1.0"}, {"ref": "refs/tags/26.3-0.2.0"},
            {"ref": "refs/tags/26.3-0.3.0"}, {"ref": "refs/tags/26.3-0.2.0-rc.1"},
            {"ref": "refs/tags/26.3-0.5.0"}, {"ref": "refs/tags/26.3-0.4.0"},
        ]);
        // A newer stable-looking tag whose release is a draft or a pre-release: passed over.
        let unpublished = |tag: &str, flag: &str| {
            let mut release = json!({ "tag_name": tag, "prerelease": false, "draft": false, "assets": [asset(first)] });
            release[flag] = json!(true);
            release.to_string().into_bytes()
        };
        let fetch = |url: &str| -> Result<Vec<u8>, String> {
            if url.ends_with("/git/matching-refs/tags/26.3-0.") {
                return Ok(refs.to_string().into_bytes());
            }
            match url {
                u if u == api("26.3-0.5.0") => Ok(unpublished("26.3-0.5.0", "draft")),
                u if u == api("26.3-0.4.0") => Ok(unpublished("26.3-0.4.0", "prerelease")),
                // Reserved by a release that never published.
                u if u == api("26.3-0.3.0") => Err("404".into()),
                u if u == api("26.3-0.2.0") => Ok(json!({
                    "tag_name": "26.3-0.2.0", "prerelease": false, "draft": false,
                    "assets": [asset(first), asset("SHA256SUMS"), asset("SHA256SUMS.sig")],
                })
                .to_string()
                .into_bytes()),
                "https://example.invalid/SHA256SUMS" => Ok(sums.clone().into_bytes()),
                "https://example.invalid/SHA256SUMS.sig" => Ok(signed.clone().into_bytes()),
                other => panic!("asked for {other}"),
            }
        };
        let picked = newest_on_line("26.3-0.1.0", fetch, &[key.public()]).unwrap();
        assert_eq!(picked.tag, "26.3-0.2.0");
        assert_eq!(picked.sha256, sha256_hex(b"server"));
        assert!(update_available(&picked.tag, "26.3-0.1.0"));
        // On the newest release already: it is what GitHub has, so no update.
        assert!(!update_available(&picked.tag, "26.3-0.2.0"));
        // The release served for 26.3-0.2.0 names another tag, with sums validly signed for that
        // tag: refused for its tag, never installed.
        let other = key.sign("26.4-1.0.0", sums.as_bytes());
        let swapped = |url: &str| -> Result<Vec<u8>, String> {
            if url == "https://example.invalid/SHA256SUMS.sig" {
                return Ok(other.clone().into_bytes());
            }
            let body = fetch(url)?;
            Ok(if url == api("26.3-0.2.0") {
                String::from_utf8(body)
                    .unwrap()
                    .replace("26.3-0.2.0", "26.4-1.0.0")
                    .into_bytes()
            } else {
                body
            })
        };
        let err = newest_on_line("26.3-0.1.0", swapped, &[key.public()]).unwrap_err();
        assert!(err.contains("names another tag"), "{err}");
    }

    #[test]
    fn an_explicit_check_suggests_the_tag_asked_for() {
        let line = check_line(Some("nightly"), "nightly-2026-11-21-0c02067", "26.3-0.1.0");
        assert!(
            line.contains("`sunrise-server update --version nightly`"),
            "{line}"
        );
        assert!(line.contains("nightly-2026-11-21-0c02067"), "{line}");
        // Without --version, the update the server's own channel offers, and none once it runs it.
        let line = check_line(None, "26.3-0.2.0", "26.3-0.1.0");
        assert!(line.contains("`sunrise-server update`"), "{line}");
        assert!(line.contains("26.3-0.2.0"), "{line}");
        assert!(!check_line(None, "26.3-0.1.0", "26.3-0.1.0").contains("`sunrise-server update"));
    }

    #[test]
    fn pruning_orders_dated_and_semver_releases() {
        // `prune` sorts with `newer`: semver releases first, then the dated ones.
        let mut tags = vec!["v2026.10.19.15", "v0.2.1", "v2026.10.21", "v0.2.0", "local"];
        tags.sort_by(|a, b| {
            if newer(a, b) {
                std::cmp::Ordering::Less
            } else if newer(b, a) {
                std::cmp::Ordering::Greater
            } else {
                b.cmp(a)
            }
        });
        assert_eq!(
            tags,
            ["v0.2.1", "v0.2.0", "v2026.10.21", "v2026.10.19.15", "local"]
        );
    }

    #[test]
    fn sunrise_server_ships_with_a_release_key() {
        // With no key in release-keys.txt every update after the pinned releases would be refused:
        // this fails until the release environment's public key is listed (docs/security.md).
        assert!(
            !sunrise_release_sign::trusted_keys_for(REPO).is_empty(),
            "crates/sunrise-release-sign/release-keys.txt lists no key"
        );
    }

    #[test]
    fn upstreams_unsigned_releases_are_not_pinned_for_sunrise() {
        if let Some(first) = ASSETS.first() {
            assert!(
                sunrise_release_sign::legacy_digest_for(
                    "horizon-rust/horizon",
                    "v2026.10.19",
                    first
                )
                .is_some(),
                "v2026.10.19 is pinned for upstream's repository"
            );
            let old = json!({ "tag_name": "v2026.10.19", "assets": [{ "name": first, "browser_download_url": "u" }] });
            let nothing = |_: &str| -> Result<Vec<u8>, String> { Err("404".into()) };
            let refused =
                pick(&old, nothing, &sunrise_release_sign::trusted_keys_for(REPO)).unwrap_err();
            assert!(refused.contains("not signed"), "{refused}");
        }
    }

    #[test]
    fn the_archive_for_this_computer_is_picked_with_its_signed_digest() {
        use sunrise_release_sign::{SigningKey, sha256_hex, sums_file};
        let asset = |name: &str| json!({ "name": name, "browser_download_url": format!("https://example.invalid/{name}") });
        let (key, _) = SigningKey::generate().unwrap();
        let listed: Vec<(String, String)> = ASSETS
            .iter()
            .map(|n| (n.to_string(), sha256_hex(n.as_bytes())))
            .collect();
        let sums = sums_file(&listed);
        let signature = key.sign("v2099.01.01", sums.as_bytes());
        let files = |url: &str| -> Result<Vec<u8>, String> {
            match url.rsplit('/').next() {
                Some("SHA256SUMS") => Ok(sums.clone().into_bytes()),
                Some("SHA256SUMS.sig") => Ok(signature.clone().into_bytes()),
                _ => Err("404".into()),
            }
        };
        let names = ASSETS.iter().rev().copied().chain(["other.zip"]);
        let signed = json!({
            "tag_name": "v2099.01.01",
            "assets": names.chain(["SHA256SUMS", "SHA256SUMS.sig"]).map(asset).collect::<Vec<_>>(),
        });
        if let Some(first) = ASSETS.first() {
            let picked = pick(&signed, files, &[key.public()]).unwrap();
            assert_eq!(picked.tag, "v2099.01.01");
            // The most specific archive (Linux: the server build), even when listed last.
            assert_eq!(picked.asset, *first);
            assert_eq!(picked.sha256, sha256_hex(first.as_bytes()));
            // Signed by another key, or not signed at all: refused.
            let (other, _) = SigningKey::generate().unwrap();
            assert!(pick(&signed, files, &[other.public()]).is_err());
            let unsigned = json!({ "tag_name": "v1", "assets": [asset(first)] });
            assert!(
                pick(&unsigned, files, &[key.public()])
                    .unwrap_err()
                    .contains("not signed")
            );
            // The same signed list under another tag: refused.
            let mut replayed = signed.clone();
            replayed["tag_name"] = json!("v2099.01.02");
            assert!(pick(&replayed, files, &[key.public()]).is_err());
        }
        assert!(
            pick(
                &json!({ "tag_name": "v1", "assets": [asset("other.zip")] }),
                files,
                &[]
            )
            .unwrap_err()
            .contains("no archive for this computer")
        );
    }

    #[test]
    fn backup_names_are_valid() {
        let name = backup_name("v2026.10.19");
        assert_eq!(name, "before-v2026-10-19");
        crate::backup::file_name(std::time::SystemTime::now(), Some(&name)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn switching_keeps_the_previous_release() {
        let install = installed("switch", &["v1", "v2"]);
        let exe = install.root.join("current/sunrise-server");
        assert_eq!(Install::around(&exe), Some(install.clone()));
        assert_eq!(
            Install::around(&install.release("v2").join("sunrise-server")),
            Some(install.clone())
        );
        assert_eq!(install.current().as_deref(), Some("v1"));
        assert_eq!(install.previous(), None);
        install.switch("v2").unwrap();
        assert_eq!(install.current().as_deref(), Some("v2"));
        assert_eq!(install.previous().as_deref(), Some("v1"));
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "v2");
        // Undoing it leaves `previous` as it was.
        install.switch_back("v1", None).unwrap();
        assert_eq!(install.current().as_deref(), Some("v1"));
        assert_eq!(install.previous(), None);
        install.switch("v2").unwrap();
        // Rolling back is a switch to the previous release, which then becomes the one to go back to.
        install.switch("v1").unwrap();
        assert_eq!(install.current().as_deref(), Some("v1"));
        assert_eq!(install.previous().as_deref(), Some("v2"));
        // Switching to the release in use changes nothing; to a missing one is refused.
        install.switch("v1").unwrap();
        assert_eq!(install.previous().as_deref(), Some("v2"));
        assert!(
            install
                .switch("v3")
                .unwrap_err()
                .contains("no sunrise-server")
        );
        assert_eq!(install.current().as_deref(), Some("v1"));
        std::fs::remove_dir_all(&install.root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn pruning_keeps_current_previous_and_the_newest_others() {
        let tags = [
            "v2026.10.19",
            "v2026.10.1",
            "v2026.10.2",
            "v2026.10.10",
            "v2026.10.18.3",
            "local",
        ];
        let install = installed("prune", &tags);
        // After a rollback `previous` can be any release, even the oldest.
        std::fs::write(install.root.join("previous"), "v2026.10.1\n").unwrap();
        std::fs::create_dir_all(install.root.join("releases/.v2026.10.20.unpacking")).unwrap();
        std::fs::write(install.root.join("releases/notes.txt"), "").unwrap();
        let left = || {
            let mut names: Vec<String> = std::fs::read_dir(install.root.join("releases"))
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        };
        let (removed, failed) = install.prune(1);
        assert_eq!(removed, ["v2026.10.10", "v2026.10.2", "local"]);
        assert!(failed.is_empty());
        assert_eq!(
            left(),
            [
                ".v2026.10.20.unpacking",
                "notes.txt",
                "v2026.10.1",
                "v2026.10.18.3",
                "v2026.10.19"
            ]
        );
        assert_eq!(
            pruned(&removed, &failed),
            "Deleted old releases: v2026.10.10, v2026.10.2, local"
        );
        // Nothing more to delete at the same count; none at all kept besides current and previous.
        assert_eq!(install.prune(1), (Vec::new(), Vec::new()));
        assert_eq!(install.prune(0).0, ["v2026.10.18.3"]);
        assert_eq!(install.current().as_deref(), Some("v2026.10.19"));
        assert_eq!(install.previous().as_deref(), Some("v2026.10.1"));
        // Without a `current` nothing is deleted.
        std::fs::remove_file(install.root.join("current")).unwrap();
        assert_eq!(install.prune(0), (Vec::new(), Vec::new()));
        assert!(install.release("v2026.10.19").is_dir());
        std::fs::remove_dir_all(&install.root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "daily"]
    fn an_archive_unpacks_beside_the_others_with_the_game_files_linked() {
        let install = installed("unpack", &["v1"]);
        let source = scratch("unpack-source");
        let inner = source.join("sunrise-v2");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(inner.join("sunrise-server"), "v2").unwrap();
        std::fs::write(inner.join("sunrise-engine"), "engine").unwrap();
        let archive = source.join("sunrise-server-linux-x86_64.tar.gz");
        let status = std::process::Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&source)
            .arg("sunrise-v2")
            .status()
            .unwrap();
        assert!(status.success());
        let release = unpack(&install, &archive, "v2").unwrap();
        assert_eq!(release, install.release("v2"));
        assert_eq!(
            std::fs::read_to_string(release.join("sunrise-engine")).unwrap(),
            "engine"
        );
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(release.join("sunrise-engine"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111);
        }
        assert_eq!(
            std::fs::canonicalize(release.join(".local/sunrise-client")).unwrap(),
            std::fs::canonicalize(install.root.join("game-files/sunrise-client")).unwrap()
        );
        // Not switched yet; nothing left behind; the release in use can't be unpacked over.
        assert_eq!(install.current().as_deref(), Some("v1"));
        assert!(!install.root.join("releases/.v2.unpacking").exists());
        assert!(
            unpack(&install, &archive, "v1")
                .unwrap_err()
                .contains("in use")
        );
        assert!(unpack(&install, &archive, "../x").is_err());
        let empty = source.join("empty.zip");
        zip::ZipWriter::new(std::fs::File::create(&empty).unwrap())
            .finish()
            .unwrap();
        assert!(
            unpack(&install, &empty, "v3")
                .unwrap_err()
                .contains("holds no sunrise-server")
        );
        assert!(!install.release("v3").exists());
        std::fs::remove_dir_all(&install.root).unwrap();
        std::fs::remove_dir_all(&source).unwrap();
    }
}
