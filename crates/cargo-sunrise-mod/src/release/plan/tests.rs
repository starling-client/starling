use super::*;
use base64::Engine;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{Value, json};

const SOURCE: &str = "0123456789abcdef0123456789abcdef01234567";
const PUBLISHED: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const GAP: &str = "cccccccccccccccccccccccccccccccccccccccc";
const TIP: &str = "dddddddddddddddddddddddddddddddddddddddd";
const OTHER: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
const ID: &str = "release_probe";
const REPO: &str = "test-author/release_probe";
const LINUX: &str = "x86_64-unknown-linux-gnu";
const WINDOWS: &str = "x86_64-pc-windows-gnu";
const MAC: &str = "aarch64-apple-darwin";
const GAME_TAG: &str = "26.3-0.1.0";
const BUILD_TAG: &str = "26.3-0.1.1";
const GAP_TAG: &str = "26.3-0.9.0";
const TIP_TAG: &str = "26.3-0.10.0";
const OTHER_TAG: &str = "26.3-1.0.0";
const FIXTURES: &str = "../sunrise-mod-package/tests/index/stable-native";

fn stable_list(pairs: &[(&str, &str)]) -> Vec<SunriseRelease> {
    registry::parse_releases(
        &json!(
            pairs
                .iter()
                .map(|(tag, commit)| json!({"tag": tag,"commit": commit}))
                .collect::<Vec<_>>()
        )
        .to_string(),
    )
    .unwrap()
}

fn project(at: &Path) {
    crate::new_native(at, ID, Some(SOURCE), &crate::tests::offline, |text| {
        text.replace("{id}", ID).replace("{name}", "Release probe")
    })
    .unwrap();
}

