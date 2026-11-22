use super::*;
use ring::signature::{Ed25519KeyPair, KeyPair};

const BUILD: &str = "0123456789abcdef0123456789abcdef01234567";

/// A release folder `sdk-<BUILD>` under a base folder, signed with a fresh key: the base and the
/// key.
fn release(nightly: Option<&str>) -> (tempfile::TempDir, PublicKey) {
    let base = tempfile::tempdir().unwrap();
    let folder = base.path().join(format!("sdk-{BUILD}"));
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join(SURFACE), r#"{"hash":"ab"}"#).unwrap();
    // Any archive the unpack reads stands in for the API source and the build data.
    for archive in [API_SOURCE, build_data::ARCHIVE] {
        fs::write(
            folder.join(archive),
            include_bytes!("../../../sunrise-mod-package/src/sdk/fixture.tar.xz"),
        )
        .unwrap();
    }
    let build = sdk::Build {
        commit: BUILD.into(),
        game: sdk::GAME.into(),
        date: "2026-11-13".into(),
        nightly: nightly.map(str::to_owned),
    };
    let index = Index::of_folder(&folder, build, "1.99.1".into()).unwrap();
    fs::write(folder.join(sdk::INDEX), index.to_json()).unwrap();
    let digest = |name: &str| format!("{:x}", Sha256::digest(fs::read(folder.join(name)).unwrap()));
    let sums = sunrise_release_sign::sums_file(
        &[sdk::INDEX, SURFACE, API_SOURCE].map(|name| (name.to_owned(), digest(name))),
    );
    fs::write(folder.join(sunrise_release_sign::SUMS), &sums).unwrap();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
    let message = sunrise_release_sign::message(&format!("sdk-{BUILD}"), sums.as_bytes());
    use base64::Engine;
    let signature = base64::engine::general_purpose::STANDARD.encode(pair.sign(&message).as_ref());
    fs::write(
        folder.join(sunrise_release_sign::SIGNATURE),
        signature + "\n",
    )
    .unwrap();
    let key: PublicKey = pair.public_key().as_ref().try_into().unwrap();
    (base, key)
}

#[test]
fn a_signed_release_is_unpacked_once_and_reused() {
    let (base, key) = release(Some("nightly-2026-11-13"));
    let project = tempfile::tempdir().unwrap();
    let dir = project.path().join(DIR);
    let base = Base::Folder(base.path().to_owned());
    let pinned = fetch(&base, &[key], BUILD, &dir, true).unwrap();
    assert_eq!(pinned.build, BUILD);
    assert_eq!(pinned.name, "nightly-2026-11-13 (0123456)");
    assert!(dir.join(SURFACE).is_file());
    assert!(dir.join("api/deps/a.rmeta").is_file());
    assert!(!dir.join(API_SOURCE).exists());
    // With the same trust, the folder is used as it is, with nothing read from the base.
    let gone = Base::Folder(project.path().join("nowhere"));
    assert_eq!(
        fetch(&gone, &[key], BUILD, &dir, true).unwrap().name,
        pinned.name
    );
}

#[test]
fn a_cached_local_sdk_is_refused_under_release_only_trust() {
    let (base, key) = release(None);
    let base = Base::Folder(base.path().to_owned());
    let project = tempfile::tempdir().unwrap();
    let release_keys = sunrise_release_sign::trusted_keys();
    let mut local_keys = release_keys.clone();
    local_keys.push(key);
    for api in [true, false] {
        // Same verifier for the project's API source and the per-user native SDK metadata.
        let dir = project.path().join(if api { DIR } else { "sdk" });
        fetch(&base, &local_keys, BUILD, &dir, api).unwrap();
        assert!(fetch(&base, &release_keys, BUILD, &dir, api).is_err());
        assert!(!dir.join(STAMP).exists());
        assert!(!dir.join(sdk::INDEX).exists());
        assert!(!dir.join("api").exists());
    }
}

