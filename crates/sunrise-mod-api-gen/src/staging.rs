//! One source staging pass for the launcher and `sunrise-stage`.
use crate::{internals, replace};
/// The build a staged copy is of, named in each build-data file's header.
pub use mod_surface::build_data::Build;
use mod_surface::build_data::Document;
use std::{path::Path, time::Instant};

/// Discovery instrumentation must never be used to build or publish a game.
#[derive(Clone, Copy)]
pub enum Mode {
    /// Widen visibility and give functions their replacement slots,
    /// without diagnostic attributes.
    Build,
    /// Add internal-use diagnostics for an isolated metadata-only mod check.
    Discovery,
}

/// Counts and catalog produced by source staging.
pub struct Staged {
    /// Replacement slot counts.
    pub replace: replace::Staged,
    /// Exposed internal signatures and changed visibility counts.
    pub internals: internals::Widened,
}

/// Expose internals, give functions replacement slots and write the build data
/// (`internals.json`, `loops.json`, `functions.json`), each with a header naming `build`.
/// Catalog only the original game items, before adding the replacement helpers. For lower-crate
/// mixin targets, declarations from literal includes are widened in their own staged files.
/// A pass stamps the copy before edits; even a failed pass requires fresh source for a retry.
pub fn stage(
    root: &Path,
    internals_json: &Path,
    mode: Mode,
    build: &Build,
) -> Result<Staged, String> {
    let stamp = root.join(".sunrise-source-staged");
    if stamp.exists() {
        return Err("source tree is already staged; use a fresh copy of the source".into());
    }
    std::fs::write(
        &stamp,
        "Source staging started. Use a fresh copy to stage again.\n",
    )
    .map_err(|error| format!("{}: {error}", stamp.display()))?;
    // Listed before any pass rewrites the source: the markers are the source's.
    let started = Instant::now();
    #[derive(serde::Serialize)]
    struct Loops {
        loops: Vec<crate::loops::PublishedLoop>,
    }
    let loops = Loops {
        loops: crate::loops::collect(root)?,
    };
    eprintln!(
        "stage_phase=loops seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    // Indexed before any pass edits a file: its spans and hashes are of the source a mixin
    // build downloads.
    let started = Instant::now();
    let generic: std::collections::BTreeSet<String> = loops
        .loops
        .iter()
        .map(|l| crate::functions::key(&l.generic_target()))
        .collect();
    let mut functions = crate::functions::index(root, &generic)?;
    eprintln!(
        "stage_phase=functions seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    write(
        &internals_json.with_file_name(crate::loops::FILE),
        &Document::new(build, loops),
    )?;
    let started = Instant::now();
    let mut internals = match mode {
        Mode::Build => internals::widen(root)?,
        Mode::Discovery => internals::discover(root)?,
    };
    internals
        .entries
        .extend(crate::crossing::foreign_internals(root)?);
    #[derive(serde::Serialize)]
    struct Internals<'a> {
        internals: &'a [internals::Internal],
    }
    write(
        internals_json,
        &Document::new(
            build,
            Internals {
                internals: &internals.entries,
            },
        ),
    )?;
    eprintln!(
        "stage_phase=internals seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    let started = Instant::now();
    let replace = replace::stage(root)?;
    eprintln!(
        "stage_phase=replace seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    functions.retain_slotted(&replace.slots, &generic);
    // One line: the index is the largest build-data file, and only programs read it.
    let path = internals_json.with_file_name(crate::functions::FILE);
    let text = serde_json::to_string(&Document::new(build, &functions))
        .map_err(|error| error.to_string())?;
    std::fs::write(&path, text + "\n").map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(Staged { replace, internals })
}

fn write(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    std::fs::write(path, text + "\n").map_err(|error| format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(root: &Path, internals: &Path, mode: Mode) -> Result<Staged, String> {
        let build = Build {
            commit: "c".repeat(40),
        };
        super::stage(root, internals, mode, &build)
    }

    #[test]
    #[ignore = "daily"]
    fn lower_mixin_copy_calls_private_included_helper() {
        use crate::{functions, patch};
        use std::{fs, process::Command};

        let target_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
        fs::create_dir_all(&target_dir).unwrap();
        let scratch = tempfile::tempdir_in(target_dir).unwrap();
        let out = scratch.path();
        let pristine = out.join("pristine");
        let staged = out.join("staged");
        let lower_src = Path::new("crates/lower/src");
        let selected = lower_src.join("hidden.rs");
        let helper = lower_src.join("fragments/nested/helper.rs");
        let write = |root: &Path, relative: &Path, text: &str| {
            let path = root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        let source = [
            (
                "crates/sunrise-client/Cargo.toml",
                "[package]\nname = 'sunrise-client'\n[dependencies]\n",
            ),
            ("crates/sunrise-client/src/lib.rs", "mod game; mod entry;"),
            ("crates/sunrise-client/src/game.rs", ""),
            ("crates/sunrise-client/src/entry.rs", ""),
            ("crates/sunrise-mod-api-gen/internals.txt", ""),
            (crate::crossing::CROSSING_FOREIGN_FILE, "[]"),
            (
                "Cargo.lock",
                "version = 4\n\n[[package]]\nname = \"lower\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"sunrise-client\"\nversion = \"0.1.0\"\n",
            ),
            (
                "crates/lower/Cargo.toml",
                "[package]\nname = 'lower'\nversion = '0.1.0'\n",
            ),
            ("crates/lower/src/lib.rs", "mod hidden;"),
        ];
        let marker = "#[sunrise_mod_api_macros::replaceable]";
        let marked = format!(
            r#"
{marker}
fn listed(value: u32) -> u32 {{ child::deep(imported(value)) - 1 + pick(0) }}
include!("fragments/outer.rs");
#[cfg(unix)]
include!("fragments/unix.rs");
#[cfg_attr(unix, cfg(not(unix)))]
include!("fragments/other.rs");
mod child {{ include!("fragments/child.rs"); }}
#[cfg(feature = "fast")]
include!("fragments/fast.rs");
#[cfg(not(feature = "fast"))]
include!("fragments/slow.rs");
fn fast_impl(value: u32) -> u32 {{ value }}
fn slow_impl(value: u32) -> u32 {{ value + 1000 }}
"#
        );
        for root in [&pristine, &staged] {
            for (relative, text) in source {
                write(root, Path::new(relative), text);
            }
            write(root, &selected, &marked);
            write(
                root,
                &lower_src.join("fragments/outer.rs"),
                r#"include!("nested/helper.rs");"#,
            );
            write(
                root,
                &helper,
                "fn helper(value: u32) -> u32 { value * 7 }\nuse self::helper as imported;",
            );
            write(
                root,
                &lower_src.join("fragments/unix.rs"),
                r#"include!("nested/unix.rs");"#,
            );
            write(
                root,
                &lower_src.join("fragments/nested/unix.rs"),
                "fn unix_only() {}\nuse std::path::Path as PlatformPath;",
            );
            // A helper only the inline child's fragment declares, visible to its parent.
            write(
                root,
                &lower_src.join("fragments/child.rs"),
                "pub(super) fn deep(value: u32) -> u32 { value + 1 }",
            );
            // The lower crate is built with `fast`, the patch without: the copy must still
            // call what the prebuilt crate's import chose.
            write(
                root,
                &lower_src.join("fragments/fast.rs"),
                "use self::fast_impl as pick;",
            );
            write(
                root,
                &lower_src.join("fragments/slow.rs"),
                "use self::slow_impl as pick;",
            );
            write(
                root,
                &lower_src.join("fragments/other.rs"),
                "fn non_unix_only() {}\nuse std::path::PathBuf as PlatformPath;",
            );
        }

        stage(&staged, &staged.join("internals.json"), Mode::Build).unwrap();
        let text = fs::read_to_string(staged.join(functions::FILE)).unwrap();
        let (_, index): (Build, functions::FunctionIndex) =
            mod_surface::build_data::read(functions::FILE, &text).unwrap();
        let index = index.checked().unwrap();
        let targets = ["lower::hidden::listed".to_owned()];
        let generate = || {
            patch::generate(
                &pristine,
                "fixture_patch",
                &targets,
                &Default::default(),
                &index,
            )
        };
        let generated = generate().unwrap();
        let [(patch_name, patch_source)] = generated.libs.as_slice() else {
            panic!("expected exactly one lower-crate patch library");
        };
        let install = generated.patched[0].install.as_deref().unwrap();

        // The fixture links with rustc alone, so remove only the original no-op marker from
        // the staged file. The generated slot and all visibility edits are left as staged.
        let text = fs::read_to_string(staged.join(&selected)).unwrap();
        fs::write(staged.join(&selected), text.replace(marker, "")).unwrap();
        let rustc = |source: &Path, args: &[&str]| {
            let result = Command::new("rustc")
                .args(["--edition=2024", "--out-dir"])
                .arg(out)
                .args(args)
                .arg(source)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}: {}",
                source.display(),
                String::from_utf8_lossy(&result.stderr)
            );
        };
        rustc(
            &staged.join(lower_src).join("lib.rs"),
            &[
                "--crate-type=rlib",
                "--crate-name=lower",
                "--cfg",
                "feature=\"fast\"",
            ],
        );
        let lower = format!("lower={}", out.join("liblower.rlib").display());
        let patch_file = out.join("patch.rs");
        fs::write(&patch_file, patch_source).unwrap();
        rustc(
            &patch_file,
            &[
                "--crate-type=rlib",
                "--crate-name",
                patch_name,
                "--extern",
                &lower,
            ],
        );
        let patch = format!(
            "{patch_name}={}",
            out.join(format!("lib{patch_name}.rlib")).display()
        );
        let search = format!("dependency={}", out.display());
        write(
            out,
            Path::new("driver.rs"),
            &format!(
                r#"
fn main() {{
    assert_eq!({patch_name}::hidden::listed(6), 42);
    assert_eq!({patch_name}::hidden::helper(6), 42);
    assert!({install});
    assert_eq!(lower::hidden::listed(6), 42);
}}
"#
            ),
        );
        rustc(
            &out.join("driver.rs"),
            &["--extern", &lower, "--extern", &patch, "-L", &search],
        );
        let result = Command::new(out.join(format!("driver{}", std::env::consts::EXE_SUFFIX)))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );

        // The target and its span are unchanged. The mirror must still read and validate the
        // fragment by its indexed path, and give a source diagnostic if it disappears.
        fs::remove_file(pristine.join(&helper)).unwrap();
        let refused = generate().unwrap_err();
        assert_eq!(refused[0].code, "E_MIXIN_SOURCE");
        assert!(refused[0].why.contains("helper.rs"), "{}", refused[0].why);
    }

    #[test]
    fn staging_writes_catalogs_and_slots_and_refuses_reuse() {
        // Each iteration starts from fresh source, not the preceding staged fixture.
        for copy in 0..3 {
            let temporary = tempfile::tempdir().unwrap();
            let root = temporary.path().to_path_buf();
            let source = root.join("crates/sunrise-client/src/game.rs");
            std::fs::create_dir_all(source.parent().unwrap()).unwrap();
            std::fs::write(
                &source,
                "impl Game<'_> { fn unit(&self) {} fn optional(&self) -> Option<u32> { None } }",
            )
            .unwrap();
            std::fs::write(source.parent().unwrap().join("entry.rs"), "").unwrap();
            std::fs::write(
                source.parent().unwrap().join("lib.rs"),
                "mod game; mod entry;\n",
            )
            .unwrap();
            std::fs::write(
                root.join("crates/sunrise-client/Cargo.toml"),
                "[package]\nname = 'sunrise-client'\n[dependencies]\n",
            )
            .unwrap();
            std::fs::write(
                root.join("Cargo.lock"),
                "version = 4\n\n[[package]]\nname = \"sunrise-client\"\nversion = \"0.1.0\"\n",
            )
            .unwrap();
            let lists = root.join("crates/sunrise-mod-api-gen");
            std::fs::create_dir_all(&lists).unwrap();
            std::fs::write(lists.join("internals.txt"), "").unwrap();
            std::fs::write(root.join(crate::crossing::CROSSING_FOREIGN_FILE), "[]").unwrap();
            let output = root.join("internals.json");
            let functions = root.join(crate::functions::FILE);
            if copy == 2 {
                // Refuse retries even when a late output failure leaves edited source.
                std::fs::create_dir(&functions).unwrap();
                assert!(stage(&root, &output, Mode::Build).is_err());
            } else {
                stage(&root, &output, Mode::Build).unwrap();
                let bytes = std::fs::read(&functions).unwrap();
                assert_eq!(bytes.last(), Some(&b'\n'));
                // Every build-data file opens with the header naming the build.
                for file in mod_surface::build_data::FILES {
                    let text = std::fs::read_to_string(root.join(file)).unwrap();
                    let (build, _): (Build, serde_json::Value) =
                        mod_surface::build_data::read(file, &text).unwrap();
                    assert_eq!(build.commit, "c".repeat(40), "{file}");
                }
            }
            let before = std::fs::read(&source).unwrap();
            let text = String::from_utf8_lossy(&before);
            // Even a failure writing the last catalog leaves this copy with replacement slots.
            assert!(text.contains("__fnr_unit") && text.contains("__fnr_optional"));
            for mode in [Mode::Build, Mode::Discovery] {
                let error = stage(&root, &output, mode)
                    .err()
                    .expect("staging twice must fail");
                assert_eq!(
                    error,
                    "source tree is already staged; use a fresh copy of the source"
                );
                assert_eq!(std::fs::read(&source).unwrap(), before);
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
