//! sunrise-release-sign: checks a release's signed checksums, and makes new keys
//! (docs/release-flow.md, "Signed checksums"). The signing itself is tools/ci/sign_release.sh: the
//! job that holds the private key runs only the runner's sha256sum and OpenSSL, no code from the
//! repository.
//!
//!     sunrise-release-sign verify --tag v2026.10.19 [--key <base64>] [--signature <file>] [--only <file>]... [--publishing] <folder>
//!     sunrise-release-sign keygen --public-out key.pub | gh secret set RELEASE_SIGNING_KEY --env release
//!
//! `verify` checks SHA256SUMS.sig (or `--signature`, e.g. a Nightly's SHA256SUMS.v2.sig with
//! `--tag nightly-<date>-<commit>`) and every file SHA256SUMS lists with the keys built in from
//! release-keys.txt (or `--key`), as the launcher does; with `--only`, just the files named (each must
//! be listed), for a check that downloaded only those. With `--publishing` (release.yml) a release
//! may go out unsigned, or signed with a key nobody can check yet, only while release-keys.txt
//! lists no key; once it lists one, anything launchers would refuse fails the release.

use std::io::{IsTerminal, Write};
use std::process::ExitCode;
use sunrise_release_sign::SigningKey;
use sunrise_release_sign::folder::{check_folder, options};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("verify") => check_folder(&args[1..]),
        Some("keygen") => keygen(&args[1..]),
        _ => Err(
            "usage: sunrise-release-sign verify --tag <tag> [--key <base64>]... [--repository <owner/name>] [--signature <file>] \
             [--only <file>]... [--publishing] <folder> | keygen --public-out <file>"
                .into(),
        ),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sunrise-release-sign: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Prints a new private key to stdout, which must be a pipe (into `gh secret set`), never a
/// terminal, and writes its public key to `--public-out`.
fn keygen(args: &[String]) -> Result<(), String> {
    let (public_out, _) = options(args, "--public-out");
    let public_out = public_out
        .into_iter()
        .next()
        .ok_or("keygen needs --public-out <file>")?;
    if std::io::stdout().is_terminal() {
        return Err(
            "keygen writes the private key to its output: pipe it into `gh secret set`".into(),
        );
    }
    let (key, secret) = SigningKey::generate()?;
    std::fs::write(&public_out, key.public_base64() + "\n").map_err(|e| e.to_string())?;
    let mut out = std::io::stdout().lock();
    out.write_all(secret.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|e| e.to_string())
}
