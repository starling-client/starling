//! Exercise the real CLI packers, including their archive and exporter setup.
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "daily"]
fn client_and_server_layouts_require_and_ship_std() {
    let root = std::env::temp_dir().join(format!("xtask-packaging-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    for name in [
        "xtask/Cargo.toml",
        "tools/fixture",
        "mods/sunrise/src/test/java/fixture",
        "crates/sunrise-prepare/fixture",
        "research-lock.json",
        "LICENSE",
        "packaging/PLAY.txt",
        "exporters/manifest.json",
    ] {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"fixture").unwrap();
    }
    git(&root, &["init"]);
    git(&root, &["add", "."]);
    git(
        &root,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-m",
            "fixture",
        ],
    );
    for target in [
        "linux",
        "linux-musl",
        "windows",
        "windows-arm64",
        "macos",
        "macos-x64",
    ] {
        let source = root.join(target);
        fs::create_dir_all(&source).unwrap();
        let suffix = if target.starts_with("windows") {
            ".exe"
        } else {
            ""
        };
        for name in [
            "sunrise-client",
            "sunrise-ctl",
            "sunrise-prepare",
            "sunrise-server",
        ] {
            fs::write(source.join(format!("{name}{suffix}")), b"program").unwrap();
        }
        for task in ["package", "package-server"] {
            let out = root.join(format!("{target}-{task}"));
            let run = || {
                Command::new(env!("CARGO_BIN_EXE_xtask"))
                    .arg(task)
                    .arg(source.join(format!("sunrise-client{suffix}")))
                    .arg(&out)
                    .current_dir(&root)
                    .output()
                    .unwrap()
            };
            let missing = run();
            assert!(!missing.status.success());
            assert!(String::from_utf8_lossy(&missing.stderr).contains("sunrise-wasm-std.wasm"));
            fs::write(source.join("sunrise-wasm-std.wasm"), b"wasm-member").unwrap();
            let packed = run();
            assert!(
                packed.status.success(),
                "{}",
                String::from_utf8_lossy(&packed.stderr)
            );
            assert_eq!(
                fs::read(out.join("sunrise-wasm-std.wasm")).unwrap(),
                b"wasm-member"
            );
            let engine = if task == "package" {
                "sunrise-client"
            } else {
                "sunrise-engine"
            };
            assert_eq!(
                fs::read(out.join(format!("{engine}{suffix}"))).unwrap(),
                b"program"
            );
            assert!(out.join("exporters/manifest.json").is_file());
            fs::remove_file(source.join("sunrise-wasm-std.wasm")).unwrap();
        }
    }
    fs::remove_dir_all(root).unwrap();
}
