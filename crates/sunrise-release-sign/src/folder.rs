//! `verify`: checks a folder holding a release's assets as the launcher does. Shared by the
//! `sunrise-release-sign` and `sunrise-verify` binaries, so there is one implementation.

use crate::{PublicKey, SIGNATURE, SUMS, Sha256, parse_keys, verify};
use std::io::Read;
use std::path::{Path, PathBuf};

/// The values of every `--name value` in `args`, and the other arguments.
pub fn options(args: &[String], name: &str) -> (Vec<String>, Vec<String>) {
    let mut values = Vec::new();
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == name {
            values.extend(it.next().cloned());
        } else {
            rest.push(arg.clone());
        }
    }
    (values, rest)
}

fn tag_of(args: &[String]) -> Result<(String, Vec<String>), String> {
    let (tags, rest) = options(args, "--tag");
    let tag = tags.into_iter().next().ok_or("--tag <tag> is needed")?;
    Ok((tag, rest))
}

fn file_hash(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hash = Sha256::default();
    let mut buffer = vec![0; 1 << 16];
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if n == 0 {
            return Ok(hash.hex());
        }
        hash.update(&buffer[..n]);
    }
}

/// Checks a folder holding a release's assets, as the launcher does.
pub fn check_folder(args: &[String]) -> Result<(), String> {
    let (tag, rest) = tag_of(args)?;
    let (keys, rest) = options(&rest, "--key");
    let (repositories, rest) = options(&rest, "--repository");
    let repository = repositories
        .first()
        .map(String::as_str)
        .unwrap_or(crate::REPOSITORY);
    let (only, rest) = options(&rest, "--only");
    let (signature_file, rest) = options(&rest, "--signature");
    let signature_file = signature_file
        .into_iter()
        .next()
        .unwrap_or_else(|| SIGNATURE.to_owned());
    if signature_file.contains(['/', '\\']) {
        return Err(format!(
            "--signature names a file of the folder, not {signature_file:?}"
        ));
    }
    let publishing = rest.iter().any(|a| a == "--publishing");
    let folder = PathBuf::from(
        rest.iter()
            .find(|a| *a != "--publishing")
            .ok_or("verify needs the release's folder")?,
    );
    let keys = if keys.is_empty() {
        crate::trusted_keys_for(repository)
    } else {
        parse_keys(&keys.join("\n"))?
    };
    let sums = std::fs::read(folder.join(SUMS)).map_err(|e| format!("{SUMS}: {e}"))?;
    let listed = match std::fs::read_to_string(folder.join(&signature_file)) {
        Ok(signature) if !keys.is_empty() => verify(&tag, &sums, &signature, &keys)?,
        Ok(_) if publishing && repository == crate::REPOSITORY => {
            println!(
                "::warning::{tag}: signed, but release-keys.txt lists no key yet, so launchers can't check it"
            );
            crate::parse_sums(&String::from_utf8_lossy(&sums))?
        }
        Err(_) if publishing && keys.is_empty() && repository == crate::REPOSITORY => {
            println!(
                "::warning::{tag}: published with {SUMS} but unsigned: release-keys.txt lists no key yet"
            );
            crate::parse_sums(&String::from_utf8_lossy(&sums))?
        }
        Ok(_) => return Err("no key to check the signature with".into()),
        Err(_) => {
            return Err(format!(
                "{tag} has no {signature_file}: launchers would refuse it (is the release environment's \
                 RELEASE_SIGNING_KEY set?)"
            ));
        }
    };
    check_listed(&tag, &listed, &only, &folder)
}

/// What `sunrise-verify` checks: the folder's `SUMS` is signed for `tag` in `signature_file` by one of `keys`
/// (a missing or unreadable signature, or no key, refuses), and each file `only` names (all of them when empty) is
/// listed and has the signed hash.
pub fn verify_release(
    tag: &str,
    keys: &[PublicKey],
    signature_file: &str,
    only: &[String],
    folder: &Path,
) -> Result<(), String> {
    if signature_file.contains(['/', '\\']) {
        return Err(format!(
            "--signature names a file of the folder, not {signature_file:?}"
        ));
    }
    let sums = std::fs::read(folder.join(SUMS)).map_err(|e| format!("{SUMS}: {e}"))?;
    let signature = std::fs::read_to_string(folder.join(signature_file)).map_err(|_| {
        format!("{tag} has no {signature_file}: launchers would refuse it, so it was not installed")
    })?;
    let listed = verify(tag, &sums, &signature, keys)?;
    check_listed(tag, &listed, only, folder)
}

fn check_listed(
    tag: &str,
    listed: &[(String, String)],
    only: &[String],
    folder: &Path,
) -> Result<(), String> {
    if let Some(name) = only
        .iter()
        .find(|name| !listed.iter().any(|(n, _)| n == *name))
    {
        return Err(format!("{name} is not in {tag}'s {SUMS}"));
    }
    let checked: Vec<_> = listed
        .iter()
        .filter(|(name, _)| only.is_empty() || only.contains(name))
        .collect();
    for (name, hex) in &checked {
        crate::check(name, hex, &file_hash(&folder.join(name))?)?;
    }
    println!("{tag}: {} files match {SUMS}", checked.len());
    Ok(())
}
