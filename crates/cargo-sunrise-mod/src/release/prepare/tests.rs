use super::*;
use serde_json::{Value, json};
use sunrise_mod_package::Package;

const SOURCE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TARGET: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const MIXINS: &str = "cccccccccccccccccccccccccccccccccccccccc";
const LOCK: &str = "version = 4\n\n[[package]]\nname = 'mod-probe'\nversion = '0.1.0'\n";
const LIB: &str = "pub fn exact_reviewed_source() {}\n";

fn reviewed() -> (Vec<u8>, Value) {
    let manifest = format!(
        "id = 'probe'\nname = 'Probe'\nversion = '0.1.0'\n[native]\ngame = '26.3'\nsunrise = '{SOURCE}'\n"
    );
    let cargo = include_str!("../../../template/native/api.toml").replace("{id}", "probe");
    let bytes = Package::write_files(
        &manifest,
        &[
            ("Cargo.toml", cargo.as_bytes()),
            ("Cargo.lock", LOCK.as_bytes()),
            ("src/lib.rs", LIB.as_bytes()),
        ],
    )
    .unwrap();
    let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let signature = minisign::sign(
        Some(&pair.pk),
        &pair.sk,
        bytes.as_slice(),
        Some("release test"),
        None,
    )
    .unwrap()
    .into_string();
    let plan = json!({
        "schema": 1, "tool_commit": env!("SUNRISE_TOOL_BUILT_FROM"), "plan": {
            "mode": "rebuild", "id": "probe", "version": "0.1.0", "mixins": MIXINS,
            "source": {"tag": "26.3-0.1.0", "commit": SOURCE, "line": "26.3-0"},
            "target": {"tag": "26.3-0.2.0", "commit": TARGET, "line": "26.3-0"},
            "reviewed_source": {"url": "https://github.com/test/probe/releases/download/v0.1.0/probe-0.1.0.smod",
                "sha256": index::sha256(&bytes), "size": bytes.len(), "key": pair.pk.to_base64(), "signature": signature,
                "owners": ["test"], "owner_ids": [42]},
        },
    });
    (bytes, plan)
}

#[test]
fn only_the_scratch_manifest_is_repinned_and_signing_never_unpacks_code() {
    let temp = tempfile::tempdir().unwrap();
    let (bytes, plan) = reviewed();
    let file = temp.path().join("probe.smod");
    fs::write(&file, &bytes).unwrap();
    let at = temp.path().join("plan.json");
    fs::write(&at, plan.to_string()).unwrap();
    let build = temp.path().join("build");
    let metadata = temp.path().join("metadata");
    for (build, dest) in [(true, &build), (false, &metadata)] {
        let mut args = vec![
            "--plan".into(),
            at.display().to_string(),
            "--package".into(),
            file.display().to_string(),
            "--out".into(),
            dest.display().to_string(),
        ];
        if build {
            args.push("--build".into());
        }
        run(&args).unwrap();
    }
    let pinned = |project: &Path| {
        Manifest::parse(&fs::read_to_string(project.join(MANIFEST)).unwrap())
            .unwrap()
            .native
            .unwrap()
    };
    assert_eq!(pinned(&build).sunrise, TARGET);
    assert_eq!(pinned(&build).mixins.as_deref(), Some(MIXINS));
    assert_eq!(pinned(&metadata).sunrise, SOURCE);
    assert_eq!(fs::read_dir(&metadata).unwrap().count(), 1);
    assert_eq!(fs::read(build.join("Cargo.lock")).unwrap(), LOCK.as_bytes());
    assert_eq!(fs::read(build.join("src/lib.rs")).unwrap(), LIB.as_bytes());
    assert_eq!(fs::read(file).unwrap(), bytes);
}

#[test]
fn a_different_package_signer_or_pin_is_refused_before_unpacking() {
    let temp = tempfile::tempdir().unwrap();
    let (bytes, plan) = reviewed();
    let file = temp.path().join("probe.smod");
    fs::write(&file, &bytes).unwrap();
    let dest = temp.path().join("dest");
    let wrong_key = minisign::KeyPair::generate_unencrypted_keypair()
        .unwrap()
        .pk
        .to_base64();
    for (path, replacement, error) in [
        ("/reviewed_source/sha256", "different bytes", "SHA-256"),
        ("/reviewed_source/key", wrong_key.as_str(), "signature"),
        ("/source/commit", TARGET, "is not the reviewed"),
    ] {
        let mut plan = plan["plan"].clone();
        *plan.pointer_mut(path).unwrap() = json!(replacement);
        let plan = serde_json::from_value(plan).unwrap();
        let failure = prepare(&plan, &file, &dest, true).unwrap_err();
        assert!(failure.contains(error), "{failure}");
        assert!(!dest.exists());
    }
}
