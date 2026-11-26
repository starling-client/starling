use super::*;
use std::fs;

fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
    move |name| {
        pairs
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| OsString::from(value))
    }
}

#[test]
fn the_cache_is_the_override_else_the_platforms_cache_folder() {
    let pairs = [("SUNRISE_MOD_CACHE", "/mine"), ("HOME", "/home/a")];
    assert_eq!(
        cache_in(&env(&pairs), Platform::Unix).unwrap(),
        PathBuf::from("/mine")
    );
    let pairs = [("XDG_CACHE_HOME", "/xdg"), ("HOME", "/home/a")];
    assert_eq!(
        cache_in(&env(&pairs), Platform::Unix).unwrap(),
        PathBuf::from("/xdg/sunrise-mod")
    );
    let pairs = [("HOME", "/home/a")];
    assert_eq!(
        cache_in(&env(&pairs), Platform::Unix).unwrap(),
        PathBuf::from("/home/a/.cache/sunrise-mod")
    );
    assert_eq!(
        cache_in(&env(&pairs), Platform::Mac).unwrap(),
        PathBuf::from("/home/a/Library/Caches/sunrise-mod")
    );
    let pairs = [("LOCALAPPDATA", "C:/Local")];
    assert_eq!(
        cache_in(&env(&pairs), Platform::Windows).unwrap(),
        PathBuf::from("C:/Local/sunrise-mod")
    );
    let error = cache_in(&env(&[]), Platform::Unix).unwrap_err();
    assert!(error.contains("SUNRISE_MOD_CACHE"), "{error}");
}

#[test]
fn each_project_folder_has_its_own_work_folder() {
    let cache = Path::new("/cache");
    let (project, worktree) = (Path::new("/src/ae2"), Path::new("/src/ae2-feature"));
    let work = work_dir(cache, "ae2", project);
    assert_eq!(
        work,
        work_dir(cache, "ae2", project),
        "stable for one project"
    );
    assert_ne!(
        work,
        work_dir(cache, "ae2", worktree),
        "same mod id, another folder"
    );
    assert_ne!(work, work_dir(cache, "jei", project));
    assert_eq!(work.parent(), Some(Path::new("/cache/work")));
    let name = work.file_name().unwrap().to_str().unwrap();
    let hash = name.strip_prefix("ae2-").unwrap();
    assert_eq!(hash.len(), 12, "{name}");
    assert!(hash.bytes().all(|b| b.is_ascii_hexdigit()), "{name}");
    #[cfg(unix)]
    {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let first = Path::new(OsStr::from_bytes(b"/src/ae2-\xfe"));
        let second = Path::new(OsStr::from_bytes(b"/src/ae2-\xff"));
        assert_ne!(
            work_dir(cache, "ae2", first),
            work_dir(cache, "ae2", second),
            "non-UTF-8 paths differing by one byte must not collide"
        );
    }
}

#[test]
fn the_rust_is_rustups_else_the_exact_one_on_the_path_else_an_error_saying_how() {
    let target = "x86_64-unknown-linux-gnu";
    let found = |version: &str, host: &str| Found {
        version: version.into(),
        host: host.into(),
        sysroot: "/sys".into(),
    };
    assert_eq!(
        choose("1.99.1", target, true, Some(&found("1.80.0", target))).unwrap(),
        Source::Rustup("1.99.1-x86_64-unknown-linux-gnu".into())
    );
    assert_eq!(
        choose("1.99.1", target, false, Some(&found("1.99.1", target))).unwrap(),
        Source::System
    );
    for other in [
        Some(found("1.98.0", target)),
        Some(found("1.99.1", "aarch64-apple-darwin")),
        None,
    ] {
        let error = choose("1.99.1", target, false, other.as_ref()).unwrap_err();
        assert!(
            error.contains("1.99.1") && error.contains("rustup.rs"),
            "{error}"
        );
    }
}

