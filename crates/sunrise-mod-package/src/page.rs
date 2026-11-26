//! Registry release metadata, shipped separately from the `.smod`.
use crate::{
    Manifest,
    checkout::read_regular,
    manifest::{MAX_MANIFEST, plain_path},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, io::ErrorKind, path::Path};
use url::Url;

/// The page manifest beside `sunrise-mod.toml`.
pub use mod_source::package::PAGE_FILE;
/// The release format `cargo sunrise-mod` speaks: `check --release <tag>` and the `<id>.page.tar` bundle `build`
/// writes. The rolling `mod-tool` release's `tool.json` publishes it as `release_contract`
/// (`tools/ci/mod_tool.py`), so the registry's release workflow can tell a tool that speaks it from an older
/// one. Raise it when either changes in a way that workflow must know.
pub const RELEASE_CONTRACT: u32 = 1;
/// Modrinth's gallery upload limit (`apps/labrinth/src/routes/internal/globals.rs`).
pub const MAX_SCREENSHOT_BYTES: u64 = 5 * 1024 * 1024;
/// The most a screenshot may take decoded. Modrinth decodes each gallery image with the `image` crate's
/// default limits (`apps/labrinth/src/util/img.rs:137`, `image::load_from_memory`), whose `max_alloc` is 512 MiB
/// (`image` 0.25.10, `src/io/limits.rs:54`).
pub const MAX_DECODED_SCREENSHOT_BYTES: u64 = 512 * 1024 * 1024;
// Modrinth's minimums and category cap (`apps/labrinth/src/validate/project/{summary,description,tags}.rs`).
const MIN_SUMMARY: usize = 25;
const MIN_DESCRIPTION: usize = 125;
const MAX_CATEGORIES: usize = 8;

/// The metadata shown on a project's registry page, with the manifest defaults applied.
#[derive(Debug)]
pub struct Page {
    pub summary: String,
    pub description: String,
    pub categories: Vec<String>,
    pub screenshots: Vec<Screenshot>,
    pub name: String,
    pub authors: Vec<String>,
    pub homepage: Option<String>,
    pub issues: Option<String>,
    kind: Kind,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageToml {
    summary: String,
    description: Option<String>,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    screenshots: Vec<Screenshot>,
    name: Option<String>,
    authors: Option<Vec<String>>,
    homepage: Option<String>,
    issues: Option<String>,
}

impl Page {
    /// Parses `page.toml`, applies package defaults, and checks the resolved name, authors and links.
    /// Releasing also requires [`Self::release_problems`] to be empty and every named image valid.
    pub fn parse(text: &str, manifest: &Manifest) -> Result<Self, String> {
        let page: PageToml = toml::from_str(text).map_err(|e| format!("{PAGE_FILE}: {e}"))?;
        let name = page.name.unwrap_or_else(|| manifest.name.clone());
        let authors = page.authors.unwrap_or_else(|| manifest.authors.clone());
        let homepage = page.homepage.or_else(|| manifest.homepage.clone());
        let issues = page.issues.or_else(|| manifest.issues.clone());
        if name.trim().is_empty() {
            return Err(format!("{PAGE_FILE}: name must not be blank"));
        }
        if authors.iter().any(|author| author.trim().is_empty()) {
            return Err(format!("{PAGE_FILE}: author names must not be blank"));
        }
        for (field, link) in [("homepage", &homepage), ("issues", &issues)] {
            if link.as_deref().is_some_and(|link| !valid_web_link(link)) {
                return Err(format!(
                    "{PAGE_FILE}: {field} must be an absolute HTTP(S) URL without credentials"
                ));
            }
        }
        Ok(Self {
            summary: page.summary,
            description: page
                .description
                .unwrap_or_else(|| manifest.description.clone()),
            categories: page.categories,
            screenshots: page.screenshots,
            name,
            authors,
            homepage,
            issues,
            kind: if manifest.native.is_some() {
                Kind::NativeMod
            } else {
                Kind::Mod
            },
        })
    }

