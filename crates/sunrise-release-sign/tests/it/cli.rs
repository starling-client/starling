//! The release steps as release.yml runs them: `sums`, tools/ci/sign_release.sh (OpenSSL), `verify`.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use sunrise_release_sign::{SIGNATURE, SIGNATURE_V2, SUMS, SigningKey, trusted_keys};

const TAG: &str = "v2026.10.19";

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sunrise-release-sign-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("sunrise-linux-x86_64.tar.gz"), b"linux build").unwrap();
    std::fs::write(dir.join("sunrise-windows-x64.zip"), b"windows build").unwrap();
    let sums = sunrise_release_sign::sums_file(&[
        (
            "sunrise-linux-x86_64.tar.gz".into(),
            sunrise_release_sign::sha256_hex(b"linux build"),
        ),
        (
            "sunrise-windows-x64.zip".into(),
            sunrise_release_sign::sha256_hex(b"windows build"),
        ),
    ]);
    std::fs::write(dir.join(SUMS), sums).unwrap();
    dir
}

fn tool(args: &[&str], paths: &[&Path]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sunrise-release-sign"))
        .args(args)
        .args(paths)
        .output()
        .unwrap()
}

fn verify(dir: &Path, extra: &[&str]) -> Output {
    let mut args = vec!["verify", "--tag", TAG];
    args.extend(extra);
    tool(&args, &[dir])
}

/// An OpenSSL 3 (the runner's), or None on a machine without one (macOS ships LibreSSL).
fn openssl() -> Option<String> {
    [
        "openssl",
        "/opt/homebrew/opt/openssl@3/bin/openssl",
        "/usr/local/opt/openssl@3/bin/openssl",
    ]
    .into_iter()
    .find(|o| {
        Command::new(o)
            .arg("version")
            .output()
            .is_ok_and(|v| v.stdout.starts_with(b"OpenSSL 3"))
    })
    .map(String::from)
}

fn sign_script(dir: &Path, env: &[(&str, &str)]) -> Output {
    sign_script_for(TAG, dir, env)
}

