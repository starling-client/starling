use super::*;
use wasmtime::{Instance, Store};

fn wasm(answer: i32) -> Vec<u8> {
    wat::parse_str(format!(
        "(module (func (export \"answer\") (result i32) i32.const {answer}))"
    ))
    .unwrap()
}
fn fixture() -> (tempfile::TempDir, ModuleCache, Engine, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    let cache = ModuleCache::new(dir.path().join(".horizon-cache/wasm"));
    (dir, cache, Engine::default(), wasm(42))
}
fn entry(cache: &ModuleCache, engine: &Engine, bytes: &[u8]) -> PathBuf {
    cache
        .root
        .join(format!("{}.cwasm", ModuleCache::key(engine, bytes)))
}
fn answer(engine: &Engine, module: &Module) -> i32 {
    let mut store = Store::new(engine, ());
    // Production engines use epoch interruption.
    store.set_epoch_deadline(u64::MAX / 2);
    let instance = Instance::new(&mut store, module, &[]).unwrap();
    instance
        .get_typed_func::<(), i32>(&mut store, "answer")
        .unwrap()
        .call(&mut store, ())
        .unwrap()
}
fn entries(cache: &ModuleCache) -> Vec<PathBuf> {
    fs::read_dir(&cache.root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect()
}
fn replace_artifact(path: &Path, bytes: &[u8]) {
    let mut temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap()).unwrap();
    temporary.write_all(bytes).unwrap();
    temporary.as_file().sync_all().unwrap();
    temporary.persist(path).unwrap();
}
fn assert_compiled(cache: &ModuleCache, engine: &Engine, bytes: &[u8]) {
    let (module, source) = cache.load_or_compile(engine, bytes).unwrap();
    assert_eq!(source, Source::Compiled);
    assert_eq!(answer(engine, &module), 42);
}
fn assert_cached(cache: &ModuleCache, engine: &Engine, bytes: &[u8]) {
    let (module, source) = cache.load_or_compile(engine, bytes).unwrap();
    assert_eq!(source, Source::Cached);
    assert_eq!(answer(engine, &module), 42);
}

#[test]
fn first_compile_then_cached_across_launches_preserves_execution() {
    let (_dir, cache, engine, bytes) = fixture();
    assert_compiled(&cache, &engine, &bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&cache.root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(entry(&cache, &engine, &bytes))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let paths = entries(&cache);
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].extension().unwrap(), "cwasm");
    let next_launch = ModuleCache::new(cache.root.clone());
    assert_cached(&next_launch, &Engine::default(), &bytes);
}

#[test]
fn one_changed_wasm_byte_keeps_both_entries() {
    let (_dir, cache, engine, bytes) = fixture();
    let different = wasm(43);
    assert_eq!(
        bytes.iter().zip(&different).filter(|(a, b)| a != b).count(),
        1
    );
    assert_ne!(
        ModuleCache::key(&engine, &bytes),
        ModuleCache::key(&engine, &different)
    );
    assert_compiled(&cache, &engine, &bytes);
    let (module, source) = cache.load_or_compile(&engine, &different).unwrap();
    assert_eq!(source, Source::Compiled);
    assert_eq!(answer(&engine, &module), 43);
    assert_eq!(entries(&cache).len(), 2);
}

#[test]
fn configurations_coexist_without_eviction() {
    let (_dir, cache, _, bytes) = fixture();
    let production = crate::runtime::engine().unwrap();
    let mut config = wasmtime::Config::new();
    config.epoch_interruption(crate::runtime::ENGINE_CONFIG.1);
    config.cranelift_opt_level(wasmtime::OptLevel::None);
    let other = Engine::new(&config).unwrap();
    assert_ne!(
        ModuleCache::key(&production, &bytes),
        ModuleCache::key(&other, &bytes)
    );
    for engine in [&production, &other] {
        assert_compiled(&cache, engine, &bytes);
    }
    assert_eq!(entries(&cache).len(), 2);
    for engine in [&production, &other] {
        assert_cached(&cache, engine, &bytes);
    }
}

