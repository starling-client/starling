use super::{RELEASE_PIN, RELEASE_TAG, Registry, fit};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};
use sunrise_mod_package::{
    index::{FitVerdict, Index, registry, sha256},
    manifest::API,
    sdk,
};

const ID: &str = "release_probe";
const VERSION: &str = "1.0.0";
const TARGET: &str = "x86_64-unknown-linux-gnu";
const BUILD_TAG: &str = "26.3-0.1.1";
const BUILD: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const URL: &str = "https://github.com/test-author/release_probe/releases/download/v1.0.0";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../sunrise-mod-package/tests/index/stable-native")
}

struct Release(Registry);

impl Release {
    fn new(source: &str) -> Self {
        Self::with_signer(source, &fixtures())
    }

    fn with_signer(source: &str, signer: &Path) -> Self {
        let r = Registry::new();
        let root = r.0.path();
        let placeholder = r.add(ID, VERSION, "26.3", "");
        let bytes = fs::read(fixtures().join(source)).unwrap();
        let signature = fs::read_to_string(signer.join(format!("{source}.minisig"))).unwrap();
        let hash = Self::keep(&r, &bytes);
        let key = fs::read_to_string(signer.join("fixture.pub")).unwrap();
        r.edit(ID, |t| {
            t.replace("kind='mod'", "kind='native-mod'")
                .replace(
                    "https://example.test/release_probe",
                    "https://github.com/Test-Author/Release_Probe",
                )
                .replace(
                    "[[versions]]",
                    &format!("key='{}'\n\n[[versions]]", key.trim()),
                )
                .replace(
                    &format!("file='{ID}-{VERSION}.smod'"),
                    &format!("file='{URL}/{ID}-{VERSION}.smod'"),
                )
                .replace(&placeholder, &hash)
                + &format!("sunrise='{RELEASE_TAG}'\nsignature='''{signature}'''\n")
        });
        r.make_base();
        for commit in [RELEASE_PIN, BUILD] {
            let sdk_data = root.join("sdk");
            let dir = sdk_data.join(commit);
            fs::create_dir_all(&dir).unwrap();
            for side in [sdk::Side::Client, sdk::Side::Server] {
                for kind in [sdk::Kind::Runtime, sdk::Kind::Api] {
                    fs::write(
                        dir.join(sdk::Asset::name(kind, TARGET, side)),
                        b"not built in registry CI",
                    )
                    .unwrap();
                }
            }
            fit::sdk(
                &sdk_data,
                commit,
                mod_surface::SCHEMA,
                "api",
                API,
                json!({}),
            );
        }
        fs::write(
            root.join("releases.json"),
            json!([
                {"tag": RELEASE_TAG, "commit": RELEASE_PIN}, {"tag": BUILD_TAG, "commit": BUILD}
            ])
            .to_string(),
        )
        .unwrap();
        for target in [TARGET.to_owned(), format!("{TARGET}-server")] {
            let name = registry::builds::archive_name(ID, VERSION, BUILD_TAG, &target);
            Self::keep(&r, &fs::read(fixtures().join(name)).unwrap());
        }
        Self(r)
    }

    fn keep(r: &Registry, bytes: &[u8]) -> String {
        let hash = sha256(bytes);
        fs::write(r.0.path().join("files").join(&hash), bytes).unwrap();
        hash
    }

    fn add_build(&self, descriptor: &[u8], signature: &str) {
        let hash = Self::keep(&self.0, descriptor);
        self.0.edit(ID, |t| {
            let entry: registry::Entry = toml::from_str(&t).unwrap();
            let name = registry::builds::descriptor_name(ID, &entry.versions[0].version, BUILD_TAG);
            format!(
                "{t}[[versions.builds]]\nsunrise='{BUILD_TAG}'\n\
            binaries='{URL}/{name}'\nsha256='{hash}'\nsignature='''{signature}'''\n"
            )
        });
    }