#[test]
fn the_staged_record_is_the_builds_libraries_as_the_loader_reads_them() {
    let row = Library {
        file: "libmod_greeter.so".into(),
        sha256: "ab".repeat(32),
        kind: "mod".into(),
        mod_id: "greeter".into(),
        identity: "b greeter 0.1.0 1.99.1".into(),
    };
    let json: serde_json::Value = serde_json::from_str(&record(&[row]).unwrap()).unwrap();
    assert_eq!(json["libraries"][0]["file"], "libmod_greeter.so");
    assert_eq!(json["libraries"][0]["mod"], "greeter");
    assert_eq!(json["libraries"][0]["identity"], "b greeter 0.1.0 1.99.1");
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn a_retained_sdk_builds_a_lower_mixin_with_its_signed_tool() {
    use lzma_rust2::{XzOptions, XzWriter};
    use serde_json::json;
    use sunrise_release_sign::{self as sign, SigningKey};

    const BUILD: &str = "0123456789abcdef0123456789abcdef01234567";
    const HOST: &str = "x86_64-unknown-linux-gnu";
    const MODULE: &str = "sunrise_mod_render::render";
    const TARGET: &str = "sunrise_mod_render::render::world_drawn";
    const FILE: &str = "crates/sunrise-mod-render/src/render.rs";
    let root = tempfile::tempdir().unwrap();
    let base = root.path().join("releases");
    let tag = format!("sdk-{BUILD}");
    let release = base.join(&tag);
    fs::create_dir_all(&release).unwrap();
    fs::write(release.join(sdk_fetch::SURFACE), r#"{"hash":"ab"}"#).unwrap();
    let pack = |kind, path: &str, bytes: &[u8], mode| {
        let name = sdk::Asset::name(kind, HOST, sdk::Side::Client);
        let archive = fs::File::create(release.join(&name)).unwrap();
        let encoder = XzWriter::new(archive, XzOptions::with_preset(0)).unwrap();
        let mut tar = tar::Builder::new(encoder);
        let mut header = tar::Header::new_ustar();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(mode);
        header.set_size(bytes.len() as u64);
        tar.append_data(&mut header, path, bytes).unwrap();
        tar.into_inner().unwrap().finish().unwrap();
        name
    };
    let mut identity = json!({
        "package_format": sdk::PACKAGE_FORMAT,
        "commit": BUILD,
        "rustc": format!("release: {}", sunrise_native_build::rust::VERSION),
        "side": "client",
        "target": HOST,
        "registry_lock": "version = 4\n",
        "registry_patches": {},
        "native_links": [],
        "facade_crates": [],
        "game_features": [],
        "slices": { "runtime": ["sunrise-prebuilt.json"], "api": ["internals.json"] }
    });
    pack(
        sdk::Kind::Runtime,
        "sunrise-prebuilt.json",
        &serde_json::to_vec(&identity).unwrap(),
        0o644,
    );
    pack(sdk::Kind::Api, "internals.json", b"{}", 0o644);
    let source = b"pub fn world_drawn() {}";
    let hash = sign::sha256_hex(source);
    let functions = json!({
        "build": { "commit": BUILD },
        "crates": { "sunrise_mod_render": {
            "library": "mod_render",
            "modules": { (MODULE): [{ "path": FILE, "sha256": hash }] }
        }},
        "functions": { (TARGET): {
            "module": MODULE, "file": FILE, "span": [0, source.len()],
            "signature_hash": hash, "sha256": hash
        }},
        "refused": {}
    });
    pack(
        sdk::Kind::Engine,
        "functions.json",
        &serde_json::to_vec(&functions).unwrap(),
        0o644,
    );

    // The older executable stands in for native-build, not for the current generator. Its
    // transcript proves what received the plan without compiling a real game or downloading Rust.
    let result = contract::Event::Built {
        route: "prebuilt".into(),
        binary: "from-pinned-tool".into(),
        libraries: Vec::new(),
        records: Vec::new(),
        mods: vec!["fixture".into()],
        internals: Default::default(),
        mixins: sunrise_native_build::MIXINS_REV.to_owned(),
        used: Vec::new(),
    };
    let script = format!(
        "#!/bin/sh\n[ \"$1\" = native-build ] || exit 2\nIFS= read -r plan || exit 2\n\
         case \"$plan\" in *'\"contract\":2'*'\"registry\":true'*) ;; *) exit 2;; esac\n\
         printf '%s\\n' '{}'\n",
        serde_json::to_string(&result).unwrap(),
    );
    let tool = pack(
        sdk::Kind::Tool,
        "cargo-sunrise-mod",
        script.as_bytes(),
        0o755,
    );
    let index = Index::of_folder(
        &release,
        sdk::Build {
            commit: BUILD.into(),
            game: sdk::GAME.into(),
            date: "2026-11-13".into(),
            nightly: None,
        },
        sunrise_native_build::rust::VERSION.into(),
    )
    .unwrap();
    let bytes = index.to_json().into_bytes();
    let sums = sign::sums_file(&[(sdk::INDEX.into(), sign::sha256_hex(&bytes))]);
    let (key, _) = SigningKey::generate().unwrap();
    fs::write(release.join(sdk::INDEX), bytes).unwrap();
    fs::write(release.join(sign::SUMS), &sums).unwrap();
    fs::write(
        release.join(sign::SIGNATURE),
        key.sign(&tag, sums.as_bytes()),
    )
    .unwrap();
    let cache = root.path().join("cache");
    let keys = [key.public()];
    let base = Base::Folder(base);
    let sdk = native_sdk_from(&cache, base, &keys, HOST, BUILD, |_, target, cargo_home| {
        Ok(Unpacked {
            dir: root.path().into(),
            target: target.into(),
            cargo_home,
            cargo_config: Vec::new(),
            c_bin: None,
        })
    })
    .unwrap();
    let plan = contract::Plan {
        contract: contract::CONTRACT,
        package: sdk.slices(sdk::Side::Client, Route::Engine).unwrap(),
        toolchain: sdk.rust.clone(),
        target: contract::TargetName::Client,
        mods: root.path().join("mod"),
        work: root.path().join("work"),
        registry: Some(true),
        mixins_from_source: false,
    };
    let bundle = Bundle {
        dir: plan.mods.clone(),
        manifest: sunrise_mod_package::Manifest::parse(&format!(
            "id='fixture'\nname='Fixture'\nversion='1.0.0'\n[native]\ngame='26.3'\nsunrise='{BUILD}'\n\
             [[native.mixin]]\nid='draw'\nkind='HEAD'\ntarget='{TARGET}'\nhelper='src/draw.rs'\n"
        )).unwrap(),
    };
    sunrise_native_build::MixinsIdentity::resolve([&bundle], Some(&sdk.index.mixins)).unwrap();

    let tool = release.join(tool);
    let good = fs::read(&tool).unwrap();
    fs::write(&tool, b"not the signed asset").unwrap();
    let mut events = Events::default();
    assert!(
        sdk.compile(std::slice::from_ref(&bundle), &plan, &mut events)
            .is_err()
    );
    assert!(events.built.is_none());
    fs::write(&tool, good).unwrap();
    sdk.compile(std::slice::from_ref(&bundle), &plan, &mut events)
        .unwrap();
    assert_eq!(events.built, Some(result));

    // An explicitly empty map is still this tool's schema, not an older one. A missing
    // lower-crate entry in that schema must not run the pinned tool in its place.
    identity["lower_crate_cfg"] = json!({});
    fs::write(
        plan.package.runtime.join("sunrise-prebuilt.json"),
        serde_json::to_vec(&identity).unwrap(),
    )
    .unwrap();
    assert!(!prebuilt::needs_pinned_patch_tool(&plan.package, &bundle).unwrap());
}

/// The Motion Blur 0.1.5 case: the pin moved, the SDK's `sunrise-core` changed its dependencies,
/// and the committed lock still named the old ones.
#[test]
#[ignore = "daily"]
fn a_lock_the_pinned_sdk_moved_is_refused_until_refreshed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let write = |path: &str, text: &str| {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    let package = |path: &str, name: &str, dependencies: &str| {
        write(
            &format!("{path}Cargo.toml"),
            &format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\n{dependencies}"
            ),
        );
        write(&format!("{path}src/lib.rs"), "");
    };
    let sdk = ".sunrise/sdk/api/crates/";
    package(
        "",
        "mod-fixture",
        "sunrise-mod-api = { path = \".sunrise/sdk/api/crates/sunrise-mod-api\" }\n",
    );
    package(
        &format!("{sdk}sunrise-mod-api/"),
        "sunrise-mod-api",
        "sunrise-core = { path = \"../sunrise-core\" }\n",
    );
    // Local stand-ins for the registry packages, so Cargo needs no network.
    package(&format!("{sdk}crc-fast/"), "crc-fast", "");
    package(&format!("{sdk}stacker/"), "stacker", "");
    let core = |dependency: &str| {
        package(
            &format!("{sdk}sunrise-core/"),
            "sunrise-core",
            &format!("{dependency} = {{ path = \"../{dependency}\" }}\n"),
        )
    };
    core("crc-fast");
    let found = found().expect("a rustc on PATH");
    let rust = Unpacked {
        dir: found.sysroot,
        target: found.host,
        cargo_home: root.join("cargo-home"),
        cargo_config: Vec::new(),
        c_bin: None,
    };
    refresh_lock(&rust, root).unwrap();
    check_lock(&rust, root, "old").unwrap();

    // The new pin's SDK.
    core("stacker");
    let stale = fs::read_to_string(root.join("Cargo.lock")).unwrap();
    let error = check_lock(&rust, root, "new").unwrap_err();
    assert!(error.contains("pinned SDK (Sunrise new)"), "{error}");
    assert!(error.contains("cargo sunrise-mod pin new"), "{error}");
    assert_eq!(fs::read_to_string(root.join("Cargo.lock")).unwrap(), stale);

    refresh_lock(&rust, root).unwrap();
    check_lock(&rust, root, "new").unwrap();
    let lock = fs::read_to_string(root.join("Cargo.lock")).unwrap();
    assert!(
        lock.contains("\"stacker\"") && !lock.contains("crc-fast"),
        "{lock}"
    );
}

/// `run` reports a listed jar that is missing from the store, or whose bytes differ, before it
/// builds: with no Cargo project in the folder, a build would fail with another message.
#[test]
fn run_refuses_a_missing_or_wrong_jar_before_building() {
    const SHA512: &str = "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
                          2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f";
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("garden");
    let resources = root.path().join("resources");
    let jars = root.path().join("originals");
    for folder in [&project, &resources, &jars] {
        fs::create_dir(folder).unwrap();
    }
    fs::write(
        project.join(sunrise_mod_package::manifest::MANIFEST),
        format!(
            "id = 'garden'\nname = 'Garden'\nversion = '0.1.0'\napi = '0.2'\ncapabilities = ['registries']\n\
             [[jar]]\nid = 'original'\nversion = '1.0'\nsha512 = '{SHA512}'\n"
        ),
    )
    .unwrap();
    let package = Package::read(&project).unwrap();
    let original = package.manifest.jar[0].cache_path(&jars);
    let run = || {
        run(
            &project,
            &package,
            &[],
            Some(resources.to_str().unwrap()),
            Some(&jars),
            &[],
        )
        .unwrap_err()
    };

    let missing = run();
    for expected in ["garden", "original", "1.0", SHA512, "use-jar"] {
        assert!(missing.contains(expected), "{missing}");
    }
    // The fix is its own line, as staging words it.
    assert!(
        missing
            .lines()
            .any(|line| line.starts_with("Add the exact original with `cargo sunrise-mod use-jar")),
        "{missing}"
    );
    fs::write(&original, b"not abc").unwrap();
    let wrong = run();
    assert!(
        wrong.contains("the file does not match the listed SHA-512"),
        "{wrong}"
    );
    assert!(!project.join(crate::RUN_DIR).exists());
}

#[test]
fn two_callers_initialize_an_empty_sdk_cache_concurrently() {
    use lzma_rust2::{XzOptions, XzWriter};
    use sunrise_release_sign::{self as sign, SigningKey};

    const BUILD: &str = "0123456789abcdef0123456789abcdef01234567";
    const HOST: &str = "x86_64-unknown-linux-gnu";
    const SURFACE_BYTES: &[u8] = br#"{"hash":"ab"}"#;
    const RUNTIME_BYTES: &[u8] = b"runtime-fixture";
    const API_BYTES: &[u8] = b"api-fixture";
    const ENGINE_BYTES: &[u8] = b"engine-fixture";

    let root = tempfile::tempdir().unwrap();
    let base_dir = root.path().join("releases");
    let tag = format!("sdk-{BUILD}");
    let release = base_dir.join(&tag);
    fs::create_dir_all(&release).unwrap();
    fs::write(release.join(sdk_fetch::SURFACE), SURFACE_BYTES).unwrap();
    let pack = |kind, path: &str, bytes: &[u8]| {
        let name = sdk::Asset::name(kind, HOST, sdk::Side::Client);
        let archive = fs::File::create(release.join(&name)).unwrap();
        let encoder = XzWriter::new(archive, XzOptions::with_preset(0)).unwrap();
        let mut tar = tar::Builder::new(encoder);
        let mut header = tar::Header::new_ustar();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(0o644);
        header.set_size(bytes.len() as u64);
        tar.append_data(&mut header, path, bytes).unwrap();
        tar.into_inner().unwrap().finish().unwrap();
    };
    pack(sdk::Kind::Runtime, "sunrise-prebuilt.json", RUNTIME_BYTES);
    pack(sdk::Kind::Api, "internals.json", API_BYTES);
    pack(sdk::Kind::Engine, "functions.json", ENGINE_BYTES);
    let index = Index::of_folder(
        &release,
        sdk::Build {
            commit: BUILD.into(),
            game: sdk::GAME.into(),
            date: "2026-11-13".into(),
            nightly: None,
        },
        sunrise_native_build::rust::VERSION.into(),
    )
    .unwrap();
    let bytes = index.to_json().into_bytes();
    let sums = sign::sums_file(&[(sdk::INDEX.into(), sign::sha256_hex(&bytes))]);
    let (key, _) = SigningKey::generate().unwrap();
    fs::write(release.join(sdk::INDEX), bytes).unwrap();
    fs::write(release.join(sign::SUMS), &sums).unwrap();
    fs::write(
        release.join(sign::SIGNATURE),
        key.sign(&tag, sums.as_bytes()),
    )
    .unwrap();

    let keys = [key.public()];
    let root_dir = root.path();
    let stub_rust = |_: &str, target: &str, cargo_home: PathBuf| {
        Ok(Unpacked {
            dir: root_dir.into(),
            target: target.into(),
            cargo_home,
            cargo_config: Vec::new(),
            c_bin: None,
        })
    };

    for round in 0..4 {
        let cache = root.path().join(format!("cache-{round}"));
        let metadata_start = std::sync::Barrier::new(2);
        let sdks: Vec<NativeSdk> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..2)
                .map(|_| {
                    scope.spawn(|| {
                        metadata_start.wait();
                        let sdk = native_sdk_from(
                            &cache,
                            Base::Folder(base_dir.clone()),
                            &keys,
                            HOST,
                            BUILD,
                            stub_rust,
                        )
                        .unwrap();
                        assert_eq!(sdk.index.build.commit, BUILD);
                        assert_eq!(fs::read(sdk.surface()).unwrap(), SURFACE_BYTES);
                        sdk
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect()
        });
        assert_eq!(sdks[0].dir, sdks[1].dir);

        let slices_start = std::sync::Barrier::new(2);
        let outputs: Vec<(NativeSdk, prebuilt::Package)> = std::thread::scope(|scope| {
            let handles: Vec<_> = sdks
                .into_iter()
                .map(|sdk| {
                    let slices_start = &slices_start;
                    scope.spawn(move || {
                        slices_start.wait();
                        let slices = sdk.slices(sdk::Side::Client, Route::Engine).unwrap();
                        assert_eq!(
                            fs::read(slices.runtime.join("sunrise-prebuilt.json")).unwrap(),
                            RUNTIME_BYTES
                        );
                        assert_eq!(
                            fs::read(slices.api.join("internals.json")).unwrap(),
                            API_BYTES
                        );
                        assert_eq!(
                            fs::read(slices.engine.join("functions.json")).unwrap(),
                            ENGINE_BYTES
                        );
                        (sdk, slices)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect()
        });

        let (sdk, slices) = &outputs[0];
        assert_eq!(slices, &outputs[1].1);
        let locks_dir = sdk.dir.join("locks");
        let mut lock_count = 0;
        for entry in fs::read_dir(&locks_dir).unwrap() {
            let path = entry.unwrap().path();
            assert_eq!(path.extension().and_then(|ext| ext.to_str()), Some("lock"));
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .unwrap();
            file.try_lock().expect("cache lock must be released");
            lock_count += 1;
        }
        assert_eq!(lock_count, 2);

        // Refreshing metadata must not remove unpacked slices in use.
        fs::write(sdk.surface(), b"{}").unwrap();
        let refreshed = native_sdk_from(
            &cache,
            Base::Folder(base_dir.clone()),
            &keys,
            HOST,
            BUILD,
            stub_rust,
        )
        .unwrap();
        assert_eq!(fs::read(refreshed.surface()).unwrap(), SURFACE_BYTES);
        assert_eq!(
            fs::read(slices.runtime.join("sunrise-prebuilt.json")).unwrap(),
            RUNTIME_BYTES
        );
        assert_eq!(
            fs::read(slices.api.join("internals.json")).unwrap(),
            API_BYTES
        );
        assert_eq!(
            fs::read(slices.engine.join("functions.json")).unwrap(),
            ENGINE_BYTES
        );
    }
}
