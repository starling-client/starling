//! The standalone tool updates independently of any project's pinned game or SDK. It follows the
//! rolling `mod-tool` release, which only moves forward (`tools/ci/mod_tool.py newer`), so a
//! signed release naming another commit than a rolling build's own is a newer one.

use crate::{dev, options::Options, sdk_fetch};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::{self, Read, Seek, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use sunrise_addresses::SDK_RELEASES;
use sunrise_release_sign::{self as release, MOD_TOOL, PublicKey, SIGNATURE, SUMS};
use tempfile::NamedTempFile;

const BUILD: &str = sunrise_native_build::contract::TOOL_COMMIT;
const CHANNEL: &str = env!("SUNRISE_TOOL_CHANNEL");
const INFO: &str = "tool.json";
#[cfg(not(windows))]
const EXE: &str = "cargo-sunrise-mod";
#[cfg(windows)]
const EXE: &str = "cargo-sunrise-mod.exe";
const NOTICE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Deserialize)]
struct Tool {
    commit: String,
}

impl Tool {
    fn read(bytes: &[u8]) -> Result<Self, String> {
        let tool: Self = serde_json::from_slice(bytes).map_err(|e| format!("{INFO}: {e}"))?;
        if !sunrise_mod_package::manifest::revision(&tool.commit) {
            return Err(format!("{INFO} does not name a full commit"));
        }
        Ok(tool)
    }

    fn differs_from(&self, current: &str) -> bool {
        self.commit != current
    }
}

/// Only the book's standalone install can be replaced. Do not rewrite a copy owned by an SDK,
/// Cargo, or a system package manager, even when the user includes it on PATH.
fn standalone_exe() -> Result<PathBuf, String> {
    let (home_var, bin) = if cfg!(windows) {
        ("USERPROFILE", ".sunrise/bin")
    } else {
        ("HOME", ".local/bin")
    };
    let exe = env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(|e| format!("could not locate the running tool: {e}"))?;
    let installed_here = env::var_os(home_var)
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
        .and_then(|home| home.canonicalize().ok())
        .is_some_and(|home| exe == home.join(bin).join(EXE));
    if !installed_here {
        return Err(format!(
            "{} is not in the standalone install location (${home_var}/{bin}/{EXE}); update it using its original install method",
            exe.display()
        ));
    }
    Ok(exe)
}

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    Options::new("self-update", args, &[]).finish()?;
    if CHANNEL != MOD_TOOL {
        return Err(format!(
            "this tool ({BUILD}) was not installed from the rolling {MOD_TOOL} release, which is the only one self-update follows; to switch to it, rerun the install block for the newest tool from main"
        ));
    }
    let exe = standalone_exe()?;
    let asset = format!("tool-{}.tar.xz", dev::computer()?);
    let base = sdk_fetch::Base::Url(SDK_RELEASES.to_owned());
    match update(&base, &sdk_fetch::release_keys(), &asset, &exe, BUILD)? {
        Some(tool) => println!("Updated cargo sunrise-mod: {BUILD} -> {}", tool.commit),
        None => println!("cargo sunrise-mod is already the newest ({BUILD})."),
    }
    Ok(())
}

/// Return the verified build that replaced `exe`, or `None` when it already matches.
fn update(
    base: &sdk_fetch::Base,
    keys: &[PublicKey],
    asset: &str,
    exe: &Path,
    current: &str,
) -> Result<Option<Tool>, String> {
    // The decision to do nothing also needs authenticated metadata from this release.
    let (listed, tool) = published(base, keys)?;
    if !tool.differs_from(current) {
        return Ok(None);
    }

    let dir = exe.parent().ok_or("the tool has no install folder")?;
    let want = listed(asset)?;
    let mut body = base.open(MOD_TOOL, asset)?;
    let mut archive = tempfile::tempfile_in(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let archive_error = |e| format!("{MOD_TOOL}/{asset}: {e}");
    io::copy(&mut body, &mut archive).map_err(archive_error)?;
    archive.rewind().map_err(archive_error)?;
    let mut hash = Sha256::new();
    io::copy(&mut archive, &mut hash).map_err(archive_error)?;
    release::check(asset, &want, &format!("{:x}", hash.finalize()))?;
    archive.rewind().map_err(archive_error)?;

    let mut staged = NamedTempFile::new_in(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    unpack_exe(archive, &mut staged).map_err(archive_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        staged
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o755))
            .map_err(archive_error)?;
    }
    staged.as_file().sync_all().map_err(archive_error)?;
    install(staged, exe)?;
    Ok(Some(tool))
}

