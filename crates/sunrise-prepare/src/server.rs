//! The dedicated preparation's only downloads: Mojang's pinned server jar and its Java runtime.

use crate::{
    artifacts::{safe_relative, Lock},
    digest,
    java::{self, Jdk},
    mkdir,
    pyjson::Value,
    read_json, Log,
};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

const RUNTIMES: &str = "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

/// Never follow a location to another host, or accept HTTP from manifest metadata. The request goes
/// through the proxy the environment names, by Sunrise's proxy rules (`sunrise-proxy`).
fn request(url: &str) -> Result<ureq::Request, String> {
    let authority = url
        .strip_prefix("https://")
        .and_then(|s| s.split_once('/'))
        .map(|(host, _)| host);
    if !matches!(
        authority,
        Some(
            "piston-data.mojang.com"
                | "piston-meta.mojang.com"
                | "launchermeta.mojang.com"
                | "resources.download.minecraft.net"
        )
    ) {
        return Err(format!("Not a Mojang download URL: {url}"));
    }
    let agent = sunrise_proxy::agent("sunrise-prepare", url, |builder| builder.redirects(0))
        .map_err(|e| format!("{url}: {e}"))?;
    Ok(agent.get(url))
}

fn checked(path: &Path, sha1: &str) -> bool {
    digest(path, "sha1").is_ok_and(|actual| actual == sha1)
}

fn pin(sha1: &str) -> Result<&str, String> {
    if sha1.len() != 40 || !sha1.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("Invalid SHA-1 pin: {sha1}"));
    }
    Ok(sha1)
}

/// Keep interrupted downloads private and resumable, and publish only after verifying the pin.
fn download(url: &str, path: &Path, sha1: &str) -> Result<(), String> {
    if checked(path, sha1) {
        return Ok(());
    }
    let request = request(url)?;
    mkdir(path.parent().unwrap())?;
    let mut partial = path.as_os_str().to_os_string();
    partial.push(".part");
    let partial = PathBuf::from(partial);
    let offset = fs::metadata(&partial).map_or(0, |info| info.len());
    let error = |e: io::Error| format!("{}: {e}", path.display());
    if !checked(&partial, sha1) {
        let response = if offset == 0 {
            request.call()
        } else {
            match request
                .clone()
                .set("Range", &format!("bytes={offset}-"))
                .call()
            {
                Err(ureq::Error::Status(416, _)) => request.call(),
                received => received,
            }
        }
        .map_err(|e| e.to_string())?;
        let append = match response.status() {
            200 => false,
            206 if offset != 0
                && response
                    .header("Content-Range")
                    .is_some_and(|range| range.starts_with(&format!("bytes {offset}-"))) =>
            {
                true
            }
            code => return Err(format!("Unexpected response ({code}) for {url}")),
        };
        let mut out = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(&partial)
            .map_err(error)?;
        io::copy(&mut response.into_reader(), &mut out).map_err(error)?;
        drop(out);
        if !checked(&partial, sha1) {
            fs::remove_file(&partial).map_err(error)?;
            return Err(format!("{} did not match SHA-1 {sha1}", path.display()));
        }
    }
    fs::rename(&partial, path).map_err(error)
}

pub(crate) fn jar(
    root: &Path,
    lock: &Lock,
    supplied: Option<&Path>,
    log: Log,
) -> Result<PathBuf, String> {
    let sha1 = pin(lock.pin("server_sha1")?)?;
    if let Some(path) = supplied {
        let path = std::path::absolute(path).map_err(|e| e.to_string())?;
        let actual = digest(&path, "sha1")?;
        if actual != sha1 {
            return Err(format!(
                "{}: expected SHA-1 {sha1}, found {actual}",
                path.display()
            ));
        }
        return Ok(path);
    }
    let cache = root.join(".local/server-downloads").join(sha1);
    let _lock = crate::hold_lock(&cache, log)?;
    let path = cache.join("server.jar");
    if !checked(&path, sha1) {
        let version = &lock.version;
        log(&format!(
            "Downloading Minecraft {version}'s server.jar from Mojang (SHA-1 checked)"
        ));
        let url = format!("https://piston-data.mojang.com/v1/objects/{sha1}/server.jar");
        download(&url, &path, sha1).map_err(|e| format!(
            "couldn't download Minecraft {version}'s server.jar from Mojang ({e}).\n  Download it on another computer and copy it here, then:\n  sunrise-prepare --server --server-jar /path/to/server.jar\n  (it must have sha1 {sha1})"
        ))?;
    }
    Ok(path)
}

pub(crate) fn java(root: &Path, lock: &Lock, home: Option<&Path>, log: Log) -> Result<Jdk, String> {
    if let Some(home) = home {
        return java::runtime_in(home, lock.java_major);
    }
    // A source checkout needs javac to make its bundle. A release only needs a runtime.
    if !crate::exporters::available(root) {
        return java::find(lock.java_major, &[]);
    }
    match java::find_runtime(lock.java_major, &[]) {
        Ok(jdk) => Ok(jdk),
        Err(_) => {
            let home = install_runtime(root, lock, log)?;
            java::runtime_in(&home, lock.java_major)
        }
    }
}

fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Missing {field} in Mojang's Java metadata"))
}

fn install_runtime(root: &Path, lock: &Lock, log: Log) -> Result<PathBuf, String> {
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "mac-os-arm64",
        ("macos", "x86_64") => "mac-os",
        ("linux", "x86_64") => "linux",
        ("linux", "x86") => "linux-i386",
        ("windows", "aarch64") => "windows-arm64",
        ("windows", "x86_64") => "windows-x64",
        ("windows", "x86") => "windows-x86",
        _ => {
            return Err(format!(
                "Mojang has no Java runtime for this system; install Java {} or pass --java-home",
                lock.java_major
            ));
        }
    };
    // `java_component` is the pinned version JSON's javaVersion.component.
    let component = lock.pin("java_component")?;
    let sha1 = pin(lock.pin("java_index_sha1")?)?;
    let index_dir = root.join(".local/server-downloads").join(sha1);
    let index = {
        let _lock = crate::hold_lock(&index_dir, log)?;
        let path = index_dir.join("java-index.json");
        download(RUNTIMES, &path, sha1).map_err(|e| format!("Mojang's Java index: {e}"))?;
        read_json(&path)?
    };
    let manifest = index
        .get(platform)
        .and_then(|p| p.get(component))
        .and_then(|list| list.index(0))
        .and_then(|entry| entry.get("manifest"))
        .ok_or_else(|| format!("Mojang has no {component} for {platform}; pass --java-home"))?;
    let sha1 = pin(text(manifest, "sha1")?)?;
    let cache = root.join(".local/server-downloads").join(sha1);
    let _lock = crate::hold_lock(&cache, log)?;
    let manifest_path = cache.join("manifest.json");
    download(text(manifest, "url")?, &manifest_path, sha1)?;
    let manifest = read_json(&manifest_path)?;
    let files = manifest
        .get("files")
        .and_then(Value::as_object)
        .ok_or("Mojang's Java manifest has no files")?;
    let home = cache.join("runtime");
    let mut downloads = Vec::new();
    let mut links = Vec::new();
    for (name, entry) in files {
        let relative = safe_relative(name)
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| format!("Unsafe path in Mojang's Java manifest: {name}"))?;
        let path = home.join(&relative);
        match text(entry, "type")? {
            "directory" => mkdir(&path)?,
            "file" => {
                let raw = entry
                    .get("downloads")
                    .and_then(|downloads| downloads.get("raw"))
                    .ok_or("Mojang's Java manifest has no raw download for a file")?;
                downloads.push((
                    path,
                    text(raw, "url")?,
                    pin(text(raw, "sha1")?)?,
                    entry.get("executable") == Some(&Value::Bool(true)),
                ));
            }
            "link" => {
                let to = text(entry, "target")?;
                if !link_stays_inside(&relative, to) {
                    return Err(format!(
                        "Unsafe link in Mojang's Java manifest: {name} -> {to}"
                    ));
                }
                links.push((path, to));
            }
            kind => return Err(format!("Unknown Mojang Java manifest entry type: {kind}")),
        }
    }
    log(&format!(
        "Preparing Mojang's Java {} runtime (SHA-1 checked)",
        lock.java_major
    ));
    crate::pool::map(&downloads, |(path, url, sha1, executable)| {
        download(url, path, sha1)?;
        #[cfg(unix)]
        if *executable {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755))
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }
        #[cfg(not(unix))]
        let _ = executable;
        Ok(())
    })?;
    for (path, to) in links {
        if fs::read_link(&path).is_ok_and(|link| link == Path::new(to)) {
            continue;
        }
        mkdir(path.parent().unwrap())?;
        if fs::symlink_metadata(&path).is_ok() {
            fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(to, &path).map_err(|e| format!("{}: {e}", path.display()))?;
        #[cfg(not(unix))]
        return Err(format!(
            "Mojang's Java runtime needs an unsupported link: {} -> {to}; pass --java-home",
            path.display()
        ));
    }
    Ok(if cfg!(target_os = "macos") {
        home.join("jre.bundle/Contents/Home")
    } else {
        home
    })
}

fn link_stays_inside(link: &Path, target: &str) -> bool {
    if target.contains('\\') {
        return false;
    }
    let mut depth = link.components().count() - 1;
    for part in Path::new(target).components() {
        match part {
            std::path::Component::Normal(_) => depth += 1,
            std::path::Component::CurDir => (),
            std::path::Component::ParentDir if depth > 0 => depth -= 1,
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supplied_server_jar_must_match_the_locked_sha1() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let supplied = root.join("Cargo.toml");
        let lock = Lock::read(&root.join("../..")).unwrap();
        let refused = jar(root, &lock, Some(&supplied), &|_| unreachable!()).unwrap_err();
        assert!(
            refused.contains("expected SHA-1") && refused.contains(&supplied.display().to_string()),
            "{refused}"
        );
    }
}