    fn use_descriptor(&self, name: &str) {
        let filename = format!("{name}.binaries.json");
        self.add_build(
            &fs::read(fixtures().join(&filename)).unwrap(),
            &fs::read_to_string(fixtures().join(format!("{filename}.minisig"))).unwrap(),
        );
    }

    fn flags(&self) -> [String; 4] {
        let root = self.0.0.path();
        [
            "--releases".into(),
            root.join("releases.json").to_str().unwrap().into(),
            "--sdk-data".into(),
            root.join("sdk").to_str().unwrap().into(),
        ]
    }

    fn check(&self) -> Result<(), String> {
        self.0.run(
            "check",
            &["base", "head"],
            &self.flags().each_ref().map(String::as_str),
        )
    }

    fn build(&self) -> Index {
        self.0
            .build(&self.flags().each_ref().map(String::as_str))
            .unwrap()
    }
}

#[test]
fn a_signed_stable_source_can_publish_without_prebuilt_binaries() {
    let release = Release::new("source.smod");
    let index = release.build();
    let version = &index.project(ID).unwrap().versions[0];
    assert_eq!(version.sunrise.as_deref(), Some(RELEASE_TAG));
    assert!(version.file.signature.is_some());
    assert!(version.native_binaries.is_empty());
}

#[test]
fn an_unsigned_stable_source_is_refused_without_prebuilt_binaries() {
    let release = Release::new("source.smod");
    release.0.edit(ID, |text| {
        let mut entry: registry::Entry = toml::from_str(&text).unwrap();
        entry.key = None;
        entry.versions[0].signature = None;
        assert!(entry.versions[0].builds.is_empty());
        entry.to_toml().unwrap()
    });
    assert!(
        release
            .0
            .build(&release.flags().each_ref().map(String::as_str))
            .is_err()
    );
    assert!(!release.0.0.path().join("index.json").exists());
}

#[test]
fn signed_build_appends_to_the_source_and_supplies_exact_fit_on_its_own_sdk() {
    let release = Release::new("source.smod");
    let without = release.build();
    let source = &without.project(ID).unwrap().versions[0];
    let verdict = &source.verdicts[BUILD_TAG];
    assert!(matches!(verdict.fit, FitVerdict::Exact { .. }));
    assert!(verdict.targets.is_empty());
    release.use_descriptor("valid");
    release.check().unwrap();
    let mut with = release.build();
    let version = &with.project(ID).unwrap().versions[0];
    assert_eq!(&version.file, &source.file);
    assert!(matches!(
        version.verdicts[BUILD_TAG].fit,
        FitVerdict::Exact { .. }
    ));
    assert_eq!(
        version.verdicts[BUILD_TAG].targets,
        [TARGET, &format!("{TARGET}-server")]
    );
    assert!(version.verdicts[RELEASE_TAG].targets.is_empty());
    let fixture =
        Index::parse(&fs::read_to_string(fixtures().join("schema-1.json")).unwrap()).unwrap();
    assert_eq!(
        version.native_binaries,
        fixture.project(ID).unwrap().versions[0].native_binaries
    );

    let version = &mut with
        .projects
        .iter_mut()
        .find(|p| p.id == ID)
        .unwrap()
        .versions[0];
    version.verdicts.get_mut(BUILD_TAG).unwrap().fit = FitVerdict::Rebuild {
        review: vec!["game::tick".into()],
    };
    let error = super::super::fit::check_binaries(&with).unwrap_err();
    assert!(
        error.contains("source does not fit: function bodies changed: game::tick"),
        "{error}"
    );
}

#[test]
fn a_stable_source_needs_a_full_version_label() {
    let release = Release::new("source.smod");
    release
        .0
        .edit(ID, |text| text.replace("version='1.0.0'", "version='1.0'"));
    let error = release.check().unwrap_err();
    assert!(
        error.contains("stable source must use the full SemVer"),
        "{error}"
    );
}

