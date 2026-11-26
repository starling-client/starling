//! Minimal signed SDKs for integration tests of the debug tool. No downloads or build slices.
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use sunrise_mod_package::sdk::{self, Index};
use sunrise_release_sign::{self as sign, SigningKey};

pub struct OfflineSdk {
    base: PathBuf,
    key: SigningKey,
}

impl OfflineSdk {
    pub fn new(project: &Path) -> Self {
        Self {
            base: project.join("releases"),
            key: SigningKey::generate().unwrap().0,
        }
    }

    /// A signed `sdk-<commit>` release, with `build_data`'s files as its `build-data.tar.xz` when
    /// given.
    pub fn release(&self, commit: &str, mixins: &str, build_data: Option<&[(&str, Vec<u8>)]>) {
        let surface = format!(r#"{{"schema":{},"hash":"ab"}}"#, mod_surface::SCHEMA);
        self.release_with_surface(commit, mixins, build_data, &surface);
    }

    /// A signed `sdk-<commit>` release whose `surface.json` is `surface` (one this tool may refuse
    /// to read), with `build_data`'s files as its `build-data.tar.xz` when given.
    pub fn release_with_surface(
        &self,
        commit: &str,
        mixins: &str,
        build_data: Option<&[(&str, Vec<u8>)]>,
        surface: &str,
    ) {
        let tag = format!("sdk-{commit}");
        let folder = self.base.join(&tag);
        fs::create_dir_all(&folder).unwrap();
        if let Some(files) = build_data {
            let mut archive = tar::Builder::new(Vec::new());
            for (name, bytes) in files {
                let mut header = tar::Header::new_ustar();
                header.set_size(bytes.len() as u64);
                header.set_mode(0o644);
                archive.append_data(&mut header, name, &bytes[..]).unwrap();
            }
            let tar = archive.into_inner().unwrap();
            fs::write(folder.join("build-data.tar.xz"), xz_stored(&tar)).unwrap();
        }
        // A test may put its own surface in the folder first, such as one of a schema this tool
        // no longer reads.
        let path = folder.join("surface.json");
        if !path.exists() {
            fs::write(&path, surface).unwrap();
        }
        // Repinning also fetches the project's API archive. These tests have no Rust sources to scan.
        fs::write(
            folder.join("api-source.tar.xz"),
            include_bytes!("../../../sunrise-mod-package/src/sdk/fixture.tar.xz"),
        )
        .unwrap();
        let build = sdk::Build {
            commit: commit.into(),
            game: sdk::GAME.into(),
            date: "2026-11-13".into(),
            nightly: None,
        };
        let mut index = Index::of_folder(&folder, build, "1.99.1".into()).unwrap();
        index.mixins.rev = mixins.into();
        let index = index.to_json();
        fs::write(folder.join(sdk::INDEX), &index).unwrap();
        let sums = sign::sums_file(&[(sdk::INDEX.into(), format!("{:x}", Sha256::digest(&index)))]);
        fs::write(folder.join(sign::SUMS), &sums).unwrap();
        fs::write(
            folder.join(sign::SIGNATURE),
            self.key.sign(&tag, sums.as_bytes()),
        )
        .unwrap();
    }

    pub fn command(&self, project: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"));
        command
            .arg("sunrise-mod")
            .current_dir(project)
            .env("SUNRISE_SDK_BASE", &self.base)
            .env(sign::SDK_KEY_VAR, self.key.public_base64());
        command
    }

    /// Let the tool verify and stamp both SDKs before the test seeds their unpacked build data.
    pub fn prefetch(&self, project: &Path, old: &str, new: &str) {
        let output = self
            .command(project)
            .args(["sdk", "diff", old, new])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// `data` as a one-block `.xz` stream of stored LZMA2 chunks with no integrity check: valid for any
/// decoder (the `.xz` file format, sections 2 and 3, and LZMA2's uncompressed chunks), and enough
/// for fixtures, since the tool's dependencies hold no xz encoder.
fn xz_stored(data: &[u8]) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> [u8; 4] {
        let mut crc = !0u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
            }
        }
        (!crc).to_le_bytes()
    }
    fn varint(out: &mut Vec<u8>, mut value: u64) {
        while value >= 0x80 {
            out.push(value as u8 | 0x80);
            value >>= 7;
        }
        out.push(value as u8);
    }
    // Stream flags: check type None.
    let flags = [0, 0];
    let mut out = vec![0xFD, b'7', b'z', b'X', b'Z', 0];
    out.extend(flags);
    out.extend(crc32(&flags));
    // Block header: 12 bytes, one filter (LZMA2, a 4 KiB dictionary), no sizes.
    let header = [2, 0, 0x21, 1, 0, 0, 0, 0];
    out.extend(header);
    out.extend(crc32(&header));
    let block_start = out.len() - 12;
    for (n, chunk) in data.chunks(1 << 16).enumerate() {
        // 1 resets the dictionary on the first chunk; 2 stores without a reset.
        out.push(if n == 0 { 1 } else { 2 });
        out.extend(((chunk.len() - 1) as u16).to_be_bytes());
        out.extend(chunk);
    }
    out.push(0);
    let unpadded = (out.len() - block_start) as u64;
    out.resize(out.len().next_multiple_of(4), 0);
    let mut index = vec![0];
    varint(&mut index, 1);
    varint(&mut index, unpadded);
    varint(&mut index, data.len() as u64);
    index.resize(index.len().next_multiple_of(4), 0);
    let index_crc = crc32(&index);
    index.extend(index_crc);
    let mut footer = ((index.len() / 4 - 1) as u32).to_le_bytes().to_vec();
    footer.extend(flags);
    out.extend(index);
    out.extend(crc32(&footer));
    out.extend(footer);
    out.extend(b"YZ");
    out
}
