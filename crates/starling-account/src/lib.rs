//! Microsoft account sign-in inside the game, for Starling (a phone has no launcher): the OAuth
//! device-code flow, then Xbox Live, XSTS and the Minecraft services, the game ownership check
//! and the profile. It follows Horizon's launcher (`launcher/src/account.rs`), which signs in
//! for the desktop game and hands it a session file; here the game keeps the session itself.
//!
//! The Azure app is Starling's, approved by Mojang for these scopes only: `XboxLive.signin` and
//! `offline_access`, personal accounts (the `consumers` endpoint). Secrets (the Microsoft refresh
//! token, the Minecraft access token) go to a [`SecretStore`] the game provides: on Android, one
//! sealed by the Android Keystore. `accounts.json` keeps only names and UUIDs. Tokens never reach
//! logs, errors or output, and no server of ours is in between.
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub mod ely;
pub mod files;
pub mod java;
mod profiles;
pub mod skins;
pub use profiles::{
    Player, activate_profile, add_offline, add_offline_if, keep_ely, remove_profile,
    remove_profile_if, select_profile, select_profile_if, valid_offline_name,
};

static ACCOUNT_LOCK: Mutex<()> = Mutex::new(());
fn account_lock() -> std::sync::MutexGuard<'static, ()> {
    ACCOUNT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Starling's Azure app (a public client: its ID is no secret).
pub const CLIENT_ID: &str = "f2719b6f-5617-4631-b2d7-a21defa191fc";
/// The only scopes the app was approved for.
const SCOPE: &str = "XboxLive.signin offline_access";
const SIGN_IN_AGAIN: &str = "Your Microsoft sign-in expired. Sign in again.";
/// The Minecraft token is renewed when it has less than this left.
const RENEW_BEFORE: u64 = 3600;

/// The Azure app: `STARLING_MSA_CLIENT_ID` overrides Starling's own (a test app); empty means
/// none.
pub fn client_id() -> Option<String> {
    match std::env::var("STARLING_MSA_CLIENT_ID") {
        Ok(id) => Some(id.trim().to_owned()).filter(|id| !id.is_empty()),
        Err(_) => Some(CLIENT_ID.to_owned()),
    }
}

/// Where the requests go: the real services, or a mock in tests.
#[derive(Clone, Debug)]
pub struct Endpoints {
    /// The OAuth 2.0 v2 endpoints for personal accounts (`…/devicecode`, `…/token`).
    pub microsoft: String,
    pub xbox_user: String,
    pub xsts: String,
    /// api.minecraftservices.com.
    pub minecraft: String,
}

