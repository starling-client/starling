//! Folder checks and the separate, reproducible registry page bundle.
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};
use sunrise_mod_package::{
    Manifest, Package,
    index::MAX_FILE,
    manifest::is_unresolved,
    page::{Categories, PAGE_FILE, Page},
};

pub(crate) fn check(root: &Path, package: &Package, release: Option<&str>) -> Result<(), String> {
    let manifest = &package.manifest;
    let mut problems = package_problems(package);
    problems.extend(page_problems(
        root,
        manifest,
        release.is_some(),
        fetch_categories,
    ));
    if let Some(tag) = release {
        match release_tag(root, manifest) {
            Ok(expected) if tag != expected => problems.push(format!(
                "release tag {tag:?} does not match sunrise-mod.toml; expected {expected:?}"
            )),
            Err(e) => problems.push(e),
            Ok(_) => {}
        }
    }
    if !problems.is_empty() {
        let report = problems
            .iter()
            .flat_map(|problem| problem.lines())
            .map(|line| format!("needed before release: {line}\n"))
            .collect::<String>();
        if release.is_some() {
            return Err(format!("release check failed:\n{}", report.trim_end()));
        }
        print!("{report}");
    }
    Ok(())
}

/// What the registry needs of the package itself before it lists a release.
fn package_problems(package: &Package) -> Vec<String> {
    let manifest = &package.manifest;
    let mut problems = Vec::new();
    if package.icon.is_none() {
        problems.push(
            "add icon.png beside sunrise-mod.toml: the registry shows it as the mod's icon".into(),
        );
    }
    if manifest.license.trim().is_empty() {
        problems.push("sunrise-mod.toml: set license".into());
    }
    if is_unresolved(&manifest.version) {
        problems.push(format!(
            "sunrise-mod.toml: version {} is a placeholder; set the mod's own version",
            manifest.version
        ));
    }
    problems
}

/// The size of the page bundle [`write_bundle`] writes for files of these sizes: a ustar header
/// and the data padded to 512 bytes for each, then the two zero blocks that end the archive.
fn bundle_size(files: impl IntoIterator<Item = u64>) -> u64 {
    files
        .into_iter()
        .map(|size| 512 + size.div_ceil(512) * 512)
        .sum::<u64>()
        + 1024
}

fn page_problems(
    root: &Path,
    manifest: &Manifest,
    strict: bool,
    fetch_categories: impl FnOnce() -> Result<Categories, String>,
) -> Vec<String> {
    let (page, text) = match Page::read(root, manifest) {
        Ok(Some(read)) => read,
        Ok(None) => return vec![format!("add {PAGE_FILE} beside sunrise-mod.toml")],
        Err(e) => return vec![e],
    };
    let categories = (!page.categories.is_empty()).then(fetch_categories);
    let mut problems =
        page.release_problems(categories.as_ref().and_then(|result| result.as_ref().ok()));
    let mut sizes = BTreeMap::new();
    for shot in &page.screenshots {
        match shot.read(root) {
            Ok(bytes) => {
                sizes.insert(shot.path(), bytes.len() as u64);
            }
            Err(e) => problems.push(e),
        }
    }
    // The registry downloads the bundle within its bound for one release asset.
    let size = bundle_size(std::iter::once(text.len() as u64).chain(sizes.into_values()));
    if size > MAX_FILE {
        problems.push(format!(
            "the page bundle would be {} MiB, over the registry's {} MiB for one release file",
            size.div_ceil(1 << 20),
            MAX_FILE >> 20
        ));
    }
    if let Some(Err(reason)) = categories {
        let reason = reason.lines().next().unwrap_or_default();
        if strict {
            problems.push(format!("cannot check registry categories: {reason}"));
        } else {
            println!("note: skipped the page category check: {reason}");
        }
    }
    problems
}

fn fetch_categories() -> Result<Categories, String> {
    let url = format!(
        "{}/categories.toml",
        sunrise_addresses::MOD_REGISTRY_METADATA
    );
    let response = sunrise_proxy::get(&url, &[], None).map_err(|e| e.to_string())?;
    let bytes = crate::bounded::read_limited(
        response.into_reader(),
        crate::bounded::MAX_MANIFEST,
        Path::new("categories.toml"),
    )?;
    let text = std::str::from_utf8(&bytes).map_err(|e| format!("categories.toml: {e}"))?;
    Categories::parse(text)
}

/// The tag for this version, qualified by mod ID only when the package is in a subfolder.
pub(crate) fn release_tag(root: &Path, manifest: &Manifest) -> Result<String, String> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("could not find the Git repository root: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "could not find the Git repository root: {}",
            String::from_utf8_lossy(&output.stderr).trim_end()
        ));
    }
    let repo = std::str::from_utf8(&output.stdout)
        .map_err(|e| format!("could not read the Git repository root: {e}"))?;
    let repo = fs::canonicalize(repo.trim_end_matches(['\r', '\n']))
        .map_err(|e| format!("could not read the Git repository root: {e}"))?;
    let root = fs::canonicalize(root).map_err(|e| format!("could not read the mod folder: {e}"))?;
    let relative = root
        .strip_prefix(repo)
        .map_err(|_| "the mod folder must be in the Git repository")?;
    let version = &manifest.version;
    if relative.as_os_str().is_empty() {
        Ok(format!("v{version}"))
    } else {
        Ok(format!("{}/v{version}", manifest.id))
    }
}

