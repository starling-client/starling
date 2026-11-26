//! `shader_fuzz` (the `sunrise-render-shaderpack` example behind tools/shader_fuzz.py) through its
//! command line: what a run does with corpus packs that cost only their own cases, with runs in
//! which no case ran, and with its child processes at the end. Each test starts the example,
//! which `python3 tools/ci/ignored_shards.py --prepare sunrise-render-shaderpack` builds
//! (`cargo build --profile test -p sunrise-render-shaderpack --example shader_fuzz`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

/// The example, built into the profile folder above this test.
fn fuzzer() -> PathBuf {
    let test = std::env::current_exe().unwrap();
    let name = format!("shader_fuzz{}", std::env::consts::EXE_SUFFIX);
    test.ancestors()
        .skip(1)
        .map(|dir| dir.join("examples").join(&name))
        .find(|exe| exe.is_file())
        .unwrap_or_else(|| {
            panic!(
                "no examples/{name} above {}: build it first",
                test.display()
            )
        })
}

/// A new folder of this test's own.
fn temp() -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "shader-fuzz-cli-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A folder pack at `dir` with `files` (paths in `shaders`, and texts).
fn folder_pack(dir: &Path, files: &[(&str, &str)]) {
    for (path, text) in files {
        let file = dir.join("shaders").join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
}

const FINAL: [(&str, &str); 2] = [
    ("final.vsh", "void main() {}\n"),
    ("final.fsh", "void main() {}\n"),
];

/// What a run gave: its exit code, its standard error, and its report.
struct Ran {
    code: Option<i32>,
    errors: String,
    report: serde_json::Value,
}

/// `shader_fuzz` with `args`, writing in a new folder of `dir`.
fn run(dir: &Path, args: &[&str]) -> Ran {
    let work = dir.join("run");
    let out = Command::new(fuzzer())
        .args(args)
        .arg("--work-dir")
        .arg(&work)
        .output()
        .unwrap();
    let report = std::fs::read(work.join("report.json"))
        .unwrap_or_else(|e| panic!("no report ({e}): {}", String::from_utf8_lossy(&out.stderr)));
    Ran {
        code: out.status.code(),
        errors: String::from_utf8_lossy(&out.stderr).into_owned(),
        report: serde_json::from_slice(&report).unwrap(),
    }
}

/// A zip of `files` (paths and texts), stored without compression.
fn stored_zip(files: &[(&str, &str)]) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &b in bytes {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (name, text) in files {
        let (name, data) = (name.as_bytes(), text.as_bytes());
        let offset = out.len() as u32;
        let (crc, size, length) = (crc32(data), data.len() as u32, name.len() as u16);
        let common = |v: &mut Vec<u8>| {
            v.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            v.extend_from_slice(&crc.to_le_bytes());
            v.extend_from_slice(&size.to_le_bytes());
            v.extend_from_slice(&size.to_le_bytes());
            v.extend_from_slice(&length.to_le_bytes());
            v.extend_from_slice(&0u16.to_le_bytes());
        };
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        common(&mut out);
        out.extend_from_slice(name);
        out.extend_from_slice(data);
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        common(&mut central);
        // Comment length, disk, internal attributes.
        central.extend_from_slice(&[0; 6]);
        central.extend_from_slice(&0u32.to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name);
    }
    let start = out.len() as u32;
    let count = files.len() as u16;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// Packs that cost only their own cases, and the run goes on with the others:
/// - `big`, whose files take more memory than `--memory-limit`, ends its case process while the
///   case is sampled: each case on it is a `memory` failure of that case;
/// - `odd` holds a file name the run can't write, and `clash.zip` a file `lib` beside a folder
///   `lib/`: their cases make no case (`skipped`, each said), and are not findings.
///
/// The run itself never reads a pack.
#[test]
#[ignore = "daily"]
fn a_pack_that_exhausts_memory_or_cant_be_staged_costs_only_its_cases() {
    let dir = temp();
    let corpus = dir.join("corpus");
    folder_pack(&corpus.join("big"), &FINAL);
    // Sparse: 128 MiB to read, none on disk. Read once it fits the limit; held twice, not.
    std::fs::File::create(corpus.join("big/shaders/noise.dat"))
        .unwrap()
        .set_len(128 << 20)
        .unwrap();
    let mut odd = FINAL.to_vec();
    // A device name on Windows.
    odd.push(("CON.txt", "notes\n"));
    folder_pack(&corpus.join("odd"), &odd);
    let clash = stored_zip(&[
        ("shaders/final.vsh", "void main() {}\n"),
        ("shaders/final.fsh", "void main() {}\n"),
        ("shaders/lib", "a file\n"),
        ("shaders/lib/a.glsl", "float a;\n"),
    ]);
    std::fs::write(corpus.join("clash.zip"), clash).unwrap();
    // Seed 1's 16 cases take the seed pack, `big`, `clash.zip` and `odd` 5, 3, 5 and 3 times.
    let corpus_arg = corpus.to_str().unwrap();
    let ran = run(
        &dir,
        &[
            "--corpus",
            corpus_arg,
            "--seed",
            "1",
            "--count",
            "16",
            "--memory-limit",
            "160",
            "--shrink-seconds",
            "5",
        ],
    );
    assert_eq!(ran.code, Some(1), "{}", ran.errors);
    let report = &ran.report;
    assert_eq!(
        report["corpus"],
        serde_json::json!(["seeds", "big", "clash.zip", "odd"])
    );
    assert_eq!(report["cases"], 8);
    assert_eq!(report["skipped"], 8);
    for pack in ["clash.zip", "odd"] {
        let said = ran
            .errors
            .lines()
            .filter(|l| l.contains(&format!("{pack}: can't stage the pack")));
        assert_eq!(
            said.count(),
            if pack == "odd" { 3 } else { 5 },
            "{}",
            ran.errors
        );
    }
    let groups = report["groups"].as_array().unwrap();
    let big: Vec<&serde_json::Value> = groups.iter().filter(|g| g["pack"] == "big").collect();
    assert_eq!(big.len(), 1, "{groups:?}");
    assert_eq!(big[0]["invariant"], "memory");
    assert_eq!(big[0]["step"], "sample");
    assert_eq!(big[0]["detail"], "over 160 MiB");
    assert_eq!(big[0]["count"], 3);
    assert!(
        groups
            .iter()
            .all(|g| g["pack"] == "seeds" || g["pack"] == "big"),
        "{groups:?}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A run in which no case ran fails, says how many it skipped, and why each:
/// - no pack opens (neither the seed pack nor the corpus's);
/// - `--replay` of a case its case process can't write (a file named `aux.glsl`);
/// - `--smoke` with a seed pack that doesn't open.
#[test]
#[ignore = "daily"]
fn a_run_in_which_no_case_ran_fails() {
    let dir = temp();
    let unopened = dir.join("no-pack");
    std::fs::create_dir_all(&unopened).unwrap();
    let corpus = dir.join("corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    std::fs::write(corpus.join("broken.zip"), "not a zip").unwrap();
    let (unopened, corpus) = (unopened.to_str().unwrap(), corpus.to_str().unwrap());
    let ran = run(
        &dir.join("none-open"),
        &["--seed-pack", unopened, "--corpus", corpus, "--count", "4"],
    );
    assert_eq!(ran.code, Some(1), "{}", ran.errors);
    assert_eq!(
        (&ran.report["cases"], &ran.report["skipped"]),
        (&0.into(), &4.into())
    );
    // Each skipped case is said with its pack's name.
    let said: Vec<&str> = ran
        .errors
        .lines()
        .filter(|l| l.contains("doesn't open"))
        .collect();
    assert_eq!(said.len(), 4, "{}", ran.errors);
    assert!(
        said.iter()
            .all(|l| l.contains("broken.zip") || l.contains("no-pack")),
        "{}",
        ran.errors
    );

    let aux = dir.join("aux-pack");
    let mut files = FINAL.to_vec();
    files.push(("aux.glsl", "float a;\n"));
    folder_pack(&aux, &files);
    let ran = run(
        &dir.join("replay"),
        &["--seed-pack", aux.to_str().unwrap(), "--replay", "5"],
    );
    assert_eq!(ran.code, Some(1), "{}", ran.errors);
    assert_eq!(
        (&ran.report["cases"], &ran.report["skipped"]),
        (&0.into(), &1.into())
    );
    assert!(ran.errors.contains("aux.glsl"), "{}", ran.errors);

    let ran = run(&dir.join("smoke"), &["--seed-pack", unopened, "--smoke"]);
    assert_eq!(ran.code, Some(1), "{}", ran.errors);
    assert_eq!(
        (&ran.report["cases"], &ran.report["skipped"]),
        (&0.into(), &1.into())
    );
    assert!(ran.errors.contains("no-pack"), "{}", ran.errors);
    let _ = std::fs::remove_dir_all(dir);
}

/// A replay of a smoke case that makes none is that case skipped, with why: it fails the run as
/// one in which no case ran, counts under `skipped`, and says so naming the case and the seed
/// pack. Covered: an id past the last smoke case, and one past the first of a seed pack that
/// doesn't open (which, enumerating `--smoke`, ends the smoke cases there without a word).
#[test]
#[ignore = "daily"]
fn a_replayed_smoke_case_that_makes_no_case_is_skipped_with_why() {
    let dir = temp();
    let unopened = dir.join("no-pack");
    std::fs::create_dir_all(&unopened).unwrap();
    let unopened = unopened.to_str().unwrap();
    for (name, args, case, pack) in [
        ("past", vec!["--replay", "smoke:999"], "smoke:999", "seeds"),
        (
            "unopened",
            vec!["--seed-pack", unopened, "--replay", "smoke:3"],
            "smoke:3",
            "no-pack",
        ),
    ] {
        let ran = run(&dir.join(name), &args);
        assert_eq!(ran.code, Some(1), "{name}: {}", ran.errors);
        assert_eq!(
            (&ran.report["cases"], &ran.report["skipped"]),
            (&0.into(), &1.into()),
            "{name}"
        );
        let said: Vec<&str> = ran
            .errors
            .lines()
            .filter(|l| l.contains(&format!("case {case}:")) && l.contains(pack))
            .collect();
        assert_eq!(said.len(), 1, "{name}: {}", ran.errors);
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Each case process, its cases done, is closed and waited for, as is its eval worker: both
/// end by themselves (exit status 0), and the report says so.
#[test]
#[ignore = "daily"]
fn every_child_process_is_closed_and_waited_for() {
    let dir = temp();
    let ran = run(&dir, &["--smoke", "--jobs", "2"]);
    assert_eq!(ran.code, Some(0), "{}", ran.errors);
    assert_eq!(ran.report["cases"], 16);
    let ended = serde_json::json!({"exit": "exit status: 0", "worker": "exit status: 0"});
    assert_eq!(ran.report["processes"], serde_json::json!([ended, ended]));
    let _ = std::fs::remove_dir_all(dir);
}

/// The strings of a JSON array.
fn strings(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect()
}

/// What makes a reported group the same case failing the same way: its group, the case that
/// replays it and the arguments that do, and that case's pack, programs, dimension and terrain.
fn identity(g: &serde_json::Value) -> [serde_json::Value; 7] {
    [
        "group",
        "replay",
        "replay_args",
        "pack",
        "programs",
        "dimension",
        "flat",
    ]
    .map(|k| g[k].clone())
}

/// Each reported group of `report` (but `time`, which timing decides), run again alone through
/// its own `replay_args`, makes one case that reports the same group with the same identity.
fn replays_as_itself(dir: &Path, report: &serde_json::Value) {
    let groups = report["groups"].as_array().unwrap();
    assert!(!groups.is_empty());
    for (n, g) in groups.iter().enumerate() {
        if g["invariant"] == "time" {
            continue;
        }
        let replay = strings(&g["replay_args"]);
        let mut list: Vec<&str> = replay.iter().map(String::as_str).collect();
        list.extend(["--shrink-seconds", "0"]);
        let again = run(&dir.join(format!("replay{n}")), &list);
        assert_eq!(again.report["cases"], 1, "{}", again.errors);
        let same = again.report["groups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|h| h["group"] == g["group"]);
        assert_eq!(same.map(identity), Some(identity(g)), "{replay:?}");
        assert_eq!(again.code, Some(1));
    }
}

/// Each group a run reports replays as itself: its printed `replay_args`, run alone through the
/// fuzzer's command line, make that same case (pack, programs, dimension, terrain) and fail it
/// the same way, and report the same arguments again. Covered:
/// - a generated case of a pack in a corpus whose path holds a space, failing only under the
///   run's `--memory-limit` (replayed without it, the same case passes);
/// - the fixed smoke cases of a run's own `--seed-pack`, each failing under `--size-limit 0`
///   (replayed without the seed pack, `smoke:N` is another pack's case).
#[test]
#[ignore = "daily"]
fn a_reported_group_replays_as_itself() {
    let dir = temp();
    let corpus = dir.join("my corpus");
    folder_pack(&corpus.join("big"), &FINAL);
    // Sparse: 128 MiB to read, none on disk; past a 160 MiB limit once held twice.
    std::fs::File::create(corpus.join("big/shaders/noise.dat"))
        .unwrap()
        .set_len(128 << 20)
        .unwrap();
    let full = std::fs::canonicalize(&corpus).unwrap();
    let full = full.to_str().unwrap();
    let ran = run(
        &dir.join("generated"),
        &[
            "--seed",
            "1",
            "--count",
            "6",
            "--corpus",
            corpus.to_str().unwrap(),
            "--memory-limit",
            "160",
            "--time-limit",
            "30",
            "--shrink-seconds",
            "0",
        ],
    );
    assert_eq!(ran.code, Some(1), "{}", ran.errors);
    let groups = ran.report["groups"].as_array().unwrap();
    let big: Vec<&serde_json::Value> = groups.iter().filter(|g| g["pack"] == "big").collect();
    assert_eq!(big.len(), 1, "{groups:?}");
    assert_eq!(big[0]["group"], "memory|sample|over 160 MiB");
    let id = big[0]["replay"].as_str().unwrap().to_owned();
    assert!(id.parse::<u64>().is_ok(), "{id}");
    assert_eq!(
        strings(&big[0]["replay_args"]),
        [
            "--replay",
            &id,
            "--corpus",
            full,
            "--time-limit",
            "30",
            "--memory-limit",
            "160",
            "--size-limit",
            "16"
        ]
    );
    replays_as_itself(&dir.join("generated-again"), &ran.report);
    // Without the run's memory limit the same case passes: the limit is part of the case.
    let unlimited = run(
        &dir.join("unlimited"),
        &["--replay", &id, "--corpus", full, "--shrink-seconds", "0"],
    );
    assert_eq!(unlimited.code, Some(0), "{}", unlimited.errors);
    assert_eq!(unlimited.report["cases"], 1);
    assert_eq!(unlimited.report["groups"], serde_json::json!([]));

    let seeds = dir.join("my seeds");
    folder_pack(&seeds, &FINAL);
    let own = std::fs::canonicalize(&seeds).unwrap();
    let own = own.to_str().unwrap();
    let ran = run(
        &dir.join("smoke"),
        &[
            "--smoke",
            "--seed-pack",
            seeds.to_str().unwrap(),
            "--size-limit",
            "0",
            "--time-limit",
            "30",
            "--shrink-seconds",
            "0",
        ],
    );
    assert_eq!(ran.code, Some(1), "{}", ran.errors);
    let groups = ran.report["groups"].as_array().unwrap();
    assert!(!groups.is_empty());
    for g in groups {
        let id = g["replay"].as_str().unwrap();
        assert!(id.starts_with("smoke:"), "{id}");
        assert_eq!(g["programs"], serde_json::json!(["final"]));
        assert_eq!(
            strings(&g["replay_args"]),
            [
                "--replay",
                id,
                "--seed-pack",
                own,
                "--time-limit",
                "30",
                "--memory-limit",
                "2048",
                "--size-limit",
                "0"
            ]
        );
        // Without the run's seed pack, the same id is a case of the committed one.
        let other = run(
            &dir.join(format!("without-{}", id.replace(':', "-"))),
            &["--replay", id, "--size-limit", "0", "--shrink-seconds", "0"],
        );
        let same = other.report["groups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|h| h["group"] == g["group"])
            .map(|h| (h["pack"].clone(), h["programs"].clone()));
        let same = same.expect("the committed seed pack's case fails the same way");
        assert_ne!(same, (g["pack"].clone(), g["programs"].clone()));
    }
    replays_as_itself(&dir.join("smoke-again"), &ran.report);
    let _ = std::fs::remove_dir_all(dir);
}