#[test]
fn a_signed_descriptor_must_use_the_canonical_label() {
    let fixture = fixtures().join("wrong-architecture");
    let release = Release::with_signer("source.smod", &fixture);
    release.add_build(
        &fs::read(fixture.join("short-label.binaries.json")).unwrap(),
        &fs::read_to_string(fixture.join("short-label.binaries.json.minisig")).unwrap(),
    );
    let error = release.check().unwrap_err();
    assert!(
        error.contains("descriptor id or version does not match the entry"),
        "{error}"
    );
}

#[test]
fn descriptors_must_be_in_the_projects_own_repository() {
    let release = Release::new("source.smod");
    release.use_descriptor("valid");
    release.0.edit(ID, |text| {
        text.replace(
            URL,
            "https://github.com/test-author/another_repo/releases/download/v1.0.0",
        )
    });
    let error = release.check().unwrap_err();
    assert!(
        error.contains("the project's repository Test-Author/Release_Probe"),
        "{error}"
    );
}

#[test]
fn an_owner_append_is_routine_only_when_unmixed_across_the_checkout() {
    let release = Release::new("source.smod");
    let r = &release.0;
    r.add("another", "1.0.0", "26.3", "");
    r.make_base();
    let root = r.0.path();
    let base = root.join("base");
    let head = root.join("head");
    for project in [ID, "another"] {
        for asset in [
            format!("icons/{project}.png"),
            format!("screenshots/{project}/one.png"),
        ] {
            let destination = base.join(&asset);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(head.join(&asset), destination).unwrap();
        }
    }
    release.use_descriptor("valid");
    let before = registry::load(&base).unwrap();
    let status = || {
        let after = registry::load(&head).unwrap();
        let review = registry::review_checkout(&before, &after, "author");
        assert!(review.errors.is_empty(), "{:?}", review.errors);
        review.build_reviews[0].status
    };
    use registry::ReviewStatus::{ReviewRequired, Routine};
    assert_eq!(status(), Routine);
    r.edit("another", |text| {
        text.replace("A mod for registry validation.", "An updated summary.")
    });
    assert_eq!(status(), ReviewRequired);
    fs::copy(
        base.join("mods/another.toml"),
        head.join("mods/another.toml"),
    )
    .unwrap();
    assert_eq!(status(), Routine);
    let collection = head.join("collections/set.toml");
    fs::create_dir_all(collection.parent().unwrap()).unwrap();
    fs::write(&collection, "id='set'\nname='Set'\nsummary='A fixture collection.'\nowners=['author']\nicon='icons/set.png'\nmembers=[{id='another'}]\n").unwrap();
    assert_eq!(status(), ReviewRequired);
    fs::remove_file(collection).unwrap();
    fs::write(
        head.join("revoked.toml"),
        "[[revoked]]\nid='another'\nreason='Withdrawn.'\n",
    )
    .unwrap();
    assert_eq!(status(), ReviewRequired);
}

#[test]
fn taking_down_an_archive_does_not_require_it_or_hide_another_host() {
    let release = Release::new("source.smod");
    release.use_descriptor("valid");
    release.0.make_base();
    let original = release.build();
    let version = &original.project(ID).unwrap().versions[0];
    let client = version.native_binary(BUILD, TARGET).unwrap();
    let root = release.0.0.path();
    let head_path = root.join("head");
    let registry_row = head_path.join(format!("mods/{ID}.toml"));
    let unchanged = fs::read(&registry_row).unwrap();
    fs::write(
        head_path.join("revoked.toml"),
        format!(
            "[[revoked]]\nid='{ID}'\nsha256='{}'\nreason='Withdrawn test archive.'\n",
            client.file.sha256
        ),
    )
    .unwrap();
    let files = root.join("files");
    fs::remove_file(files.join(&client.file.sha256)).unwrap();
    let checkout = registry::load(&head_path).unwrap();
    let assets =
        registry::builds::assets(&checkout.entries, Some(&files), &checkout.revoked).unwrap();
    assert_eq!(assets.len(), 1);
    assert!(assets[0].name.ends_with("-server.tar.xz"));
    release.check().unwrap();
    release.build();
    // The raw writer output must already omit the archive, before tolerant reader filtering.
    let built: Index = serde_json::from_slice(&fs::read(root.join("index.json")).unwrap()).unwrap();
    let row = &built.project(ID).unwrap().versions[0];
    assert_eq!(row.file, version.file);
    assert!(row.native_binary(BUILD, TARGET).is_err());
    assert_eq!(
        row.verdicts[BUILD_TAG].targets,
        [format!("{TARGET}-server")]
    );
    assert_eq!(row.native_binaries, version.native_binaries[1..]);
    assert_eq!(fs::read(registry_row).unwrap(), unchanged);
}

