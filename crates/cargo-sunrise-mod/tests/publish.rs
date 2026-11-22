use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use sunrise_mod_package::index::{self, registry::Entry};

#[path = "publish/native_release.rs"]
mod native_release;

const BIN: &str = env!("CARGO_BIN_EXE_cargo-sunrise-mod");
const ENTRY: &str = "mods/coords_hud.toml";
const PAGE: &[&str] = &[
    "--summary",
    "Shows coordinates.",
    "--owner",
    "ada",
    "--categories",
    "utility",
];

/// A complete 64 × 64 PNG, not just the header.
fn icon_png() -> Vec<u8> {
    STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAEAAAABACAAAAACPAi4CAAAAG0lEQVR4nO3BgQAAAADDoPlT3+AEVQEAAAB8AxBAAAFGOA0dAAAAAElFTkSuQmCC").unwrap()
}

fn manifest(dir: &Path, version: &str) {
    fs::write(
        dir.join("sunrise-mod.toml"),
        format!(
            "id = 'coords_hud'\nname = 'Coordinates HUD'\nversion = '{version}'\n\
             description = 'A coordinates display for players who are exploring new places.'\n\
             license = 'MIT'\nhomepage = 'https://github.com/ada/coords_hud'\n"
        ),
    )
    .unwrap();
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

fn success(result: Output) -> String {
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

#[cfg(unix)]
fn run_printed_check(root: &Path, publish_output: &str) -> String {
    let printed = publish_output
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("cargo sunrise-mod check "))
        .unwrap();
    // Substitute the test binary for the installed Cargo subcommand without changing its arguments.
    success(
        Command::new("sh")
            .current_dir(root)
            .env("SUNRISE_TEST_BIN", BIN)
            .arg("-c")
            .arg(format!(
                "cargo() {{ shift; \"$SUNRISE_TEST_BIN\" \"$@\"; }}\n{printed} --json"
            ))
            .output()
            .unwrap(),
    )
}

#[test]
#[ignore = "daily"]
fn bad_version_or_checkout_leaves_destination_and_outside_files_unchanged() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    let out = root.join("out");
    let checkout = root.join("registry");
    for directory in [out.join("mods"), checkout.join("mods")] {
        fs::create_dir_all(directory).unwrap();
    }
    fs::write(out.join(ENTRY), b"old destination").unwrap();
    for sentinel in ["sentinel.smod", "sentinel.toml"] {
        fs::write(root.join(sentinel), b"outside submission").unwrap();
    }
    let attempt = |expected: Option<&str>| {
        let result = run(root, &["publish", "--out", "out", "--registry", "registry"]);
        assert!(!result.status.success());
        if let Some(expected) = expected {
            let error = String::from_utf8_lossy(&result.stderr);
            assert!(error.contains(expected), "{error}");
        }
        assert_eq!(fs::read(out.join(ENTRY)).unwrap(), b"old destination");
        for sentinel in ["sentinel.smod", "sentinel.toml"] {
            assert_eq!(
                fs::read(root.join(sentinel)).unwrap(),
                b"outside submission"
            );
        }
        assert_eq!(fs::read_dir(&out).unwrap().count(), 1);
        assert!(!root.join("target").exists());
    };
    manifest(root, "1/../../sentinel");
    attempt(Some("sunrise-mod.toml: version"));
    manifest(root, "1.0.0+local");
    attempt(None);
    let too_long = format!("1.0.0-{}", "a".repeat(59));
    manifest(root, &too_long);
    attempt(None);

    manifest(root, "1");
    fs::write(checkout.join(ENTRY), "id = [").unwrap();
    attempt(Some(ENTRY));
    let entry = "kind = 'mod'\nowners = ['ada']\n";
    fs::write(
        checkout.join(ENTRY),
        format!("id = '../../sentinel'\n{entry}"),
    )
    .unwrap();
    attempt(Some(ENTRY));

    let correct = format!("id = 'coords_hud'\n{entry}");
    fs::write(checkout.join(ENTRY), &correct).unwrap();
    fs::write(checkout.join("mods/other.toml"), &correct).unwrap();
    attempt(Some("mods/other.toml"));

    fs::remove_file(checkout.join("mods/other.toml")).unwrap();
    fs::write(checkout.join(ENTRY), format!("{correct}\n[[versions]]\nversion = '1'\npublished = '2026-11-18'\ngame = '26.3'\nfile = 'coords_hud-1.smod'\nsha256 = '{}'\n", "0".repeat(64))).unwrap();
    attempt(Some("coords_hud 1"));

    fs::remove_file(checkout.join(ENTRY)).unwrap();
    fs::create_dir(checkout.join("collections")).unwrap();
    fs::write(
        checkout.join("collections/coords_hud.toml"),
        "id = 'coords_hud'\nname = 'HUD mods'\nsummary = 'A selection of HUD mods.'\n\
         owners = ['ada']\nicon = 'icons/coords_hud.png'\nmembers = [{ id = 'other_mod' }]\n",
    )
    .unwrap();
    // The checkout itself loads: the refusal below is the id collision, not a broken collection file.
    index::registry::load(&checkout).unwrap();
    attempt(None);
}

