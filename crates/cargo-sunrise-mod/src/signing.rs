//! Author-held Minisign keys and detached signatures; unrelated to Sunrise's SDK release key.
use crate::options::Options;
use minisign::{KeyPair, PublicKey, SecretKey};
use std::{
    env,
    io::Read,
    path::{Path, PathBuf},
};
use sunrise_mod_package::manifest::{MANIFEST, MAX_MANIFEST, Manifest};
use zeroize::Zeroizing;

mod files;
use files::Directory;

const KEY_NAME: &str = "signing.key";
const PUBLIC_NAME: &str = "signing.pub";
const KEY_ENV: &str = "SUNRISE_MOD_SIGNING_KEY";
const PASSPHRASE_ENV: &str = "SUNRISE_MOD_SIGNING_PASSPHRASE";
const KEY_LIMIT: u64 = 16 << 10;

pub(crate) fn keygen(args: &[String]) -> Result<(), String> {
    let mut options = Options::new("keygen", args, &["--out"]);
    let out = options.value("--out")?;
    let no_password = options.switch("--no-password");
    options.finish()?;
    #[cfg(windows)]
    if no_password {
        return Err("keygen --no-password is not supported on Windows; generate a passphrase-encrypted key instead".into());
    }
    let out_is_default = out.is_none();
    let path = match out {
        Some(path) => PathBuf::from(path),
        None => default_key_directory()?,
    };
    let out = Directory::open(&path, true)?;
    out.require_private(out_is_default)?;
    out.require_absent(KEY_NAME.as_ref())?;
    out.require_absent(PUBLIC_NAME.as_ref())?;
    let pair = if no_password {
        KeyPair::generate_unencrypted_keypair()
    } else {
        let password = prompt("Minisign passphrase: ")?;
        if password.is_empty() {
            return Err("the passphrase must not be empty".into());
        }
        let again = prompt("Repeat the Minisign passphrase: ")?;
        if password != again {
            return Err("the passphrases do not match".into());
        }
        KeyPair::generate_encrypted_keypair(Some(password.to_string()))
    }
    .map_err(|_| "could not generate a Minisign key pair")?;
    let key_text = Zeroizing::new(
        pair.sk
            .to_box(None)
            .map_err(|_| "could not encode the new secret key")?
            .into_string(),
    );
    let public_box = pair
        .pk
        .to_box()
        .map_err(|_| "could not encode the new public key")?;
    out.write_pair(
        (PUBLIC_NAME.as_ref(), &public_box.to_bytes()),
        (KEY_NAME.as_ref(), key_text.as_bytes()),
    )?;
    eprintln!(
        "Wrote {} and {}. Keep the private key out of Git and release assets.",
        out.path().join(KEY_NAME).display(),
        out.path().join(PUBLIC_NAME).display(),
    );
    println!("{}", pair.pk.to_base64());
    Ok(())
}

fn default_key_directory() -> Result<PathBuf, String> {
    let project = Directory::open(Path::new("."), false)?;
    let text = String::from_utf8(project.read(MANIFEST.as_ref(), MAX_MANIFEST)?)
        .map_err(|_| format!("{MANIFEST}: expected UTF-8 text"))?;
    let id = Manifest::parse(&text)?.id;
    Ok(crate::dev::data_dir()?.join("keys").join(id))
}

