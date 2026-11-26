//! `sunrise-stage surface-diff` end to end: exit codes, notes and the gate, on two small
//! surfaces written here.
use mod_surface::{Behaviour, BehaviourContract, Function, FunctionContract, Surface};
use std::path::Path;
use std::process::Output;

fn function(path: &str) -> Function {
    Function::new(
        format!("fn:{path}"),
        "",
        FunctionContract {
            signature: format!("fn {path}()"),
            impl_generics: String::new(),
            refs: vec![],
            behaviour_versions: Default::default(),
        },
    )
}

fn surface(functions: &[&str]) -> Surface {
    let functions = functions.iter().map(|f| function(f)).collect();
    Surface::new(
        vec![],
        vec![],
        vec![],
        functions,
        vec![],
        vec![],
        vec![],
        vec![],
    )
}

/// A folder with `old.json` (fn:spawn, fn:keep) and `new.json` (fn:keep, fn:summon): one
/// breaking removal and one addition.
fn folder() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("old.json"),
        surface(&["spawn", "keep"]).to_json(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("new.json"),
        surface(&["keep", "summon"]).to_json(),
    )
    .unwrap();
    dir
}

fn note(dir: &Path, folder: &str, name: &str, text: &str) {
    let notes = dir.join(folder);
    std::fs::create_dir_all(&notes).unwrap();
    std::fs::write(notes.join(name), text).unwrap();
}

