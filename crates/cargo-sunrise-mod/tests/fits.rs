//! `check --build` and `pin` against offline SDKs: no downloads.
#[path = "support/sdk.rs"]
mod sdk_fixture;

use mod_surface::uses::{PLAIN_SCHEMA, USES_FILE, UsesTable};
use sdk_fixture::OfflineSdk;
use std::{collections::BTreeSet, fs, path::PathBuf, process::Output};
use sunrise_mod_package::{Manifest, Package, manifest::MANIFEST, sdk};

const OLD: &str = "0123456789abcdef0123456789abcdef01234567";
const NEW: &str = "89abcdef0123456789abcdef0123456789abcdef";
const MIXINS: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c";

const STEADY: &str = "sunrise_client::game::Game";
const RESHAPED: &str = "sunrise_client::game::Settings";
const DROPPED: &str = "sunrise_client::game::Game::creative";
/// A method: its fingerprint needs its body, which these builds' function indexes do not hold.
const BODILESS: &str = "sunrise_client::game::Game<'_>::reach";
const CHANGED_IDS: &[&str] = &[
    "any:sunrise_client::game::Game::creative",
    "any:sunrise_client::game::Settings",
];

struct Project {
    dir: PathBuf,
    sdk: OfflineSdk,
}

impl Project {
    fn new(name: &str, internals: &[&str]) -> Self {
        Self::made(name, internals, mod_surface::SCHEMA)
    }

    /// A project pinned to `OLD`, whose SDK's surface has schema `old_schema`.
    fn made(name: &str, internals: &[&str], old_schema: u32) -> Self {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/i3-d/tests")
            .join(format!("fit-check-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let declared = internals
            .iter()
            .map(|path| format!("  {{ path = \"{path}\" }},\n"))
            .collect::<String>();
        fs::write(
            dir.join(MANIFEST),
            format!(
                "id = 'example'\nname = 'Example'\nversion = '0.1.0'\napi = '0.2'\ncapabilities = ['registries']\n\
                 internals = [\n{declared}]\n[native]\ngame = '26.3'\nsunrise = '{OLD}'\n"
            ),
        )
        .unwrap();
        let sdk = OfflineSdk::new(&dir);
        let old = [
            (STEADY, "struct", "1111"),
            (RESHAPED, "struct", "2222"),
            (DROPPED, "field", "3333"),
            (BODILESS, "method", "4444"),
        ];
        let new = [
            (STEADY, "struct", "1111"),
            (RESHAPED, "struct", "2299"),
            (BODILESS, "method", "4499"),
        ];
        if old_schema != mod_surface::SCHEMA {
            let folder = dir.join("releases").join(format!("sdk-{OLD}"));
            fs::create_dir_all(&folder).unwrap();
            let surface = format!(r#"{{"schema":{old_schema},"hash":"ab"}}"#);
            fs::write(folder.join("surface.json"), surface).unwrap();
        }
        sdk.release(OLD, MIXINS, Some(&build_data(OLD, &old)));
        sdk.release(NEW, MIXINS, Some(&build_data(NEW, &new)));
        // `sdk diff` reads both surfaces, so a surface this tool does not read is fetched by the
        // command under test.
        if old_schema == mod_surface::SCHEMA {
            sdk.prefetch(&dir, OLD, NEW);
        }
        Self { dir, sdk }
    }

    fn run(&self, args: &[&str]) -> Output {
        self.sdk.command(&self.dir).args(args).output().unwrap()
    }
}

/// `build`'s internals catalog and an empty function index: the files of its signed SDK's build
/// data.
fn build_data(build: &str, internals: &[(&str, &str, &str)]) -> Vec<(&'static str, Vec<u8>)> {
    let listed = internals
        .iter()
        .map(|(path, kind, hash)| {
            serde_json::json!({"path": path, "kind": kind, "signature": path, "hash": hash})
        })
        .collect::<Vec<_>>();
    let catalog = serde_json::json!({"schema": 1, "build": {"commit": build}, "internals": listed});
    let functions = format!(
        r#"{{"schema":1,"build":{{"commit":"{build}"}},"crates":{{}},"functions":{{}},"refused":{{}}}}"#
    );
    let loops = format!(r#"{{"schema":1,"build":{{"commit":"{build}"}},"loops":[]}}"#);
    vec![
        ("internals.json", catalog.to_string().into_bytes()),
        ("functions.json", functions.into_bytes()),
        ("loops.json", loops.into_bytes()),
    ]
}

impl Drop for Project {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.dir).unwrap();
    }
}

fn assert_answer(output: Output, code: i32, game_ids: &[&str]) -> String {
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(code), "{stdout}{stderr}");
    let reported: BTreeSet<_> = stdout
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|id| id.contains("sunrise_client::") || *id == "reach")
        .collect();
    assert_eq!(
        reported,
        game_ids.iter().copied().collect(),
        "{stdout}{stderr}"
    );
    stdout
}