#[test]
#[ignore = "daily"]
fn check_command_accepts_an_update_without_shipping_history_or_the_icon_again() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    let checkout = "Ada's registry";
    let out = "Ada's new submission";
    manifest(root, "1.0.0");
    fs::write(root.join("icon.png"), icon_png()).unwrap();
    let mut args = vec!["publish", "--out", checkout];
    args.extend_from_slice(PAGE);
    success(run(root, &args));
    let published = fs::read_to_string(root.join(checkout).join(ENTRY)).unwrap();
    let old: Entry = toml::from_str(&published).unwrap();
    // A whole-index check of that checkout succeeds, and uses Sunrise's own asset host.
    success(run(
        root,
        &[
            "registry",
            "build",
            checkout,
            "--files",
            "Ada's registry/files",
            "--out",
            "old-index.json",
        ],
    ));
    let old_index: index::Index =
        serde_json::from_slice(&fs::read(root.join("old-index.json")).unwrap()).unwrap();
    assert_eq!(
        old_index.projects[0].versions[0].file.url,
        "https://github.com/sunrise-org/sunrise-mods/releases/download/coords_hud-1.0.0/coords_hud-1.0.0.smod"
    );

    manifest(root, "2.0.0");
    let manifest_path = root.join("sunrise-mod.toml");
    let source = fs::read_to_string(&manifest_path).unwrap();
    fs::write(
        manifest_path,
        format!("{source}\n[dependencies]\nfixture_missing_dep = '^2'\n"),
    )
    .unwrap();
    let output = success(run(
        root,
        &["publish", "--registry", checkout, "--out", out],
    ));
    assert!(
        output.lines().any(|line| line
            .trim_start()
            .strip_prefix("- ")
            .is_some_and(|bullet| bullet.contains("fixture_missing_dep") && bullet.contains("^2"))),
        "{output}"
    );
    let staged_entry = fs::read_to_string(root.join(out).join(ENTRY)).unwrap();
    let new: Entry = toml::from_str(&staged_entry).unwrap();
    assert_eq!(new.versions[0].version, "2.0.0");
    assert_eq!(&new.versions[1..], &old.versions);
    assert_eq!(new.icon, old.icon);
    assert!(!root.join(out).join(new.icon.unwrap()).exists());
    assert!(
        !root
            .join(out)
            .join("files")
            .join(&old.versions[0].sha256)
            .exists()
    );
    assert_eq!(
        fs::read_to_string(root.join(checkout).join(ENTRY)).unwrap(),
        published
    );

    assert!(output.contains("cargo sunrise-mod check "));
    #[cfg(unix)]
    let checked = run_printed_check(root, &output);
    #[cfg(not(unix))]
    let checked = success(run(
        root,
        &[
            "check",
            "Ada's new submission/coords_hud-2.0.0.smod",
            "--json",
        ],
    ));
    let checked: serde_json::Value = serde_json::from_str(&checked).unwrap();
    assert_eq!(
        checked["report"]["sha256"].as_str(),
        Some(new.versions[0].sha256.as_str())
    );
}