#[test]
fn truncated_entry_is_replaced() {
    let (_dir, cache, engine, bytes) = fixture();
    assert_compiled(&cache, &engine, &bytes);
    let (mapped, source) = cache.load_or_compile(&engine, &bytes).unwrap();
    assert_eq!(source, Source::Cached);
    let path = entry(&cache, &engine, &bytes);
    let artifact = fs::read(&path).unwrap();
    replace_artifact(&path, &artifact[..10]);
    assert_eq!(answer(&engine, &mapped), 42);
    assert_compiled(&cache, &engine, &bytes);
    assert_cached(&cache, &engine, &bytes);
    assert_eq!(entries(&cache).len(), 1);
    assert_eq!(answer(&engine, &mapped), 42);
}

#[test]
fn damaged_header_is_replaced() {
    let (_dir, cache, engine, bytes) = fixture();
    assert_compiled(&cache, &engine, &bytes);
    let path = entry(&cache, &engine, &bytes);
    let mut artifact = fs::read(&path).unwrap();
    artifact[0] ^= 0xff;
    replace_artifact(&path, &artifact);
    assert_compiled(&cache, &engine, &bytes);
    assert_cached(&cache, &engine, &bytes);
}

#[test]
fn regular_file_root_always_falls_back() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("file");
    fs::write(&root, b"not a directory").unwrap();
    let cache = ModuleCache::new(root.clone());
    assert!(cache.disabled.as_ref().unwrap().contains("folder"));
    for _ in 0..2 {
        assert_compiled(&cache, &Engine::default(), &wasm(42));
    }
    assert_eq!(fs::read(root).unwrap(), b"not a directory");
}

#[test]
fn stale_temporaries_are_removed_and_fresh_ones_kept() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(".horizon-cache/wasm");
    drop(ModuleCache::new(root.clone()));
    let stale = root.join(".1-0.tmp");
    let fresh = root.join(".2-0.tmp");
    let entry = root.join("kept.cwasm");
    for path in [&stale, &fresh, &entry] {
        fs::write(path, b"x").unwrap();
    }
    let two_hours_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
    for path in [&stale, &entry] {
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(two_hours_ago)
            .unwrap();
    }
    drop(ModuleCache::new(root.clone()));
    assert!(!stale.exists());
    assert!(fresh.exists());
    assert!(entry.exists());
}

#[test]
fn the_key_is_one_sha256_per_input() {
    let engine = Engine::default();
    let a = ModuleCache::key(&engine, &wasm(42));
    assert_eq!(a.len(), 64);
    assert_eq!(a, ModuleCache::key(&engine, &wasm(42)));
    assert_ne!(a, ModuleCache::key(&engine, &wasm(43)));
}

#[cfg(unix)]
#[test]
fn writable_folder_disables_cache_with_folder_reason() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, cache, engine, bytes) = fixture();
    fs::set_permissions(&cache.root, fs::Permissions::from_mode(0o777)).unwrap();
    let disabled = ModuleCache::new(cache.root.clone());
    let reason = disabled.disabled.as_ref().unwrap();
    assert!(reason.contains("folder"));
    assert!(reason.contains(cache.root.to_str().unwrap()));
    assert_compiled(&disabled, &engine, &bytes);
    assert!(entries(&disabled).is_empty());
}

#[cfg(unix)]
#[test]
fn writable_entry_is_replaced_with_private_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, cache, engine, bytes) = fixture();
    assert_compiled(&cache, &engine, &bytes);
    let path = entry(&cache, &engine, &bytes);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
    assert_compiled(&cache, &engine, &bytes);
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_cached(&cache, &engine, &bytes);
}

#[cfg(unix)]
#[test]
fn symlink_entry_is_not_followed() {
    let (dir, cache, engine, bytes) = fixture();
    let target = dir.path().join("target");
    fs::write(
        &target,
        Module::new(&engine, &bytes).unwrap().serialize().unwrap(),
    )
    .unwrap();
    let before = fs::read(&target).unwrap();
    std::os::unix::fs::symlink(&target, entry(&cache, &engine, &bytes)).unwrap();
    assert_compiled(&cache, &engine, &bytes);
    assert_eq!(fs::read(target).unwrap(), before);
    assert_cached(&cache, &engine, &bytes);
}

#[cfg(unix)]
#[test]
fn hard_linked_entry_is_not_read() {
    use std::os::unix::fs::MetadataExt;
    let (dir, cache, engine, bytes) = fixture();
    assert_compiled(&cache, &engine, &bytes);
    let path = entry(&cache, &engine, &bytes);
    let alias = dir.path().join("alias");
    fs::hard_link(&path, &alias).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().nlink(), 2);
    assert_compiled(&cache, &engine, &bytes);
    assert_eq!(fs::metadata(path).unwrap().nlink(), 1);
    assert_eq!(fs::metadata(alias).unwrap().nlink(), 1);
    assert_cached(&cache, &engine, &bytes);
}