fn assert_build_order(stdout: &str) {
    let builds: Vec<_> = stdout
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|id| id.len() == 7 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .collect();
    assert_eq!(builds, ["0123456", "89abcde"], "{stdout}");
}

#[test]
#[ignore = "daily"]
fn check_with_builds_judges_declared_game_code_by_each_builds_data() {
    let steady = Project::new("steady", &[STEADY]);
    assert_answer(steady.run(&["check", "--build", NEW]), 0, &[]);
    let stdout = assert_answer(
        steady.run(&["check", "--build", OLD, "--build", NEW]),
        0,
        &[],
    );
    assert_build_order(&stdout);

    let moved = Project::new("moved", &[STEADY, RESHAPED, DROPPED]);
    assert_answer(moved.run(&["check", "--build", NEW]), 1, CHANGED_IDS);
    // The build it was made for still fits; one build that does not decides the answer.
    assert_answer(moved.run(&["check", "--build", OLD]), 0, &[]);
    let stdout = assert_answer(
        moved.run(&["check", "--build", OLD, "--build", NEW]),
        1,
        CHANGED_IDS,
    );
    assert_build_order(&stdout);
    // A build with no SDK is not known, and the other builds are still judged, in either order:
    // a known misfit decides the answer, otherwise the unknown one does.
    const MISSING: &str = "fedcba9876543210fedcba9876543210fedcba98";
    for builds in [[MISSING, NEW], [NEW, MISSING]] {
        let args = ["check", "--build", builds[0], "--build", builds[1]];
        assert_answer(moved.run(&args), 1, CHANGED_IDS);
        assert_answer(steady.run(&args), 2, &[]);
    }

    // Neither build can fingerprint the method, so the answer is unknown, not a fit.
    let bodiless = Project::new("bodiless", &[STEADY, BODILESS]);
    assert_answer(bodiless.run(&["check", "--build", NEW]), 2, &["reach"]);
}

#[test]
#[ignore = "daily"]
fn check_with_a_file_and_build_reads_the_packages_uses_list() {
    let project = Project::new("package", &[]);
    let manifest = fs::read_to_string(project.dir.join(MANIFEST)).unwrap();
    // Make the project uncheckable so accidentally using it instead of the package will fail.
    fs::remove_file(project.dir.join(MANIFEST)).unwrap();
    let uses = serde_json::to_vec(&UsesTable {
        schema: PLAIN_SCHEMA,
        made_for: OLD.into(),
        entries: vec![],
    })
    .unwrap();
    let mut files: Vec<(&str, &[u8])> = vec![
        (
            "Cargo.toml",
            b"[package]\nname = 'example'\nversion = '0.1.0'\n",
        ),
        ("src/lib.rs", b""),
        (USES_FILE, &uses),
    ];
    let file = "example.smod";
    fs::write(
        project.dir.join(file),
        Package::write_files(&manifest, &files).unwrap(),
    )
    .unwrap();
    let report = assert_answer(project.run(&["check", file, "--build", NEW]), 0, &[]);
    assert!(
        report.lines().any(|line| line.starts_with("89abcde")),
        "{report}"
    );

    files.pop();
    fs::write(
        project.dir.join(file),
        Package::write_files(&manifest, &files).unwrap(),
    )
    .unwrap();
    assert_answer(project.run(&["check", file, "--build", NEW]), 2, &[]);
}

#[test]
#[ignore = "daily"]
fn pin_names_the_declared_game_code_the_new_build_changed() {
    let moved = Project::new("pin-moved", &[STEADY, RESHAPED, DROPPED]);
    assert_answer(moved.run(&["pin", NEW]), 0, CHANGED_IDS);
    let steady = Project::new("pin-steady", &[STEADY]);
    assert_answer(steady.run(&["pin", NEW]), 0, &[]);
}

