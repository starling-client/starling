//! `fits` and `update` on the game code a mod declares, judged by each build's build data, against
//! offline SDKs: no downloads.
#[path = "support/sdk.rs"]
mod sdk_fixture;

use sdk_fixture::OfflineSdk;
use std::{collections::BTreeSet, fs, path::PathBuf, process::Output};
use sunrise_mod_package::manifest::MANIFEST;

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
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/i3-d/tests")
            .join(format!("fits-{name}-{}", std::process::id()));
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
        sdk.release(OLD, MIXINS);
        sdk.release(NEW, MIXINS);
        sdk.prefetch(&dir, OLD, NEW);
        let project = Self { dir, sdk };
        project.build_data(
            OLD,
            &[
                (STEADY, "struct", "1111"),
                (RESHAPED, "struct", "2222"),
                (DROPPED, "field", "3333"),
                (BODILESS, "method", "4444"),
            ],
        );
        project.build_data(
            NEW,
            &[
                (STEADY, "struct", "1111"),
                (RESHAPED, "struct", "2299"),
                (BODILESS, "method", "4499"),
            ],
        );
        project
    }

    /// `build`'s internals catalog and an empty function index, as the fetched SDK's build data.
    fn build_data(&self, build: &str, internals: &[(&str, &str, &str)]) {
        let dir = self
            .dir
            .join(".sunrise/cache")
            .join(build)
            .join("build-data");
        fs::create_dir_all(&dir).unwrap();
        let listed = internals
            .iter()
            .map(|(path, kind, hash)| {
                serde_json::json!({"path": path, "kind": kind, "signature": path, "hash": hash})
            })
            .collect::<Vec<_>>();
        let catalog =
            serde_json::json!({"schema": 1, "build": {"commit": build}, "internals": listed});
        fs::write(dir.join("internals.json"), catalog.to_string()).unwrap();
        fs::write(
            dir.join("functions.json"),
            format!(r#"{{"schema":1,"build":{{"commit":"{build}"}},"crates":{{}},"functions":{{}},"refused":{{}}}}"#),
        )
        .unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        self.sdk.command(&self.dir).args(args).output().unwrap()
    }
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
fn fits_judges_declared_game_code_by_each_builds_data() {
    let steady = Project::new("steady", &[STEADY]);
    assert_answer(steady.run(&["fits", "--build", NEW]), 0, &[]);
    let stdout = assert_answer(
        steady.run(&["fits", "--build", OLD, "--build", NEW]),
        0,
        &[],
    );
    assert_build_order(&stdout);

    let moved = Project::new("moved", &[STEADY, RESHAPED, DROPPED]);
    assert_answer(moved.run(&["fits", "--build", NEW]), 1, CHANGED_IDS);
    // The build it was made for still fits; one build that does not decides the answer.
    assert_answer(moved.run(&["fits", "--build", OLD]), 0, &[]);
    let stdout = assert_answer(
        moved.run(&["fits", "--build", OLD, "--build", NEW]),
        1,
        CHANGED_IDS,
    );
    assert_build_order(&stdout);

    // Neither build can fingerprint the method, so the answer is unknown, not a fit.
    let bodiless = Project::new("bodiless", &[STEADY, BODILESS]);
    assert_answer(bodiless.run(&["fits", "--build", NEW]), 2, &["reach"]);
}

#[test]
#[ignore = "daily"]
fn update_names_the_declared_game_code_the_new_build_changed() {
    let moved = Project::new("update-moved", &[STEADY, RESHAPED, DROPPED]);
    assert_answer(moved.run(&["update", "--sunrise", NEW]), 0, CHANGED_IDS);
    let steady = Project::new("update-steady", &[STEADY]);
    assert_answer(steady.run(&["update", "--sunrise", NEW]), 0, &[]);
}
