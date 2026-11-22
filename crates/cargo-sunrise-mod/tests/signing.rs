use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn tool(project: &Path) -> Command {
    let mut tool = Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"));
    tool.current_dir(project)
        .env("XDG_DATA_HOME", project.parent().unwrap().join("data"))
        .env("LOCALAPPDATA", project.parent().unwrap().join("data"))
        .env("HOME", project.parent().unwrap().join("home"));
    tool
}

const MANIFEST: &str = "id = 'signing_test'\nname = 'Signing test'\nversion = '1.0.0'\n";

fn project(temp: &tempfile::TempDir) -> std::path::PathBuf {
    let project = temp.path().canonicalize().unwrap().join("project");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("sunrise-mod.toml"), MANIFEST).unwrap();
    project
}

#[cfg(unix)]
fn default_keys(temp: &tempfile::TempDir) -> std::path::PathBuf {
    let base = if cfg!(target_os = "macos") {
        "home/Library/Application Support"
    } else {
        "data"
    };
    temp.path().join(base).join("sunrise-mod/keys/signing_test")
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn keygen_writes_registry_key_and_refuses_to_replace_either_file() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = project(&temp);
    let project = project_dir.as_path();
    let public = success(
        tool(project)
            .args(["keygen", "--no-password"])
            .output()
            .unwrap(),
    );
    let key = public.trim();
    minisign_verify::PublicKey::from_base64(key).unwrap();
    let secret_file = default_keys(&temp).join("signing.key");
    let public_file = default_keys(&temp).join("signing.pub");
    let secret = fs::read(&secret_file).unwrap();
    assert_eq!(
        fs::read_to_string(&public_file).unwrap().lines().last(),
        Some(key)
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(default_keys(&temp))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&secret_file).unwrap().permissions().mode() & 0o077,
            0
        );
    }
    let refused = tool(project)
        .args(["keygen", "--no-password"])
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("signing.key"));
    assert!(refused.stdout.is_empty());
    assert!(fs::read(&secret_file).unwrap() == secret);

    fs::create_dir(project.join(".sunrise")).unwrap();
    fs::write(project.join(".sunrise/signing.pub"), b"keep me").unwrap();
    let refused = tool(project)
        .args(["keygen", "--no-password", "--out", ".sunrise"])
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert_eq!(
        fs::read(project.join(".sunrise/signing.pub")).unwrap(),
        b"keep me"
    );
    assert!(!project.join(".sunrise/signing.key").exists());
}