/// The release's verified sums, as a lookup by asset name, and its `tool.json` checked against them.
fn published(
    base: &sdk_fetch::Base,
    keys: &[PublicKey],
) -> Result<(impl Fn(&str) -> Result<String, String>, Tool), String> {
    let sums = base.read(MOD_TOOL, SUMS, sdk_fetch::SMALL)?;
    let signature = base.read(MOD_TOOL, SIGNATURE, sdk_fetch::SMALL)?;
    let listed = release::verify(MOD_TOOL, &sums, &String::from_utf8_lossy(&signature), keys)?;
    let expected = move |asset: &str| {
        listed
            .iter()
            .find(|(name, _)| name == asset)
            .map(|(_, sha)| sha.clone())
            .ok_or_else(|| format!("{MOD_TOOL}/{SUMS} does not list {asset}"))
    };
    let info = base.read(MOD_TOOL, INFO, sdk_fetch::SMALL)?;
    release::check(INFO, &expected(INFO)?, &release::sha256_hex(&info))?;
    Ok((expected, Tool::read(&info)?))
}

/// Only copy the root executable from the already verified archive. No archive paths go to disk.
fn unpack_exe(archive: impl Read, into: &mut impl Write) -> io::Result<()> {
    let xz = lzma_rust2::XzReader::new(archive, false);
    for entry in tar::Archive::new(xz).entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        if entry.header().entry_type().is_file()
            && path.strip_prefix(".").unwrap_or(&path) == Path::new(EXE)
        {
            io::copy(&mut entry, into)?;
            return Ok(());
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("archive does not contain {EXE}"),
    ))
}

fn install(staged: NamedTempFile, exe: &Path) -> Result<(), String> {
    #[cfg(not(windows))]
    let result = staged.persist(exe);
    #[cfg(windows)]
    let result = {
        // Windows cannot replace a mapped executable. Move it aside on the same volume; keep
        // just one backup, which is still mapped until this process exits.
        let previous = exe.with_file_name(".cargo-sunrise-mod.exe.old");
        fs::rename(exe, &previous).map_err(|e| format!("{}: {e}", exe.display()))?;
        let result = staged.persist_noclobber(exe);
        if let Err(error) = &result
            && let Err(restore) = fs::rename(&previous, exe)
        {
            return Err(format!(
                "could not replace {}: {error}; could not restore the old tool from {}: {restore}",
                exe.display(),
                previous.display()
            ));
        }
        result
    };
    result
        .map(drop)
        .map_err(|e| format!("could not replace {}: {}", exe.display(), e.error))
}

pub(crate) fn notice() {
    let offline = env::var_os("CARGO_NET_OFFLINE").is_some_and(|v| v != "false");
    if !checks_allowed(
        env::var_os("CI").is_some() || offline,
        CHANNEL,
        standalone_exe().is_ok(),
    ) {
        return;
    }
    // Claim the day's attempt before the GET, so failures and concurrent invocations don't
    // keep hitting the network. The lock is released before any network I/O.
    let Ok(cache) = dev::cache_dir() else { return };
    if claim_notice_attempt(&cache, SystemTime::now()).is_none() {
        return;
    }
    let base = sdk_fetch::Base::Url(SDK_RELEASES.to_owned());
    if published(&base, &sdk_fetch::release_keys()).is_ok_and(|(_, tool)| tool.differs_from(BUILD))
    {
        eprintln!("A newer cargo sunrise-mod is out: run cargo sunrise-mod self-update");
    }
}

fn claim_notice_attempt(cache: &Path, now: SystemTime) -> Option<()> {
    fs::create_dir_all(cache).ok()?;
    let path = cache.join("mod-tool-update-check");
    match fs::File::create_new(&path) {
        Ok(_) => return Some(()), // The newly created file's mtime records the attempt.
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(_) => return None,
    }
    if !fs::symlink_metadata(&path).ok()?.is_file() {
        return None;
    }
    let stamp = fs::OpenOptions::new().write(true).open(path).ok()?;
    stamp.try_lock().ok()?;
    let last = stamp.metadata().ok()?.modified().ok()?;
    let age = now.duration_since(last).unwrap_or_default();
    if age < NOTICE_INTERVAL {
        return None;
    }
    stamp.set_modified(now).ok()
}

/// Only a standalone copy of the rolling release looks for its successor, and never in CI or offline.
fn checks_allowed(ci_or_offline: bool, channel: &str, standalone: bool) -> bool {
    !ci_or_offline && channel == MOD_TOOL && standalone
}

#[cfg(test)]
mod tests;
