use std::io::{Cursor, Write};
use sunrise_mod_package::index::{Index, resolve::RequestResult, sha256};

mod builds;
mod fit;

/// A 320x180 PNG header: all the screenshot check reads of an image's size.
const SHOT: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\x01\x40\0\0\0\xb4\x08\x06\0\0\0";

struct Registry(tempfile::TempDir);

impl Registry {
    fn new() -> Self {
        let r = Self(tempfile::tempdir().unwrap());
        for dir in [
            "base/mods",
            "head/mods",
            "head/icons",
            "head/screenshots",
            "files",
        ] {
            std::fs::create_dir_all(r.0.path().join(dir)).unwrap();
        }
        r
    }

    fn add(&self, id: &str, version: &str, game: &str, dependencies: &str) -> String {
        self.add_package(id, version, game, "mod", &[(sunrise_mod_package::manifest::MANIFEST,
            &format!("id='{id}'\nname='{id}'\nversion='{version}'\napi='0.2'\ncapabilities=['registries']\n[dependencies]\n{dependencies}")
        )])
    }

    fn add_package(
        &self,
        id: &str,
        version: &str,
        game: &str,
        kind: &str,
        contents: &[(&str, &str)],
    ) -> String {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for &(name, text) in contents {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        let bytes = zip.finish().unwrap().into_inner();
        let hash = sha256(&bytes);
        std::fs::write(self.0.path().join("files").join(&hash), bytes).unwrap();
        let path = self.0.path().join(format!("head/mods/{id}.toml"));
        let mut entry = if path.exists() {
            std::fs::read_to_string(&path).unwrap()
        } else {
            let icon = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x80\0\0\0\x80\x08\x06\0\0\0";
            std::fs::write(self.0.path().join(format!("head/icons/{id}.png")), icon).unwrap();
            self.screenshot(id, SHOT);
            format!(
                "id='{id}'\nkind='{kind}'\nname='{id}'\nsummary='A mod for registry validation.'\n\
                 description='An example package for testing version selection and registry policy.'\n\
                 owners=['author']\nlicense='MIT'\nsource='https://example.test/{id}'\n\
                 icon='icons/{id}.png'\ncategories=['utility']\n\
                 screenshots=['screenshots/{id}/one.png']\n"
            )
        };
        entry.push_str(&format!(
            "\n[[versions]]\nversion='{version}'\npublished='2026-11-18'\ngame='{game}'\nfile='{id}-{version}.smod'\nsha256='{hash}'\n"
        ));
        std::fs::write(path, entry).unwrap();
        hash
    }

    fn pin_source(&self, id: &str, tag: &str) {
        self.edit(id, |t| {
            let repo = format!("https://github.com/author/{id}");
            let entry: sunrise_mod_package::index::registry::Entry = toml::from_str(&t).unwrap();
            let bytes =
                std::fs::read(self.0.path().join("files").join(&entry.versions[0].sha256)).unwrap();
            let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
            let key = pair.pk.to_base64();
            let signature = minisign::sign(
                Some(&pair.pk),
                &pair.sk,
                bytes.as_slice(),
                Some("registry test"),
                None,
            )
            .unwrap()
            .into_string();
            format!(
                "{}sunrise='{tag}'\nsignature='''{signature}'''\n",
                t.replace(&format!("https://example.test/{id}"), &repo)
                    .replace("[[versions]]", &format!("key='{key}'\n\n[[versions]]"))
                    .replace(
                        &format!("file='{id}-1.0.0.smod'"),
                        &format!("file='{repo}/releases/download/v1.0.0/{id}-1.0.0.smod'")
                    )
            )
        });
    }

    fn stable_review(&self) -> Result<(), String> {
        let releases = self.0.path().join("releases.json");
        std::fs::write(
            &releases,
            format!(r#"[{{"tag":"{RELEASE_TAG}","commit":"{RELEASE_PIN}"}}]"#),
        )
        .unwrap();
        self.run(
            "review",
            &["base", "head"],
            &[
                "--author",
                "author",
                "--releases",
                releases.to_str().unwrap(),
            ],
        )
    }

    fn screenshot(&self, id: &str, bytes: &[u8]) {
        let dir = self.0.path().join(format!("head/screenshots/{id}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("one.png"), bytes).unwrap();
    }

    /// Edits the head entry's text.
    fn edit(&self, id: &str, edit: impl Fn(String) -> String) {
        let path = self.0.path().join(format!("head/mods/{id}.toml"));
        std::fs::write(&path, edit(std::fs::read_to_string(&path).unwrap())).unwrap();
    }

    /// The head's mods as they are now become the base.
    fn make_base(&self) {
        let base = self.0.path().join("base/mods");
        std::fs::create_dir_all(&base).unwrap();
        for entry in std::fs::read_dir(self.0.path().join("head/mods")).unwrap() {
            let entry = entry.unwrap().path();
            std::fs::copy(&entry, base.join(entry.file_name().unwrap())).unwrap();
        }
    }

    fn review(&self) -> Result<(), String> {
        self.run(
            "review",
            &["base", "head"],
            &["--author", "author", "--json"],
        )
    }

    fn bare_review(&self) -> Result<(), String> {
        let path = |p| self.0.path().join(p).to_str().unwrap().to_owned();
        super::main(&[
            "review".into(),
            path("base"),
            path("head"),
            "--author".into(),
            "author".into(),
        ])
    }

    fn run(&self, command: &str, paths: &[&str], flags: &[&str]) -> Result<(), String> {
        let path = |p| self.0.path().join(p).to_str().unwrap().to_owned();
        let mut args = vec![command.to_owned()];
        args.extend(paths.iter().copied().map(path));
        args.extend(["--files".to_owned(), path("files")]);
        args.extend(flags.iter().map(|s| (*s).to_owned()));
        super::main(&args)
    }

    fn check(&self) -> Result<(), String> {
        self.run("check", &["base", "head"], &["--json"])
    }

    fn build(&self, flags: &[&str]) -> Result<Index, String> {
        let out = self.0.path().join("index.json");
        let mut args = vec!["--out", out.to_str().unwrap()];
        args.extend_from_slice(flags);
        self.run("build", &["head"], &args)?;
        Index::parse(&std::fs::read_to_string(out).unwrap())
    }
}

const PROJECT_PAGE: &str = "id='example'\nkind='native-mod'\nname='Example'\nsummary='A source example.'\nowners=['author']\n";
const RELEASE_TAG: &str = "26.3-0.1.0";
const RELEASE_PIN: &str = "0123456789abcdef0123456789abcdef01234567";

#[test]
fn a_stable_native_version_must_pin_the_named_release_commit() {
    use sunrise_mod_package::index::check_file;

    let r = Registry::new();
    let manifest = format!(
        "id='coords'\nname='Coords'\nversion='1.0.0'\napi='0.2'\n[native]\ngame='26.3'\nsunrise='{RELEASE_PIN}'\n\
         [[native.module]]\nid='example'\nfile='game.rs'\nname='example'\nsource='src/example.rs'\n"
    );
    let hash = r.add_package(
        "coords",
        "1.0.0",
        "26.3",
        "native-mod",
        &[
            (sunrise_mod_package::manifest::MANIFEST, &manifest),
            (
                "Cargo.toml",
                "[package]\nname='coords'\nversion='1.0.0'\nedition='2024'\n",
            ),
            ("src/lib.rs", ""),
            ("src/example.rs", ""),
        ],
    );
    r.pin_source("coords", RELEASE_TAG);
    let path = r.0.path().join("releases.json");
    let flag = ["--releases", path.to_str().unwrap()];
    let write_release = |tag: &str, commit: &str| {
        std::fs::write(
            &path,
            serde_json::json!([{ "tag": tag, "commit": commit }]).to_string(),
        )
        .unwrap();
    };
    write_release(RELEASE_TAG, &RELEASE_PIN.to_ascii_uppercase());
    assert_eq!(r.run("check", &["base", "head"], &flag), Ok(()));
    let sdk_data = r.0.path().join("sdk");
    fit::sdk(
        &sdk_data,
        RELEASE_PIN,
        mod_surface::SCHEMA,
        "api",
        sunrise_mod_package::manifest::API,
        serde_json::json!({}),
    );
    let build_flags = [flag[0], flag[1], "--sdk-data", sdk_data.to_str().unwrap()];
    let mut index = r.build(&build_flags).unwrap();
    let release = &index.releases.as_ref().unwrap()[0];
    assert_eq!(
        (&*release.tag, &*release.commit, &*release.line),
        (RELEASE_TAG, RELEASE_PIN, "26.3-0")
    );
    let package = r.0.path().join("files").join(&hash);
    let verify = |index: &Index| {
        let p = index.project("coords").unwrap();
        assert_eq!(p.versions[0].sunrise.as_deref(), Some(RELEASE_TAG));
        check_file(index, p, &p.versions[0], &package)
    };
    assert!(verify(&index).is_ok());

    // A row that names a stable release is never published unchecked.
    for unchecked in [
        r.run("check", &["base", "head"], &[]).unwrap_err(),
        r.build(&[]).unwrap_err(),
    ] {
        assert!(
            unchecked.contains("coords 1.0.0") && unchecked.contains(RELEASE_TAG),
            "{unchecked}"
        );
    }

    let wrong = "f".repeat(40);
    index.releases.as_mut().unwrap()[0].commit = wrong.clone();
    write_release(RELEASE_TAG, &wrong);
    let err = r.run("check", &["base", "head"], &flag).unwrap_err();
    assert!(
        err.contains("coords 1.0.0") && err.contains(RELEASE_TAG) && err.contains(RELEASE_PIN),
        "{err}"
    );
    let mismatch = verify(&index).unwrap_err();
    assert!(
        mismatch.contains(RELEASE_TAG) && mismatch.contains(RELEASE_PIN),
        "{mismatch}"
    );

    write_release("26.3-0.2.0", RELEASE_PIN);
    let err = r.build(&flag).unwrap_err();
    assert!(
        err.contains("coords 1.0.0") && err.contains(RELEASE_TAG),
        "{err}"
    );
}

#[test]
fn review_without_packages_checks_stable_release_declarations() {
    let r = Registry::new();
    r.add("coords", "1.0.0", "26.3", "");
    r.make_base();
    let releases = r.0.path().join("releases.json");
    std::fs::write(&releases, "[]").unwrap();
    let review = || {
        super::main(&[
            "review".into(),
            r.0.path().join("base").to_str().unwrap().into(),
            r.0.path().join("head").to_str().unwrap().into(),
            "--author".into(),
            "author".into(),
            "--releases".into(),
            releases.to_str().unwrap().into(),
        ])
    };
    assert_eq!(review(), Ok(())); // Legacy row without a release declaration.
    r.pin_source("coords", RELEASE_TAG);
    r.make_base();
    assert!(review().is_err());
}

#[test]
fn a_nightly_only_project_publishes_its_source_and_supplied_metadata() {
    let r = Registry::new();
    std::fs::write(
        r.0.path().join("head/mods/example.toml"),
        format!("{PROJECT_PAGE}\n[nightly]\nrepo='author/example'\npath='mods/example'\n"),
    )
    .unwrap();
    assert_eq!(r.check(), Ok(()));
    assert_eq!(r.review(), Ok(()));
    let index = r.build(&[]).unwrap();
    let project = index.project("example").unwrap();
    assert!(project.versions.is_empty());
    let source = project.nightly.as_ref().unwrap();
    assert_eq!(
        (&*source.repo, source.path.as_deref()),
        ("author/example", Some("mods/example"))
    );
    assert!(source.metadata.commit.is_none());

    let path = r.0.path().join("nightly.json");
    let metadata = serde_json::json!({"branch": "main", "commit": "ab".repeat(20), "pin": RELEASE_PIN, "updated": "2026-11-20T18:00:00Z"});
    std::fs::write(&path, serde_json::json!({"example": metadata}).to_string()).unwrap();
    let index = r.build(&["--nightly", path.to_str().unwrap()]).unwrap();
    let source = index.project("example").unwrap().nightly.as_ref().unwrap();
    assert_eq!(serde_json::to_value(&source.metadata).unwrap(), metadata);
}

#[test]
fn a_takedown_of_the_whole_project_stops_listing_its_nightly_source() {
    let r = Registry::new();
    let hash = r.add("example", "1.0.0", "26.3", "");
    r.edit("example", |t| {
        t.replacen(
            "\n[[versions]]",
            "\n[nightly]\nrepo='author/example'\n\n[[versions]]",
            1,
        )
    });
    std::fs::write(
        r.0.path().join("head/mods/source_only.toml"),
        format!(
            "{}\n[nightly]\nrepo='author/source'\n",
            PROJECT_PAGE.replace("'example'", "'source_only'")
        ),
    )
    .unwrap();
    let takedowns = |rows: &[(&str, Option<&str>)]| {
        let text: String = rows
            .iter()
            .map(|(id, hash)| {
                let hash = hash.map(|h| format!("sha256='{h}'\n")).unwrap_or_default();
                format!("[[revoked]]\nid='{id}'\n{hash}reason='Withdrawn test project.'\n")
            })
            .collect();
        std::fs::write(r.0.path().join("head/revoked.toml"), text).unwrap();
        r.build(&[]).unwrap()
    };

    // By hash: the named package alone.
    let index = takedowns(&[("example", Some(&hash))]);
    let example = index.project("example").unwrap();
    assert!(example.nightly.is_some() && example.versions.len() == 1);
    assert!(index.project("source_only").unwrap().nightly.is_some());

    // By id: the source goes too, and a project left with nothing is not listed.
    let index = takedowns(&[("example", None), ("source_only", None)]);
    let example = index.project("example").unwrap();
    assert!(example.nightly.is_none() && example.versions.len() == 1);
    assert!(index.project("source_only").is_none() && index.left_out.is_empty());
}

#[test]
fn a_nightly_only_project_keeps_its_images_in_the_registry() {
    let r = Registry::new();
    let write = |images: &str| {
        std::fs::write(
            r.0.path().join("head/mods/example.toml"),
            format!("{PROJECT_PAGE}{images}\n[nightly]\nrepo='author/example'\n"),
        )
        .unwrap();
    };
    let icon = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x80\0\0\0\x80\x08\x06\0\0\0";
    std::fs::write(r.0.path().join("head/icons/example.png"), icon).unwrap();
    r.screenshot("example", SHOT);
    write("icon='icons/example.png'\nscreenshots=['screenshots/example/one.png']");
    assert_eq!(r.bare_review(), Ok(()));
    assert_eq!(r.run("check", &["base", "head"], &[]), Ok(()));
    let index = r.build(&[]).unwrap();
    let project = index.project("example").unwrap();
    assert!(project.icon.is_some() && project.screenshots.len() == 1);

    for (images, path) in [
        (
            "icon='https://example.test/icon.png'",
            "https://example.test/icon.png",
        ),
        (
            "screenshots=['https://example.test/one.png']",
            "https://example.test/one.png",
        ),
        (
            "screenshots=['screenshots/other/one.png']",
            "screenshots/other/one.png",
        ),
        (
            "screenshots=['screenshots/example/sub/one.png']",
            "screenshots/example/sub/one.png",
        ),
    ] {
        write(images);
        assert!(r.bare_review().is_err(), "{path}");
        for result in [
            r.run("check", &["base", "head"], &[]),
            r.build(&[]).map(drop),
        ] {
            let err = result.unwrap_err();
            assert!(err.contains("example:") && err.contains(path), "{err}");
        }
    }
}

#[test]
fn a_project_waiting_to_publish_keeps_its_id_out_of_the_index() {
    let r = Registry::new();
    std::fs::write(r.0.path().join("head/mods/example.toml"), PROJECT_PAGE).unwrap();
    assert_eq!(r.check(), Ok(()));
    assert_eq!(r.review(), Ok(()));
    assert_eq!(r.bare_review(), Ok(()));
    let index = r.build(&[]).unwrap();
    assert!(index.projects.is_empty() && index.left_out.is_empty());

    // It publishes no page, but who holds the id is still checked.
    for (id, page, field) in [
        (
            "submit",
            PROJECT_PAGE.replace("'example'", "'submit'"),
            None,
        ),
        ("example", PROJECT_PAGE.replace("['author']", "[]"), None),
        (
            "example",
            format!("{PROJECT_PAGE}owner_ids=[1, 2]\n"),
            Some("owner_ids"),
        ),
        ("example", PROJECT_PAGE.replace("'author'", "' '"), None),
        (
            "example",
            PROJECT_PAGE.replace("'author'", "'TODO(publish) your login'"),
            None,
        ),
    ] {
        let path = r.0.path().join(format!("head/mods/{id}.toml"));
        std::fs::write(&path, page).unwrap();
        assert!(r.bare_review().is_err(), "{id} {field:?}");
        for result in [
            r.run("check", &["base", "head"], &[]),
            r.build(&[]).map(drop),
        ] {
            let err = result.unwrap_err();
            assert!(
                err.contains(id) && field.is_none_or(|name| err.contains(name)),
                "{err}"
            );
        }
        std::fs::write(r.0.path().join("head/mods/example.toml"), PROJECT_PAGE).unwrap();
        if id != "example" {
            std::fs::remove_file(path).unwrap();
        }
    }
}

#[test]
fn nightly_rejects_unsafe_repos_paths_and_a_branch_key() {
    let r = Registry::new();
    for (fields, problem) in [
        ("repo='https://github.com/author/example'", "nightly.repo"),
        (
            "repo='author/example'\npath='mods/../outside'",
            "nightly.path",
        ),
        (
            "repo='author/example'\nbranch='main'",
            "unknown field `branch`",
        ),
    ] {
        std::fs::write(
            r.0.path().join("head/mods/example.toml"),
            format!("{PROJECT_PAGE}\n[nightly]\n{fields}"),
        )
        .unwrap();
        let err = r.check().unwrap_err();
        assert!(err.contains(problem), "{err}");
    }
}

#[test]
fn new_versions_must_resolve_disjoint_transitive_ranges_together() {
    let r = Registry::new();
    r.add("library", "1.0.0", "26.3", "");
    r.add("library", "2.0.0", "26.3", "");
    r.add("first", "1.0.0", "26.3", "library='^1'");
    r.add("second", "1.0.0", "26.3", "library='^2'");
    r.add("root", "1.0.0", "26.3", "first='^1'\nsecond='^1'");
    let err = r.check().unwrap_err();
    for name in ["root", "library", "first", "second", "^1", "^2"] {
        assert!(err.contains(name), "{err}");
    }
}

#[test]
fn new_versions_need_their_dependencies_on_the_same_minecraft_release() {
    let r = Registry::new();
    r.add("library", "1.0.0", "26.4", "");
    r.add("root", "1.0.0", "26.3", "library='^1'");
    let err = r.check().unwrap_err();
    for name in ["root", "library", "^1", "Minecraft 26.3"] {
        assert!(err.contains(name), "{err}");
    }
}

#[test]
fn old_pairs_do_not_fail_publication_if_newer_releases_avoid_the_cycle() {
    let r = Registry::new();
    r.add("old_a", "1.0.0", "26.3", "old_b='^1'");
    r.add("old_a", "1.1.0", "26.3", "");
    r.add("old_b", "1.0.0", "26.3", "old_a='^1'");
    r.add("old_b", "1.1.0", "26.3", "");
    assert_eq!(r.check(), Ok(()));
}

#[test]
fn taking_down_the_last_matching_file_still_passes_review_check_and_build() {
    let r = Registry::new();
    r.add("library", "2.0.0", "26.3", "");
    let library = r.0.path().join("head/mods/library.toml");
    let kept = std::fs::read_to_string(&library).unwrap();
    // The old file is the only one that matches `root`; another release keeps the project listed.
    let taken = r.add("library", "1.0.0", "26.3", "");
    r.add("root", "1.0.0", "26.3", "library='^1'");
    assert_eq!(r.check(), Ok(()));
    let base = r.0.path().join("base/mods");
    std::fs::create_dir_all(&base).unwrap();
    for id in ["library", "root"] {
        let entry = format!("{id}.toml");
        std::fs::copy(r.0.path().join("head/mods").join(&entry), base.join(&entry)).unwrap();
    }
    std::fs::write(&library, kept).unwrap();
    std::fs::remove_file(r.0.path().join("files").join(&taken)).unwrap();
    std::fs::write(
        r.0.path().join("head/revoked.toml"),
        format!("[[revoked]]\nid='library'\nsha256='{taken}'\nreason='Withdrawn test file.'\n"),
    )
    .unwrap();
    assert_eq!(
        r.run(
            "review",
            &["base", "head"],
            &["--author", "author", "--json"]
        ),
        Ok(())
    );
    assert_eq!(r.check(), Ok(()));
    let out = r.0.path().join("index.json");
    assert_eq!(
        r.run("build", &["head"], &["--out", out.to_str().unwrap()]),
        Ok(())
    );
    let index = Index::parse(&std::fs::read_to_string(out).unwrap()).unwrap();
    let plan = sunrise_mod_package::index::resolve::install(
        &index,
        &[sunrise_mod_package::index::resolve::Request {
            id: "root".into(),
            version: None,
        }],
        &[],
        "26.3",
        sunrise_mod_package::manifest::Side::Client,
        |_, _| true,
    );
    let RequestResult::Refused { reason } = &plan.requested["root"] else {
        panic!("{plan:?}");
    };
    assert!(
        reason.contains("library") && reason.contains("^1"),
        "{reason}"
    );
}

#[test]
fn a_refused_repo_cache_fails_build_check_and_review_without_any_collection() {
    use std::fs;
    use sunrise_mod_package::index::registry::Cache;

    let r = Registry::new();
    let root = r.0.path();
    let out = root.join("index.json");
    let build = || r.run("build", &["head"], &["--out", out.to_str().unwrap()]);
    r.add("following", "1.0.0", "26.3", "");
    build().unwrap();
    let index = Index::parse(&fs::read_to_string(&out).unwrap()).unwrap();
    let cache = Cache {
        versions: index.project("following").unwrap().versions.clone(),
        ..Cache::default()
    };
    let entry = root.join("head/mods/following.toml");
    let inline = fs::read_to_string(&entry).unwrap();
    let (page, _) = inline.split_once("[[versions]]").unwrap();
    fs::write(&entry, format!("{page}repo='owner/following'\n")).unwrap();
    let cache_path = root.join("head/versions/following.json");
    fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
    fs::write(&cache_path, serde_json::to_vec(&cache).unwrap()).unwrap();
    build().unwrap();
    assert!(
        Index::parse(&fs::read_to_string(&out).unwrap())
            .unwrap()
            .project("following")
            .is_some()
    );

    fs::create_dir_all(root.join("base/mods")).unwrap();
    fs::copy(&entry, root.join("base/mods/following.toml")).unwrap();
    let review = || {
        super::main(&[
            "review".into(),
            root.join("base").to_str().unwrap().into(),
            root.join("head").to_str().unwrap().into(),
            "--author".into(),
            "author".into(),
        ])
    };
    fs::write(&cache_path, b"{invalid JSON").unwrap();
    for result in [build(), r.check(), review()] {
        let error = result.unwrap_err();
        assert!(error.contains("following.json"), "{error}");
    }
    fs::remove_file(&cache_path).unwrap();
    fs::create_dir(&cache_path).unwrap();
    let error = build().unwrap_err();
    assert!(error.contains("following.json"), "{error}");
}

#[test]
fn a_registry_command_reads_its_own_options_and_refuses_others() {
    let args = |line: &str| line.split(' ').map(str::to_owned).collect::<Vec<_>>();
    // Each is refused before the checkout is read, naming the option.
    for line in [
        "build repo --files f --out o --relase",
        "review a b --author x --jsno",
        "check a b --files f --jsno",
        "list repo --files f",
        "release repo id file --tag t --published d --nope",
        "page repo id file --nope",
    ] {
        let error = super::main(&args(line)).unwrap_err();
        let option = line.rsplit(' ').find(|w| w.starts_with("--")).unwrap();
        assert!(error.contains(option), "{line}: {error}");
        assert!(error.contains("does not take"), "{line}: {error}");
    }
    // Every option a parser reads is taken, `release --name` among them: the command goes on to
    // the checkout, which is not there.
    for line in [
        "build repo --files f --out o --release r --site s --generated g --counts c",
        "review a b --author x --files f --json",
        "check a b --files f --json",
        "release repo id file --tag t --published d --notes n --signature s --name n --game 26.3",
    ] {
        let error = super::main(&args(line)).unwrap_err();
        assert!(!error.contains("does not take"), "{line}: {error}");
    }
}

#[test]
fn check_refuses_a_malformed_base_checkout() {
    let r = Registry::new();
    r.add("coords", "1.0.0", "26.3", "");
    let base = r.0.path().join("base/mods/broken.toml");
    std::fs::write(base, "id = [").unwrap();
    let error = r.check().unwrap_err();
    assert!(error.contains("mods/broken.toml"), "{error}");
}

#[test]
fn an_unchanged_legacy_row_still_passes_review_check_and_build() {
    let r = Registry::new();
    r.add("coords", "1.0.0", "26.3", "");
    r.make_base();
    r.edit("coords", |t| {
        t.replace("A mod for registry validation.", "Updated project summary.")
    });
    assert_eq!(r.review(), Ok(()));
    assert_eq!(r.check(), Ok(()));
    let index = r.build(&[]).unwrap();
    let versions = &index.project("coords").unwrap().versions;
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].version, "1.0.0");
    assert!(versions[0].sunrise.is_none());
}

#[test]
fn the_last_legacy_row_can_leave_without_a_takedown() {
    let r = Registry::new();
    r.add("coords", "1.0.0", "26.3", "");
    r.make_base();
    r.edit("coords", |t| {
        t.split_once("[[versions]]").unwrap().0.to_owned()
    });
    assert_eq!(r.review(), Ok(()));
    let base = super::registry::load(&r.0.path().join("base")).unwrap();
    let head = super::registry::load(&r.0.path().join("head")).unwrap();
    let review = super::registry::review_checkout(&base, &head, "author");
    assert_eq!(
        review.notes,
        ["coords 1.0.0: leaves the entry, it never named a stable release"]
    );
    assert_eq!(r.check(), Ok(()));
    assert!(r.build(&[]).unwrap().project("coords").is_none());
}

const NEEDS: &str = "needs at least one screenshot";

#[test]
fn a_new_project_needs_a_screenshot_image_in_the_checkout() {
    let r = Registry::new();
    r.add("coords", "1.0.0", "26.3", "");
    assert_eq!(r.check(), Ok(()));
    r.pin_source("coords", RELEASE_TAG);
    assert_eq!(r.stable_review(), Ok(()));
    // Listed but missing, empty or remote: refused by check and review alike.
    for (what, edit) in [
        ("missing", None),
        ("empty", Some(&b""[..])),
        ("not an image", Some(&b"text"[..])),
    ] {
        let r = Registry::new();
        r.add("coords", "1.0.0", "26.3", "");
        let file = r.0.path().join("head/screenshots/coords/one.png");
        match edit {
            None => std::fs::remove_file(&file).unwrap(),
            Some(bytes) => std::fs::write(&file, bytes).unwrap(),
        }
        assert!(r.check().unwrap_err().contains(NEEDS), "{what}");
        r.pin_source("coords", RELEASE_TAG);
        assert!(r.stable_review().is_err(), "{what}");
    }
    let r = Registry::new();
    r.add("coords", "1.0.0", "26.3", "");
    r.edit("coords", |t| {
        t.replace("screenshots/coords/one.png", "https://example.test/one.png")
    });
    assert!(r.check().unwrap_err().contains(NEEDS));
    r.pin_source("coords", RELEASE_TAG);
    assert!(r.stable_review().is_err());
}

#[test]
fn an_existing_listing_without_screenshots_needs_one_only_for_a_new_version() {
    let r = Registry::new();
    for id in ["coords", "second"] {
        r.add(id, "1.0.0", "26.3", "");
        r.edit(id, |t| {
            t.replace(&format!("screenshots=['screenshots/{id}/one.png']\n"), "")
        });
    }
    r.make_base();
    // Untouched: still valid.
    assert_eq!(r.check(), Ok(()));
    assert_eq!(r.review(), Ok(()));
    // A new version of it is refused until it has a screenshot.
    r.add("coords", "1.1.0", "26.3", "");
    assert!(r.check().unwrap_err().contains(NEEDS));
    r.edit("coords", |t| {
        t.replacen(
            "\n[[versions]]",
            "screenshots=['screenshots/coords/one.png']\n\n[[versions]]",
            1,
        )
    });
    assert_eq!(r.check(), Ok(()));
}

#[test]
fn a_library_mod_and_a_collection_need_no_screenshot() {
    let r = Registry::new();
    r.add("coords", "1.0.0", "26.3", "");
    r.edit("coords", |t| {
        t.replace("screenshots=['screenshots/coords/one.png']\n", "")
            .replace("['utility']", "['library']")
    });
    assert_eq!(r.check(), Ok(()));
    // The category must be `library`: another one with no screenshot is refused.
    r.edit("coords", |t| t.replace("['library']", "['utility']"));
    assert!(r.check().unwrap_err().contains(NEEDS));
    r.edit("coords", |t| t.replace("['utility']", "['library']"));
    r.pin_source("coords", RELEASE_TAG);
    assert_eq!(r.stable_review(), Ok(()));
    // A collection of mods that are already listed.
    let r = Registry::new();
    for id in ["coords", "second"] {
        r.add(id, "1.0.0", "26.3", "");
        r.edit(id, |t| {
            t.replace(&format!("screenshots=['screenshots/{id}/one.png']\n"), "")
        });
    }
    r.make_base();
    std::fs::create_dir_all(r.0.path().join("head/collections")).unwrap();
    std::fs::write(
        r.0.path().join("head/collections/pack.toml"),
        "id='pack'\nname='Pack'\nsummary='Some mods together.'\nowners=['author']\nicon='icons/pack.png'\nmembers=[{id='coords'},{id='second'}]\n",
    )
    .unwrap();
    std::fs::copy(
        r.0.path().join("head/icons/coords.png"),
        r.0.path().join("head/icons/pack.png"),
    )
    .unwrap();
    assert_eq!(r.check(), Ok(()));
    assert_eq!(r.review(), Ok(()));
}
