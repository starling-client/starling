use std::{path::Path, process::Command};

/// A starter native mod in a folder whose name has an `=`, with `extra` added to its manifest
/// and `edit` applied to its source, and the tool run in it.
struct Mod {
    _scratch: tempfile::TempDir,
    cache: std::path::PathBuf,
    project: std::path::PathBuf,
}

impl Mod {
    fn new(extra: &str) -> Self {
        let scratch = tempfile::tempdir().unwrap();
        let parent = scratch.path().join("mods=one");
        std::fs::create_dir(&parent).unwrap();
        let mut this = Self {
            cache: scratch.path().join("cache"),
            project: parent.join("greeter"),
            _scratch: scratch,
        };
        let made = this.tool(&parent, &["new", "greeter", "--template", "native"]);
        assert!(
            made.status.success(),
            "`new` failed:\n{}{}",
            String::from_utf8_lossy(&made.stdout),
            String::from_utf8_lossy(&made.stderr)
        );
        let manifest = this.project.join("Cargo.toml");
        let text = std::fs::read_to_string(&manifest).unwrap();
        std::fs::write(&manifest, format!("{text}{extra}")).unwrap();
        this.project = std::fs::canonicalize(&this.project).unwrap();
        this
    }

    fn tool(&self, dir: &Path, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"))
            .current_dir(dir)
            .env("SUNRISE_MOD_CACHE", &self.cache)
            .args(args)
            .output()
            .unwrap()
    }

    /// The mod's source with `change` applied, then built.
    fn build(&self, change: impl Fn(&str) -> String) -> std::process::Output {
        let source = self.project.join("src/lib.rs");
        let template = std::fs::read_to_string(&source).unwrap();
        std::fs::write(&source, change(&template)).unwrap();
        self.tool(&self.project, &["build"])
    }

    /// What a failed build shows names the mod's own file, and no location in the cache.
    fn assert_names_the_authors_file(&self, built: &std::process::Output) {
        assert!(!built.status.success(), "{built:?}");
        let shown = String::from_utf8_lossy(&built.stderr);
        assert!(
            shown.contains(&self.project.join("src/lib.rs").display().to_string()),
            "{shown}"
        );
        // The error's locations; the line naming the build log is in the cache, as it should be.
        let locations: Vec<_> = shown.lines().filter(|line| line.contains("-->")).collect();
        assert!(!locations.is_empty(), "{shown}");
        assert!(
            locations
                .iter()
                .all(|line| !line.contains(&self.cache.display().to_string())),
            "{shown}"
        );
    }
}

fn type_error(source: &str) -> String {
    source.replace("format!(", "{ let wrong: u32 = \"text\"; wrong }; format!(")
}

/// A starter native mod with a type error, built by the tool on the direct route: the error
/// names the mod's own file, not the copy the build compiles in its cache.
#[test]
#[ignore = "book-quickstart.yml: the run's own SDK"]
fn a_failed_native_build_names_the_authors_file_and_not_the_cache_copy() {
    let project = Mod::new("");
    let built = project.build(type_error);
    project.assert_names_the_authors_file(&built);
}

/// The same on the registry route (a crates.io dependency, built by Cargo): it builds when the
/// source is right, and its error names the mod's own file when it is not.
#[test]
#[ignore = "book-quickstart.yml: the run's own SDK"]
fn a_failed_registry_build_names_the_authors_file_and_a_good_one_still_builds() {
    let project = Mod::new("itoa = \"1\"\n");
    let built = project.build(|source| {
        source.replace(
            "fn greeting(player: &str) -> String {",
            "fn greeting(player: &str) -> String {\n    let _ = itoa::Buffer::new().format(7u32);",
        )
    });
    assert!(built.status.success(), "{built:?}");
    assert!(
        project
            .project
            .join("target/sunrise-mod/greeter.smod")
            .is_file()
    );
    let built = project.build(type_error);
    project.assert_names_the_authors_file(&built);
}
