use super::*;
use std::io::Write;

const NAV: &[u8] = b"baked lookup table";
const IMAGE: &[u8] = b"\x7fELF an interpreter image";

fn manifest_text() -> String {
    format!(
        "id='ruby'\nname='Ruby'\nversion='0.1.0'\napi='0.2'\ncapabilities=['registries']\n\
         [native]\ngame='26.3'\nsunrise='{}'\nmixins='{}'\n\
         [runtime]\n\"tables/nav.bin\"='{}'\n\"image\"='{}'\n",
        "a".repeat(40),
        "b".repeat(40),
        sha512_of(NAV),
        sha512_of(IMAGE),
    )
}

struct Folder(std::path::PathBuf);
impl Folder {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("runtime-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("runtime/tables")).unwrap();
        std::fs::write(dir.join(MANIFEST), manifest_text()).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname='ruby'\nversion='0.1.0'\n",
        )
        .unwrap();
        std::fs::write(dir.join("src/lib.rs"), "pub fn hooks() {}").unwrap();
        std::fs::write(dir.join("runtime/tables/nav.bin"), NAV).unwrap();
        std::fs::write(dir.join("runtime/image"), IMAGE).unwrap();
        Self(dir)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_folder_with_the_listed_files_reads_and_serves_the_verified_bytes() {
    let folder = Folder::new("folder");
    let package = Package::read(folder.path()).unwrap();
    let runtime = package.read_runtime(folder.path()).unwrap().unwrap();
    assert_eq!(runtime.names(), ["image", "tables/nav.bin"]);
    assert_eq!(runtime.file("tables/nav.bin").unwrap().0, NAV);
    // Any file type: an ELF-looking image is data here, not an executable component.
    let (bytes, path) = runtime.file("image").unwrap();
    assert_eq!(bytes, IMAGE);
    assert!(runtime.file("tables/other.bin").is_none());
    assert_eq!(path, Some(folder.path().join("runtime/image").as_path()));
    assert_eq!(runtime.digest(), package.manifest.runtime.unwrap().digest());
}

#[test]
fn the_digest_follows_the_names_and_the_bytes() {
    let table = |pairs: &[(&str, &[u8])]| {
        RuntimeTable::try_from(
            pairs
                .iter()
                .map(|(name, bytes)| (name.to_string(), sha512_of(bytes)))
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
        .unwrap()
    };
    let base = table(&[("a", b"1"), ("b", b"2")]);
    assert_eq!(base.digest().len(), 64);
    assert_eq!(base.digest(), table(&[("b", b"2"), ("a", b"1")]).digest());
    assert_ne!(base.digest(), table(&[("a", b"1"), ("b", b"3")]).digest());
    assert_ne!(base.digest(), table(&[("a", b"1"), ("c", b"2")]).digest());
}

#[test]
fn a_missing_extra_or_mismatched_file_is_refused_by_name() {
    let folder = Folder::new("refused");
    let nav = folder.path().join("runtime/tables/nav.bin");
    std::fs::write(&nav, b"edited by hand").unwrap();
    let package = Package::read(folder.path()).unwrap();
    let error = package.read_runtime(folder.path()).unwrap_err();
    assert!(
        error.contains("runtime/tables/nav.bin") && error.contains("doesn't match"),
        "{error}"
    );
    std::fs::write(&nav, NAV).unwrap();
    std::fs::write(folder.path().join("runtime/extra.bin"), b"x").unwrap();
    let error = Package::read(folder.path()).err().unwrap();
    assert!(
        error.contains("runtime/extra.bin") && error.contains("not listed"),
        "{error}"
    );
    std::fs::remove_file(folder.path().join("runtime/extra.bin")).unwrap();
    std::fs::remove_file(&nav).unwrap();
    let error = Package::read(folder.path()).err().unwrap();
    assert!(
        error.contains("tables/nav.bin") && error.contains("no runtime/tables/nav.bin"),
        "{error}"
    );
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_in_runtime_is_refused() {
    let folder = Folder::new("link");
    let image = folder.path().join("runtime/image");
    std::fs::remove_file(&image).unwrap();
    std::os::unix::fs::symlink(folder.path().join("Cargo.toml"), &image).unwrap();
    assert!(Package::read(folder.path()).is_err());
}

#[test]
fn a_content_mod_has_no_runtime() {
    let text = "id='ruby'\nname='Ruby'\nversion='0.1.0'\napi='0.2'\ncapabilities=['registries']\n\
                [runtime]\n\"a\"='"
        .to_owned()
        + &sha512_of(b"1")
        + "'\n";
    assert!(
        Manifest::parse(&text)
            .unwrap_err()
            .contains("a content mod has no code")
    );
    let folder = Folder::new("content");
    std::fs::write(
        folder.path().join(MANIFEST),
        "id='ruby'\nname='Ruby'\nversion='0.1.0'\napi='0.2'\ncapabilities=['registries']\n",
    )
    .unwrap();
    std::fs::remove_file(folder.path().join("Cargo.toml")).unwrap();
    std::fs::remove_dir_all(folder.path().join("src")).unwrap();
    let error = Package::read(folder.path()).err().unwrap();
    assert!(error.contains("a content mod has no code"), "{error}");
}

#[test]
fn the_table_and_the_folder_have_no_file_count_or_size_caps() {
    let hash = sha512_of(b"1");
    let many: std::collections::BTreeMap<String, String> =
        (0..300).map(|i| (format!("f{i}"), hash.clone())).collect();
    let table = RuntimeTable::try_from(many).unwrap();
    let sizes: Vec<_> = (0..300)
        .map(|i| (format!("f{i}"), (64 << 20) + 1))
        .collect();
    RuntimeTable::check_layout(Some(&table), &sizes).unwrap();
}

#[test]
fn a_packed_smod_carries_the_files_and_is_checked_as_the_folder_is() {
    let folder = Folder::new("pack");
    let bytes = Package::pack_directory(folder.path()).unwrap();
    let archive = folder.path().with_extension("smod");
    std::fs::write(&archive, &bytes).unwrap();
    let package = Package::read(&archive).unwrap();
    let runtime = package.read_runtime(&archive).unwrap().unwrap();
    assert_eq!(runtime.file("tables/nav.bin"), Some((NAV, None)));
    assert_eq!(runtime.digest(), package.manifest.runtime.unwrap().digest());
    // An entry the table does not list is refused.
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    for (name, body) in [
        (MANIFEST, manifest_text().into_bytes()),
        (
            "Cargo.toml",
            b"[package]\nname='ruby'\nversion='0.1.0'\n".to_vec(),
        ),
        ("src/lib.rs", b"pub fn hooks() {}".to_vec()),
        ("runtime/image", IMAGE.to_vec()),
        ("runtime/tables/nav.bin", NAV.to_vec()),
        ("runtime/stray", b"x".to_vec()),
    ] {
        zip.start_file(name, options).unwrap();
        zip.write_all(&body).unwrap();
    }
    zip.finish().unwrap();
    let error = Package::read(&archive).err().unwrap();
    assert!(
        error.contains("runtime/stray") && error.contains("not listed"),
        "{error}"
    );
    std::fs::remove_file(archive).unwrap();
}

#[test]
fn packing_refuses_a_file_that_does_not_match_its_hash() {
    let folder = Folder::new("packbad");
    std::fs::write(folder.path().join("runtime/image"), b"changed").unwrap();
    let error = Package::pack_directory(folder.path()).unwrap_err();
    assert!(error.contains("runtime/image"), "{error}");
}
