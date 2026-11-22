//! sunrise-verify: only `verify`, for the install scripts (packaging/server/install.sh and install.ps1). It
//! is the same check `sunrise-release-sign verify` and the launcher run, with the keys built in from
//! release-keys.txt. The scripts pin this binary's SHA-256 (tools/ci/pin_verifier.py), so nothing the caller passes
//! can change who is trusted: it takes no `--key`, `--repository` or `--publishing`, and refuses any other option.
//!
//!     sunrise-verify verify --tag <tag> [--signature <file>] [--only <file>]... <folder>
//!
//! Built with the `test-key` feature (the install tests only, never a release), it trusts exactly the one key in
//! `SUNRISE_VERIFY_TEST_KEY` instead of the built-in ones, and refuses to run without it; without the feature the
//! variable is not read.

use std::path::PathBuf;
use std::process::ExitCode;
use sunrise_release_sign::PublicKey;
use sunrise_release_sign::folder::verify_release;

const USAGE: &str =
    "usage: sunrise-verify verify --tag <tag> [--signature <file>] [--only <file>]... <folder>";

fn main() -> ExitCode {
    match run(std::env::args_os().skip(1).map(|a| a.into_string())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sunrise-verify: {e}");
            ExitCode::FAILURE
        }
    }
}

struct Request {
    tag: String,
    signature: String,
    only: Vec<String>,
    folder: PathBuf,
}

fn parse(
    args: impl Iterator<Item = Result<String, std::ffi::OsString>>,
) -> Result<Request, String> {
    let mut args = args.map(|a| a.map_err(|a| format!("{a:?} is not valid UTF-8")));
    if args.next().transpose()?.as_deref() != Some("verify") {
        return Err(USAGE.into());
    }
    let (mut tag, mut signature, mut only, mut folder) = (None, None, Vec::new(), None);
    while let Some(arg) = args.next().transpose()? {
        match arg.as_str() {
            "--tag" | "--signature" | "--only" => {
                let value = args
                    .next()
                    .transpose()?
                    .filter(|v| !v.is_empty() && !v.starts_with('-'))
                    .ok_or(format!("{arg} needs a value"))?;
                if arg == "--signature" && value.contains(['/', '\\', ':']) {
                    return Err(format!(
                        "--signature names a file of the folder, not {value:?}"
                    ));
                }
                match arg.as_str() {
                    "--tag" if tag.is_none() => tag = Some(value),
                    "--signature" if signature.is_none() => signature = Some(value),
                    "--only" => only.push(value),
                    _ => return Err(format!("{arg} is given twice")),
                }
            }
            option if option.starts_with('-') => {
                return Err(format!("verify does not take {option}"));
            }
            _ if folder.is_none() => folder = Some(PathBuf::from(arg)),
            _ => return Err(format!("verify takes one folder, not also {arg:?}")),
        }
    }
    Ok(Request {
        tag: tag.ok_or("--tag <tag> is needed")?,
        signature: signature.unwrap_or_else(|| sunrise_release_sign::SIGNATURE.to_owned()),
        only,
        folder: folder.ok_or("verify needs the release's folder")?,
    })
}

#[cfg(feature = "test-key")]
fn keys() -> Result<Vec<PublicKey>, String> {
    match std::env::var("SUNRISE_VERIFY_TEST_KEY") {
        Ok(text) => match sunrise_release_sign::parse_keys(&text)?.as_slice() {
            [key] => Ok(vec![*key]),
            other => Err(format!(
                "SUNRISE_VERIFY_TEST_KEY must be exactly one key, not {}",
                other.len()
            )),
        },
        Err(e) => Err(format!("SUNRISE_VERIFY_TEST_KEY: {e}")),
    }
}

#[cfg(not(feature = "test-key"))]
fn keys() -> Result<Vec<PublicKey>, String> {
    Ok(sunrise_release_sign::trusted_keys())
}

fn run(args: impl Iterator<Item = Result<String, std::ffi::OsString>>) -> Result<(), String> {
    let request = parse(args)?;
    let keys = keys()?;
    if keys.is_empty() {
        return Err("no key to check the signature with".into());
    }
    verify_release(
        &request.tag,
        &keys,
        &request.signature,
        &request.only,
        &request.folder,
    )
}