fn sign_script_for(tag: &str, dir: &Path, env: &[(&str, &str)]) -> Output {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/ci/sign_release.sh");
    let mut cmd = Command::new("bash");
    cmd.arg(script)
        .arg(tag)
        .arg(dir.join("sunrise-linux-x86_64.tar.gz"))
        .arg(dir.join("sunrise-windows-x64.zip"))
        .env_remove("RELEASE_SIGNING_KEY")
        .env_remove("RELEASE_SIGNING_KEY_NEXT");
    cmd.env("OPENSSL", openssl().unwrap());
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

#[test]
#[ignore = "daily"]
fn a_signature_made_like_openssls_checks_and_tampering_fails() {
    let dir = folder("lib");
    let (key, _) = SigningKey::generate().unwrap();
    let sums = std::fs::read(dir.join(SUMS)).unwrap();
    std::fs::write(dir.join(SIGNATURE), key.sign(TAG, &sums) + "\n").unwrap();
    assert!(
        verify(&dir, &["--key", &key.public_base64()])
            .status
            .success()
    );
    std::fs::write(dir.join("sunrise-windows-x64.zip"), b"tampered").unwrap();
    let out = verify(&dir, &["--key", &key.public_base64()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("doesn't match its signed checksum"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "daily"]
fn unsigned_releases_go_out_only_until_a_key_is_listed() {
    let dir = folder("unsigned");
    assert_eq!(
        verify(&dir, &["--publishing"]).status.success(),
        trusted_keys().is_empty()
    );
    assert!(!verify(&dir, &[]).status.success());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "daily"]
fn the_openssl_script_signs_what_the_launcher_checks() {
    // The script runs on Linux runners only (the `sign` jobs of launcher.yml and nightly-build.yml).
    // On Windows, Command finds System32's bash (WSL's) before the one on PATH.
    if cfg!(windows) || openssl().is_none() {
        eprintln!("skipped: tools/ci/sign_release.sh is tested on Linux (bash, OpenSSL 3)");
        return;
    }
    // A dry run: a throwaway key, its public key for the check.
    let dir = folder("ephemeral");
    std::fs::remove_file(dir.join(SUMS)).unwrap(); // the script writes its own
    let out = sign_script(&dir, &[("EPHEMERAL", "yes")]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let key = std::fs::read_to_string(dir.join("ephemeral.pub")).unwrap();
    assert!(verify(&dir, &["--key", key.trim()]).status.success());
    std::fs::remove_dir_all(dir).unwrap();

    // A release: both secrets (rotating), as keygen makes them; each signature line checks alone.
    let dir = folder("secret");
    let (old, old_secret) = SigningKey::generate().unwrap();
    let (new, new_secret) = SigningKey::generate().unwrap();
    let out = sign_script(
        &dir,
        &[
            ("RELEASE_SIGNING_KEY", &old_secret),
            ("RELEASE_SIGNING_KEY_NEXT", &new_secret),
        ],
    );
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{printed}");
    assert!(!printed.contains(&old_secret) && !printed.contains(&new_secret));
    assert_eq!(
        std::fs::read_to_string(dir.join(SIGNATURE))
            .unwrap()
            .lines()
            .count(),
        2
    );
    assert!(
        verify(&dir, &["--key", &old.public_base64()])
            .status
            .success()
    );
    assert!(
        verify(&dir, &["--key", &new.public_base64()])
            .status
            .success()
    );
    let (other, _) = SigningKey::generate().unwrap();
    assert!(
        !verify(&dir, &["--key", &other.public_base64()])
            .status
            .success()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "daily"]
fn keygen_writes_the_secret_to_a_pipe_and_the_public_key_to_a_file() {
    let dir = folder("keygen");
    let public = dir.join("key.pub");
    let out = tool(&["keygen", "--public-out"], &[&public]);
    assert!(out.status.success());
    let key = SigningKey::from_base64(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(
        std::fs::read_to_string(&public).unwrap().trim(),
        key.public_base64()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "daily"]
fn a_nightly_is_signed_for_its_channel_and_for_its_version() {
    if cfg!(windows) || openssl().is_none() {
        eprintln!("skipped: tools/ci/sign_release.sh is tested on Linux (bash, OpenSSL 3)");
        return;
    }
    const VERSION: &str = "nightly-2026-10-28-0c02067";
    let dir = folder("nightly");
    std::fs::remove_file(dir.join(SUMS)).unwrap();
    let (key, secret) = SigningKey::generate().unwrap();
    let out = sign_script_for(
        "nightly",
        &dir,
        &[("RELEASE_SIGNING_KEY", &secret), ("ALSO_FOR", VERSION)],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let public = key.public_base64();
    let check = |tag: &str, signature: &str| {
        tool(
            &[
                "verify",
                "--tag",
                tag,
                "--key",
                &public,
                "--signature",
                signature,
            ],
            &[&dir],
        )
        .status
        .success()
    };
    // Launchers from before v2 check the channel's; current ones, the version's.
    assert!(check("nightly", SIGNATURE));
    assert!(check(VERSION, SIGNATURE_V2));
    assert!(!check(VERSION, SIGNATURE));
    assert!(!check("nightly", SIGNATURE_V2));
    assert!(!check("nightly-2026-10-27-0c02067", SIGNATURE_V2));
    // What a current launcher does with it.
    let sums = std::fs::read(dir.join(SUMS)).unwrap();
    let v2 = std::fs::read_to_string(dir.join(SIGNATURE_V2)).unwrap();
    let digest = sunrise_release_sign::build_digest(
        sunrise_release_sign::Channel::Nightly,
        VERSION,
        "sunrise-windows-x64.zip",
        Some(&sums),
        Some(&v2),
        &[key.public()],
    );
    assert_eq!(
        digest,
        Ok(sunrise_release_sign::sha256_hex(b"windows build"))
    );
    // A bad ALSO_FOR is refused before anything is signed.
    let out = sign_script_for(
        "nightly",
        &dir,
        &[("RELEASE_SIGNING_KEY", &secret), ("ALSO_FOR", "v0.2.81")],
    );
    assert!(!out.status.success());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "daily"]
fn publishing_never_bypasses_signatures_for_fork_or_unknown_repositories() {
    let dir = folder("repository-fail-closed");
    for repo in [
        "horizon-rust/horizon",
        "arismoko/horizon",
        "unknown/sunrise",
    ] {
        let output = verify(&dir, &["--repository", repo, "--publishing"]);
        assert!(
            !output.status.success(),
            "unsigned {repo} archive was accepted"
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// The test-key build of `sunrise-verify`, with the key in the environment (or none).
fn verifier(args: &[&str], paths: &[&Path], key: Option<&str>) -> Output {
    let mut cmd = Command::new(crate::common::test_key_verifier());
    cmd.args(args)
        .args(paths)
        .env_remove("SUNRISE_VERIFY_TEST_KEY");
    if let Some(key) = key {
        cmd.env("SUNRISE_VERIFY_TEST_KEY", key);
    }
    cmd.output().unwrap()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A folder signed with a throwaway key (its public half returned), the signature also saved as `alt.sig`.
fn signed_folder(name: &str) -> (PathBuf, String) {
    let dir = folder(name);
    let (key, _) = SigningKey::generate().unwrap();
    let sums = std::fs::read(dir.join(SUMS)).unwrap();
    let signature = key.sign(TAG, &sums) + "\n";
    std::fs::write(dir.join(SIGNATURE), &signature).unwrap();
    std::fs::write(dir.join("alt.sig"), signature).unwrap();
    (dir, key.public_base64())
}

#[test]
#[ignore = "daily"]
fn the_verifier_takes_no_option_that_changes_who_is_trusted() {
    let (dir, public) = signed_folder("verifier-options");
    let ok = ["verify", "--tag", TAG];
    let good = verifier(&ok, &[&dir], Some(&public));
    assert!(good.status.success(), "{}", stderr(&good));
    // Both builds (the feature one and cargo's own binary) refuse alike, before any key is looked at.
    let binaries = [
        crate::common::test_key_verifier().to_owned(),
        PathBuf::from(env!("CARGO_BIN_EXE_sunrise-verify")),
    ];
    for binary in &binaries {
        let run = |extra: &[&str], paths: &[&Path]| {
            Command::new(binary)
                .args(ok)
                .args(extra)
                .args(paths)
                .env("SUNRISE_VERIFY_TEST_KEY", &public)
                .output()
                .unwrap()
        };
        for (extra, named) in [
            (vec!["--key", public.as_str()], "--key"),
            (vec!["--repository", "someone/else"], "--repository"),
            (vec!["--publishing"], "--publishing"),
            (vec!["--bogus"], "--bogus"),
            (vec!["-x"], "-x"),
            (vec!["--only"], "--only needs a value"),
            (vec!["--only", "--bogus"], "--only needs a value"),
            (vec!["--signature", "-x"], "--signature needs a value"),
            (vec!["--tag", TAG], "--tag is given twice"),
        ] {
            // The option goes after the folder, so a missing value is the end of the arguments.
            let out = run(&extra, &[]);
            let folder_last = Command::new(binary)
                .args(ok)
                .arg(&dir)
                .args(&extra)
                .env("SUNRISE_VERIFY_TEST_KEY", &public)
                .output()
                .unwrap();
            for out in [out, folder_last] {
                assert!(!out.status.success(), "{extra:?}");
                assert!(
                    stderr(&out).contains(named)
                        || stderr(&out).contains("needs the release's folder"),
                    "{extra:?}: {}",
                    stderr(&out)
                );
            }
        }
        let out = run(&[], &[&dir, &dir]);
        assert!(!out.status.success() && stderr(&out).contains("one folder"));
        let out = Command::new(binary)
            .args(["verify", "--only", "x"])
            .arg(&dir)
            .output()
            .unwrap();
        assert!(!out.status.success() && stderr(&out).contains("--tag"));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "daily"]
fn the_verifier_checks_the_selected_file_and_signature_with_exactly_one_test_key() {
    let (dir, public) = signed_folder("verifier-select");
    let (other, _) = SigningKey::generate().unwrap();
    let base = ["verify", "--tag", TAG];
    let run = |extra: &[&str], key: Option<&str>| {
        let mut args = base.to_vec();
        args.extend(extra);
        verifier(&args, &[&dir], key)
    };
    // The selected file, from the alternate signature file: the default one is not a signature at all here.
    std::fs::write(dir.join(SIGNATURE), "garbage\n").unwrap();
    assert!(
        run(
            &[
                "--only",
                "sunrise-linux-x86_64.tar.gz",
                "--signature",
                "alt.sig"
            ],
            Some(&public)
        )
        .status
        .success()
    );
    std::fs::copy(dir.join("alt.sig"), dir.join(SIGNATURE)).unwrap();
    let out = run(&["--only", "missing.zip"], Some(&public));
    assert!(stderr(&out).contains("missing.zip is not in"));
    for name in ["../alt.sig", "C:alt.sig", "sub\\alt.sig"] {
        let out = run(&["--signature", name], Some(&public));
        assert!(!out.status.success());
        assert!(
            stderr(&out).contains("names a file of the folder"),
            "{name}"
        );
    }
    // Not the signed bytes: refused naming the file, for the selected one only.
    std::fs::write(dir.join("sunrise-windows-x64.zip"), b"tampered").unwrap();
    assert!(
        run(&["--only", "sunrise-linux-x86_64.tar.gz"], Some(&public))
            .status
            .success()
    );
    assert!(!run(&[], Some(&public)).status.success());
    // The key: another one, none of the built-in ones, and not exactly one.
    let other = other.public_base64();
    assert!(stderr(&run(&[], Some(&other))).contains("signature doesn't match"));
    let out = run(&[], None);
    assert!(!out.status.success() && stderr(&out).contains("SUNRISE_VERIFY_TEST_KEY"));
    let two = format!("{public}\n{other}");
    assert!(stderr(&run(&[], Some(&two))).contains("exactly one key"));
    assert!(stderr(&run(&[], Some("not a key"))).contains("not base64"));
    // No signature file at all.
    std::fs::remove_file(dir.join(SIGNATURE)).unwrap();
    let out = run(&[], Some(&public));
    assert!(stderr(&out).contains("has no SHA256SUMS.sig"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "daily"]
fn the_verifier_has_no_keygen_and_writes_no_key() {
    let dir = folder("verifier-keygen");
    let public = dir.join("k");
    let out = verifier(&["keygen", "--public-out"], &[&public], None);
    assert!(!out.status.success() && stderr(&out).contains("usage: sunrise-verify"));
    assert!(out.stdout.is_empty() && !public.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "daily"]
#[cfg(not(feature = "test-key"))]
fn a_release_verifier_ignores_the_test_key() {
    // The binary cargo builds for release has no `test-key` feature: the variable is never read, so the key that
    // signed the fixture is not trusted.
    let (dir, public) = signed_folder("verifier-release-build");
    let out = Command::new(env!("CARGO_BIN_EXE_sunrise-verify"))
        .args(["verify", "--tag", TAG])
        .arg(&dir)
        .env("SUNRISE_VERIFY_TEST_KEY", &public)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("signature doesn't match"));
    std::fs::remove_dir_all(dir).unwrap();
}