#[test]
fn a_release_not_signed_by_a_trusted_key_or_not_matching_its_sums_is_refused() {
    let (base, key) = release(None);
    let project = tempfile::tempdir().unwrap();
    let dir = project.path().join(DIR);
    let folder = base.path().join(format!("sdk-{BUILD}"));
    let base = Base::Folder(base.path().to_owned());
    let (_, other) = release(None);
    let error = fetch(&base, &[other], BUILD, &dir, true).unwrap_err();
    assert!(error.contains("signature doesn't match"), "{error}");
    assert!(!dir.join(STAMP).exists());
    // A changed sdk.json is not the one the signed sums list.
    let index = fs::read_to_string(folder.join(sdk::INDEX)).unwrap();
    fs::write(
        folder.join(sdk::INDEX),
        index.replacen("\"ab\"", "\"cd\"", 1),
    )
    .unwrap();
    let error = fetch(&base, &[key], BUILD, &dir, true).unwrap_err();
    assert!(
        error.contains("is not the one its signed SHA256SUMS list"),
        "{error}"
    );
    fs::write(folder.join(sdk::INDEX), index).unwrap();
    // An archive that is not the one sdk.json lists.
    fs::write(folder.join(API_SOURCE), b"not the archive").unwrap();
    let error = fetch(&base, &[key], BUILD, &dir, true).unwrap_err();
    assert!(error.starts_with(&format!("sdk-{BUILD}: ")), "{error}");
    assert!(!dir.join(STAMP).exists());
    // Another build's release under this build's name.
    let other = "89abcdef0123456789abcdef0123456789abcdef";
    fs::rename(&folder, base_folder(&base).join(format!("sdk-{other}"))).unwrap();
    assert!(fetch(&base, &[key], other, &dir, false).is_err());
}

#[test]
fn build_data_comes_from_the_signed_sdk_once_and_only_from_it() {
    let (base, key) = release(None);
    let folder = base.path().join(format!("sdk-{BUILD}"));
    let base = Base::Folder(base.path().to_owned());
    let project = tempfile::tempdir().unwrap();
    // A build with no SDK release is refused by name.
    let other = "89abcdef0123456789abcdef0123456789abcdef";
    let error = build_data_from(&base, &[key], project.path(), other).unwrap_err();
    assert!(
        error.starts_with(&format!(
            "game build {other} has no signed SDK (release sdk-{other}): "
        )),
        "{error}"
    );
    // An archive that is not the one sdk.json lists leaves no build data.
    let archive = fs::read(folder.join(build_data::ARCHIVE)).unwrap();
    fs::write(folder.join(build_data::ARCHIVE), b"not the archive").unwrap();
    let error = build_data_from(&base, &[key], project.path(), BUILD).unwrap_err();
    assert!(error.starts_with(&format!("sdk-{BUILD}: ")), "{error}");
    let into = project.path().join(CACHE).join(BUILD).join(BUILD_DATA);
    assert!(!into.exists());
    fs::write(folder.join(build_data::ARCHIVE), archive).unwrap();
    assert_eq!(
        build_data_from(&base, &[key], project.path(), BUILD).unwrap(),
        into
    );
    assert!(into.join("deps/a.rmeta").is_file());
    assert!(!into.with_file_name(build_data::ARCHIVE).exists());
    // Unpacked once: later reads need no release.
    let gone = Base::Folder(project.path().join("nowhere"));
    assert_eq!(
        build_data_from(&gone, &[key], project.path(), BUILD).unwrap(),
        into
    );
}

#[test]
fn cached_local_build_data_is_refused_under_release_only_trust() {
    let (base, key) = release(None);
    let base = Base::Folder(base.path().to_owned());
    let project = tempfile::tempdir().unwrap();
    let release_keys = sunrise_release_sign::trusted_keys();
    let mut local_keys = release_keys.clone();
    local_keys.push(key);
    let into = build_data_from(&base, &local_keys, project.path(), BUILD).unwrap();
    assert!(into.is_dir());
    assert!(build_data_from(&base, &release_keys, project.path(), BUILD).is_err());
    assert!(!into.exists());
}

