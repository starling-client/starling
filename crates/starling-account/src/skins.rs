// Copyright (c) 2026 Ludovic Marie and the Horizon contributors.
// SPDX-License-Identifier: MIT
// Adapted from Vasysik/sunrise-android (`launcher/src/skins/cloud.rs`); see THIRD_PARTY_NOTICES.md.
//! A profile's skin on its account's service. A Microsoft account's goes to its Minecraft profile
//! (`POST /minecraft/profile/skins`: the PNG and its arms), which servers then hand to the other
//! players. Ely.by's public API takes no uploads: an Ely.by account changes its skin on the
//! website ([`ELY_SKINS`]), where the game looks it up by name.
use super::*;

/// Where an Ely.by account changes its skin.
pub const ELY_SKINS: &str = "https://ely.by/skins";
/// The largest PNG sent, as Horizon Android's import takes.
pub const MAX_PNG: usize = 1024 * 1024;
const BOUNDARY: &str = "starling-skin-upload-v1";

/// The multipart form the Minecraft services take: the arms (`variant`), then the PNG.
fn form(png: &[u8], slim: bool) -> Vec<u8> {
    let variant = if slim { "slim" } else { "classic" };
    let mut body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"variant\"\r\n\r\n{variant}\r\n\
         --{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"skin.png\"\r\n\
         Content-Type: image/png\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(png);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    body
}

/// Make `png` (a 64x64 or legacy 64x32 skin) the skin of the profile `key` on its Microsoft
/// account, with slim or classic arms. `key` is the profile the player chose the skin for: when
/// another one plays by now, nothing is sent. The profile Mojang has afterwards (the new skin's
/// URL) is kept in `accounts.json` and returned.
pub fn upload_skin(
    ep: &Endpoints,
    store: &dyn SecretStore,
    dir: &Path,
    key: &str,
    png: &[u8],
    slim: bool,
) -> Result<Account, String> {
    if png.len() > MAX_PNG || !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("starling.skin.error.not_png".into());
    }
    let (account, secrets) = session(ep, store, dir)?.ok_or("starling.skin.error.signed_out")?;
    if account.key() != key {
        return Err("starling.account.error.cancelled".into());
    }
    if account.provider != Provider::Microsoft {
        return Err("starling.skin.error.ely_website".into());
    }
    let request = bearer(
        agent().post(&format!("{}/minecraft/profile/skins", ep.minecraft)),
        secrets.access_token(),
    )
    .set(
        "Content-Type",
        &format!("multipart/form-data; boundary={BOUNDARY}"),
    );
    match request.send_bytes(&form(png, slim)) {
        Ok(_) => {}
        Err(ureq::Error::Status(400, _)) => return Err("starling.skin.error.refused".into()),
        Err(ureq::Error::Status(401 | 403, _)) => {
            return Err("starling.account.error.signin_again".into());
        }
        Err(ureq::Error::Status(429, _)) => return Err("starling.skin.error.busy".into()),
        Err(ureq::Error::Status(..)) => return Err("starling.skin.error.service".into()),
        Err(ureq::Error::Transport(t)) => {
            return Err(format!(
                "{UNREACHABLE}the Minecraft services ({}). Check your internet connection.",
                transport_detail(&t)
            ));
        }
    }
    let renewed = profile(ep, secrets.access_token())?;
    if renewed.uuid != account.uuid {
        return Err("starling.account.error.cancelled".into());
    }
    let _lock = account_lock();
    let mut list = Accounts::load(dir);
    if let Some(kept) = list.accounts.iter_mut().find(|a| a.key() == key) {
        *kept = renewed.clone();
    }
    list.save(dir)?;
    Ok(renewed)
}

