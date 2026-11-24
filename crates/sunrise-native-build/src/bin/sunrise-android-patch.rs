//! `sunrise-android-patch STAGED SOURCE COMMIT OUT TARGET FEATURES MOD_DIR...`: the patch crates of the
//! mods' `[[native.mixin]]` rows (with their support modules, bridges and loops), for Starling's
//! Android store (tools/android_native_sdk.py). STAGED is the staged copy, for its `functions.json`
//! and `loops.json` (tools/prebuilt_game.py `stage`); SOURCE the commit's own source, unstaged, which
//! the index's byte ranges are of. Nothing is
//! downloaded but the horizon-mixins tool, once per pin, into `$SUNRISE_ANDROID_PATCH_HOME`
//! (`curl`, where the network is). The tool plans against TARGET's cfg and the game crate's
//! FEATURES (comma separated). Prints JSON: the folder holding one crate per name, the crate
//! names, and the install expressions the library entry's `installs()` lists; `null` when no mod
//! has a row.
//!
//! `sunrise-android-patch --tool`: only fetch the tool into the same home, for a base with the
//! network that a later offline build reads (mod-builds' store-base).
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use sunrise_native_build::{MixinsIdentity, api, host, patch, prebuilt, rust};

fn home() -> PathBuf {
    std::env::var_os("SUNRISE_ANDROID_PATCH_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("sunrise-android-patch"))
}

/// A GET through `curl`: a 404 is `None`, the network's absence an error.
fn get(url: &str) -> Result<host::Body, String> {
    let out = Command::new("curl")
        .args([
            "-fsSL",
            "--retry",
            "3",
            "-w",
            "%{http_code}",
            "-o",
            "-",
            url,
        ])
        .output()
        .map_err(|e| format!("curl {url}: {e}"))?;
    if out.status.success() {
        let mut body = out.stdout;
        // `-w` writes the status after the body.
        body.truncate(body.len().saturating_sub(3));
        return Ok(Some(
            Box::new(std::io::Cursor::new(body)) as Box<dyn Read + Send>
        ));
    }
    if out.stdout.ends_with(b"404") {
        return Ok(None);
    }
    Err(format!(
        "curl {url}: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    ))
}

/// This computer's toolchain, as rustup set it up: the one the store builds with.
fn toolchain() -> Result<rust::Unpacked, String> {
    let rustc = |arg: &str| -> Result<String, String> {
        let out = Command::new("rustc")
            .arg(arg)
            .output()
            .map_err(|e| format!("rustc {arg}: {e}"))?;
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    };
    let sysroot = rustc("--print=sysroot")?;
    let host = rustc("-vV")?
        .lines()
        .find_map(|line| line.strip_prefix("host: ").map(str::to_owned))
        .ok_or("rustc -vV names no host")?;
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))
        .ok_or("no CARGO_HOME")?;
    Ok(rust::Unpacked {
        dir: PathBuf::from(sysroot),
        target: host,
        cargo_home,
        cargo_config: Vec::new(),
        c_bin: None,
    })
}

fn fail(message: impl std::fmt::Display) -> ! {
    eprintln!("sunrise-android-patch: {message}");
    std::process::exit(1);
}

fn main() {
    host::install(host::Host {
        tr: |en| en,
        root: home,
        get,
        mark_used: |_| Ok(()),
    });
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rust = toolchain().unwrap_or_else(|e| fail(e));
    if args.first().map(String::as_str) == Some("--tool") {
        let identity =
            MixinsIdentity::resolve(std::iter::empty::<&sunrise_native_build::Bundle>(), None)
                .unwrap_or_else(|e| fail(e));
        match patch::mixins_tool(&rust, &identity) {
            Ok(tool) => println!("{}", tool.display()),
            Err(e) => fail(e),
        }
        return;
    }
    let [staged, source, commit, out, target, features, mods @ ..] = args.as_slice() else {
        fail("usage: sunrise-android-patch STAGED SOURCE COMMIT OUT TARGET FEATURES MOD_DIR...")
    };
    let staged = Path::new(staged);
    let source = Path::new(source);
    let bundles: Vec<_> = mods
        .iter()
        .map(
            |dir| match sunrise_native_build::bundle_in(Path::new(dir)) {
                Ok(Some(bundle)) => bundle,
                Ok(None) => fail(format!("{dir}: no sunrise-mod.toml")),
                Err(e) => fail(format!("{dir}: {e}")),
            },
        )
        .collect();
    let projects: Vec<_> = bundles
        .iter()
        .map(|bundle| api::Project::read(bundle).unwrap_or_else(|e| fail(format!("{e:?}"))))
        .collect();
    let features: Vec<&str> = features.split(',').filter(|f| !f.is_empty()).collect();
    let sdk: prebuilt::Identity = serde_json::from_value(serde_json::json!({
        "commit": commit,
        "rustc": "",
        "target": target,
        "side": "client",
        "game_features": features,
    }))
    .unwrap_or_else(|e| fail(format!("identity: {e}")));
    let mixins = MixinsIdentity::resolve(&bundles, None).unwrap_or_else(|e| fail(e));
    let out = Path::new(out);
    std::fs::create_dir_all(out).unwrap_or_else(|e| fail(format!("{}: {e}", out.display())));
    let prepared = patch::prepare_with(
        &rust,
        staged,
        Some(source),
        out,
        &projects,
        &sdk,
        &mixins,
        Some(target),
    )
    .unwrap_or_else(|e| fail(e));
    let json = match prepared {
        None => serde_json::Value::Null,
        Some(prepared) => serde_json::json!({
            "dir": prepared.dir,
            "names": prepared.names,
            "installs": prepared.installs,
            "mods": prepared.bundles.iter().map(|b| b.manifest.id.clone()).collect::<Vec<_>>(),
        }),
    };
    println!("{json}");
}