#[test]
#[ignore = "daily"]
fn keygen_rejects_destinations_inside_package_inputs() {
    let temp = tempfile::tempdir().unwrap();
    let project = project(&temp);
    for path in [".", "assets/signing_test/.sunrise"] {
        // No terminal: the path must be refused before the passphrase prompt.
        let output = tool(&project)
            .args(["keygen", "--out", path])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("inside the mod's package inputs")
        );
        assert!(!project.join(path).join("signing.key").exists());
    }
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn signing_paths_refuse_a_symlinked_ancestor() {
    let temp = tempfile::tempdir().unwrap();
    let project = project(&temp);
    let outside = temp.path().join("outside");
    fs::create_dir_all(outside.join("nested")).unwrap();
    std::os::unix::fs::symlink(&outside, project.join("linked")).unwrap();
    // The ancestor under an excluded output tree must still not be followed.
    fs::create_dir(project.join(".sunrise")).unwrap();
    std::os::unix::fs::symlink(&outside, project.join(".sunrise/linked")).unwrap();
    let output = tool(&project)
        .args(["keygen", "--out", ".sunrise/linked/nested"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("linked"));
    assert!(!outside.join("nested/signing.key").exists());

    let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    fs::write(
        outside.join("nested/key"),
        pair.sk.to_box(None).unwrap().into_string(),
    )
    .unwrap();
    fs::write(outside.join("nested/asset"), b"release").unwrap();
    fs::write(project.join("asset"), b"release").unwrap();
    for (asset, key) in [
        ("asset", "linked/nested/key"),
        ("linked/nested/asset", "asset"),
    ] {
        let output = tool(&project)
            .args(["sign", asset, "--key", key, "--no-password"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("linked"));
    }
    assert!(!project.join("asset.minisig").exists());
    assert!(!outside.join("nested/asset.minisig").exists());
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn keygen_preserves_a_dangling_output_symlink() {
    let temp = tempfile::tempdir().unwrap();
    let project = project(&temp);
    let keys = project.join(".sunrise");
    fs::create_dir(&keys).unwrap();
    let target = project.join("must-stay-absent");
    let link = keys.join("signing.key");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let output = tool(&project)
        .args(["keygen", "--no-password", "--out", ".sunrise"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("signing.key"));
    assert_eq!(fs::read_link(link).unwrap(), target);
    assert!(!target.exists());
    assert!(!keys.join("signing.pub").exists());
}

#[test]
fn packing_refuses_copied_minisign_private_keys_but_allows_the_public_key() {
    use sunrise_mod_package::Package;
    let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let public = pair.pk.to_box().unwrap().into_string();
    let raw = pair.sk.to_bytes();
    let text = pair
        .sk
        .to_box(Some("renamed fixture"))
        .unwrap()
        .into_string()
        .replace('\n', "\r\n");
    let path = "assets/signing_test/.sunrise/renamed.txt";
    for secret in [text.as_bytes(), &raw] {
        assert!(Package::write_files(MANIFEST, &[(path, secret)]).is_err());
    }
    assert!(Package::write_files(MANIFEST, &[(path, public.as_bytes())]).is_ok());
}

const KEY_ENV: &str = "SUNRISE_MOD_SIGNING_KEY";
const PASSPHRASE_ENV: &str = "SUNRISE_MOD_SIGNING_PASSPHRASE";

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn the_registry_verifies_the_detached_signature_of_the_exact_file() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = project(&temp);
    let project = project_dir.as_path();
    let public = success(
        tool(project)
            .args(["keygen", "--no-password"])
            .output()
            .unwrap(),
    );
    let key = minisign_verify::PublicKey::from_base64(public.trim()).unwrap();
    let wrong = success(
        tool(project)
            .args(["keygen", "--no-password", "--out", ".sunrise"])
            .output()
            .unwrap(),
    );
    let wrong = minisign_verify::PublicKey::from_base64(wrong.trim()).unwrap();
    let bytes = b"signed release asset bytes\x00\xff";
    fs::write(project.join("mod.smod"), bytes).unwrap();
    success(
        tool(project)
            .args(["sign", "mod.smod", "--no-password"])
            .output()
            .unwrap(),
    );
    let first = fs::read_to_string(project.join("mod.smod.minisig")).unwrap();
    let signature = minisign_verify::Signature::decode(&first).unwrap();
    key.verify(bytes, &signature, false).unwrap();
    assert!(key.verify(b"changed bytes", &signature, false).is_err());
    assert!(wrong.verify(bytes, &signature, false).is_err());
    let refused = tool(project)
        .args(["sign", "mod.smod", "--no-password"])
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert_eq!(
        fs::read_to_string(project.join("mod.smod.minisig")).unwrap(),
        first
    );

    let secret = fs::read_to_string(default_keys(&temp).join("signing.key")).unwrap();
    fs::remove_file(default_keys(&temp).join("signing.key")).unwrap();
    fs::write(project.join("build.tar.xz"), bytes).unwrap();
    let output = tool(project)
        .args(["sign", "build.tar.xz", "--key-env"])
        .env(KEY_ENV, &secret)
        .env(PASSPHRASE_ENV, "")
        .output()
        .unwrap();
    let secrets = secret.lines().nth(1).unwrap().as_bytes();
    assert!(
        output
            .stdout
            .windows(secrets.len())
            .all(|window| window != secrets)
    );
    assert!(
        output
            .stderr
            .windows(secrets.len())
            .all(|window| window != secrets)
    );
    success(output);
    let text = fs::read_to_string(project.join("build.tar.xz.minisig")).unwrap();
    key.verify(
        bytes,
        &minisign_verify::Signature::decode(&text).unwrap(),
        false,
    )
    .unwrap();
}

#[test]
#[ignore = "daily"]
fn ci_unlocks_an_encrypted_key_only_with_its_explicit_passphrase() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = project(&temp);
    let project = project_dir.as_path();
    let passphrase = "fixture passphrase";
    let pair = minisign::KeyPair::generate_encrypted_keypair(Some(passphrase.into())).unwrap();
    let secret = pair.sk.to_box(None).unwrap().into_string();
    let bytes = b"release descriptor";
    fs::write(project.join("descriptor.json"), bytes).unwrap();
    let sign = |password: Option<&str>| {
        let mut cmd = tool(project);
        cmd.args(["sign", "descriptor.json", "--key-env"])
            .env(KEY_ENV, &secret)
            .env_remove(PASSPHRASE_ENV);
        if let Some(password) = password {
            cmd.env(PASSPHRASE_ENV, password);
        }
        let output = cmd.output().unwrap();
        for pipe in [&output.stdout, &output.stderr] {
            for value in [secret.lines().nth(1).unwrap(), passphrase] {
                assert!(
                    pipe.windows(value.len())
                        .all(|window| window != value.as_bytes())
                );
            }
        }
        output
    };
    for password in [None, Some("not the passphrase")] {
        let output = sign(password);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains(PASSPHRASE_ENV));
        assert!(!project.join("descriptor.json.minisig").exists());
    }
    success(sign(Some(passphrase)));
    let key = minisign_verify::PublicKey::from_base64(&pair.pk.to_base64()).unwrap();
    let signature = fs::read_to_string(project.join("descriptor.json.minisig")).unwrap();
    key.verify(
        bytes,
        &minisign_verify::Signature::decode(&signature).unwrap(),
        false,
    )
    .unwrap();
}