    /// Reads the page and its original TOML bytes for bundling. `None` means it does not exist.
    pub fn read(root: &Path, manifest: &Manifest) -> Result<Option<(Self, Vec<u8>)>, String> {
        let bytes = match read_regular(root, Path::new(PAGE_FILE), MAX_MANIFEST) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("{PAGE_FILE}: {e}")),
        };
        let text = std::str::from_utf8(&bytes).map_err(|e| format!("{PAGE_FILE}: {e}"))?;
        Ok(Some((Self::parse(text, manifest)?, bytes)))
    }

    /// Lists release prerequisites not met by the metadata. `None` skips category membership
    /// when the caller has no registry list. Check the named files separately with [`Screenshot::read`]
    /// for a checkout or [`Screenshot::check_image`] for an archive.
    pub fn release_problems(&self, categories: Option<&Categories>) -> Vec<String> {
        let mut problems = Vec::new();
        for (field, text, min) in [
            ("summary", &self.summary, MIN_SUMMARY),
            ("description", &self.description, MIN_DESCRIPTION),
        ] {
            if text.trim().chars().count() < min {
                problems.push(format!(
                    "{PAGE_FILE}: {field} needs at least {min} characters"
                ));
            }
        }
        if self.screenshots.is_empty() {
            problems.push(format!(
                "{PAGE_FILE}: add at least one [[screenshots]] entry"
            ));
        }
        // The registry publishes each screenshot at its path below `page/`, beside the package's
        // `icon.png`: each needs a destination of its own that every host can create.
        let mut destinations = HashSet::new();
        for shot in &self.screenshots {
            let destination = shot.path.strip_prefix("page/").unwrap_or(&shot.path);
            if !destinations.insert(destination) {
                problems.push(format!("{PAGE_FILE}: {} is listed twice", shot.path));
            } else if destination == "icon.png" {
                problems.push(format!(
                    "{PAGE_FILE}: {} would replace the package icon on the page",
                    shot.path
                ));
            } else if !portable_path(destination) {
                problems.push(format!(
                    "{PAGE_FILE}: {} is not a path Windows can create",
                    shot.path
                ));
            }
        }
        problems.extend(
            category_problems(
                self.categories.iter().map(String::as_str),
                self.kind,
                categories,
            )
            .into_iter()
            .map(|problem| format!("{PAGE_FILE}: {problem}")),
        );
        problems
    }
}

/// Whether a resolved page link is an absolute HTTP(S) URL without credentials or whitespace.
pub fn valid_web_link(link: &str) -> bool {
    if !(link.starts_with("http://") || link.starts_with("https://"))
        || link.contains('\\')
        || link.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return false;
    }
    Url::parse(link)
        .is_ok_and(|url| url.has_host() && url.username().is_empty() && url.password().is_none())
}

/// Whether every component of `path` is a name Windows can create: no reserved device name
/// (`CON`, `COM1`...), no character Win32 refuses, no trailing dot or space, and no `.git`.
/// The registry bot publishes page images under these rules.
pub fn portable_path(path: &str) -> bool {
    path.split('/').all(|part| {
        if part.contains(['<', '>', '"', '|', '?', '*', ':'])
            || part.ends_with(['.', ' '])
            || part.eq_ignore_ascii_case(".git")
        {
            return false;
        }
        let stem = part
            .split('.')
            .next()
            .unwrap_or(part)
            .trim_end()
            .to_ascii_uppercase();
        !matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) && !stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|n| {
                matches!(
                    n,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
    })
}

/// One image for the registry page. The path uses forward slashes and stays below `page/`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Screenshot {
    #[serde(deserialize_with = "screenshot_path")]
    path: String,
    pub caption: Option<String>,
}

fn screenshot_path<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    use serde::de::Error;
    let path = String::deserialize(d)?;
    if !path.starts_with("page/") || !plain_path(&path) || path.contains('\0') {
        return Err(D::Error::custom(format!(
            "{path:?}: expected a relative screenshot file under page/ without . or .. components"
        )));
    }
    // Release check must also refuse paths that cannot be stored in the required bundle format.
    tar::Header::new_ustar()
        .set_path(&path)
        .map_err(|e| D::Error::custom(format!("{path:?}: not a ustar path: {e}")))?;
    Ok(path)
}

impl Screenshot {
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Validates the bytes of a named screenshot (for example, from an archive).
    pub fn check_image(&self, bytes: &[u8]) -> Result<(), String> {
        check_screenshot(Path::new(&self.path), bytes).map_err(|e| format!("{}: {e}", self.path))
    }

    /// Reads and checks the image within [`MAX_SCREENSHOT_BYTES`], without following links.
    pub fn read(&self, root: &Path) -> Result<Vec<u8>, String> {
        let bytes = read_regular(root, Path::new(&self.path), MAX_SCREENSHOT_BYTES)
            .map_err(|e| format!("{}: {e}", self.path))?;
        self.check_image(&bytes)?;
        Ok(bytes)
    }
}