/// Writes exactly the original page TOML and its named images, with no checkout metadata.
pub(crate) fn write_bundle(
    root: &Path,
    dir: &Path,
    manifest: &Manifest,
) -> Result<Option<PathBuf>, String> {
    let path = dir.join(format!("{}.page.tar", manifest.id));
    let output_error = |e| format!("{}: {e}", path.display());
    if let Err(e) = fs::remove_file(&path)
        && e.kind() != io::ErrorKind::NotFound
    {
        return Err(output_error(e));
    }
    let Some((page, bytes)) = Page::read(root, manifest)? else {
        return Ok(None);
    };
    let mut entries = BTreeMap::from([(PAGE_FILE, None)]);
    entries.extend(
        page.screenshots
            .iter()
            .map(|shot| (shot.path(), Some(shot))),
    );
    let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(output_error)?;
    let mut tar = tar::Builder::new(temp.as_file_mut());
    for (name, shot) in entries {
        let image;
        let data = if let Some(shot) = shot {
            image = shot.read(root)?;
            image.as_slice()
        } else {
            bytes.as_slice()
        };
        let mut header = tar::Header::new_ustar();
        header.set_path(name).map_err(|e| format!("{name}: {e}"))?;
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        tar.append(&header, data).map_err(output_error)?;
    }
    tar.into_inner().map_err(output_error)?;
    let size = temp.as_file().metadata().map_err(output_error)?.len();
    if size > MAX_FILE {
        return Err(format!(
            "{}: {} MiB, over the registry's {} MiB for one release file",
            path.display(),
            size.div_ceil(1 << 20),
            MAX_FILE >> 20
        ));
    }
    temp.persist(&path).map_err(|e| output_error(e.error))?;
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest::parse("id = 'runes'\nname = 'Runes'\nversion = '1.2.3'\n").unwrap()
    }

    #[test]
    fn a_missing_category_list_blocks_only_a_release_check() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("page")).unwrap();
        fs::write(
            root.path().join("page/runes.jpg"),
            include_bytes!("../../sunrise-mod-package/tests/fixtures/page.jpg"),
        )
        .unwrap();
        fs::write(root.path().join(PAGE_FILE), format!("summary = '{}'\ndescription = '{}'\ncategories = ['utility']\n[[screenshots]]\npath = 'page/runes.jpg'\n", "a".repeat(25), "a".repeat(125))).unwrap();
        let unavailable = || Err("fixture unavailable".into());
        assert!(page_problems(root.path(), &manifest(), false, unavailable).is_empty());
        assert_eq!(
            page_problems(root.path(), &manifest(), true, unavailable),
            ["cannot check registry categories: fixture unavailable"]
        );
        assert!(
            page_problems(root.path(), &manifest(), true, || {
                Categories::parse(
                    "[[category]]\nid = 'utility'\nicon = 'hammer'\nkinds = ['mod']\n",
                )
            })
            .is_empty()
        );
    }

    #[test]
    fn checks_without_categories_do_not_fetch_the_catalog() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("page")).unwrap();
        fs::write(
            root.path().join("page/runes.png"),
            include_bytes!("../../sunrise-mod-package/tests/fixtures/page.png"),
        )
        .unwrap();
        fs::write(
            root.path().join(PAGE_FILE),
            format!(
                "summary = '{}'\ndescription = '{}'\n[[screenshots]]\npath = 'page/runes.png'\n",
                "a".repeat(25),
                "a".repeat(125)
            ),
        )
        .unwrap();
        for strict in [false, true] {
            assert!(
                page_problems(root.path(), &manifest(), strict, || {
                    panic!("without categories the check must not fetch the catalog");
                })
                .is_empty()
            );
        }
    }

    #[test]
    fn the_counted_bundle_size_is_the_written_one() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("page")).unwrap();
        let png = include_bytes!("../../sunrise-mod-package/tests/fixtures/page.png");
        fs::write(root.path().join("page/runes.png"), png).unwrap();
        let text = format!(
            "summary = '{}'\ndescription = '{}'\n[[screenshots]]\npath = 'page/runes.png'\n",
            "a".repeat(25),
            "a".repeat(125)
        );
        fs::write(root.path().join(PAGE_FILE), &text).unwrap();
        let out = tempfile::tempdir().unwrap();
        let bundle = write_bundle(root.path(), out.path(), &manifest())
            .unwrap()
            .unwrap();
        assert_eq!(
            fs::metadata(bundle).unwrap().len(),
            bundle_size([text.len() as u64, png.len() as u64])
        );
    }

    #[test]
    fn a_release_needs_an_icon_a_licence_and_a_real_version() {
        let package = |manifest: &str| Package {
            manifest: Manifest::parse(manifest).unwrap(),
            icon: None,
        };
        let problems = package_problems(&package(
            "id = 'runes'\nname = 'Runes'\nversion = '1.2.3'\n",
        ));
        assert_eq!(problems.len(), 2);
        let licensed = package_problems(&package(
            "id = 'runes'\nname = 'Runes'\nversion = '1.2.3'\nlicense = 'MIT'\n",
        ));
        assert_eq!(licensed.len(), 1);
    }

    #[test]
    #[ignore = "daily"]
    fn tags_include_the_mod_id_only_in_a_git_repository_subfolder() {
        let repo = tempfile::tempdir().unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--quiet"])
                .current_dir(repo.path())
                .status()
                .unwrap()
                .success()
        );
        let nested = repo.path().join("packages/different_folder_name");
        fs::create_dir_all(&nested).unwrap();
        let manifest = manifest();
        assert_eq!(release_tag(repo.path(), &manifest).unwrap(), "v1.2.3");
        assert_eq!(release_tag(&nested, &manifest).unwrap(), "runes/v1.2.3");
    }
}