#[test]
fn artifact_shipped_beside_package_is_ignored() {
    let (dir, cache, engine, _) = fixture();
    let bytes = wat::parse_str(
        r#"(module
        (@custom "dylink.0" "\01\04\00\00\00\00")
        (func (export "horizon_layout"))
        (func (export "horizon_in"))
        (func (export "horizon_hook"))
        (func (export "answer") (result i32) i32.const 42))"#,
    )
    .unwrap();
    let package = dir.path().join("mods/planted");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("mod.wasm"), &bytes).unwrap();
    fs::write(package.join("horizon-mod.toml"), format!(
        "id='planted'\nname='Planted'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n[wasm]\nsha512='{}'\ntoolchain='test'\nbuilt_with='fixture'\n",
        horizon_mod_package::manifest::sha512_of(&bytes)
    )).unwrap();
    fs::write(
        package.join(format!("{}.cwasm", ModuleCache::key(&engine, &bytes))),
        Module::new(&engine, &bytes).unwrap().serialize().unwrap(),
    )
    .unwrap();
    let loaded = horizon_mod_package::Package::read_wasm(&package).unwrap();
    assert_compiled(&cache, &engine, &loaded);
    assert_cached(&cache, &engine, &loaded);
}

#[test]
#[ignore = "local timing measurement; set HORIZON_CACHE_BENCH_WASM"]
fn measure_compile_and_cached_load() {
    let path = std::env::var("HORIZON_CACHE_BENCH_WASM").unwrap();
    let bytes = fs::read(&path).unwrap();
    let (_dir, cache, _, _) = fixture();
    let engine = crate::runtime::engine().unwrap();
    let mut cold = Vec::new();
    let mut warm = Vec::new();
    for _ in 0..5 {
        let start = std::time::Instant::now();
        let module = Module::new(&engine, &bytes).unwrap();
        cold.push(start.elapsed().as_secs_f64() * 1000.0);
        drop(module);
    }
    assert_eq!(
        cache.load_or_compile(&engine, &bytes).unwrap().1,
        Source::Compiled
    );
    for _ in 0..5 {
        let start = std::time::Instant::now();
        let (module, source) = cache.load_or_compile(&engine, &bytes).unwrap();
        warm.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(source, Source::Cached);
        drop(module);
    }
    cold.sort_by(f64::total_cmp);
    warm.sort_by(f64::total_cmp);
    eprintln!(
        "wasm cache measurement: {path}, {} bytes; compile median {:.3} ms, cached median {:.3} ms; compile {cold:?}, cached {warm:?}",
        bytes.len(),
        cold[2],
        warm[2]
    );
}

#[test]
fn group_linker_uses_cache_and_programmatic_linking_leaves_it_empty() {
    use crate::link::{Group, Member};
    let (_dir, cache, engine, _) = fixture();
    let bytes = wat::parse_str(
        r#"(module
        (@custom "dylink.0" "\01\04\00\00\00\00")
        (func (export "answer") (result i32) i32.const 42))"#,
    )
    .unwrap();
    let members = [Member {
        name: "fixture",
        module: &bytes,
    }];
    let mut plain = Group::link(&engine, &members).unwrap();
    let mut result = [wasmtime::Val::I32(0)];
    plain.call("fixture", "answer", &[], &mut result).unwrap();
    assert_eq!(result[0].unwrap_i32(), 42);
    assert!(entries(&cache).is_empty());
    for _ in 0..2 {
        let mut group = Group::link_cached_with(
            Store::new(&engine, ()),
            &members,
            Some(&cache),
            |_, _, _| None,
        )
        .unwrap();
        group.call("fixture", "answer", &[], &mut result).unwrap();
        assert_eq!(result[0].unwrap_i32(), 42);
        assert_eq!(entries(&cache).len(), 1);
    }
    assert_cached(&cache, &engine, &bytes);
}

#[test]
fn failed_replacement_keeps_compiled_module_and_cleans_temp_files() {
    let (_dir, cache, engine, bytes) = fixture();
    let path = entry(&cache, &engine, &bytes);
    fs::create_dir(&path).unwrap();
    for _ in 0..2 {
        assert_compiled(&cache, &engine, &bytes);
    }
    assert_eq!(entries(&cache), vec![path]);
}