impl Endpoints {
    pub fn live() -> Self {
        Self {
            microsoft: "https://login.microsoftonline.com/consumers/oauth2/v2.0".into(),
            xbox_user: "https://user.auth.xboxlive.com/user/authenticate".into(),
            xsts: "https://xsts.auth.xboxlive.com/xsts/authorize".into(),
            minecraft: "https://api.minecraftservices.com".into(),
        }
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .user_agent(&format!("starling/{}", env!("CARGO_PKG_VERSION")))
        .build()
}

/// What went wrong on the way, short, for a player's report (`dns failed: ...`, a refused or
/// timed-out connection, a certificate): the transport's kind and message, never its URL.
pub(crate) fn transport_detail(t: &ureq::Transport) -> String {
    let mut detail = t.kind().to_string().to_lowercase();
    if let Some(message) = t.message() {
        detail.push_str(": ");
        detail.push_str(message);
    }
    // The cause underneath (the system's own words: refused, unreachable, a certificate).
    if let Some(cause) = std::error::Error::source(t) {
        detail.push_str(": ");
        detail.push_str(&cause.to_string());
    }
    if detail.chars().count() > 120 {
        detail = detail.chars().take(117).collect::<String>() + "...";
    }
    detail
}

/// How `send` starts an error of the network itself (no answer at all), as against a refusal.
const UNREACHABLE: &str = "Could not reach ";

/// Whether `error` (from `send`) is the network failing, worth asking again: a phone cuts an
/// app's network in the background (battery and data savers, a maker's own restrictions) while
/// the player signs in in the browser, and the lookup fails then (`dns failed: ... No address
/// associated with hostname`).
fn unreachable(error: &str) -> bool {
    error.starts_with(UNREACHABLE)
}

/// Between two tries of a step the network failed for.
const RETRY_WAIT: Duration = if cfg!(test) {
    Duration::from_millis(100)
} else {
    Duration::from_secs(3)
};

/// How long the steps after Microsoft's tokens are tried again while the network fails.
const RETRY_FOR: Duration = Duration::from_secs(120);

/// `step` again, every `RETRY_WAIT`, while the network fails, for up to `RETRY_FOR`; any other
/// error at once. `cancelled` stops the waiting.
fn while_unreachable<T>(
    mut step: impl FnMut() -> Result<T, String>,
    cancelled: &dyn Fn() -> bool,
) -> Result<T, String> {
    let give_up = std::time::Instant::now() + RETRY_FOR;
    loop {
        match step() {
            Err(e) if unreachable(&e) && std::time::Instant::now() < give_up => {
                let until = std::time::Instant::now() + RETRY_WAIT;
                while std::time::Instant::now() < until {
                    if cancelled() {
                        return Err("Sign-in cancelled.".into());
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            result => return result,
        }
    }
}

/// One request's result: the status and the JSON body (empty when there is none). Transport
/// errors name the service and what went wrong, never the URL's query or the request.
fn send(request: ureq::Request, body: Option<Body>, service: &str) -> Result<(u16, Value), String> {
    let result = match body {
        None => request.call(),
        Some(Body::Form(fields)) => request.send_form(&fields),
        Some(Body::Json(value)) => request.set("Accept", "application/json").send_json(value),
    };
    let response = match result {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(ureq::Error::Transport(t)) => {
            return Err(format!(
                "{UNREACHABLE}{service} ({}). Check your internet connection.",
                transport_detail(&t)
            ));
        }
    };
    let status = response.status();
    let text = response.into_string().unwrap_or_default();
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

enum Body<'a> {
    Form(Vec<(&'a str, &'a str)>),
    Json(Value),
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

// ----- The device-code flow ----------------------------------------------------------------------

/// What the player types at the verification page.
#[derive(Clone)]
pub struct DeviceCode {
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u64,
    pub expires_at: u64,
    device_code: String,
}

impl std::fmt::Debug for DeviceCode {
    // The device code can be traded for tokens once the player approves: never shown.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceCode")
            .field("user_code", &self.user_code)
            .field("verification_uri", &self.verification_uri)
            .finish_non_exhaustive()
    }
}

/// The Microsoft tokens; the access token only lasts for the Xbox step.
#[derive(Clone)]
pub struct MsTokens {
    access: String,
    refresh: String,
}

impl std::fmt::Debug for MsTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MsTokens(<hidden>)")
    }
}

pub enum Poll {
    Pending,
    /// The server asked to poll less often: wait 5 s more.
    SlowDown,
    Done(MsTokens),
}

pub fn device_code(ep: &Endpoints, client_id: &str) -> Result<DeviceCode, String> {
    let (status, v) = send(
        agent().post(&format!("{}/devicecode", ep.microsoft)),
        Some(Body::Form(vec![("client_id", client_id), ("scope", SCOPE)])),
        "Microsoft",
    )?;
    match (
        status,
        str_field(&v, "user_code"),
        str_field(&v, "device_code"),
    ) {
        (200, Some(user_code), Some(device_code)) => Ok(DeviceCode {
            user_code,
            verification_uri: str_field(&v, "verification_uri")
                .unwrap_or_else(|| "https://www.microsoft.com/link".into()),
            interval: v
                .get("interval")
                .and_then(Value::as_u64)
                .unwrap_or(5)
                .max(1),
            expires_at: now() + v.get("expires_in").and_then(Value::as_u64).unwrap_or(900),
            device_code,
        }),
        _ => Err(oauth_error(&v)),
    }
}

/// The OAuth error codes as messages for the player.
fn oauth_error(v: &Value) -> String {
    match v.get("error").and_then(Value::as_str) {
        Some("authorization_declined") => "Sign-in was declined in the browser.".into(),
        Some("expired_token") => "The sign-in code expired. Start again.".into(),
        Some("bad_verification_code") => "The sign-in code was not recognised. Start again.".into(),
        Some("invalid_grant") => SIGN_IN_AGAIN.into(),
        Some("invalid_client" | "unauthorized_client") => {
            "Microsoft refused Starling's app registration (client ID).".into()
        }
        Some(other) => format!("Microsoft sign-in failed ({other})."),
        None => "Microsoft sign-in failed.".into(),
    }
}

/// One poll of the token endpoint while the player signs in.
pub fn poll(ep: &Endpoints, client_id: &str, code: &DeviceCode) -> Result<Poll, String> {
    let (status, v) = send(
        agent().post(&format!("{}/token", ep.microsoft)),
        Some(Body::Form(vec![
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("client_id", client_id),
            ("device_code", &code.device_code),
        ])),
        "Microsoft",
    )?;
    match v.get("error").and_then(Value::as_str) {
        Some("authorization_pending") => return Ok(Poll::Pending),
        Some("slow_down") => return Ok(Poll::SlowDown),
        _ => {}
    }
    ms_tokens(status, &v).map(Poll::Done)
}

fn ms_tokens(status: u16, v: &Value) -> Result<MsTokens, String> {
    match (
        status,
        str_field(v, "access_token"),
        str_field(v, "refresh_token"),
    ) {
        (200, Some(access), Some(refresh)) => Ok(MsTokens { access, refresh }),
        _ => Err(oauth_error(v)),
    }
}

/// Wait for the player to finish in the browser, polling as asked, until the code expires or
/// `cancelled` says stop. A poll the network failed for is asked again at the next interval: the
/// app waits in the background meanwhile, where a phone may cut its network, and the poll after
/// the player comes back gets the tokens.
pub fn wait_for_sign_in(
    ep: &Endpoints,
    client_id: &str,
    code: &DeviceCode,
    cancelled: &dyn Fn() -> bool,
) -> Result<MsTokens, String> {
    let mut interval = code.interval;
    let mut lost = None;
    loop {
        let until = std::time::Instant::now() + Duration::from_secs(interval);
        while std::time::Instant::now() < until {
            if cancelled() {
                return Err("Sign-in cancelled.".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if now() > code.expires_at {
            return Err(lost.unwrap_or_else(|| "The sign-in code expired. Start again.".into()));
        }
        match poll(ep, client_id, code) {
            Ok(Poll::Pending) => {}
            Ok(Poll::SlowDown) => interval += 5,
            Ok(Poll::Done(tokens)) => return Ok(tokens),
            Err(e) if unreachable(&e) => lost = Some(e),
            Err(e) => return Err(e),
        }
    }
}

fn refresh_ms(ep: &Endpoints, client_id: &str, refresh: &str) -> Result<MsTokens, String> {
    let (status, v) = send(
        agent().post(&format!("{}/token", ep.microsoft)),
        Some(Body::Form(vec![
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", refresh),
            ("scope", SCOPE),
        ])),
        "Microsoft",
    )?;
    ms_tokens(status, &v)
}

// ----- Xbox Live, XSTS, Minecraft ----------------------------------------------------------------

/// The XSTS `XErr` codes, as the official launcher explains them.
fn xsts_error(code: u64) -> String {
    match code {
        2148916233 => "This Microsoft account has no Xbox profile yet. Sign in once at xbox.com to create one, then try again.".into(),
        2148916235 => "Xbox Live is not available in your country or region.".into(),
        2148916236 | 2148916237 => "This account needs adult verification on xbox.com before it can play.".into(),
        2148916238 => "This is a child account: an adult has to add it to a Microsoft family first.".into(),
        other => format!("Xbox Live refused the sign-in (error {other})."),
    }
}

/// The Minecraft access token from the Microsoft one, through Xbox Live and XSTS.
struct Minecraft {
    token: String,
    expires_at: u64,
    xuid: Option<String>,
}

fn minecraft_login_progress(
    ep: &Endpoints,
    ms_access: &str,
    progress: &dyn Fn(SignInStage),
) -> Result<Minecraft, String> {
    progress(SignInStage::Xbox);
    let (status, xbl) = send(
        agent().post(&ep.xbox_user),
        Some(Body::Json(json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": format!("d={ms_access}"),
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT",
        }))),
        "Xbox Live",
    )?;
    let xbl_token = str_field(&xbl, "Token")
        .filter(|_| status == 200)
        .ok_or("Xbox Live refused the sign-in.")?;
    progress(SignInStage::XboxSecurity);
    let (status, xsts) = send(
        agent().post(&ep.xsts),
        Some(Body::Json(json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl_token] },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT",
        }))),
        "Xbox Live",
    )?;
    if let Some(code) = xsts.get("XErr").and_then(Value::as_u64) {
        return Err(xsts_error(code));
    }
    let claims = xsts.pointer("/DisplayClaims/xui/0");
    let (Some(xsts_token), Some(uhs)) = (
        str_field(&xsts, "Token").filter(|_| status == 200),
        claims.and_then(|c| str_field(c, "uhs")),
    ) else {
        return Err("Xbox Live refused the sign-in.".into());
    };
    let xuid = claims.and_then(|c| str_field(c, "xid"));
    progress(SignInStage::Minecraft);
    let (status, mc) = send(
        agent().post(&format!("{}/authentication/login_with_xbox", ep.minecraft)),
        Some(Body::Json(
            json!({ "identityToken": format!("XBL3.0 x={uhs};{xsts_token}") }),
        )),
        "the Minecraft services",
    )?;
    match (status, str_field(&mc, "access_token")) {
        (200, Some(token)) => Ok(Minecraft {
            token,
            expires_at: now()
                + mc.get("expires_in")
                    .and_then(Value::as_u64)
                    .unwrap_or(86400),
            xuid,
        }),
        // An Azure app Mojang has not approved (Minecraft API access) is refused.
        (403, _) => Err(
            "The Minecraft services refused Starling: its app is not approved by Mojang.".into(),
        ),
        (429, _) => Err("Too many sign-ins in a short time. Wait a minute and try again.".into()),
        _ => Err("The Minecraft services refused the sign-in.".into()),
    }
}