#[test]
#[ignore = "daily"]
fn signed_publish_stages_the_exact_prebuilt_package_with_the_checkout_key() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../sunrise-mod-package/tests/index");
    let signed = fs::read(fixtures.join("coords_hud-0.1.0.smod")).unwrap();
    let key_file = fs::read_to_string(fixtures.join("test.pub")).unwrap();
    let key = key_file.lines().last().unwrap();
    let signature = fixtures.join("coords_hud-0.1.0.smod.minisig");
    manifest(root, "0.1.0");
    fs::create_dir_all(root.join("registry/mods")).unwrap();
    let listed = format!("id = 'coords_hud'\nkind = 'mod'\nowners = ['ada']\nkey = '{key}'\n");
    fs::write(root.join("registry").join(ENTRY), listed).unwrap();
    let args = ["publish", "--registry", "registry", "--out", "out"];
    let unsigned = run(root, &args);
    assert!(!unsigned.status.success());
    assert!(String::from_utf8_lossy(&unsigned.stderr).contains("--signature"));
    assert!(!root.join("target").exists());
    assert!(!root.join("out").exists());

    let built = root.join("target/sunrise-mod/coords_hud.smod");
    fs::create_dir_all(built.parent().unwrap()).unwrap();
    fs::write(&built, &signed).unwrap();
    let mut args = args.to_vec();
    args.extend(["--key", key, "--signature", signature.to_str().unwrap()]);
    // Same id and version, but a dependency added to the source after signing: the staged
    // package does not require it, so publish reports no unmet dependency for it.
    let source = fs::read_to_string(root.join("sunrise-mod.toml")).unwrap();
    fs::write(
        root.join("sunrise-mod.toml"),
        format!("{source}\n[dependencies]\nunlisted_after_signing = '^1'\n"),
    )
    .unwrap();
    let output = success(run(root, &args));
    assert!(!output.contains("unlisted_after_signing"), "{output}");
    assert!(output.contains("--key ") && output.contains("--signature "));
    #[cfg(unix)]
    {
        let checked: serde_json::Value =
            serde_json::from_str(&run_printed_check(root, &output)).unwrap();
        assert_eq!(checked["report"]["signed"].as_bool(), Some(true));
    }
    let staged = root.join("out");
    let entry: Entry = toml::from_str(&fs::read_to_string(staged.join(ENTRY)).unwrap()).unwrap();
    assert_eq!(entry.key.as_deref(), Some(key));
    assert_eq!(
        entry.versions[0].signature.as_deref(),
        Some(fs::read_to_string(signature).unwrap().as_str())
    );
    assert_eq!(
        fs::read(staged.join("coords_hud-0.1.0.smod")).unwrap(),
        signed
    );
    assert_eq!(fs::read(&built).unwrap(), signed);
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn output_replaces_file_symlinks_but_refuses_linked_directories() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    manifest(root, "1");
    let outside = root.join("outside");
    fs::create_dir(&outside).unwrap();
    let sentinel = outside.join("coords_hud.toml");
    fs::write(&sentinel, b"untouched").unwrap();
    fs::set_permissions(&sentinel, fs::Permissions::from_mode(0o700)).unwrap();
    fs::create_dir(root.join("out")).unwrap();
    fs::write(root.join("out/PULL_REQUEST.md"), b"old request").unwrap();
    symlink(&outside, root.join("out/mods")).unwrap();
    let args = ["publish", "--out", "out"];
    let blocked = run(root, &args);
    assert!(!blocked.status.success());
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("out/mods"));
    assert_eq!(fs::read(&sentinel).unwrap(), b"untouched");
    assert_eq!(
        fs::read(root.join("out/PULL_REQUEST.md")).unwrap(),
        b"old request"
    );
    assert!(!root.join("target").exists());

    fs::remove_file(root.join("out/mods")).unwrap();
    fs::create_dir(root.join("out/mods")).unwrap();
    let dest = root.join("out").join(ENTRY);
    symlink(&sentinel, &dest).unwrap();
    success(run(root, &args));
    let metadata = fs::symlink_metadata(dest).unwrap();
    assert!(metadata.is_file());
    assert_eq!(metadata.permissions().mode() & 0o111, 0);
    assert_eq!(
        fs::metadata(&sentinel).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(fs::read(&sentinel).unwrap(), b"untouched");

    // The build an unsigned publish runs first writes through no link either.
    fs::remove_dir_all(root.join("target")).unwrap();
    symlink(&outside, root.join("target")).unwrap();
    assert!(!run(root, &args).status.success());
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
    fs::remove_file(root.join("target")).unwrap();
    fs::create_dir_all(root.join("target/sunrise-mod")).unwrap();
    // A link at a guessable temporary name beside the package is not written through.
    symlink(
        &sentinel,
        root.join("target/sunrise-mod/coords_hud.smod.partial"),
    )
    .unwrap();
    success(run(root, &args));
    assert_eq!(fs::read(&sentinel).unwrap(), b"untouched");
}
