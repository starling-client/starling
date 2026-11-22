use std::fmt::Write as _;
use std::process::Command;
use std::{env, fs, path::PathBuf};
fn main() {
    locales();
    println!("cargo:rerun-if-env-changed=SUNRISE_GAME_BUILD_ID");
    let commit = game_build_id(std::env::var("SUNRISE_GAME_BUILD_ID").ok());
    assert!(
        !commit.trim().is_empty(),
        "game build identity must not be empty"
    );
    let rustc = Command::new(std::env::var_os("RUSTC").expect("RUSTC"))
        .arg("-vV")
        .output()
        .expect("rustc");
    let version = String::from_utf8(rustc.stdout).expect("rustc utf8");
    let release = version
        .lines()
        .find_map(|line| line.strip_prefix("release: "))
        .expect("release");
    println!("cargo:rustc-env=SUNRISE_GAME_BUILD_ID={commit}");
    println!("cargo:rustc-env=SUNRISE_RUSTC_VERSION={release}");
}

/// Release and SDK builds name their commit; any other build is `local`, never a git call whose answer goes stale.
fn game_build_id(named: Option<String>) -> String {
    named.unwrap_or_else(|| "local".to_owned())
}

/// This crate's lang files (`assets/lang/<code>.json`) as a static `Files` value, as
/// sunrise-reconnect's build does: the title screen's Mods button label travels with the crate
/// that supplies the row.
fn locales() {
    let source = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("assets/lang");
    println!("cargo:rerun-if-changed={}", source.display());
    let mut paths: Vec<_> = fs::read_dir(&source)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort_unstable();
    let mut list = String::from("&[\n");
    for path in paths {
        let locale = path.file_stem().unwrap().to_str().unwrap();
        writeln!(list, "    ({locale:?}, include_str!({:?})),", path).unwrap();
    }
    list.push_str("]\n");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::write(out.join("locales.rs"), list).unwrap();
}

#[cfg(test)]
mod tests {
    use super::game_build_id;

    #[test]
    fn a_build_that_names_no_commit_is_local() {
        assert_eq!(game_build_id(None), "local");
        assert_eq!(game_build_id(Some("abc123".into())), "abc123");
    }
}
