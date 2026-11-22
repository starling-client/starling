//! Installing from a local test registry: a real content `.smod`, an index
//! naming it, and the game folder the launcher shares.
use super::*;
use std::path::PathBuf;
use sunrise_mod_package::index::browser_state::MANIFEST;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sunrise-mod-registry-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A content mod `ruby` at `version`: a manifest and one lang file.
fn ruby(version: &str) -> Vec<u8> {
    let manifest = format!(
        "id = \"ruby\"\nname = \"Ruby\"\nversion = \"{version}\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n"
    );
    sunrise_mod_package::Package::write_files(
        &manifest,
        &[(
            "assets/ruby/lang/en_us.json",
            br#"{"item.ruby.ruby": "Ruby"}"#,
        )],
    )
    .unwrap()
}

/// A registry listing `ruby` at `version` with these bytes (the file at
/// `url`), and a native mod and a mod for another Minecraft version that the
/// game doesn't offer.
fn registry(version: &str, bytes: &[u8], url: &str) -> Index {
    let file = |name: &str, size: usize, sha: String| serde_json::json!({ "name": name, "url": url, "size": size, "sha256": sha });
    let ruby = serde_json::json!({
        "id": "ruby", "kind": "mod", "name": "Ruby", "summary": "Rubies",
        "description": "# Ruby\nGems.", "authors": ["someone"], "license": "MIT",
        "source": "https://github.com/someone/ruby", "categories": ["decoration"],
        "tags": ["gems"], "downloads": 5,
        "versions": [{ "version": version, "published": "2026-11-05", "api": "0.2",
            "side": "both", "capabilities": ["registries"], "game": crate::mod_registry::GAME,
            "file": file(&format!("ruby-{version}.smod"), bytes.len(), index::sha256(bytes)) }]
    });
    let old = serde_json::json!({
        "id": "old", "kind": "mod", "name": "Old", "summary": "For 1.21",
        "license": "MIT", "source": "https://github.com/someone/old",
        "versions": [{ "version": "1", "api": "0.2", "game": "1.21.1",
            "capabilities": ["registries"],
            "file": file("old-1.smod", 1, "a".repeat(64)) }]
    });
    let native = serde_json::json!({
        "id": "zoom", "kind": "native-mod", "name": "Zoom", "summary": "Zoom",
        "license": "MIT", "source": "https://github.com/someone/zoom", "downloads": 99,
        "versions": [{ "version": "1", "api": "0.2", "game": crate::mod_registry::GAME,
            "file": file("zoom-1.smod", 1, "b".repeat(64)) }]
    });
    let text = serde_json::json!({ "schema": 1, "projects": [ruby, old, native] }).to_string();
    Index::parse_local(&text).unwrap()
}

fn serve(bytes: Vec<u8>) -> impl Fn(&str, u64) -> Result<Vec<u8>, String> {
    move |_, max| match bytes.len() as u64 <= max {
        true => Ok(bytes.clone()),
        false => Err("too large".into()),
    }
}