/// Checks an encoded PNG or JPEG screenshot at either an author's or a registry's path.
/// The extension must match the image format. The size limit also applies to callers reading
/// bytes from an archive instead of using [`Screenshot::read`].
pub fn check_screenshot(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() as u64 > MAX_SCREENSHOT_BYTES {
        return Err("screenshot exceeds 5 MiB".into());
    }
    let extension = path.extension().and_then(|ext| ext.to_str());
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        if extension != Some("png") {
            return Err("PNG screenshots must have a .png extension".into());
        }
        return check_png(bytes).map_err(|e| format!("unreadable PNG screenshot: {e}"));
    }
    if bytes.starts_with(b"\xff\xd8\xff") {
        if !matches!(extension, Some("jpg" | "jpeg")) {
            return Err("JPEG screenshots must have a .jpg or .jpeg extension".into());
        }
        // The JPEG decoder can recover at EOF even in strict mode. Require the end marker
        // as well so a truncated scan is not published as a complete screenshot.
        if !bytes.ends_with(b"\xff\xd9") {
            return Err("unreadable JPEG screenshot: missing end-of-image marker".into());
        }
        let options =
            zune_jpeg::zune_core::options::DecoderOptions::new_safe().set_strict_mode(true);
        let mut decoder = zune_jpeg::JpegDecoder::new_with_options(bytes, options);
        decoder
            .decode_headers()
            .map_err(|e| format!("unreadable JPEG screenshot: {e}"))?;
        // The decoder allocates the whole picture before it reads the scan, and may allocate its
        // coefficients as well, so refuse what would not fit the decoding budget from
        // its header alone, before decoding it.
        let output = decoder.output_buffer_size().unwrap_or(usize::MAX) as u64;
        let info = decoder
            .info()
            .ok_or("unreadable JPEG screenshot: no frame header")?;
        let decoded = output.saturating_add(coefficient_planes(&info));
        if decoded > MAX_DECODED_SCREENSHOT_BYTES {
            return Err(format!(
                "JPEG screenshot needs {decoded} bytes to decode, over {MAX_DECODED_SCREENSHOT_BYTES}"
            ));
        }
        decoder
            .decode()
            .map_err(|e| format!("unreadable JPEG screenshot: {e}"))?;
        // The decoder recovers a scan cut short (zero bits for the blocks it lacks) and does not
        // report it, so the scans are read as well: they cover the picture, and each holds all
        // of its blocks.
        return jpeg::check_complete(bytes).map_err(|e| format!("unreadable JPEG screenshot: {e}"));
    }
    Err("not a PNG or JPEG screenshot".into())
}

/// The most a JPEG decoder holds besides the picture: an `i16` coefficient for every sample of
/// every component, over the frame padded to whole MCUs. zune-jpeg 0.4.21 holds these for a
/// progressive JPEG (`mcu_prog.rs:98-105`) and for a sequential one whose first scan lacks a
/// component (`mcu.rs:172-183`), which the header cannot tell, so every JPEG is counted. JPEG
/// allows sampling factors up to 4 (ITU T.81 B.2.2), so an MCU is at most 32 samples on a side.
fn coefficient_planes(info: &zune_jpeg::ImageInfo) -> u64 {
    let padded = |side: u16| u64::from(side).div_ceil(32) * 32;
    padded(info.width) * padded(info.height) * u64::from(info.components) * 2
}

fn check_png(bytes: &[u8]) -> Result<(), png::DecodingError> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info()?;
    let frames = reader
        .info()
        .animation_control
        .as_ref()
        .map_or(1, |animation| {
            u64::from(animation.num_frames) + u64::from(reader.info().frame_control.is_none())
        });
    // Validate the pixel data without allocating a buffer for the whole image.
    for frame in 0..frames {
        if frame != 0 {
            reader.next_frame_info()?;
        }
        while reader.next_row()?.is_some() {}
    }
    reader.finish()
}

/// A project kind as named by the registry catalog.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Mod,
    NativeMod,
    Modpack,
    Resourcepack,
    Shader,
    Shaderpack,
    Datapack,
}

/// The category catalog published by the registry as `registry/categories.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Categories {
    pub category: Vec<Category>,
}

/// One registry category, its Lucide icon and the project kinds it applies to.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Category {
    pub id: String,
    pub icon: String,
    pub kinds: Vec<Kind>,
}

/// Validates the category selection on either an author page or a resolved registry page.
/// Without a catalog, only the count and uniqueness can be checked.
pub fn category_problems<'a>(
    ids: impl ExactSizeIterator<Item = &'a str>,
    kind: Kind,
    catalog: Option<&Categories>,
) -> Vec<String> {
    let mut problems = Vec::new();
    if ids.len() > MAX_CATEGORIES {
        problems.push(format!("choose at most {MAX_CATEGORIES} categories"));
    }
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(id) {
            problems.push(format!("duplicate registry category {id:?}"));
            continue;
        }
        if let Some(catalog) = catalog {
            match catalog.category.iter().find(|row| row.id == id) {
                None => problems.push(format!("unknown registry category {id:?}")),
                Some(row) if !row.kinds.contains(&kind) => {
                    problems.push(format!(
                        "registry category {id:?} does not apply to this project kind"
                    ));
                }
                Some(_) => {}
            }
        }
    }
    problems
}

impl Categories {
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| format!("categories.toml: {e}"))
    }
}

mod jpeg;

#[cfg(test)]
mod tests;