/// Keep the cape reported by Minecraft Services next to the rest of the account profile.
pub fn remember_cape(dir: &Path, key: &str, cape: Option<String>) -> Result<(), String> {
    let _lock = account_lock();
    let mut accounts = Accounts::load(dir);
    if let Some(account) = accounts
        .accounts
        .iter_mut()
        .find(|account| account.provider == Provider::Microsoft && account.key() == key)
        && account.cape != cape
    {
        account.cape = cape;
        accounts.save(dir)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};

    const UUID: &str = "069a79f444e94726a5befca90e38aaf5";

    /// What the mock profile service was asked: method and path, the Authorization header and
    /// the body.
    type Seen = Arc<Mutex<Vec<(String, String, Vec<u8>)>>>;

    /// Answers `answers` (status, JSON) in turn, one request per connection.
    fn serve(answers: Vec<(u16, Value)>) -> (Endpoints, Seen) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen: Seen = Arc::default();
        let log = seen.clone();
        std::thread::spawn(move || {
            for (status, body) in answers {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let (mut length, mut auth) = (0, String::new());
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    let header = header.trim_end();
                    if header.is_empty() {
                        break;
                    }
                    let (k, v) = header.split_once(':').unwrap();
                    match k.to_ascii_lowercase().as_str() {
                        "content-length" => length = v.trim().parse().unwrap(),
                        "authorization" => auth = v.trim().to_owned(),
                        _ => {}
                    }
                }
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                let request = line
                    .split_whitespace()
                    .take(2)
                    .collect::<Vec<_>>()
                    .join(" ");
                log.lock().unwrap().push((request, auth, bytes));
                let text = body.to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
            }
        });
        let ep = Endpoints {
            microsoft: format!("{base}/ms"),
            xbox_user: format!("{base}/xbl"),
            xsts: format!("{base}/xsts"),
            minecraft: format!("{base}/mc"),
        };
        (ep, seen)
    }

    struct Scratch(PathBuf);
    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "starling-skin-upload-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A signed-in account of `provider` whose token has a day left (no renewal on the way).
    fn signed_in(dir: &Path, provider: Provider) -> (MemoryStore, Account) {
        let account = Account {
            provider,
            uuid: UUID.into(),
            name: "Notch".into(),
            skin: Some((
                "http://textures.minecraft.net/texture/old".into(),
                "classic".into(),
            )),
            cape: None,
        };
        let secrets = Secrets {
            provider,
            refresh_token: "refresh".into(),
            minecraft_token: "mc-token".into(),
            minecraft_expires_at: now() + 86400,
            xuid: None,
        };
        let store = MemoryStore::default();
        let mut list = Accounts::load(dir);
        list.add(&store, account.clone(), &secrets).unwrap();
        list.save(dir).unwrap();
        (store, account)
    }

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR pixels";

    #[test]
    fn a_microsoft_profile_gets_the_skin_with_its_arms_and_keeps_the_new_one() {
        let scratch = Scratch::new("microsoft");
        let (store, account) = signed_in(&scratch.0, Provider::Microsoft);
        let profile = json!({
            "id": UUID, "name": "Notch",
            "skins": [{"id": "s2", "state": "ACTIVE", "url": "http://textures.minecraft.net/texture/new", "variant": "SLIM"}],
            "capes": [],
        });
        let (ep, seen) = serve(vec![(200, profile.clone()), (200, profile)]);
        let kept = upload_skin(&ep, &store, &scratch.0, &account.key(), PNG, true).unwrap();
        let skin = Some((
            "http://textures.minecraft.net/texture/new".to_owned(),
            "slim".to_owned(),
        ));
        assert_eq!(kept.skin, skin);
        assert_eq!(Accounts::load(&scratch.0).selected().unwrap().skin, skin);
        let seen = seen.lock().unwrap();
        let (request, auth, body) = &seen[0];
        assert_eq!(request, "POST /mc/minecraft/profile/skins");
        assert_eq!(auth, "Bearer mc-token");
        let text = String::from_utf8_lossy(body);
        assert!(text.contains("name=\"variant\"\r\n\r\nslim\r\n"), "{text}");
        assert!(body.windows(PNG.len()).any(|w| w == PNG));
        assert!(text.ends_with(&format!("\r\n--{BOUNDARY}--\r\n")));
        assert_eq!(seen[1].0, "GET /mc/minecraft/profile");
    }

    #[test]
    fn nothing_is_sent_for_ely_another_profile_or_a_file_that_is_no_png() {
        let scratch = Scratch::new("refused");
        // No server: any request would fail as unreachable rather than as these.
        let (ep, seen) = serve(vec![]);
        let (store, account) = signed_in(&scratch.0, Provider::Ely);
        assert_eq!(
            upload_skin(&ep, &store, &scratch.0, &account.key(), PNG, false).unwrap_err(),
            "starling.skin.error.ely_website"
        );
        let (store, account) = signed_in(&scratch.0, Provider::Microsoft);
        assert_eq!(
            upload_skin(&ep, &store, &scratch.0, "offline:Notch", PNG, false).unwrap_err(),
            "starling.account.error.cancelled"
        );
        assert_eq!(
            upload_skin(&ep, &store, &scratch.0, &account.key(), b"GIF89a", false).unwrap_err(),
            "starling.skin.error.not_png"
        );
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn a_refused_skin_says_so_and_keeps_the_old_one() {
        let scratch = Scratch::new("bad");
        let (store, account) = signed_in(&scratch.0, Provider::Microsoft);
        let (ep, _) = serve(vec![(400, json!({"errorMessage": "Invalid skin"}))]);
        assert_eq!(
            upload_skin(&ep, &store, &scratch.0, &account.key(), PNG, false).unwrap_err(),
            "starling.skin.error.refused"
        );
        assert_eq!(
            Accounts::load(&scratch.0).selected().unwrap().skin,
            account.skin
        );
    }
}