pub(crate) fn sign(args: &[String]) -> Result<(), String> {
    let mut options = Options::new("sign", args, &["--key"]);
    let key_file = options.value("--key")?;
    let from_env = options.switch("--key-env");
    let no_password = options.switch("--no-password");
    let file = options.at_most_one()?;
    options.finish()?;
    let file = Path::new(file.ok_or_else(|| crate::help::of("sign").expect("sign has help"))?);
    if from_env && (key_file.is_some() || no_password) {
        return Err("--key-env cannot be combined with --key or --no-password".into());
    }
    let name = file.file_name().ok_or("name a file to sign")?;
    let parent = file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let into = Directory::open(parent, false)?;
    let mut sidecar = name.to_os_string();
    sidecar.push(".minisig");
    into.require_absent(&sidecar)?;
    let bytes = into.read(name, sunrise_mod_package::index::MAX_FILE)?;
    let secret = if from_env {
        let text = secret_env(KEY_ENV)?
            .ok_or_else(|| format!("--key-env needs a text key in {KEY_ENV}"))?;
        if text.len() as u64 > KEY_LIMIT {
            return Err(format!("{KEY_ENV} is too large for a Minisign key"));
        }
        let password = secret_env(PASSPHRASE_ENV)?;
        unlock_ci_key(&text, password.as_ref().map(|text| text.as_str()))?
    } else {
        let key_path = match key_file {
            Some(path) => PathBuf::from(path),
            None => default_key_directory()?.join(KEY_NAME),
        };
        let key_name = key_path.file_name().ok_or("name a private key file")?;
        let key_dir = Directory::open(
            key_path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new(".")),
            false,
        )?;
        let mut bytes = Zeroizing::new(Vec::with_capacity(KEY_LIMIT as usize + 1));
        key_dir
            .open_regular(key_name)?
            .take(KEY_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("{}: {e}", key_path.display()))?;
        if bytes.len() as u64 > KEY_LIMIT {
            return Err(format!("{}: over {KEY_LIMIT} bytes", key_path.display()));
        }
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| format!("{}: expected Minisign key text", key_path.display()))?;
        if no_password {
            SecretKey::from_unencrypted_box(text.to_owned().into())
        } else {
            let password = prompt("Minisign passphrase: ")?;
            SecretKey::from_box(text.to_owned().into(), Some(password.to_string()))
        }
        .map_err(|_| format!("{}: could not unlock the Minisign key; use --no-password for a passwordless key file", key_path.display()))?
    };
    let public = PublicKey::from_secret_key(&secret).map_err(|_| "invalid Minisign signing key")?;
    let signature = minisign::sign(Some(&public), &secret, bytes.as_slice(), None, None)
        .map_err(|_| "could not make the Minisign signature")?;
    into.write_new(&sidecar, &signature.to_bytes())?;
    println!("Wrote {}", into.path().join(sidecar).display());
    Ok(())
}

fn unlock_ci_key(text: &str, password: Option<&str>) -> Result<SecretKey, String> {
    // Actions supplies an empty value for a secret that was not configured.
    match password.filter(|password| !password.is_empty()) {
        Some(password) => SecretKey::from_box(text.to_owned().into(), Some(password.to_owned())),
        None => SecretKey::from_unencrypted_box(text.to_owned().into()),
    }
    .map_err(|_| {
        format!(
            "could not unlock the Minisign key in {KEY_ENV}; check the key and {PASSPHRASE_ENV}"
        )
    })
}

#[cfg(test)]
#[test]
fn an_empty_ci_passphrase_unlocks_a_passwordless_key() {
    let pair = KeyPair::generate_unencrypted_keypair().unwrap();
    let text = Zeroizing::new(pair.sk.to_box(None).unwrap().into_string());
    let key = unlock_ci_key(&text, Some("")).unwrap();
    assert_eq!(
        PublicKey::from_secret_key(&key).unwrap().to_base64(),
        pair.pk.to_base64()
    );
}

// Keep our buffers guarded for their whole life. Neither Minisign's owned arguments and
// internal keys nor the terminal library's internal copies are ours to clear.
fn prompt(message: &str) -> Result<Zeroizing<String>, String> {
    rpassword::prompt_password(message)
        .map(Zeroizing::new)
        .map_err(|e| format!("could not read the passphrase: {e}"))
}

fn secret_env(name: &str) -> Result<Option<Zeroizing<String>>, String> {
    match env::var(name) {
        Ok(text) => Ok(Some(Zeroizing::new(text))),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(text)) => {
            drop(Zeroizing::new(text.into_encoded_bytes()));
            Err(format!("{name} must contain text"))
        }
    }
}