fn run(dir: &Path, extra: &[&str]) -> (i32, String, String) {
    let Output {
        status,
        stdout,
        stderr,
    } = std::process::Command::new(env!("CARGO_BIN_EXE_sunrise-stage"))
        .current_dir(dir)
        .args(["surface-diff", "old.json", "new.json"])
        .args(extra)
        .output()
        .unwrap();
    (
        status.code().expect("exited, not killed"),
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

#[test]
#[ignore = "daily"]
fn stable_release_gates_the_real_surface_verdict() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = std::process::Command::new("python3")
        .current_dir(root)
        .env("SUNRISE_STAGE_TOOL", env!("CARGO_BIN_EXE_sunrise-stage"))
        .env("PYTHONPATH", ".")
        .arg("tests/stable_mod_api_integration.py")
        .output()
        .expect("run the stable release integration test");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "daily"]
fn without_the_gate_a_break_reports_and_passes() {
    let dir = folder();
    let (code, out, _) = run(dir.path(), &["--notes", "changes", "--base-notes", "base"]);
    assert_eq!(code, 0);
    assert!(out.starts_with(
        "Surface: breaking (1 breaking, 0 conditional, 1 additive; 1 breaking without a note)"
    ));
    assert!(!out.contains("Engine catalogs"), "{out}");
    assert!(out.find("- fn:spawn (removed)") < out.find("- fn:summon (added)"));
}

#[test]
#[ignore = "daily"]
fn the_gate_fails_an_undeclared_break_and_passes_a_declared_one() {
    let dir = folder();
    let (code, _, err) = run(dir.path(), &["--gate"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("breaking change without a change note: fn:spawn"),
        "{err}"
    );
    note(
        dir.path(),
        "changes",
        "rename.toml",
        "ids = [\"fn:spawn\", \"fn:summon\"]\nwhy = \"Spawn reads as eggs.\"\nmigrate = \"Call `summon`.\"\n",
    );
    let (code, out, err) = run(
        dir.path(),
        &["--gate", "--notes", "changes", "--base-notes", "base"],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("    why (rename.toml): Spawn reads as eggs.\n    migrate: Call `summon`.\n")
    );
}

#[test]
#[ignore = "daily"]
fn a_removed_behaviour_fails_the_gate_without_a_note() {
    let dir = tempfile::tempdir().unwrap();
    let behaviour = Behaviour::new(
        "behaviour:pool_merging",
        "",
        vec!["a.rs::t".into()],
        BehaviourContract { version: 1 },
    );
    std::fs::write(
        dir.path().join("old.json"),
        surface(&[]).with_behaviours(vec![behaviour]).to_json(),
    )
    .unwrap();
    std::fs::write(dir.path().join("new.json"), surface(&[]).to_json()).unwrap();
    let (code, _, err) = run(dir.path(), &["--gate"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("breaking change without a change note: behaviour:pool_merging"),
        "{err}"
    );
    note(
        dir.path(),
        "changes",
        "pool-merging.toml",
        "ids = [\"behaviour:pool_merging\"]\nwhy = \"Additions are no longer merged.\"\n",
    );
    let (code, _, err) = run(
        dir.path(),
        &["--gate", "--notes", "changes", "--base-notes", "base"],
    );
    assert_eq!(code, 0, "{err}");
}

#[test]
#[ignore = "daily"]
fn a_new_note_naming_an_unchanged_id_fails_and_an_old_or_edited_one_is_ignored() {
    let dir = folder();
    let stale = "ids = [\"fn:keep\"]\nwhy = \"Nothing.\"\n";
    note(dir.path(), "changes", "stale.toml", stale);
    let (code, _, err) = run(dir.path(), &["--notes", "changes", "--base-notes", "base"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("change note stale.toml names fn:keep, which did not change"),
        "{err}"
    );
    // The same file in the base tree: an earlier change's note, neither checked nor attached.
    note(dir.path(), "base", "stale.toml", stale);
    let (code, out, err) = run(dir.path(), &["--notes", "changes", "--base-notes", "base"]);
    assert_eq!(code, 0, "{err}");
    assert!(!out.contains("Nothing."));
    // Edited since the base: still an earlier change's note, so editing it never fails.
    note(
        dir.path(),
        "changes",
        "stale.toml",
        "ids = [\"fn:keep\"]\nwhy = \"Edited.\"\n",
    );
    let (code, ..) = run(dir.path(), &["--notes", "changes", "--base-notes", "base"]);
    assert_eq!(code, 0);
    // Notes without the base's notes cannot tell new from old.
    let (code, _, err) = run(dir.path(), &["--notes", "changes"]);
    assert_eq!(code, 2);
    assert!(err.contains("--notes needs --base-notes"), "{err}");
}

#[test]
#[ignore = "daily"]
fn a_landed_note_is_checked_only_against_its_own_merge() {
    let dir = folder();
    let sha = "6597e0855f562498319f093afcf2ed1ea8d717cb";
    let own = ["--notes", "changes", "--base-notes", "base"];
    let landed = [own[0], own[1], own[2], own[3], "--landed", sha];
    // Changed in that merge: attached with `--landed`, and left out of the pull request's own diff.
    note(
        dir.path(),
        "changes",
        "late.toml",
        &format!("ids = [\"fn:spawn\"]\nwhy = \"Late.\"\nlanded = \"{sha}\"\n"),
    );
    let (code, out, err) = run(dir.path(), &own);
    assert_eq!(code, 0, "{err}");
    assert!(!out.contains("Late."), "{out}");
    let (code, out, err) = run(dir.path(), &landed);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Late."), "{out}");
    // An id that did not change in that merge is stale; another merge's notes are not read.
    note(
        dir.path(),
        "changes",
        "late.toml",
        &format!("ids = [\"fn:keep\"]\nwhy = \"Late.\"\nlanded = \"{sha}\"\n"),
    );
    let (code, _, err) = run(dir.path(), &landed);
    assert_eq!(code, 1);
    assert!(
        err.contains("change note late.toml names fn:keep, which did not change"),
        "{err}"
    );
    let other = "1".repeat(40);
    let (code, out, _) = run(
        dir.path(),
        &[own[0], own[1], own[2], own[3], "--landed", &other],
    );
    assert_eq!(code, 0);
    assert!(!out.contains("Late."), "{out}");
}

#[test]
#[ignore = "daily"]
fn a_merges_own_notes_cover_its_diff_apart_from_the_late_ones() {
    let dir = folder();
    let sha = "6597e0855f562498319f093afcf2ed1ea8d717cb";
    let late = |ids: &str| format!("ids = [{ids}]\nwhy = \"Late.\"\nlanded = \"{sha}\"\n");
    let gated = [
        "--notes",
        "changes",
        "--base-notes",
        "base",
        "--landed",
        sha,
        "--covered-by",
        "merge",
        "--covered-base",
        "parent",
        "--gate",
    ];
    // fn:spawn's removal, with no note at all, is breaking and unnoted.
    for folder in ["changes", "base", "merge", "parent"] {
        std::fs::create_dir_all(dir.path().join(folder)).unwrap();
    }
    let (code, _, err) = run(dir.path(), &gated);
    assert_eq!(code, 1);
    assert!(
        err.contains("breaking change without a change note: fn:spawn"),
        "{err}"
    );
    // The merge's own note covers it, and its unchanged fn:keep is not stale: it was checked when it merged.
    note(
        dir.path(),
        "merge",
        "late.toml",
        "ids = [\"fn:spawn\", \"fn:keep\"]\nwhy = \"Then.\"\n",
    );
    let (code, out, err) = run(dir.path(), &gated);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Then."), "{out}");
    // A late note of the same file name is read apart from it: still checked, and a wrong id still fails.
    note(dir.path(), "changes", "late.toml", &late("\"fn:keep\""));
    let (code, _, err) = run(dir.path(), &gated);
    assert_eq!(code, 1);
    assert!(
        err.contains("change note late.toml names fn:keep, which did not change"),
        "{err}"
    );
    // Nor does a file of that name in the first parent's notes hide it.
    note(
        dir.path(),
        "parent",
        "late.toml",
        "ids = [\"fn:x\"]\nwhy = \"Old.\"\n",
    );
    let (code, _, err) = run(dir.path(), &gated);
    assert_eq!(code, 1);
    assert!(err.contains("names fn:keep, which did not change"), "{err}");
    note(dir.path(), "changes", "late.toml", &late("\"fn:spawn\""));
    let (code, out, err) = run(dir.path(), &gated);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Late."), "{out}");
}

#[test]
#[ignore = "daily"]
fn sdk_note_scope_ignores_engine_ids_but_checks_api_ids() {
    let dir = folder();
    note(
        dir.path(),
        "changes",
        "internal.toml",
        "ids = [\"internal:fn:game::old_tick\"]\nwhy = \"Tick moved.\"\n",
    );
    note(
        dir.path(),
        "changes",
        "mixed.toml",
        "ids = [\"loop:old.tick\", \"fn:spawn\"]\nwhy = \"Use the new entry.\"\n",
    );
    let notes = ["--notes", "changes", "--base-notes", "base"];
    let mut publish = notes.to_vec();
    publish.extend(["--api-notes-only", "--json", "diff.json"]);
    let (code, _, err) = run(dir.path(), &publish);
    assert_eq!(code, 0, "{err}");
    assert!(err.is_empty(), "{err}");
    let diff: mod_surface::Diff =
        serde_json::from_slice(&std::fs::read(dir.path().join("diff.json")).unwrap()).unwrap();
    assert_eq!(diff.undeclared().count(), 0);
    let noted: Vec<_> = diff
        .changes
        .iter()
        .flat_map(|c| {
            c.notes
                .iter()
                .map(|note| (c.id.as_str(), note.file.as_str()))
        })
        .collect();
    assert_eq!(noted, [("fn:spawn", "mixed.toml")]);

    let (code, _, err) = run(dir.path(), &notes);
    assert_eq!(code, 1);
    for refused in [
        [
            "internal.toml",
            "internal:fn:game::old_tick",
            "which did not change",
        ],
        ["mixed.toml", "loop:old.tick", "which did not change"],
    ] {
        assert!(
            err.lines()
                .any(|line| refused.iter().all(|word| line.contains(word))),
            "{err}"
        );
    }

    note(
        dir.path(),
        "changes",
        "api-stale.toml",
        "ids = [\"internal:fn:game::new_tick\", \"fn:keep\"]\nwhy = \"No API change.\"\n",
    );
    let (code, _, err) = run(dir.path(), &publish);
    assert_eq!(code, 1);
    // The engine id in the same note is outside the SDK's scope, so only the API id is refused.
    assert!(
        err.contains("api-stale.toml")
            && err.contains("fn:keep")
            && err.contains("which did not change"),
        "{err}"
    );
    assert!(!err.contains("internal:fn:game::new_tick"), "{err}");
}

#[test]
#[ignore = "daily"]
fn a_malformed_note_or_surface_is_refused_by_name() {
    let dir = folder();
    note(
        dir.path(),
        "changes",
        "bad.toml",
        "ids = [\"fn:spawn\"]\nwhy = \"x\"\nreason = \"y\"\n",
    );
    let (code, _, err) = run(dir.path(), &["--notes", "changes", "--base-notes", "base"]);
    assert_eq!(code, 2);
    assert!(
        err.contains("change note bad.toml has the unknown key `reason`"),
        "{err}"
    );
    std::fs::write(dir.path().join("new.json"), "{\"schema\": 2, \"hooks\": [").unwrap();
    let (code, _, err) = run(dir.path(), &[]);
    assert_eq!(code, 2);
    assert!(
        err.contains("new.json: surface.json is not a valid surface"),
        "{err}"
    );
}

#[test]
#[ignore = "daily"]
fn the_json_report_is_written() {
    let dir = folder();
    let (code, ..) = run(dir.path(), &["--json", "diff.json"]);
    assert_eq!(code, 0);
    let diff: mod_surface::Diff =
        serde_json::from_slice(&std::fs::read(dir.path().join("diff.json")).unwrap()).unwrap();
    assert_eq!(diff.verdict, mod_surface::Verdict::Breaking);
    assert_eq!(diff.changes.len(), 2);
}