fn sdk_index(sdk: &Path, build: &str, hosts: &[(&str, sdk::Side)]) -> sdk::Index {
    fs::create_dir_all(sdk).unwrap();
    fs::write(sdk.join(sdk_fetch::SURFACE), r#"{"hash":"ab"}"#).unwrap();
    let archive = include_bytes!("../../../../sunrise-mod-package/src/sdk/fixture.tar.xz");
    fs::write(sdk.join("build-data.tar.xz"), archive).unwrap();
    for &(triple, side) in hosts {
        for kind in [sdk::Kind::Runtime, sdk::Kind::Api] {
            fs::write(sdk.join(sdk::Asset::name(kind, triple, side)), archive).unwrap();
        }
    }
    sdk::Index::of_folder(
        sdk,
        sdk::Build {
            commit: build.into(),
            game: "26.3".into(),
            date: "2026-11-20".into(),
            nightly: None,
        },
        "1.99.1".into(),
    )
    .unwrap()
}

fn signed_sdk(base: &Path, build: &str, hosts: &[(&str, sdk::Side)]) -> PublicKey {
    let tag = format!("sdk-{build}");
    let folder = base.join(&tag);
    let sdk = sdk_index(&folder, build, hosts);
    let text = sdk.to_json();
    fs::write(folder.join(sdk::INDEX), &text).unwrap();
    let sums =
        sunrise_release_sign::sums_file(&[(sdk::INDEX.to_owned(), index::sha256(text.as_bytes()))]);
    fs::write(folder.join(sunrise_release_sign::SUMS), &sums).unwrap();
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
    let message = sunrise_release_sign::message(&tag, sums.as_bytes());
    let signature = base64::engine::general_purpose::STANDARD.encode(pair.sign(&message).as_ref());
    fs::write(folder.join(sunrise_release_sign::SIGNATURE), signature).unwrap();
    pair.public_key().as_ref().try_into().unwrap()
}

#[test]
fn tags_and_pins_must_name_the_same_exact_stable_source() {
    let scratch = tempfile::tempdir().unwrap();
    let at = scratch.path().join(ID);
    project(&at);
    let releases = stable_list(&[(GAME_TAG, SOURCE), ("26.2-0.1.0", SOURCE)]);
    assert_eq!(
        initial(&at, "v0.1.0", &releases).unwrap().source.tag,
        GAME_TAG
    );
    assert!(
        initial(&at, "v0.1", &releases)
            .unwrap_err()
            .contains("v0.1.0")
    );
    assert!(initial(&at, "v0.1.0", &stable_list(&[(GAME_TAG, PUBLISHED)])).is_err());
    assert!(
        initial(
            &at,
            "v0.1.0",
            &stable_list(&[(GAME_TAG, SOURCE), (BUILD_TAG, SOURCE)])
        )
        .is_err()
    );
    fs::write(at.join("Cargo.lock"), "not a lock").unwrap();
    assert!(initial(&at, "v0.1.0", &releases).is_err());
}

#[test]
fn the_matrix_covers_every_sdk_host_but_only_one_produces_source() {
    let scratch = tempfile::tempdir().unwrap();
    let at = scratch.path().join(ID);
    project(&at);
    let releases = stable_list(&[(GAME_TAG, SOURCE)]);
    let hosts = [LINUX, MAC, WINDOWS]
        .into_iter()
        .flat_map(|triple| [sdk::Side::Client, sdk::Side::Server].map(|side| (triple, side)))
        .collect::<Vec<_>>();
    let sdk = sdk_index(&scratch.path().join("sdk"), SOURCE, &hosts);
    let selection = initial(&at, "v0.1.0", &releases).unwrap();
    let plan = complete(selection, sdk).unwrap();
    let hosts: Vec<_> = plan
        .hosts
        .iter()
        .map(|h| (h.triple.as_str(), h.runner, h.side, h.source_producer))
        .collect();
    use sdk::Side::{Client, Server};
    assert_eq!(
        hosts,
        [
            (MAC, "macos-15", Client, false),
            (MAC, "macos-15", Server, false),
            (WINDOWS, "windows-latest", Client, false),
            (WINDOWS, "windows-latest", Server, false),
            (LINUX, "ubuntu-22.04", Client, true),
            (LINUX, "ubuntu-22.04", Server, false),
        ]
    );
    assert_eq!(
        plan.hosts[5].archive.as_deref(),
        Some("release_probe-0.1.0+26.3-0.1.0-x86_64-unknown-linux-gnu-server.tar.xz")
    );
    assert_eq!(
        plan.descriptor.as_deref(),
        Some("release_probe-0.1.0+26.3-0.1.0.binaries.json")
    );
    let selection = initial(&at, "v0.1.0", &releases).unwrap();
    let sdk = sdk_index(
        &scratch.path().join("unknown"),
        SOURCE,
        &[("x86_64-apple-darwin", Client)],
    );
    assert!(
        complete(selection, sdk)
            .unwrap_err()
            .contains("x86_64-apple-darwin")
    );
}

#[test]
fn a_nonstandalone_project_gets_one_source_only_job() {
    use std::io::Write;
    let scratch = tempfile::tempdir().unwrap();
    let at = scratch.path().join(ID);
    project(&at);
    writeln!(
        fs::OpenOptions::new()
            .append(true)
            .open(at.join("Cargo.toml"))
            .unwrap(),
        "log = \"0.4\""
    )
    .unwrap();
    let releases = stable_list(&[(GAME_TAG, SOURCE)]);
    let selection = initial(&at, "v0.1.0", &releases).unwrap();
    let sdk = sdk_index(
        &scratch.path().join("sdk"),
        SOURCE,
        &[
            (MAC, sdk::Side::Client),
            (LINUX, sdk::Side::Client),
            (LINUX, sdk::Side::Server),
        ],
    );
    let plan = complete(selection, sdk).unwrap();
    let [source] = plan.hosts.as_slice() else {
        panic!("expected one host")
    };
    assert!(matches!(plan.release.mode, Mode::SourceOnly { .. }));
    assert!(source.source_producer);
    assert_eq!(source.name, LINUX);
    assert!(source.archive.is_none() && plan.descriptor.is_none());
}

#[test]
fn mod_dependencies_plan_a_source_only_release() {
    let scratch = tempfile::tempdir().unwrap();
    let at = scratch.path().join(ID);
    project(&at);
    let releases = stable_list(&[(GAME_TAG, SOURCE)]);
    let path = at.join(sunrise_mod_package::manifest::MANIFEST);
    let text = fs::read_to_string(&path).unwrap();
    for dependency in ["'1'", "{version = '1', optional = true}"] {
        fs::write(
            &path,
            format!("{text}\n[dependencies]\nbase_mod = {dependency}\n"),
        )
        .unwrap();
        assert!(matches!(
            initial(&at, "v0.1.0", &releases).unwrap().mode,
            Mode::SourceOnly { .. }
        ));
    }
}

struct Registry {
    _dir: tempfile::TempDir,
    checkout: registry::Checkout,
    published: index::Index,
    releases: Vec<SunriseRelease>,
}

impl Registry {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let published = index::Index::parse(
            &fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(FIXTURES)
                    .join("schema-1.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let project = published.project(ID).unwrap();
        let listed = &project.versions[0];
        let source = format!(
            "https://github.com/{REPO}/releases/download/v{}",
            listed.version
        );
        let entry: registry::Entry = serde_json::from_value(json!({
            "id": ID, "kind": "native-mod", "source": project.source, "owners": ["test-author"],
            "owner_ids": [42], "key": project.key,
            "versions": [{"version": listed.version, "published": listed.published, "game": "26.3",
                "sunrise": GAME_TAG, "file": format!("{source}/{}", listed.file.name), "sha256": listed.file.sha256, "signature": listed.file.signature,
                "builds": [{"sunrise": BUILD_TAG, "binaries": format!("{source}/release_probe-1.0.0+26.3-0.1.1.binaries.json"),
                    "sha256": listed.file.sha256, "signature": listed.file.signature}]}]
        })).unwrap();
        fs::create_dir(dir.path().join("mods")).unwrap();
        fs::write(
            dir.path().join("mods/release_probe.toml"),
            entry.to_toml().unwrap(),
        )
        .unwrap();
        let checkout = registry::load(dir.path()).unwrap();
        let releases = stable_list(&[
            (OTHER_TAG, OTHER),
            (TIP_TAG, TIP),
            (GAP_TAG, GAP),
            (BUILD_TAG, PUBLISHED),
            (GAME_TAG, SOURCE),
        ]);
        Self {
            _dir: dir,
            checkout,
            published,
            releases,
        }
    }

    fn select(&self, pair: Option<(&str, &str)>) -> Result<Option<Selection<'_>>, String> {
        rebuild(
            &self.checkout,
            &self.published,
            &self.releases,
            Rebuild {
                id: ID,
                repo: REPO,
                pair,
            },
        )
    }

    fn broke_at(&mut self, label: &str) {
        self.verdict_at(
            label,
            index::FitVerdict::Breaks {
                changed: vec!["hook removed".into()],
            },
        );
    }

    fn verdict_at(&mut self, label: &str, fit: index::FitVerdict) {
        self.published
            .projects
            .iter_mut()
            .find(|p| p.id == ID)
            .unwrap()
            .versions[0]
            .verdicts
            .insert(
                label.into(),
                index::BuildVerdict {
                    fit,
                    targets: Vec::new(),
                },
            );
    }

    fn add_version(&mut self, version: &str) {
        let rows = &mut self.checkout.entries[0].versions;
        let mut copy = rows[0].clone();
        copy.file = copy.file.replace("1.0.0", version);
        copy.version = version.into();
        rows.push(copy);
        let published = &mut self
            .published
            .projects
            .iter_mut()
            .find(|p| p.id == ID)
            .unwrap()
            .versions;
        let mut copy = published[0].clone();
        copy.version = version.into();
        copy.file.name = copy.file.name.replace("1.0.0", version);
        published.push(copy);
    }
}

#[test]
fn rebuild_selects_the_newest_reviewed_version_on_its_lines_tip_without_backfill() {
    let mut registry = Registry::new();
    registry.add_version("1.9.0");
    registry.add_version("1.10.0");
    let latest = registry.select(None).unwrap().unwrap();
    assert_eq!(
        (latest.version.as_str(), latest.target.tag.as_str()),
        ("1.10.0", TIP_TAG)
    );
    let requested = registry.select(Some(("1.9.0", GAP_TAG))).unwrap().unwrap();
    assert_eq!(requested.target.commit, GAP);
    assert!(registry.select(Some(("1.9.0", OTHER_TAG))).is_err());
    for row in &mut registry.checkout.entries[0].versions {
        // A different stable tag for the same compiled commit still covers the tip.
        row.builds[0].sunrise = "26.3-0.8.0".into();
    }
    registry
        .releases
        .extend(stable_list(&[("26.3-0.8.0", TIP)]));
    assert!(registry.select(None).unwrap().is_none());
    assert_eq!(
        registry
            .select(Some(("1.9.0", GAP_TAG)))
            .unwrap()
            .unwrap()
            .target
            .commit,
        GAP
    );
}

#[test]
fn rebuild_skips_unavailable_or_unfit_source_rows() {
    for why in [
        "takedown",
        "source-only",
        "unpublished",
        "broken",
        "moved-pin",
        "withdrawn-build",
        "shorthand",
    ] {
        let mut registry = Registry::new();
        let row = &mut registry.checkout.entries[0].versions[0];
        let version = match why {
            "takedown" => {
                registry.checkout.revoked.push(index::Revoked {
                    id: ID.into(),
                    sha256: Some(row.sha256.clone()),
                    reason: "withdrawn".into(),
                    date: "2026-11-20".into(),
                });
                "1.0.0"
            }
            "source-only" => {
                row.builds.clear();
                "1.0.0"
            }
            "unpublished" => {
                registry.published.projects.retain(|p| p.id != ID);
                "1.0.0"
            }
            "broken" => {
                registry
                    .published
                    .releases
                    .as_mut()
                    .unwrap()
                    .extend(stable_list(&[(TIP_TAG, TIP)]));
                registry.broke_at(TIP_TAG);
                "1.0.0"
            }
            "moved-pin" => {
                registry
                    .releases
                    .iter_mut()
                    .find(|r| r.tag == GAME_TAG)
                    .unwrap()
                    .commit = OTHER.into();
                "1.0.0"
            }
            "withdrawn-build" => {
                registry.releases.retain(|release| release.tag != BUILD_TAG);
                "1.0.0"
            }
            "shorthand" => {
                row.version = "1".into();
                "1"
            }
            _ => unreachable!(),
        };
        assert!(registry.select(None).unwrap().is_none(), "{why}");
        assert!(registry.select(Some((version, TIP_TAG))).is_err(), "{why}");
    }
}

#[test]
fn a_nightly_break_still_applies_to_that_commit_as_a_stable_target() {
    let mut registry = Registry::new();
    registry.published.nightly_build = Some(index::NightlyBuild {
        commit: TIP.into(),
        date: "2026-11-20".into(),
    });
    registry.broke_at("nightly");
    assert!(registry.select(None).unwrap().is_none());
    assert!(registry.select(Some(("1.0.0", TIP_TAG))).is_err());
}

#[test]
fn a_body_change_does_not_schedule_a_rebuild() {
    let mut registry = Registry::new();
    registry
        .published
        .releases
        .as_mut()
        .unwrap()
        .extend(stable_list(&[(TIP_TAG, TIP)]));
    registry.verdict_at(
        TIP_TAG,
        index::FitVerdict::Rebuild {
            review: vec!["game::tick".into()],
        },
    );
    assert!(registry.select(None).unwrap().is_none());
    assert!(registry.select(Some(("1.0.0", TIP_TAG))).is_err());
}

#[test]
fn a_stable_verdict_does_not_follow_a_moved_tag_to_another_commit() {
    let mut registry = Registry::new();
    registry
        .published
        .releases
        .as_mut()
        .unwrap()
        .extend(stable_list(&[(TIP_TAG, GAP)]));
    registry.broke_at(TIP_TAG);
    assert_eq!(registry.select(None).unwrap().unwrap().target.commit, TIP);
    assert_eq!(
        registry
            .select(Some(("1.0.0", TIP_TAG)))
            .unwrap()
            .unwrap()
            .target
            .commit,
        TIP
    );
    // The old breaking commit is still refused if asked for under its other stable tag.
    assert!(registry.select(Some(("1.0.0", GAP_TAG))).is_err());
}

#[test]
fn staging_a_rebuild_authenticates_both_sdks_before_emitting_any_output() {
    let registry = Registry::new();
    let scratch = tempfile::tempdir().unwrap();
    let dir = scratch.path();
    let keys = [SOURCE, TIP].map(|build| signed_sdk(dir, build, &[(LINUX, sdk::Side::Client)]));
    let data = dir.join("sdk-data");
    let out = dir.join("plan.json");
    let base = sdk_fetch::Base::Folder(dir.to_owned());
    let stage_plan =
        |keys: &[PublicKey]| stage(registry.select(None).unwrap(), &base, keys, &data, &out);
    fs::write(&out, b"existing plan\r\n").unwrap();
    assert!(stage_plan(&keys[..1]).is_err());
    assert!(!data.exists());
    assert_eq!(fs::read(&out).unwrap(), b"existing plan\r\n");
    let archive = dir.join(format!("sdk-{TIP}/build-data.tar.xz"));
    let original = fs::read(&archive).unwrap();
    fs::write(&archive, b"changed after signing").unwrap();
    assert!(stage_plan(&keys).is_err());
    assert!(!data.exists());
    assert_eq!(fs::read(&out).unwrap(), b"existing plan\r\n");
    fs::write(archive, original).unwrap();
    stage_plan(&keys).unwrap();
    let output: Value = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
    assert_eq!(output["schema"], 1);
    assert_eq!(output["plan"]["mode"], "rebuild");
    assert_eq!(output["plan"]["author_tag"], "v1.0.0");
    assert_eq!(output["plan"]["reviewed_source"]["owner_ids"], json!([42]));
    assert_eq!(output["plan"]["hosts"][0]["source_producer"], false);
    for build in [SOURCE, TIP] {
        let folder = data.join(build);
        assert!(folder.join("sdk.json").is_file());
        assert!(folder.join("surface.json").is_file());
        assert!(folder.join("build-data/deps/a.rmeta").is_file());
        assert_eq!(fs::read_dir(folder).unwrap().count(), 3);
    }
    let saved = fs::read(&out).unwrap();
    assert!(
        stage_plan(&keys).is_err(),
        "no clobber of existing SDK data"
    );
    assert_eq!(fs::read(&out).unwrap(), saved);
}

#[test]
fn the_plan_output_cannot_replace_any_sdk_data() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let data = root.join("sdk-data");
    let child = data.join(SOURCE);
    let alias = root.join("other");
    fs::create_dir(&alias).unwrap();
    let aliased_data = alias.join("../sdk-data");
    let base = sdk_fetch::Base::Folder(root.to_owned());
    let key = signed_sdk(root, SOURCE, &[(LINUX, sdk::Side::Client)]);
    let at = root.join(ID);
    project(&at);
    let releases = stable_list(&[(GAME_TAG, SOURCE)]);
    let output = aliased_data.join(SOURCE).join("sdk.json");
    // Even when the nested parent will only exist after a successful fetch, refuse overlap first.
    let selection = initial(&at, "v0.1.0", &releases).unwrap();
    let error = stage(Some(selection), &base, &[key], &data, &output).unwrap_err();
    assert!(error.contains("--out"), "{error}");
    assert!(!data.exists());
    fs::create_dir_all(&child).unwrap();
    for name in ["sdk.json", "surface.json"] {
        fs::write(child.join(name), format!("keep {name}\r\n")).unwrap();
    }
    for out in [
        aliased_data.clone(),
        output,
        aliased_data.join(SOURCE).join("surface.json"),
    ] {
        // A scheduled no-op still writes an output file, so it must honor this boundary too.
        let error = stage(None, &base, &[], &data, &out).unwrap_err();
        assert!(error.contains("--out"), "{error}");
    }
    for name in ["sdk.json", "surface.json"] {
        assert_eq!(
            fs::read(child.join(name)).unwrap(),
            format!("keep {name}\r\n").as_bytes()
        );
    }
}