#[test]
fn the_browser_lists_only_content_mods_for_this_game() {
    let bytes = ruby("1.0");
    let index = registry("1.0", &bytes, "file:///ruby.smod");
    let home = scratch("list");
    let listed = listings(&index, &home);
    let ids: Vec<&str> = listed.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(ids, ["ruby"], "no native mod, no mod for another version");
    assert_eq!(listed[0].version, "1.0");
    assert_eq!(listed[0].page, "/ruby");
    assert_eq!(listed[0].installed, None);
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn an_install_is_checked_then_recorded_as_the_launchers_own() {
    let bytes = ruby("1.0");
    let index = registry("1.0", &bytes, "file:///ruby.smod");
    let home = scratch("install");
    // An entry of the launcher's this game doesn't model stays as it was.
    let theirs = serde_json::json!({ "entries": [{ "source": "modrinth", "project": "abc",
        "slug": "faithful", "title": "Faithful", "kind": "resource-pack", "version": "1",
        "number": "1", "file": "resourcepacks/faithful.zip", "checksum": "SHA-512:00",
        "page": "https://modrinth.com/resourcepack/faithful" }] });
    fs::write(home.join(MANIFEST), theirs.to_string()).unwrap();

    let done = install(&index, &home, "ruby", &serve(bytes.clone())).unwrap();
    assert_eq!(done.version, "1.0");
    assert!(!done.verified, "the sample isn't signed");
    let path = home.join("sunrise-mods/ruby.smod");
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(
        sunrise_mod_package::Package::read(&path)
            .unwrap()
            .manifest
            .id,
        "ruby"
    );
    let record: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.join(MANIFEST)).unwrap()).unwrap();
    let entries = record["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["slug"], "faithful");
    assert_eq!(entries[1]["source"], "sunrise");
    assert_eq!(entries[1]["file"], "sunrise-mods/ruby.smod");
    assert_eq!(entries[1]["verdict"], "Unsigned");
    assert_eq!(listings(&index, &home)[0].installed.as_deref(), Some("1.0"));

    // An update replaces the file and the entry.
    let newer = ruby("1.1");
    let index = registry("1.1", &newer, "file:///ruby.smod");
    assert!(listings(&index, &home)[0].has_update());
    install(&index, &home, "ruby", &serve(newer.clone())).unwrap();
    assert_eq!(fs::read(&path).unwrap(), newer);
    let record: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.join(MANIFEST)).unwrap()).unwrap();
    assert_eq!(record["entries"].as_array().unwrap().len(), 2);
    assert_eq!(record["entries"][1]["version"], "1.1");
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn install_holds_the_shared_lock_through_download_and_releases_it_on_error() {
    let bytes = ruby("1.0");
    let index = registry("1.0", &bytes, "file:///ruby.smod");
    let home = scratch("transaction-error");
    let error = install(&index, &home, "ruby", &|_, _| {
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    assert!(sunrise_mod_package::install_lock::try_lock_home(&home).is_none());
                })
                .join()
                .unwrap();
        });
        Err("download failed".into())
    })
    .unwrap_err();
    assert_eq!(error, "download failed");
    assert!(!home.join(MANIFEST).exists());
    assert!(!home.join("sunrise-mods/ruby.smod").exists());
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let _guard = sunrise_mod_package::install_lock::try_lock_home(&home).unwrap();
            })
            .join()
            .unwrap();
    });
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn install_reads_the_manifest_after_the_other_installer_finishes() {
    let bytes = ruby("1.0");
    let index = registry("1.0", &bytes, "file:///ruby.smod");
    let home = scratch("transaction-read");
    std::thread::scope(|scope| {
        let guard = sunrise_mod_package::install_lock::lock_home(&home).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let target = &home;
        let registry = &index;
        let worker = scope.spawn(move || {
            tx.send(()).unwrap();
            install(registry, target, "ruby", &serve(bytes)).unwrap();
        });
        rx.recv().unwrap();
        let mut installed = Installed::default();
        installed.entries.push(serde_json::json!({
            "source": "modrinth", "project": "pack", "file": "resourcepacks/pack.zip"
        }));
        installed.save(&home).unwrap();
        drop(guard);
        worker.join().unwrap();
    });
    let installed = Installed::load(&home).unwrap();
    assert_eq!(installed.entries.len(), 2);
    assert_eq!(installed.entries[0]["project"], "pack");
    assert!(installed.position("ruby").is_some());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn a_lock_error_never_downloads_or_changes_installed_files() {
    let bytes = ruby("1.0");
    let index = registry("1.0", &bytes, "file:///ruby.smod");
    let home = scratch("lock-error");
    fs::create_dir(home.join(".mod-browser.lock")).unwrap();
    let error = install(&index, &home, "ruby", &|_, _| {
        panic!("lock errors must refuse the operation before downloading")
    })
    .unwrap_err();
    assert!(error.contains("cannot lock mod browser"), "{error}");
    assert!(!home.join(MANIFEST).exists());
    assert!(!home.join("sunrise-mods").exists());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn a_native_pin_is_not_replaced_by_a_content_version_of_the_same_project() {
    let bytes = ruby("1.1");
    let index = registry("1.1", &bytes, "file:///ruby.smod");
    let home = scratch("native-pin");
    // A missing native file is still the launcher's pin, not permission for
    // the game to replace its metadata or switch its kind.
    let entry = serde_json::json!({
        "source": "sunrise", "project": "ruby", "version": "1.0",
        "file": "sunrise-native-mods/ruby.smod", "pin": "exact-build"
    });
    let manifest = serde_json::json!({ "entries": [entry] }).to_string();
    fs::write(home.join(MANIFEST), &manifest).unwrap();
    let error = install(&index, &home, "ruby", &|_, _| {
        panic!("a native pin must be refused before download")
    })
    .unwrap_err();
    assert!(error.contains("launcher"), "{error}");
    assert_eq!(fs::read_to_string(home.join(MANIFEST)).unwrap(), manifest);
    assert!(!home.join("sunrise-mods/ruby.smod").exists());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn an_unrecognized_managed_entry_is_not_replaced() {
    let bytes = ruby("1.1");
    let index = registry("1.1", &bytes, "file:///ruby.smod");
    let home = scratch("unknown-entry");
    let manifest = serde_json::json!({ "entries": [{
        "source": "sunrise", "project": "ruby", "version": "1.0",
        "pin": "exact-build"
    }] })
    .to_string();
    fs::write(home.join(MANIFEST), &manifest).unwrap();
    let error = install(&index, &home, "ruby", &|_, _| {
        panic!("unknown managed metadata must be refused before download")
    })
    .unwrap_err();
    assert!(error.contains("launcher"), "{error}");
    assert_eq!(fs::read_to_string(home.join(MANIFEST)).unwrap(), manifest);
    assert!(!home.join("sunrise-mods/ruby.smod").exists());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn several_entries_of_the_project_are_refused_before_download() {
    let bytes = ruby("1.1");
    let index = registry("1.1", &bytes, "file:///ruby.smod");
    let home = scratch("duplicate-entries");
    fs::create_dir_all(home.join("sunrise-mods")).unwrap();
    fs::write(home.join("sunrise-mods/ruby.smod"), ruby("1.0")).unwrap();
    let row = |title: &str| {
        serde_json::json!({ "source": "sunrise", "project": "ruby", "title": title,
            "version": "1.0", "file": "sunrise-mods/ruby.smod" })
    };
    let manifest =
        serde_json::json!({ "entries": [row("Ruby"), row("Ruby, renamed later")] }).to_string();
    fs::write(home.join(MANIFEST), &manifest).unwrap();
    assert!(
        install(&index, &home, "ruby", &|_, _| {
            panic!("an ambiguous manifest must be refused before download")
        })
        .is_err()
    );
    assert_eq!(fs::read_to_string(home.join(MANIFEST)).unwrap(), manifest);
    assert_eq!(
        fs::read(home.join("sunrise-mods/ruby.smod")).unwrap(),
        ruby("1.0")
    );
    assert!(!home.join(".mod-browser.pending.json").try_exists().unwrap());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn a_same_version_install_refuses_another_projects_aliased_claim() {
    let bytes = ruby("1.0");
    let index = registry("1.0", &bytes, "file:///ruby.smod");
    let home = scratch("same-version-competing-claim");
    let target = home.join("sunrise-mods/ruby.smod");
    fs::create_dir_all(home.join("sunrise-mods")).unwrap();
    fs::write(&target, &bytes).unwrap();
    let manifest = serde_json::json!({ "entries": [
        {"source": "sunrise", "project": "ruby", "version": "1.0", "file": "sunrise-mods/ruby.smod"},
        {"source": "sunrise", "project": "gem", "title": "Gem", "file": "./sunrise-mods/ruby.smod"}
    ] }).to_string();
    fs::write(home.join(MANIFEST), &manifest).unwrap();

    let error = install(&index, &home, "ruby", &|_, _| {
        panic!("a competing file claim must be refused before download")
    })
    .unwrap_err();
    assert!(error.contains("Gem"), "{error}");
    assert_eq!(fs::read_to_string(home.join(MANIFEST)).unwrap(), manifest);
    assert_eq!(fs::read(target).unwrap(), bytes);
    assert!(!home.join(".mod-browser.pending.json").try_exists().unwrap());
    fs::remove_dir_all(home).unwrap();
}

#[cfg(unix)]
#[test]
fn an_install_refuses_a_claim_through_a_symlinked_directory_before_download() {
    let old = ruby("1.0");
    let newer = ruby("1.1");
    let index = registry("1.1", &newer, "file:///ruby.smod");
    let home = scratch("symlinked-claim");
    let target = home.join("sunrise-mods/ruby.smod");
    fs::create_dir(home.join("sunrise-mods")).unwrap();
    fs::write(&target, &old).unwrap();
    std::os::unix::fs::symlink("sunrise-mods", home.join("alias")).unwrap();
    let manifest = serde_json::json!({ "entries": [
        {"source": "sunrise", "project": "ruby", "version": "1.0", "file": "sunrise-mods/ruby.smod"},
        {"source": "modrinth", "project": "gem", "title": "Gem", "file": "alias/ruby.smod"}
    ] }).to_string();
    fs::write(home.join(MANIFEST), &manifest).unwrap();

    let error = install(&index, &home, "ruby", &|_, _| {
        panic!("a claim through a directory symlink must be refused before download")
    })
    .unwrap_err();
    assert!(error.contains("Gem"), "{error}");
    assert_eq!(fs::read_to_string(home.join(MANIFEST)).unwrap(), manifest);
    assert_eq!(fs::read(&target).unwrap(), old);
    assert_eq!(fs::read(home.join("alias/ruby.smod")).unwrap(), old);
    assert!(!home.join(".mod-browser.pending.json").try_exists().unwrap());
    fs::remove_dir_all(home).unwrap();
}

#[cfg(unix)]
#[test]
fn an_install_accepts_a_mods_directory_linked_outside_the_home() {
    let bytes = ruby("1.0");
    let index = registry("1.0", &bytes, "file:///ruby.smod");
    let home = scratch("linked-mods-home");
    let shared_mods = scratch("linked-mods-destination");
    std::os::unix::fs::symlink(&shared_mods, home.join("sunrise-mods")).unwrap();

    install(&index, &home, "ruby", &serve(bytes.clone())).unwrap();
    assert_eq!(fs::read(shared_mods.join("ruby.smod")).unwrap(), bytes);
    let list = Installed::load(&home).unwrap();
    assert_eq!(list.entries[0]["file"], "sunrise-mods/ruby.smod");
    assert!(!home.join(".mod-browser.pending.json").try_exists().unwrap());
    fs::remove_dir_all(home).unwrap();
    fs::remove_dir_all(shared_mods).unwrap();
}

#[test]
fn an_install_ignores_a_missing_unicode_name_in_an_unrelated_folder() {
    let bytes = ruby("1.0");
    let index = registry("1.0", &bytes, "file:///ruby.smod");
    let home = scratch("missing-pack-claim");
    // Neither folder exists yet. Only the filename in the pack's future directory is uncertain.
    let pack = serde_json::json!({"source": "modrinth", "project": "pack", "file": "resourcepacks/déjà.zip"});
    Installed {
        entries: vec![pack.clone()],
    }
    .save(&home)
    .unwrap();

    install(&index, &home, "ruby", &serve(bytes.clone())).unwrap();
    assert_eq!(
        fs::read(home.join("sunrise-mods/ruby.smod")).unwrap(),
        bytes
    );
    let list = Installed::load(&home).unwrap();
    assert_eq!(list.entries[0], pack);
    assert_eq!(list.entries[1]["project"], "ruby");
    assert!(!home.join("resourcepacks").try_exists().unwrap());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn an_unreadable_manifest_is_not_treated_as_an_empty_installation() {
    let bytes = ruby("1.0");
    let index = registry("1.0", &bytes, "file:///ruby.smod");
    let home = scratch("invalid-manifest");
    for manifest in ["not json", r#"{"entries": {}}"#] {
        fs::write(home.join(MANIFEST), manifest).unwrap();
        let error = install(&index, &home, "ruby", &|_, _| {
            panic!("an unreadable manifest must be refused before download")
        })
        .unwrap_err();
        assert!(error.contains(MANIFEST), "{error}");
        assert_eq!(fs::read_to_string(home.join(MANIFEST)).unwrap(), manifest);
        assert!(!home.join("sunrise-mods/ruby.smod").exists());
    }
    fs::remove_file(home.join(MANIFEST)).unwrap();
    fs::create_dir(home.join(MANIFEST)).unwrap();
    let error = install(&index, &home, "ruby", &|_, _| {
        panic!("a manifest read error must be refused before download")
    })
    .unwrap_err();
    assert!(error.contains(MANIFEST), "{error}");
    assert!(home.join(MANIFEST).is_dir());
    assert!(!home.join("sunrise-mods/ruby.smod").exists());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn bytes_other_than_the_index_lists_never_reach_the_mods_folder() {
    let bytes = ruby("1.0");
    let index = registry("1.0", &bytes, "file:///ruby.smod");
    let home = scratch("tampered");
    let mut tampered = bytes.clone();
    *tampered.last_mut().unwrap() ^= 1;
    let error = install(&index, &home, "ruby", &serve(tampered)).unwrap_err();
    assert!(error.contains("SHA-256"), "{error}");
    assert!(!home.join("sunrise-mods/ruby.smod").exists());
    assert!(!home.join("mod-browser/downloads/ruby.smod.part").exists());
    assert!(!home.join(MANIFEST).exists());
    // A taken-down file is refused before anything is downloaded.
    let mut index = index;
    index.revoked.push(index::Revoked {
        id: "ruby".into(),
        sha256: Some(index::sha256(&bytes)),
        reason: "bad".into(),
        date: "2026-11-05".into(),
    });
    assert!(install(&index, &home, "ruby", &serve(bytes)).is_err());
    assert!(!home.join("sunrise-mods/ruby.smod").exists());
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn a_mod_the_player_put_there_is_never_replaced() {
    let bytes = ruby("1.0");
    let index = registry("1.0", &bytes, "file:///ruby.smod");
    let home = scratch("theirs");
    fs::create_dir_all(home.join("sunrise-mods")).unwrap();
    fs::write(home.join("sunrise-mods/ruby.smod"), b"their own").unwrap();
    let error = install(&index, &home, "ruby", &serve(bytes)).unwrap_err();
    assert!(error.contains("already in sunrise-mods"), "{error}");
    assert_eq!(
        fs::read(home.join("sunrise-mods/ruby.smod")).unwrap(),
        b"their own"
    );
    fs::remove_dir_all(&home).unwrap();
}

/// A native package can declare only registries too; the browser must inspect the package,
/// rather than treat that capability list as proof that it contains no code.
#[test]
fn code_packages_never_reach_the_content_browser_install_path() {
    let manifest = format!(
        "id = 'ruby'\nname = 'Ruby'\nversion = '1.0'\napi = '0.2'\nside = 'both'\ncapabilities = ['registries']\n[native]\ngame = '26.3'\nsunrise = '{}'\n",
        "a".repeat(40),
    );
    let bytes = sunrise_mod_package::Package::write_files(
        &manifest,
        &[
            (
                "Cargo.toml",
                b"[package]\nname = 'ruby'\nversion = '1.0.0'\n",
            ),
            ("src/lib.rs", b"// No code needed for the fixture.\n"),
        ],
    )
    .unwrap();
    let index = registry("1.0", &bytes, "file:///ruby.smod");
    let home = scratch("code");
    let error = install(&index, &home, "ruby", &serve(bytes.clone())).unwrap_err();
    assert!(error.contains("it is a native-mod"), "{error}");
    assert!(!home.join("sunrise-mods/ruby.smod").exists());
    assert!(!home.join("mod-browser/downloads/ruby.smod.part").exists());
    assert!(!home.join(MANIFEST).exists());

    // A code package installed through the launcher is not silently replaced by content.
    fs::create_dir_all(home.join("sunrise-mods")).unwrap();
    fs::write(home.join("sunrise-mods/ruby.smod"), &bytes).unwrap();
    fs::write(home.join(MANIFEST), serde_json::json!({"entries": [{
        "source": "sunrise", "project": "ruby", "version": "1.0", "file": "sunrise-mods/ruby.smod"
    }]}).to_string()).unwrap();
    let content = ruby("1.1");
    let index = registry("1.1", &content, "file:///ruby.smod");
    let error = install(&index, &home, "ruby", &serve(content)).unwrap_err();
    assert!(error.contains("code mods"), "{error}");
    assert_eq!(
        fs::read(home.join("sunrise-mods/ruby.smod")).unwrap(),
        bytes
    );
    fs::remove_dir_all(home).unwrap();
}