#[test]
#[ignore = "daily"]
fn pin_without_a_build_moves_to_the_current_nightly() {
    let project = Project::new("pin-nightly", &[]);
    let releases = project.dir.join("releases");
    let nightly = releases.join(sunrise_release_sign::NIGHTLY);
    fs::create_dir(&nightly).unwrap();
    fs::copy(
        releases.join(format!("sdk-{NEW}")).join(sdk::INDEX),
        nightly.join(sdk::INDEX),
    )
    .unwrap();
    assert_answer(project.run(&["pin"]), 0, &[]);
    let manifest = fs::read_to_string(project.dir.join(MANIFEST)).unwrap();
    assert_eq!(
        Manifest::parse(&manifest).unwrap().native.unwrap().sunrise,
        NEW
    );
}

#[test]
#[ignore = "daily"]
fn check_answers_a_made_for_build_before_reading_its_surface() {
    // A build id whose SDK this tool can fetch but whose surface it cannot read.
    const FUTURE: &str = "cafebabecafebabecafebabecafebabecafebabe";
    // A schema from a newer tool: the build is signed and known, its surface unreadable here.
    const SURFACE: &str = r#"{"schema":9999,"hash":"ab"}"#;
    let project = Project::new("future-surface", &[]);
    let sdk = &project.sdk;
    sdk.release_with_surface(FUTURE, MIXINS, None, SURFACE);
    // A package made for exactly that build: its identity answers, whatever its surface says.
    let manifest = format!(
        "id = 'example'\nname = 'Example'\nversion = '0.1.0'\napi = '0.2'\ncapabilities = ['registries']\n\
         [native]\ngame = '26.3'\nsunrise = '{FUTURE}'\n"
    );
    let uses = serde_json::to_vec(&UsesTable {
        schema: PLAIN_SCHEMA,
        made_for: FUTURE.into(),
        entries: vec![],
    })
    .unwrap();
    let files: Vec<(&str, &[u8])> = vec![
        (
            "Cargo.toml",
            b"[package]\nname = 'example'\nversion = '0.1.0'\n",
        ),
        ("src/lib.rs", b""),
        (USES_FILE, &uses),
    ];
    let file = "example.smod";
    fs::write(
        project.dir.join(file),
        Package::write_files(&manifest, &files).unwrap(),
    )
    .unwrap();
    let report = assert_answer(project.run(&["check", file, "--build", FUTURE]), 0, &[]);
    assert!(
        report.lines().any(|line| line.ends_with("  made for")),
        "{report}"
    );
}

#[test]
#[ignore = "daily"]
fn pin_moves_a_mod_off_a_build_whose_surface_schema_the_tool_does_not_read() {
    let project = Project::made("pin-old-schema", &[], 1);
    let check = project.run(&["check", "--build", OLD]);
    let stderr = String::from_utf8_lossy(&check.stderr);
    assert!(
        !check.status.success() && stderr.contains("`cargo sunrise-mod pin <build>`"),
        "{stderr}"
    );
    let stdout = assert_answer(project.run(&["pin", NEW]), 0, &[]);
    assert!(stdout.contains("can't be compared"), "{stdout}");
    let manifest = fs::read_to_string(project.dir.join(MANIFEST)).unwrap();
    assert_eq!(
        Manifest::parse(&manifest).unwrap().native.unwrap().sunrise,
        NEW
    );
}

#[test]
#[ignore = "daily"]
fn a_changed_build_data_cache_cannot_turn_a_misfit_into_a_fit() {
    let moved = Project::new("cache", &[STEADY, RESHAPED, DROPPED]);
    // The project's cache holds both builds' catalogs rewritten to the same hashes, which would
    // read as a fit; the baseline and the target still come from the signed SDKs' build data.
    for build in [OLD, NEW] {
        let cached = moved
            .dir
            .join(".sunrise/cache")
            .join(build)
            .join("build-data");
        fs::create_dir_all(&cached).unwrap();
        for (name, bytes) in build_data(
            build,
            &[
                (STEADY, "struct", "5555"),
                (RESHAPED, "struct", "6666"),
                (DROPPED, "field", "7777"),
            ],
        ) {
            fs::write(cached.join(name), bytes).unwrap();
        }
    }
    assert_answer(moved.run(&["check", "--build", NEW]), 1, CHANGED_IDS);
}