fn bearer(request: ureq::Request, token: &str) -> ureq::Request {
    request.set("Authorization", &format!("Bearer {token}"))
}

/// Whether the account owns Minecraft: Java Edition (Game Pass included).
fn owns_game(ep: &Endpoints, token: &str) -> Result<bool, String> {
    let (status, v) = send(
        bearer(
            agent().get(&format!("{}/entitlements/mcstore", ep.minecraft)),
            token,
        ),
        None,
        "the Minecraft services",
    )?;
    if status != 200 {
        return Err("Could not check which games this account owns.".into());
    }
    Ok(v.get("items")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|i| {
                matches!(
                    i.get("name").and_then(Value::as_str),
                    Some("game_minecraft" | "product_minecraft")
                )
            })
        }))
}

/// Provider is persisted explicitly; legacy account files default to Microsoft.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    Microsoft,
    Ely,
}

/// A Minecraft profile, as the game shows and remembers it (no secrets).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Account {
    #[serde(default)]
    pub provider: Provider,
    /// The profile UUID without dashes.
    pub uuid: String,
    pub name: String,
    /// The active skin's URL (textures.minecraft.net) and model ("classic" or "slim").
    #[serde(default)]
    pub skin: Option<(String, String)>,
    #[serde(default)]
    pub cape: Option<String>,
}