#[test]
fn raw_writer_omits_a_taken_down_archive_from_the_repo_release_cache() {
    let release = Release::new("source.smod");
    release.use_descriptor("valid");
    let original = release.build();
    let version = &original.project(ID).unwrap().versions[0];
    let client = version.native_binary(BUILD, TARGET).unwrap();
    let root = release.0.0.path();
    release.0.edit(ID, |t| {
        let (page, _) = t.split_once("[[versions]]").unwrap();
        page.replace(
            "source='https://github.com/Test-Author/Release_Probe'",
            "repo='Test-Author/Release_Probe'",
        )
    });
    let cache = registry::Cache {
        versions: vec![version.clone()],
        ..Default::default()
    };
    fs::create_dir_all(root.join("head/versions")).unwrap();
    fs::write(
        root.join(format!("head/versions/{ID}.json")),
        serde_json::to_vec(&cache).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("head/revoked.toml"),
        format!(
            "[[revoked]]\nid='{ID}'\nsha256='{}'\nreason='Withdrawn test archive.'\n",
            client.file.sha256
        ),
    )
    .unwrap();
    release.build();
    let built: Index = serde_json::from_slice(&fs::read(root.join("index.json")).unwrap()).unwrap();
    let row = &built.project(ID).unwrap().versions[0];
    assert_eq!(row.file, version.file);
    assert_eq!(row.native_binaries, version.native_binaries[1..]);
    assert_eq!(
        row.verdicts[BUILD_TAG].targets,
        [format!("{TARGET}-server")]
    );
}

#[test]
fn registry_refuses_invalid_signed_build_submissions() {
    for (source, descriptor, message) in [
        ("source.smod", "missing-host", "must cover every SDK host"),
        (
            "source.smod",
            "wrong-host",
            "identity.json does not match host",
        ),
        (
            "source.smod",
            "wrong-archive-signer",
            "signature doesn't match",
        ),
        (
            "no-lock.smod",
            "valid",
            "source package must carry Cargo.lock",
        ),
        ("features.smod", "valid", "E_NATIVE_FEATURES"),
    ] {
        let release = Release::new(source);
        release.use_descriptor(descriptor);
        let error = release.check().unwrap_err();
        assert!(error.contains(message), "{source} {descriptor}: {error}");
    }
}

#[test]
fn a_valid_signature_does_not_accept_a_library_for_the_wrong_architecture() {
    let fixture = fixtures().join("wrong-architecture");
    let release = Release::with_signer("source.smod", &fixture);
    let name = registry::builds::archive_name(ID, VERSION, BUILD_TAG, TARGET);
    Release::keep(&release.0, &fs::read(fixture.join(name)).unwrap());
    release.add_build(
        &fs::read(fixture.join("valid.binaries.json")).unwrap(),
        &fs::read_to_string(fixture.join("valid.binaries.json.minisig")).unwrap(),
    );
    let error = release.check().unwrap_err();
    assert!(error.contains("X86_64 library, found I386"), "{error}");
}

#[test]
fn archive_bytes_must_match_even_in_the_correct_digest_slot() {
    let release = Release::new("source.smod");
    release.use_descriptor("valid");
    let name = registry::builds::archive_name(ID, VERSION, BUILD_TAG, TARGET);
    let mut bytes = fs::read(fixtures().join(name)).unwrap();
    let original = sha256(&bytes);
    bytes[0] ^= 1;
    fs::write(release.0.0.path().join("files").join(original), bytes).unwrap();
    assert!(
        release
            .check()
            .unwrap_err()
            .contains("archive size or SHA-256")
    );
}

