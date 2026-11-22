//! Open only the exact reviewed source in a rebuild plan. Never run Cargo here.
use super::{output_dir, plan::ReviewedSource};
use crate::{bounded, options::Options};
use serde::Deserialize;
use std::{fs, io::Write, path::Path};
use sunrise_mod_package::{
    Manifest,
    index::{self, SunriseRelease, native::Bundle},
    manifest::{MANIFEST, revision},
    mod_surface::uses::USES_FILE,
};

pub(super) const USAGE: &str =
    "release prepare-rebuild --plan <file> --package <file.smod> --out <new-dir> [--build]";

#[derive(Deserialize)]
struct Envelope {
    schema: u32,
    tool_commit: String,
    plan: Option<Rebuild>,
}

#[derive(Deserialize)]
struct Rebuild {
    mode: String,
    id: String,
    version: String,
    source: SunriseRelease,
    target: SunriseRelease,
    mixins: String,
    reviewed_source: ReviewedSource,
}

pub(super) fn run(args: &[String]) -> Result<(), String> {
    let mut flags = Options::new(
        "release prepare-rebuild",
        args,
        &["--plan", "--package", "--out"],
    )
    .usage(USAGE);
    let mut path = |flag: &str| match flags.values(flag)?.as_slice() {
        [one] => Ok(Path::new(*one)),
        _ => Err(format!("release prepare-rebuild needs one {flag}")),
    };
    let plan = path("--plan")?;
    let package = path("--package")?;
    let out = path("--out")?;
    let build = flags.switch("--build");
    flags.finish()?;
    let raw = bounded::read(plan, 1 << 20)?;
    let envelope: Envelope =
        serde_json::from_str(&raw).map_err(|error| format!("{}: {error}", plan.display()))?;
    if envelope.schema != 1 || envelope.tool_commit != env!("SUNRISE_TOOL_BUILT_FROM") {
        return Err(format!(
            "unsupported release plan schema {} or mismatched planning tool commit {}",
            envelope.schema, envelope.tool_commit
        ));
    }
    let plan = envelope
        .plan
        .filter(|plan| plan.mode == "rebuild")
        .ok_or("expected a rebuild plan")?;
    prepare(&plan, package, out, build)
}

fn prepare(plan: &Rebuild, package: &Path, out: &Path, build: bool) -> Result<(), String> {
    let out = output_dir(out)?;
    match fs::symlink_metadata(&out) {
        Ok(_) => return Err(format!("{} must be a new directory", out.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.to_string()),
    }
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut checked = tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    let reviewed = &plan.reviewed_source;
    let bytes = bounded::read_bytes(package, index::MAX_FILE, bounded::Symlinks::Refuse)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 != reviewed.size || index::sha256(&bytes) != reviewed.sha256 {
        return Err(format!(
            "{} does not match the reviewed size and SHA-256",
            package.display()
        ));
    }
    checked
        .write_all(&bytes)
        .map_err(|error| error.to_string())?;
    drop(bytes);
    index::verify_signature(&reviewed.key, &reviewed.signature, checked.path())?;
    let mut bundle = Bundle::read(checked.path())?;
    sunrise_native_build::prebuilt::release_source(&bundle)?;
    let native = bundle.manifest.native.as_ref().expect("native package");
    if bundle.manifest.id != plan.id
        || super::version(&bundle.manifest)? != plan.version
        || native.sunrise != plan.source.commit
        || index::stable_tag(&plan.source.tag).map(|(game, _)| game) != Some(native.game.as_str())
    {
        return Err(format!(
            "{} is not the reviewed {} {} pinned to Sunrise {}",
            package.display(),
            plan.id,
            plan.version,
            plan.source.tag
        ));
    }
    if build {
        if !revision(&plan.target.commit)
            || !revision(&plan.mixins)
            || index::stable_tag(&plan.target.tag).map(|(game, _)| game)
                != Some(native.game.as_str())
        {
            return Err("the rebuild has an invalid target SDK pin".into());
        }
        let text =
            std::str::from_utf8(&bundle.files[MANIFEST]).map_err(|error| error.to_string())?;
        let mut doc: toml_edit::DocumentMut = text
            .parse()
            .map_err(|error: toml_edit::TomlError| error.to_string())?;
        crate::internals::set_string(
            doc["native"]["sunrise"].as_value_mut().expect("native pin"),
            &plan.target.commit,
        );
        doc["native"]["mixins"] = toml_edit::value(&plan.mixins);
        let text = doc.to_string();
        Manifest::parse(&text)?;
        bundle.files.insert(MANIFEST.to_owned(), text.into_bytes());
    }
    let stage = tempfile::tempdir_in(parent).map_err(|error| error.to_string())?;
    for (name, bytes) in bundle.files {
        if name == USES_FILE || (!build && name != MANIFEST) {
            continue;
        }
        #[cfg(windows)]
        if name.split('/').any(|part| {
            // Win32 strips trailing spaces/dots and can treat DOS device names as files.
            let stem = part
                .split('.')
                .next()
                .unwrap_or(part)
                .trim_end()
                .to_ascii_uppercase();
            part.ends_with([' ', '.'])
                || matches!(
                    stem.as_str(),
                    "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
                )
                || ["COM", "LPT"].into_iter().any(|prefix| {
                    stem.strip_prefix(prefix).is_some_and(|number| {
                        matches!(
                            number,
                            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                        )
                    })
                })
        }) {
            return Err(format!(
                "{name}: a rebuild cannot write a Windows-aliased path"
            ));
        }
        let file = stage.path().join(&name);
        let write = || -> std::io::Result<()> {
            fs::create_dir_all(file.parent().expect("file in the staging directory"))?;
            // Also detects case-folding collisions on case-insensitive hosts.
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&file)?
                .write_all(&bytes)
        };
        write().map_err(|error| format!("{name}: {error}"))?;
    }
    fs::rename(stage.path(), &out).map_err(|error| format!("{}: {error}", out.display()))
}

#[cfg(test)]
mod tests;