impl Account {
    pub fn key(&self) -> String {
        match self.provider {
            Provider::Microsoft => self.uuid.clone(),
            Provider::Ely => format!("ely{}", self.uuid),
        }
    }
    /// The UUID as a number, as the game keeps a player's.
    pub fn uuid_u128(&self) -> Option<u128> {
        u128::from_str_radix(&self.uuid, 16).ok()
    }
}

fn profile(ep: &Endpoints, token: &str) -> Result<Account, String> {
    let (status, v) = send(
        bearer(
            agent().get(&format!("{}/minecraft/profile", ep.minecraft)),
            token,
        ),
        None,
        "the Minecraft services",
    )?;
    if status == 404 {
        return Err("This account has no Minecraft profile yet. Create one at minecraft.net, then sign in again.".into());
    }
    let (Some(uuid), Some(name)) = (str_field(&v, "id"), str_field(&v, "name")) else {
        return Err("Could not read the Minecraft profile.".into());
    };
    let active = |key: &str| {
        v.get(key)
            .and_then(Value::as_array)
            .and_then(|list| {
                list.iter()
                    .find(|s| s.get("state").and_then(Value::as_str) == Some("ACTIVE"))
            })
            .cloned()
    };
    Ok(Account {
        provider: Provider::Microsoft,
        uuid: uuid.replace('-', "").to_lowercase(),
        name,
        skin: active("skins").and_then(|s| {
            Some((
                str_field(&s, "url")?,
                str_field(&s, "variant")
                    .unwrap_or_else(|| "CLASSIC".into())
                    .to_lowercase(),
            ))
        }),
        cape: active("capes").and_then(|c| str_field(&c, "url")),
    })
}

