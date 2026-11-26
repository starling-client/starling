//! Exercise both commands without downloads, using each pin's cached build data.
#[path = "support/sdk.rs"]
mod sdk_fixture;

use sdk_fixture::OfflineSdk;
use std::{fs, path::PathBuf, process::Output};
use sunrise_mod_package::{Manifest, manifest::MANIFEST};

const OLD: &str = "0123456789abcdef0123456789abcdef01234567";
const NEW: &str = "89abcdef0123456789abcdef0123456789abcdef";
/// The manifest's pinned tool, before repinning.
const MANIFEST_MIXINS: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
/// The tool `NEW`'s `sdk.json` names.
const NEW_MIXINS: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c";
const OLD_CATALOG: &str = include_str!("fixtures/internals-old.json");
const NEW_CATALOG: &str = include_str!("fixtures/internals-new.json");

struct Project {
    dir: PathBuf,
    sdk: OfflineSdk,
}
impl Project {
    fn new(name: &str, declarations: &str) -> Self {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/i3-d/tests")
            .join(format!("internals-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(MANIFEST), format!("# A modder's description\nid = 'example'\nname = 'Example'\nversion = '0.1.0'\napi = '0.2'\ncapabilities = ['registries']\n{declarations}\n[native] # native settings\ngame = '26.3'\nsunrise = '{OLD}' # pinned game\nmixins = '{MANIFEST_MIXINS}'\n")).unwrap();
        fs::write(dir.join("Cargo.toml"), "# Keep this comment\n[package]\nname = 'example'\nversion = '0.1.0'\n[dependencies]\nsunrise-mod-api = { path = '.sunrise/sdk/api/crates/sunrise-mod-api' } # API pin\nsunrise-internals = { path = '.sunrise/sdk/api/crates/sunrise-internals' } # facade pin\n").unwrap();
        let sdk = OfflineSdk::new(&dir);
        sdk.release(OLD, MANIFEST_MIXINS, None);
        sdk.release(NEW, NEW_MIXINS, None);
        sdk.prefetch(&dir, OLD, NEW);
        let project = Self { dir, sdk };
        project.catalog(OLD, OLD_CATALOG);
        project.catalog(NEW, NEW_CATALOG);
        project
    }
    fn catalog(&self, revision: &str, text: &str) {
        let dir = self
            .dir
            .join(".sunrise/cache")
            .join(revision)
            .join("build-data");
        fs::create_dir_all(&dir).unwrap();
        // Supply the build's real commit for both bare entry lists and the shared fixtures.
        let text = if text.trim_start().starts_with('[') {
            format!(r#"{{"schema":1,"build":{{"commit":"{revision}"}},"internals":{text}}}"#)
        } else {
            text.replacen(
                r#""commit": "fixture""#,
                &format!(r#""commit": "{revision}""#),
                1,
            )
        };
        // The pin report reads the complete build data. These tests need no bodies or loops.
        let functions = format!(
            r#"{{"schema":1,"build":{{"commit":"{revision}"}},"crates":{{}},"functions":{{}},"refused":{{}}}}"#
        );
        fs::write(dir.join("internals.json"), &text).unwrap();
        fs::write(dir.join("functions.json"), &functions).unwrap();
        // The pin report reads the same files from the signed release, never from this cache.
        let loops = format!(r#"{{"schema":1,"build":{{"commit":"{revision}"}},"loops":[]}}"#);
        let mixins = if revision == OLD {
            MANIFEST_MIXINS
        } else {
            NEW_MIXINS
        };
        self.sdk.release(
            revision,
            mixins,
            Some(&[
                ("internals.json", text.into_bytes()),
                ("functions.json", functions.into_bytes()),
                ("loops.json", loops.into_bytes()),
            ]),
        );
    }
    fn run(&self, args: &[&str]) -> Output {
        self.sdk.command(&self.dir).args(args).output().unwrap()
    }
    fn text(&self) -> String {
        fs::read_to_string(self.dir.join(MANIFEST)).unwrap()
    }
    fn manifest(&self) -> Manifest {
        Manifest::parse(&self.text()).unwrap()
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.dir).unwrap();
    }
}

fn success(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
#[ignore = "daily"]
fn internals_add_writes_pinned_hashes_and_preserves_the_rest_of_the_manifest() {
    let project = Project::new("add", "");
    let output = project.run(&[
        "internals",
        "add",
        "sunrise_client::game::Game",
        "sunrise_client::game::debug",
        "sunrise_client::game::Game",
    ]);
    success(&output);
    let manifest = project.manifest();
    assert_eq!(manifest.internals.len(), 2);
    assert_eq!(manifest.internals[0].hash.as_deref(), Some("1111"));
    // A guarded alternative before the unconditional entry must not shadow it.
    assert_eq!(manifest.internals[1].hash.as_deref(), Some("7777"));
    assert!(
        project
            .text()
            .starts_with("# A modder's description\nid = 'example'")
    );
    assert!(
        project
            .text()
            .contains("[native] # native settings\ngame = '26.3'")
    );
    let before = project.text();
    success(&project.run(&["internals", "add", "sunrise_client::game::Game"]));
    assert_eq!(project.text(), before);
}

#[test]
#[ignore = "daily"]
fn native_sync_refuses_repo() {
    let project = Project::new("sync-native-repo", "");
    let output = project.run(&["internals", "sync", "--repo", "."]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage:"));
}

#[test]
#[ignore = "daily"]
fn internals_add_refuses_missing_gated_and_shape_only_paths_without_partial_edits() {
    let project = Project::new("refusals", "");
    for (path, code, wording) in [
        (
            "sunrise_client::game::no_such_function",
            "E_INTERNAL_MISSING",
            "which this game doesn't have; run cargo sunrise-mod internals sync",
        ),
        (
            "sunrise_client::pad::hardware::PadId",
            "E_INTERNAL_GATED",
            "which exists only under a cfg; target-specific internals aren't supported.",
        ),
        (
            "sunrise_world::World",
            "E_INTERNAL_MISSING",
            "which this game doesn't have; run cargo sunrise-mod internals sync",
        ),
    ] {
        let before = project.text();
        let output = project.run(&["internals", "add", "sunrise_client::game::Game", path]);
        assert!(!output.status.success());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(
            error.contains(&format!("{code}: example uses {path}, {wording}")),
            "{error}"
        );
        assert_eq!(project.text(), before);
    }
    for args in [
        vec!["internals"],
        vec!["internals", "add"],
        vec!["internals", "remove", "path"],
        vec!["internals", "add", "path", "--kind"],
        vec![
            "internals",
            "add",
            "path",
            "--kind",
            "fn",
            "--kind",
            "field",
        ],
        vec!["internals", "add", "path", "--unknown"],
    ] {
        assert!(!project.run(&args).status.success());
    }
}

#[test]
#[ignore = "daily"]
fn pin_diffs_signatures_refreshes_present_hashes_and_keeps_missing_declarations() {
    let project = Project::new(
        "pin",
        "internals = [\n  { path = 'sunrise_client::game::Game', hash = '1111' }, # stable\n  { path = \"sunrise_client::game::Game<'_>::reach\" }, # reach\n  { path = 'sunrise_client::game::Game::creative', hash = '3333' }, # removed\n  { path = 'sunrise_client::game::conditional', hash = '8888' }, # gated now\n  { path = 'sunrise_client::game::restored', hash = '0000' }, # back again\n]\n",
    );
    let report = success(&project.run(&["pin", NEW]));
    assert!(
        report.contains("unchanged: sunrise_client::game::Game\n"),
        "{report}"
    );
    assert!(report.contains("changed: sunrise_client::game::Game<'_>::reach\n  impl Game<'_> { fn reach(&self) -> f64 } → impl Game<'_> { fn reach(&self, hand: Hand) -> f64 }\n"), "{report}");
    assert!(
        report.contains("missing: sunrise_client::game::Game::creative (E_INTERNAL_MISSING:"),
        "{report}"
    );
    assert!(
        report.contains("missing: sunrise_client::game::conditional (E_INTERNAL_GATED:"),
        "{report}"
    );
    assert!(
        report.contains("<unavailable in old game> → const restored: bool = true"),
        "{report}"
    );
    let manifest = project.manifest();
    let native = manifest.native.unwrap();
    assert_eq!(native.sunrise, NEW);
    // An explicit pin of the tool is the mod's own choice: `pin` leaves it.
    assert_eq!(native.mixins.as_deref(), Some(MANIFEST_MIXINS));
    assert_eq!(
        manifest
            .internals
            .iter()
            .map(|i| i.hash.as_deref().unwrap_or_default())
            .collect::<Vec<_>>(),
        // `reach` has no hash: it follows the pin, reported but left without one.
        ["1111", "", "3333", "8888", "9999"]
    );
    for comment in [
        "# stable",
        "# reach",
        "# removed",
        "# gated now",
        "# back again",
        "# pinned game",
    ] {
        assert!(project.text().contains(comment), "{comment}");
    }
    // The SDK is named by path: only the manifest's pin moves.
    let cargo = fs::read_to_string(project.dir.join("Cargo.toml")).unwrap();
    assert!(!cargo.contains(NEW), "{cargo}");
    assert!(cargo.starts_with("# Keep this comment"));
    assert!(cargo.contains("# API pin"));
    assert!(cargo.contains("# facade pin"));
}

#[test]
#[ignore = "daily"]
fn table_declarations_keep_their_format_when_adding_and_refreshing() {
    let project = Project::new(
        "tables",
        "\n[[internals]] # the game\npath = 'sunrise_client::game::Game'\nhash = 'stale' # fingerprint\n",
    );
    success(&project.run(&[
        "internals",
        "add",
        "sunrise_client::game::Game",
        "sunrise_client::game::Game::creative",
    ]));
    assert_eq!(
        project.manifest().internals[0].hash.as_deref(),
        Some("1111")
    );
    assert!(project.text().contains("[[internals]] # the game"));
    assert!(project.text().contains("# fingerprint"));
    let report = success(&project.run(&["pin", NEW]));
    assert!(report.contains("unchanged: sunrise_client::game::Game"));
    assert_eq!(project.manifest().internals.len(), 2);
    assert_eq!(
        project.manifest().native.unwrap().mixins.as_deref(),
        Some(MANIFEST_MIXINS)
    );
    assert!(project.text().contains("# fingerprint"));
}

#[test]
#[ignore = "daily"]
fn internals_add_requires_kind_for_ambiguous_paths_and_pin_keeps_it() {
    let path = "sunrise_client::game::Round::listening";
    for kind in ["fn", "method"] {
        for (form, declarations) in [
            ("inline", String::new()),
            (
                "existing",
                format!("internals = [{{path='{path}',hash='stale'}}]\n"),
            ),
            (
                "tables",
                format!("\n[[internals]]\npath='{path}'\nhash='stale'\n"),
            ),
        ] {
            let project = Project::new(&format!("ambiguous-{kind}-{form}"), &declarations);
            let catalog = serde_json::json!([
                { "path": path, "kind": "field", "signature": "bool", "hash": "field" },
                { "path": path, "kind": kind, "signature": "fn listening(&self) -> bool", "hash": "function" }
            ]);
            project.catalog(OLD, &catalog.to_string());
            let mut new = catalog.clone();
            new[1]["hash"] = "new-function".into();
            new[1]["signature"] = "fn listening(&mut self) -> bool".into();
            project.catalog(NEW, &new.to_string());
            let before = project.text();
            let output = project.run(&["internals", "add", path]);
            assert!(!output.status.success());
            let error = String::from_utf8(output.stderr).unwrap();
            assert!(error.contains("E_INTERNAL_AMBIGUOUS"), "{error}");
            assert!(error.contains(path), "{error}");
            assert!(error.contains(&format!("field, {kind}")), "{error}");
            assert_eq!(project.text(), before);
            success(&project.run(&["internals", "add", path, "--kind", kind]));
            let declaration = &project.manifest().internals[0];
            assert_eq!(declaration.kind.as_deref(), Some(kind));
            assert_eq!(declaration.hash.as_deref(), Some("function"));
            success(&project.run(&["internals", "add", path, "--kind", "field"]));
            let before = project.text();
            success(&project.run(&["internals", "add", "--kind", kind, path]));
            assert_eq!(project.text(), before);
            let report = success(&project.run(&["pin", NEW]));
            assert!(
                report.contains("fn listening(&self) -> bool → fn listening(&mut self) -> bool"),
                "{report}"
            );
            let manifest = project.manifest();
            assert_eq!(manifest.internals.len(), 2);
            assert_eq!(manifest.internals[0].kind.as_deref(), Some(kind));
            assert_eq!(manifest.internals[0].hash.as_deref(), Some("new-function"));
            assert_eq!(manifest.internals[1].kind.as_deref(), Some("field"));
            assert_eq!(manifest.internals[1].hash.as_deref(), Some("field"));
        }
    }
}

#[test]
#[ignore = "daily"]
fn unreadable_new_catalog_does_not_move_either_pin_or_hashes() {
    let project = Project::new(
        "bad-catalog",
        "internals = [{ path = 'sunrise_client::game::Game', hash = '1111' }]\n",
    );
    project.catalog(NEW, "not JSON");
    let manifest = project.text();
    let cargo = fs::read_to_string(project.dir.join("Cargo.toml")).unwrap();
    let output = project.run(&["pin", NEW]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("internals.json"));
    assert_eq!(project.text(), manifest);
    assert_eq!(
        fs::read_to_string(project.dir.join("Cargo.toml")).unwrap(),
        cargo
    );
}

#[test]
#[ignore = "daily"]
fn pin_refuses_a_malformed_mixins_revision_and_changes_no_file() {
    let project = Project::new(
        "bad-mixins",
        "internals = [{ path = 'sunrise_client::game::Game', hash = 'stale' }]\n",
    );
    for rev in ["main", &NEW_MIXINS.to_uppercase(), &NEW_MIXINS[1..]] {
        project.sdk.release(NEW, rev, None);
        // Force pin to read the replacement signed release instead of the cached valid SDK.
        fs::remove_dir_all(project.dir.join(".sunrise/cache").join(NEW)).unwrap();
        let manifest = fs::read(project.dir.join(MANIFEST)).unwrap();
        let cargo = fs::read(project.dir.join("Cargo.toml")).unwrap();
        let output = project.run(&["pin", NEW]);
        assert!(!output.status.success(), "{rev}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("mixins revision"), "{rev}: {error}");
        assert_eq!(fs::read(project.dir.join(MANIFEST)).unwrap(), manifest);
        assert_eq!(fs::read(project.dir.join("Cargo.toml")).unwrap(), cargo);
    }
}