fn base_folder(base: &Base) -> &Path {
    match base {
        Base::Folder(folder) => folder,
        Base::Url(_) => unreachable!("a folder"),
    }
}

/// `<base>/<tag>/` holding `index` as its `sdk.json`, with SHA256SUMS listing `listed` (the index's
/// own bytes when None) signed by `pair` for `signed_for` into `signature`.
fn tag_release(
    base: &Path,
    tag: &str,
    index: &str,
    listed: Option<&str>,
    signed_for: &str,
    signature: &str,
    pair: &Ed25519KeyPair,
) {
    let folder = base.join(tag);
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join(sdk::INDEX), index).unwrap();
    let sums = sunrise_release_sign::sums_file(&[(
        sdk::INDEX.to_owned(),
        sunrise_release_sign::sha256_hex(listed.unwrap_or(index).as_bytes()),
    )]);
    fs::write(folder.join(sunrise_release_sign::SUMS), &sums).unwrap();
    let message = sunrise_release_sign::message(signed_for, sums.as_bytes());
    use base64::Engine;
    let signed = base64::engine::general_purpose::STANDARD.encode(pair.sign(&message).as_ref());
    fs::write(folder.join(signature), signed + "\n").unwrap();
}

fn index_of(commit: &str) -> String {
    format!(r#"{{"schema":1,"build":{{"commit":"{commit}"}}}}"#)
}

#[test]
fn a_build_is_named_by_sha_short_sha_or_a_release_tag() {
    let (base, _) = release(None);
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
    let keys: [PublicKey; 1] = [pair.public_key().as_ref().try_into().unwrap()];
    let index = index_of(BUILD);
    // A Nightly name is a pointer, as a mirror holds it; a Stable tag and a Dev build are signed
    // for themselves.
    fs::create_dir_all(base.path().join("nightly-2026-11-13")).unwrap();
    fs::write(
        base.path().join("nightly-2026-11-13").join(sdk::INDEX),
        &index,
    )
    .unwrap();
    let stable = "26.3-0.1.0";
    let dev = format!(
        "{}-2026-11-21-0123456",
        sunrise_release_sign::Channel::Dev.tag()
    );
    let v2 = sunrise_release_sign::SIGNATURE_V2;
    tag_release(
        base.path(),
        stable,
        &index,
        None,
        stable,
        sunrise_release_sign::SIGNATURE,
        &pair,
    );
    tag_release(base.path(), &dev, &index, None, &dev, v2, &pair);
    let base = Base::Folder(base.path().to_owned());
    assert_eq!(resolve(&base, &keys, BUILD).unwrap(), BUILD);
    assert_eq!(resolve(&base, &keys, "0123456").unwrap(), BUILD);
    for tag in ["nightly-2026-11-13", stable, &dev] {
        assert_eq!(resolve(&base, &keys, tag).unwrap(), BUILD, "{tag}");
    }
    let error = resolve(&base, &keys, "nightly").unwrap_err();
    assert!(error.starts_with("Release nightly has no SDK"), "{error}");
    assert!(
        resolve(&base, &keys, "89abcde")
            .unwrap_err()
            .contains("no SDK release")
    );
    assert!(
        resolve(&base, &keys, "main")
            .unwrap_err()
            .contains("is not a build")
    );
}

#[test]
fn a_release_tag_resolves_only_through_its_own_signed_index() {
    let base = tempfile::tempdir().unwrap();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
    let keys: [PublicKey; 1] = [pair.public_key().as_ref().try_into().unwrap()];
    let older = "89abcdef0123456789abcdef0123456789abcdef";
    let channel = sunrise_release_sign::Channel::Dev.tag();
    let v1 = sunrise_release_sign::SIGNATURE;
    let v2 = sunrise_release_sign::SIGNATURE_V2;
    // Another build's index served under the tag, the tag's sums listing the real one.
    tag_release(
        base.path(),
        "26.3-0.1.0",
        &index_of(older),
        Some(&index_of(BUILD)),
        "26.3-0.1.0",
        v1,
        &pair,
    );
    // Sums signed for another release.
    tag_release(
        base.path(),
        "26.3-0.1.1",
        &index_of(BUILD),
        None,
        "26.3-0.1.0",
        v1,
        &pair,
    );
    // A Dev tag whose signed index names a build that its short sha does not start.
    let dev = format!("{channel}-2026-11-21-0123456");
    tag_release(base.path(), &dev, &index_of(older), None, &dev, v2, &pair);
    // A Dev build signed only for its channel.
    let unsigned = format!("{channel}-2026-11-22-0123456");
    tag_release(
        base.path(),
        &unsigned,
        &index_of(BUILD),
        None,
        channel,
        v1,
        &pair,
    );
    let base = Base::Folder(base.path().to_owned());
    for tag in ["26.3-0.1.0", "26.3-0.1.1", &dev, &unsigned] {
        assert!(resolve(&base, &keys, tag).is_err(), "{tag}");
    }
}

#[test]
fn a_nightly_name_is_only_nightly_or_a_dated_one() {
    let root = tempfile::tempdir().unwrap();
    let base = root.path().join("releases");
    // A name that read through the base would reach this decoy outside it.
    fs::create_dir_all(base.join("nightly-")).unwrap();
    let decoy = root.path().join("decoy");
    fs::create_dir_all(&decoy).unwrap();
    fs::write(
        decoy.join(sdk::INDEX),
        format!(r#"{{"schema":1,"build":{{"commit":"{BUILD}"}}}}"#),
    )
    .unwrap();
    let base = Base::Folder(base);
    for text in [
        "nightly-/../../decoy",
        "nightly-2026-11-1x",
        "nightly-2026/11/13",
        "nightly-2026-11-130",
        "nightly-",
        "26.3-0.1.0-rc.1",
        "26.3-0.1",
        "026.3-0.1.0",
        "26.3.1.2-0.1.0",
        "dev-2026-11-21",
        "dev-2026-11-21-01234",
        "dev-2026-11-21-/../../decoy",
    ] {
        let error = resolve(&base, &[], text).unwrap_err();
        assert!(error.contains("is not a build"), "{text}: {error}");
    }
}

#[test]
fn a_project_inside_a_game_checkout_builds_against_it_and_a_pinned_one_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let write = |path: &Path, text: &str| {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    let workspace = "[workspace]\nmembers = ['crates/sunrise-mod-api']\n";
    let dependency = |path: &str| {
        format!(
            "[package]\nname='m'\nversion='1.0.0'\n[dependencies]\nsunrise-mod-api = {{ path = '{path}' }}\n"
        )
    };
    // The game's own fixture: a project inside the checkout, its dependency the checkout's crate.
    let game = dir.path().join("game");
    write(&game.join("Cargo.toml"), workspace);
    write(&game.join("crates/sunrise-mod-api/Cargo.toml"), "");
    let fixture = game.join("mods/fixture");
    write(
        &fixture.join("Cargo.toml"),
        &dependency("../../crates/sunrise-mod-api"),
    );
    assert_eq!(
        game_tree(&fixture).unwrap(),
        Some(game.canonicalize().unwrap())
    );
    // A mod made from the template: its dependency is the fetched SDK's own workspace.
    let project = dir.path().join("mod");
    write(&project.join(".sunrise/sdk/api/Cargo.toml"), workspace);
    write(
        &project.join(".sunrise/sdk/api/crates/sunrise-mod-api/Cargo.toml"),
        "",
    );
    write(
        &project.join("Cargo.toml"),
        &dependency(".sunrise/sdk/api/crates/sunrise-mod-api"),
    );
    assert_eq!(game_tree(&project).unwrap(), None);
    // A project outside the checkout that names its crate is still pinned, not built against it.
    let outside = dir.path().join("outside");
    write(
        &outside.join("Cargo.toml"),
        &dependency("../game/crates/sunrise-mod-api"),
    );
    assert_eq!(game_tree(&outside).unwrap(), None);
}
