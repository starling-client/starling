//! Horizon's launcher's account API (launcher/src/account.rs), as Horizon Android's launcher calls
//! it, over Starling's accounts (`crate::accounts`, crate starling-account): the same profiles the
//! game's own account screen shows, their secrets sealed by the Android Keystore, the sign-in kept
//! in the foreground while the player is in the browser. The game renews the session itself when it
//! starts, so the launcher hands it no session file.
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use starling_account::{Account, Provider, SecretStore, Secrets};

/// The code the player types at Microsoft's page.
#[derive(Clone)]
pub struct DeviceCode {
    pub user_code: String,
    pub verification_uri: String,
}

/// The Microsoft app Starling signs in with.
pub fn client_id() -> Option<String> {
    starling_account::client_id()
}

/// Starling's app is its own, approved for these scopes: no app of the player's to check.
pub fn legacy_client(_client_id: &str) -> bool {
    true
}

/// Where the secrets are: sealed by the Android Keystore.
pub fn store() -> Arc<dyn SecretStore> {
    crate::accounts::store()
}

/// The saved accounts (`accounts.json` in the private folder), as Horizon's launcher lists them.
pub struct Accounts {
    pub accounts: Vec<Account>,
    pub selected: Option<String>,
    saved: starling_account::Accounts,
}

impl Accounts {
    pub fn load() -> Self {
        let saved = starling_account::Accounts::load(&crate::accounts::dir());
        Self {
            accounts: saved.accounts.clone(),
            selected: saved.selected.clone(),
            saved,
        }
    }
    pub fn selected(&self) -> Option<&Account> {
        let key = self.selected.as_ref()?;
        self.accounts.iter().find(|a| &a.key() == key)
    }
    /// A changed selection goes through Starling's, which plays as it (and leaves an offline
    /// profile when none is chosen).
    pub fn save(&self) -> Result<(), String> {
        if self.selected != self.saved.selected {
            match &self.selected {
                Some(key) => crate::accounts::select(key.clone()),
                None => {
                    let mut offline = self.saved.clone();
                    offline.selected = None;
                    offline.save(&crate::accounts::dir())?;
                }
            }
        }
        Ok(())
    }
    pub fn remove_when_idle(
        &mut self,
        _store: Arc<dyn SecretStore>,
        key: &str,
    ) -> Result<(), String> {
        crate::accounts::remove(key.to_owned());
        self.accounts.retain(|a| a.key() != key);
        if self.selected.as_deref() == Some(key) {
            self.selected = None;
        }
        self.saved.selected = self.selected.clone();
        Ok(())
    }
}

/// Where a sign-in is, in the states Horizon's launcher shows.
#[derive(Clone)]
pub enum SignInState {
    Starting,
    Code(DeviceCode),
    Done(Result<Account, String>),
}

/// Starling's sign-in in progress (`crate::accounts::sign_in`): one at a time, for the game and
/// the launcher alike.
pub struct SignIn;

impl SignIn {
    pub fn start_with_client_id(_client_id: Option<String>) -> Self {
        crate::accounts::sign_in();
        Self
    }
    pub fn start_ely(username: String, password: String, otp: String) -> Self {
        crate::accounts::sign_in_ely(username, password, otp);
        Self
    }
    pub fn state(&self) -> SignInState {
        use crate::accounts::State;
        match crate::accounts::state() {
            State::Code { code, link } => SignInState::Code(DeviceCode {
                user_code: code,
                verification_uri: link,
            }),
            State::SignedIn(account) => SignInState::Done(Ok(account)),
            State::OfflineProfile(name) => {
                SignInState::Done(Err(format!("Playing offline as {name}")))
            }
            State::Failed(reason) => SignInState::Done(Err(crate::lang::get(&reason))),
            State::Offline => SignInState::Done(Err("Signed out".into())),
            State::Starting | State::Resuming | State::Progress(_) => SignInState::Starting,
        }
    }
    pub fn cancel(&self) {
        crate::accounts::cancel();
    }
}

pub mod session {
    use super::{Account, Secrets};
    /// The selected account with a token good for playing, renewed if needed.
    pub fn selected_session(_client_id: &str) -> Result<Option<(Account, Secrets)>, String> {
        starling_account::session(
            &starling_account::Endpoints::live(),
            super::store().as_ref(),
            &crate::accounts::dir(),
        )
    }
}

/// Starling's game renews and keeps its session itself (`accounts::resume`): no file to hand it.
pub fn write_session_file(
    _dir: &Path,
    _account: &Account,
    _secrets: &Secrets,
) -> Result<PathBuf, String> {
    Ok(PathBuf::new())
}

/// A request to the account's services, as Horizon's launcher makes them.
pub fn request_to(method: &str, url: &str) -> ureq::Request {
    crate::ha_launcher::proxy::agent(
        ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(20))
            .user_agent(&format!(
                "starling/{}",
                crate::ha_launcher::backend::VERSION
            )),
        url,
    )
    .build()
    .request(method, url)
}
