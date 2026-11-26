use super::*;
use lzma_rust2::{XzOptions, XzWriter};
use sunrise_release_sign::SigningKey;

const CURRENT: &str = "47dccbf195d32d65a184d07b75a7c5cbe63c6ac9";
const NEW: &str = "db7352509f10d83df9f026f4faf7d82e456d5525";
const ASSET: &str = "tool-fixture.tar.xz";
const ORIGINAL: &[u8] = b"the installed tool";
const REPLACEMENT: &[u8] = b"the new tool";

struct Fixture {
    root: tempfile::TempDir,
    key: PublicKey,
    exe: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let assets = root.path().join(MOD_TOOL);
        fs::create_dir(&assets).unwrap();
        let exe = root.path().join(EXE);
        fs::write(&exe, ORIGINAL).unwrap();
        let info = format!(r#"{{"commit":"{NEW}"}}"#);
        let archive = archive(REPLACEMENT);
        let sums = release::sums_file(&[
            (INFO.to_owned(), release::sha256_hex(info.as_bytes())),
            (ASSET.to_owned(), release::sha256_hex(&archive)),
        ]);
        let (key, _) = SigningKey::generate().unwrap();
        fs::write(assets.join(INFO), info).unwrap();
        fs::write(assets.join(ASSET), archive).unwrap();
        fs::write(assets.join(SUMS), &sums).unwrap();
        fs::write(assets.join(SIGNATURE), key.sign(MOD_TOOL, sums.as_bytes())).unwrap();
        Self {
            root,
            key: key.public(),
            exe,
        }
    }

    fn asset(&self, name: &str) -> PathBuf {
        self.root.path().join(MOD_TOOL).join(name)
    }

    fn update(&self, current: &str) -> Result<Option<Tool>, String> {
        let base = sdk_fetch::Base::Folder(self.root.path().to_path_buf());
        update(&base, &[self.key], ASSET, &self.exe, current)
    }
}

fn archive(exe: &[u8]) -> Vec<u8> {
    let xz = XzWriter::new(Vec::new(), XzOptions::with_preset(0)).unwrap();
    let mut tar = tar::Builder::new(xz);
    let mut header = tar::Header::new_ustar();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_mode(0o755);
    header.set_size(exe.len() as u64);
    tar.append_data(&mut header, EXE, exe).unwrap();
    tar.into_inner().unwrap().finish().unwrap()
}

#[test]
fn signed_release_replaces_the_tool_and_then_reports_it_current_without_the_archive() {
    let release = Fixture::new();
    assert_eq!(release.update(CURRENT).unwrap().unwrap().commit, NEW);
    assert_eq!(fs::read(&release.exe).unwrap(), REPLACEMENT);
    fs::remove_file(release.asset(ASSET)).unwrap();
    assert!(release.update(NEW).unwrap().is_none());
    assert_eq!(fs::read(&release.exe).unwrap(), REPLACEMENT);
}

#[test]
fn a_signature_from_the_wrong_key_never_replaces_or_reports_current() {
    let release = Fixture::new();
    let (other, _) = SigningKey::generate().unwrap();
    let sums = fs::read(release.asset(SUMS)).unwrap();
    fs::write(release.asset(SIGNATURE), other.sign(MOD_TOOL, &sums)).unwrap();
    for current in [CURRENT, NEW] {
        release.update(current).unwrap_err();
        assert_eq!(fs::read(&release.exe).unwrap(), ORIGINAL);
    }
}

#[test]
fn tampered_metadata_cannot_claim_that_the_installed_tool_is_current() {
    let release = Fixture::new();
    fs::write(release.asset(INFO), format!(r#"{{"commit":"{CURRENT}"}}"#)).unwrap();
    release.update(CURRENT).unwrap_err();
    assert_eq!(fs::read(&release.exe).unwrap(), ORIGINAL);
}

#[test]
fn a_valid_archive_with_the_wrong_hash_never_replaces_the_tool() {
    let release = Fixture::new();
    fs::write(release.asset(ASSET), archive(b"a different tool")).unwrap();
    release.update(CURRENT).unwrap_err();
    assert_eq!(fs::read(&release.exe).unwrap(), ORIGINAL);
}

#[test]
fn an_attempt_reserves_the_next_24_hours() {
    let cache = tempfile::tempdir().unwrap();
    let now = SystemTime::now();
    assert!(claim_notice_attempt(cache.path(), now).is_some());
    let stamp = cache.path().join("mod-tool-update-check");
    let written = stamp.metadata().unwrap().modified().unwrap();
    assert!(claim_notice_attempt(cache.path(), written + Duration::from_secs(86_399)).is_none());
    assert!(claim_notice_attempt(cache.path(), written + Duration::from_secs(86_400)).is_some());
}

#[test]
fn only_a_different_full_commit_is_new_to_the_tool() {
    let tool = Tool::read(br#"{"commit":"47dccbf195d32d65a184d07b75a7c5cbe63c6ac9"}"#).unwrap();
    assert!(!tool.differs_from("47dccbf195d32d65a184d07b75a7c5cbe63c6ac9"));
    assert!(tool.differs_from("db7352509f10d83df9f026f4faf7d82e456d5525"));
    assert!(Tool::read(br#"{"commit":"47dccbf"}"#).is_err());
}

#[test]
fn only_a_standalone_rolling_tool_outside_ci_and_online_checks_for_updates() {
    assert!(checks_allowed(false, MOD_TOOL, true));
    assert!(!checks_allowed(true, MOD_TOOL, true));
    assert!(!checks_allowed(false, MOD_TOOL, false));
    assert!(!checks_allowed(false, "", true));
}