/// What the secret store keeps for one account.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Secrets {
    #[serde(default)]
    provider: Provider,
    refresh_token: String,
    minecraft_token: String,
    minecraft_expires_at: u64,
    #[serde(default)]
    xuid: Option<String>,
}
impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secrets(<hidden>)")
    }
}
impl Secrets {
    /// The Minecraft access token, for the game's session in memory (joining online-mode
    /// servers): never to be logged or written out.
    pub fn access_token(&self) -> &str {
        &self.minecraft_token
    }
    pub fn xuid(&self) -> Option<&str> {
        self.xuid.as_deref()
    }
    /// Whose token it is: Microsoft's or Ely.by's.
    pub fn provider(&self) -> Provider {
        self.provider
    }
    /// Seconds since the epoch at which the Minecraft token stops working.
    pub fn expires_at(&self) -> u64 {
        self.minecraft_expires_at
    }
}

/// From Microsoft tokens to a ready account: Xbox, XSTS, Minecraft, ownership, profile.
pub fn finish_sign_in(ep: &Endpoints, ms: MsTokens) -> Result<(Account, Secrets), String> {
    finish_sign_in_progress(ep, ms, &|_| {})
}
fn finish_sign_in_progress(
    ep: &Endpoints,
    ms: MsTokens,
    progress: &dyn Fn(SignInStage),
) -> Result<(Account, Secrets), String> {
    let mc = minecraft_login_progress(ep, &ms.access, progress)?;
    progress(SignInStage::Ownership);
    if !owns_game(ep, &mc.token)? {
        return Err("This account doesn't own Minecraft: Java Edition.".into());
    }
    progress(SignInStage::Profile);
    let account = profile(ep, &mc.token)?;
    Ok((
        account,
        Secrets {
            provider: Provider::Microsoft,
            refresh_token: ms.refresh,
            minecraft_token: mc.token,
            minecraft_expires_at: mc.expires_at,
            xuid: mc.xuid,
        },
    ))
}

/// A Minecraft token with at least an hour left, renewed through the refresh token if needed.
/// The profile is read again on renewal (the name may have changed).
pub fn fresh(
    ep: &Endpoints,
    client_id: &str,
    account: &Account,
    secrets: Secrets,
) -> Result<(Account, Secrets), String> {
    if account.provider != secrets.provider {
        return Err("starling.account.error.provider_mismatch".into());
    }
    if account.provider == Provider::Ely {
        return ely::fresh(account, secrets);
    }
    if secrets.minecraft_expires_at > now() + RENEW_BEFORE {
        return Ok((account.clone(), secrets));
    }
    let ms = refresh_ms(ep, client_id, &secrets.refresh_token)?;
    let (renewed, secrets) = finish_sign_in(ep, ms)?;
    if renewed.uuid != account.uuid {
        return Err(
            "The Microsoft account now has another Minecraft profile. Sign in again.".into(),
        );
    }
    Ok((renewed, secrets))
}

