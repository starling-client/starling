#[path = "support/sdk.rs"]
mod sdk_fixture;

use sdk_fixture::OfflineSdk;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};
const BUILD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const MIXINS: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

struct Fixture {
    dir: PathBuf,
    sdk: OfflineSdk,
}
impl Fixture {
    fn new(name: &str) -> Self {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/bounded-{name}-{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let sdk = OfflineSdk::new(&dir);
        Self { dir, sdk }
    }
    fn run(&self, args: &[&str]) -> String {
        let mut child = self
            .sdk
            .command(&self.dir)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let start = Instant::now();
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if start.elapsed() > Duration::from_secs(10) {
                child.kill().unwrap();
                panic!("command hung: {args:?}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        String::from_utf8(output.stderr).unwrap()
    }
    fn manifest(&self, declarations: &str) {
        fs::write(self.dir.join("sunrise-mod.toml"), format!("id='example'\nname='Test'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{BUILD}'\nmixins='{MIXINS}'\n{declarations}")).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.dir).unwrap();
    }
}
fn large(path: &Path, limit: u64) {
    fs::File::create(path).unwrap().set_len(limit + 1).unwrap();
}
const COMMANDS: &[&[&str]] = &[&["internals", "add", "lower::run"], &["internals", "sync"]];
#[test]
#[ignore = "daily"]
fn bounded_manifest_commands() {
    let f = Fixture::new("manifest");
    let path = f.dir.join("sunrise-mod.toml");
    let limit = sunrise_mod_package::manifest::MAX_MANIFEST;
    large(&path, limit);
    for args in COMMANDS {
        let error = f.run(args);
        assert!(
            error.contains("sunrise-mod.toml") && error.contains(&limit.to_string()),
            "{error}"
        );
        assert_eq!(fs::metadata(&path).unwrap().len(), limit + 1);
    }
}
#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn bounded_special_manifests() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new("special");
    let path = f.dir.join("sunrise-mod.toml");
    symlink("/dev/zero", &path).unwrap();
    for args in COMMANDS {
        assert!(f.run(args).contains("not a regular file"));
    }
    fs::remove_file(&path).unwrap();
    let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    for args in COMMANDS {
        assert!(f.run(args).contains("not a regular file"));
    }
}
#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn sync_refuses_a_cargo_manifest_fifo_without_waiting() {
    let f = Fixture::new("cargo-fifo");
    f.manifest("");
    let cargo = f.dir.join("Cargo.toml");
    let c = std::ffi::CString::new(cargo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    let path = f.dir.join("sunrise-mod.toml");
    let before = fs::read(&path).unwrap();
    let error = f.run(&["internals", "sync"]);
    assert!(
        error.contains("Cargo.toml") && error.contains("not a regular file"),
        "{error}"
    );
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
#[ignore = "daily"]
fn bounded_catalog_commands() {
    const LIMIT: u64 = 256 << 20;
    let f = Fixture::new("internals-catalog");
    f.manifest("");
    f.sdk.release(BUILD, MIXINS);
    f.sdk.prefetch(&f.dir, BUILD, BUILD);
    let data = f.dir.join(".sunrise/cache").join(BUILD).join("build-data");
    fs::create_dir_all(&data).unwrap();
    let catalog = data.join("internals.json");
    large(&catalog, LIMIT);
    let manifest = f.dir.join("sunrise-mod.toml");
    let before = fs::read(&manifest).unwrap();
    let error = f.run(&["internals", "add", "lower::run"]);
    assert!(
        error.contains("internals.json") && error.contains(&LIMIT.to_string()),
        "{error}"
    );
    assert_eq!(fs::metadata(&catalog).unwrap().len(), LIMIT + 1);
    assert_eq!(fs::read(manifest).unwrap(), before);
}