#[cfg(unix)]
#[test]
fn a_dangling_plan_parent_cannot_become_valid_when_sdk_data_is_written() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let data = root.join("sdk-data");
    let alias = root.join("future");
    let target = data.join(SOURCE);
    std::os::unix::fs::symlink(&target, &alias).unwrap();
    let out = alias.join("sdk.json");
    let key = signed_sdk(root, SOURCE, &[(LINUX, sdk::Side::Client)]);
    let at = root.join(ID);
    project(&at);
    let releases = stable_list(&[(GAME_TAG, SOURCE)]);
    let selection = initial(&at, "v0.1.0", &releases).unwrap();
    let error = stage(
        Some(selection),
        &sdk_fetch::Base::Folder(root.to_owned()),
        &[key],
        &data,
        &out,
    )
    .unwrap_err();
    assert!(error.contains("--out"), "{error}");
    assert!(!out.exists());
    assert!(!data.exists());
    assert_eq!(fs::read_link(alias).unwrap(), target);
}

#[test]
fn a_directory_shaped_plan_output_does_not_replace_an_existing_file() {
    let scratch = tempfile::tempdir().unwrap();
    let base = sdk_fetch::Base::Folder(scratch.path().to_owned());
    let data = scratch.path().join("sdk-data");
    let file = scratch.path().join("plan.json");
    fs::write(&file, b"keep this plan\r\n").unwrap();
    for suffix in ["plan.json/", "plan.json/."] {
        let error = stage(None, &base, &[], &data, &scratch.path().join(suffix)).unwrap_err();
        assert!(error.contains("--out"), "{error}");
        assert_eq!(fs::read(&file).unwrap(), b"keep this plan\r\n");
    }
}

#[test]
fn no_rebuild_writes_a_null_plan_without_reading_an_sdk() {
    let scratch = tempfile::tempdir().unwrap();
    let data = scratch.path().join("sdk-data");
    let out = scratch.path().join("plan.json");
    stage(
        None,
        &sdk_fetch::Base::Folder(scratch.path().join("missing")),
        &[],
        &data,
        &out,
    )
    .unwrap();
    let output: Value = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
    assert_eq!(output["schema"], 1);
    assert!(output["plan"].is_null());
    assert!(!data.exists());
}