// ----- Secrets -------------------------------------------------------------------------------------

/// Where secrets are kept: sealed by the Android Keystore on a phone, memory in tests.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    fn set(&self, key: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
    /// False for memory: secrets are gone when the game closes.
    fn persistent(&self) -> bool;
}

#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);
impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        self.0.lock().unwrap().insert(key.into(), secret.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
    fn persistent(&self) -> bool {
        false
    }
}

// ----- Accounts ------------------------------------------------------------------------------------

/// `accounts.json` in the game's private folder: the signed-in accounts and the one that plays.
/// Never a token.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Accounts {
    pub accounts: Vec<Account>,
    /// The UUID of the account the game plays as; none plays offline.
    pub selected: Option<String>,
    #[serde(default)]
    pub offline: Vec<String>,
    #[serde(default)]
    pub offline_selected: Option<String>,
}

impl Accounts {
    fn path(dir: &Path) -> PathBuf {
        dir.join("accounts.json")
    }
    pub fn load(dir: &Path) -> Self {
        std::fs::read_to_string(Self::path(dir))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }
    pub fn save(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let part = dir.join("accounts.json.part");
        std::fs::write(&part, text + "\n").map_err(|e| e.to_string())?;
        std::fs::rename(part, Self::path(dir)).map_err(|e| e.to_string())
    }
    pub fn selected(&self) -> Option<&Account> {
        let uuid = self.selected.as_ref()?;
        self.accounts.iter().find(|a| &a.key() == uuid)
    }
    /// Find an account by name (any case) or UUID (with or without dashes).
    pub fn find(&self, who: &str) -> Option<&Account> {
        let undashed = who.replace('-', "").to_lowercase();
        self.accounts
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(who) || a.uuid == undashed)
    }
    /// Keep a new or renewed account, its secrets in the store, and select it.
    pub fn add(
        &mut self,
        store: &dyn SecretStore,
        account: Account,
        secrets: &Secrets,
    ) -> Result<(), String> {
        if account.provider != secrets.provider {
            return Err("starling.account.error.provider_mismatch".into());
        }
        store.set(
            &account.key(),
            &serde_json::to_string(secrets).map_err(|e| e.to_string())?,
        )?;
        self.selected = Some(account.key());
        self.offline_selected = None;
        match self.accounts.iter_mut().find(|a| a.key() == account.key()) {
            Some(a) => *a = account,
            None => self.accounts.push(account),
        }
        Ok(())
    }
    /// Sign out: the secrets leave the store, the account the list.
    pub fn remove(&mut self, store: &dyn SecretStore, uuid: &str) -> Result<(), String> {
        store.delete(uuid)?;
        self.accounts.retain(|a| a.key() != uuid);
        if self.selected.as_deref() == Some(uuid) {
            self.selected = None;
        }
        Ok(())
    }
}

/// The selected account with a token good for playing, renewed (and saved) if needed: what the
/// game asks for when it starts, and again before joining a server.
pub fn session(
    ep: &Endpoints,
    store: &dyn SecretStore,
    dir: &Path,
) -> Result<Option<(Account, Secrets)>, String> {
    let _lock = account_lock();
    let mut accounts = Accounts::load(dir);
    let Some(account) = accounts.selected().cloned() else {
        return Ok(None);
    };
    let secrets: Secrets = store
        .get(&account.key())?
        .and_then(|s| serde_json::from_str(&s).ok())
        .ok_or_else(|| {
            format!(
                "{} is no longer signed in on this device. Sign in again.",
                account.name
            )
        })?;
    let client_id = if account.provider == Provider::Microsoft {
        client_id().ok_or("Microsoft sign-in has no app configured.")?
    } else {
        String::new()
    };
    let before = secrets.minecraft_token.clone();
    let (renewed, secrets) = fresh(ep, &client_id, &account, secrets)?;
    if renewed != account || secrets.minecraft_token != before {
        accounts.add(store, renewed.clone(), &secrets)?;
        accounts.save(dir)?;
    }
    Ok(Some((renewed, secrets)))
}