#[test]
fn descriptor_authentication_binds_version_build_and_host_before_listing_archives() {
    let bytes = fs::read(fixtures().join("valid.binaries.json")).unwrap();
    let signature = fs::read_to_string(fixtures().join("valid.binaries.json.minisig")).unwrap();
    for (pointer, altered) in [
        ("/version", "1.0.1"),
        ("/build", RELEASE_PIN),
        ("/archives/0/target", "x86_64-unknown-linux-gnu-server"),
    ] {
        let mut descriptor: Value = serde_json::from_slice(&bytes).unwrap();
        *descriptor.pointer_mut(pointer).unwrap() = altered.into();
        let release = Release::new("source.smod");
        release.add_build(&serde_json::to_vec(&descriptor).unwrap(), &signature);
        let error = release.check().unwrap_err();
        assert!(
            error.contains("signature doesn't match"),
            "{pointer}: {error}"
        );
        let root = release.0.0.path();
        let head = registry::load(&root.join("head")).unwrap();
        assert!(
            registry::builds::assets(&head.entries, Some(&root.join("files")), &head.revoked)
                .is_err()
        );
    }
    let release = Release::new("source.smod");
    let other = fs::read_to_string(fixtures().join("other-signer.minisig")).unwrap();
    release.add_build(&bytes, &other);
    assert!(
        release
            .check()
            .unwrap_err()
            .contains("signature doesn't match")
    );
}

#[test]
fn list_supplies_descriptor_then_each_verified_archive_url() {
    let release = Release::new("source.smod");
    release.0.edit(ID, |t| {
        t.replace(
            "source='https://github.com/Test-Author/Release_Probe'",
            "repo='Test-Author/Release_Probe'",
        )
    });
    release.0.make_base();
    release.use_descriptor("valid");
    let root = release.0.0.path();
    let head = registry::load(&root.join("head")).unwrap();
    let assets = registry::builds::assets(&head.entries, None, &head.revoked).unwrap();
    assert_eq!(assets.len(), 1);
    assert!(assets[0].url.ends_with(&format!(
        "/{}",
        registry::builds::descriptor_name(ID, VERSION, BUILD_TAG)
    )));
    assert_eq!(assets[0].size, None);
    let assets =
        registry::builds::assets(&head.entries, Some(&root.join("files")), &head.revoked).unwrap();
    assert_eq!(assets.len(), 2);
    for (asset, target) in assets.iter().zip([TARGET, &format!("{TARGET}-server")]) {
        let name = registry::builds::archive_name(ID, VERSION, BUILD_TAG, target);
        let bytes = fs::read(fixtures().join(&name)).unwrap();
        assert_eq!(asset.url, format!("{URL}/{name}"));
        assert_eq!(asset.size, Some(bytes.len() as u64));
        assert_eq!(asset.sha256, sha256(&bytes));
    }
    release.check().unwrap();
    let built = release.build();
    let project = built.project(ID).unwrap();
    assert_eq!(
        project.source,
        "https://github.com/Test-Author/Release_Probe"
    );
    assert_eq!(project.versions[0].native_binaries.len(), 2);
}

#[test]
fn a_verified_archive_does_not_override_a_known_break() {
    let release = Release::new("source.smod");
    release.use_descriptor("valid");
    let sdk = release.0.0.path().join("sdk").join(BUILD).join(sdk::INDEX);
    let mut value: Value = serde_json::from_slice(&fs::read(&sdk).unwrap()).unwrap();
    value["content_api"] = "999".into();
    fs::write(sdk, value.to_string()).unwrap();
    let error = release.check().unwrap_err();
    assert!(
        error.contains("source does not fit: content-api"),
        "{error}"
    );
}