/// Sign out the selected account (its secrets leave the store): the game plays offline.
pub fn sign_out(store: &dyn SecretStore, dir: &Path) -> Result<(), String> {
    let _lock = account_lock();
    let mut accounts = Accounts::load(dir);
    if let Some(uuid) = accounts.selected.clone() {
        accounts.remove(store, &uuid)?;
        accounts.save(dir)?;
    }
    Ok(())
}

/// Network stages after the browser authorizes Microsoft sign-in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignInStage {
    Xbox,
    XboxSecurity,
    Minecraft,
    Ownership,
    Profile,
    Saving,
}
impl SignInStage {
    pub fn key(self) -> &'static str {
        match self {
            Self::Xbox => "starling.account.progress.xbox",
            Self::XboxSecurity => "starling.account.progress.xbox_security",
            Self::Minecraft => "starling.account.progress.minecraft",
            Self::Ownership => "starling.account.progress.ownership",
            Self::Profile => "starling.account.progress.profile",
            Self::Saving => "starling.account.progress.saving",
        }
    }
}

/// Where a background sign-in is (the game's account screen shows it).
#[derive(Clone, Debug)]
pub enum SignInState {
    Starting,
    Progress(SignInStage),
    /// Waiting for the player at `verification_uri` with `user_code`.
    Code(DeviceCode),
    Done(Result<Account, String>),
}

/// A sign-in on its own thread, from the code to the saved account.
pub struct SignIn {
    state: Arc<Mutex<SignInState>>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}

impl SignIn {
    /// Signs in against `ep`, keeping the account in `dir` and its secrets in `store`.
    pub fn start(ep: Endpoints, store: Arc<dyn SecretStore>, dir: PathBuf) -> Self {
        Self::start_cancellable(
            ep,
            store,
            dir,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
    }
    pub fn start_cancellable(
        ep: Endpoints,
        store: Arc<dyn SecretStore>,
        dir: PathBuf,
        cancel: Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        use std::sync::atomic::Ordering;
        let state = Arc::new(Mutex::new(SignInState::Starting));
        let (thread_state, thread_cancel) = (state.clone(), cancel.clone());
        std::thread::spawn(move || {
            let set = |s| *thread_state.lock().unwrap() = s;
            let result = (|| {
                let client_id = client_id().ok_or("Microsoft sign-in has no app configured.")?;
                let code = device_code(&ep, &client_id)?;
                set(SignInState::Code(code.clone()));
                let cancelled = || thread_cancel.load(Ordering::Relaxed);
                let ms = wait_for_sign_in(&ep, &client_id, &code, &cancelled)?;
                // The player may still be in the browser: Xbox, XSTS and Minecraft too are asked
                // again while the network fails.
                let (account, secrets) = while_unreachable(
                    || {
                        finish_sign_in_progress(&ep, ms.clone(), &|stage| {
                            set(SignInState::Progress(stage))
                        })
                    },
                    &cancelled,
                )?;
                set(SignInState::Progress(SignInStage::Saving));
                let _lock = account_lock();
                if cancelled() {
                    return Err("starling.account.error.sign_in_cancelled".into());
                }
                let mut accounts = Accounts::load(&dir);
                accounts.add(store.as_ref(), account.clone(), &secrets)?;
                accounts.save(&dir)?;
                Ok(account)
            })();
            set(SignInState::Done(result));
        });
        Self { state, cancel }
    }
    pub fn state(&self) -> SignInState {
        self.state.lock().unwrap().clone()
    }
    pub fn cancel(&self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests;
