// Copyright (c) 2026 Ludovic Marie and the Horizon contributors.
// SPDX-License-Identifier: MIT
//! Provider-aware selection, adapted from Horizon Android's launcher account model.
use super::*;

/// A failed refresh never replaces the profile that was playing before it.
pub fn activate_profile(
    ep: &Endpoints,
    store: &dyn SecretStore,
    dir: &Path,
    key: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), String> {
    let _lock = account_lock();
    if cancelled() {
        return Err("starling.account.error.cancelled".into());
    }
    let mut list = Accounts::load(dir);
    let player = list
        .players()
        .into_iter()
        .find(|p| p.key() == key)
        .ok_or("starling.account.error.missing")?;
    match player {
        Player::Offline(name) => {
            list.selected = None;
            list.offline_selected = Some(name);
        }
        Player::Account(account) => {
            let secrets: Secrets = store
                .get(&account.key())?
                .and_then(|s| serde_json::from_str(&s).ok())
                .ok_or("starling.account.error.signin_again")?;
            let id = if account.provider == Provider::Microsoft {
                client_id().ok_or("Microsoft sign-in has no app configured.")?
            } else {
                String::new()
            };
            let (account, secrets) = fresh(ep, &id, &account, secrets)?;
            if cancelled() {
                return Err("starling.account.error.cancelled".into());
            }
            list.add(store, account, &secrets)?;
        }
    }
    list.save(dir)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Player {
    Account(Account),
    Offline(String),
}
impl Player {
    pub fn key(&self) -> String {
        match self {
            Self::Account(a) => a.key(),
            Self::Offline(n) => format!("offline:{n}"),
        }
    }
    pub fn name(&self) -> &str {
        match self {
            Self::Account(a) => &a.name,
            Self::Offline(n) => n,
        }
    }
}
impl Accounts {
    pub fn players(&self) -> Vec<Player> {
        self.accounts
            .iter()
            .cloned()
            .map(Player::Account)
            .chain(self.offline.iter().cloned().map(Player::Offline))
            .collect()
    }
    pub fn player(&self) -> Option<Player> {
        self.selected().cloned().map(Player::Account).or_else(|| {
            self.offline_selected
                .as_ref()
                .filter(|n| self.offline.contains(n))
                .cloned()
                .map(Player::Offline)
        })
    }
}
pub fn valid_offline_name(name: &str) -> bool {
    (1..=16).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
pub fn add_offline(dir: &Path, name: &str) -> Result<(), String> {
    add_offline_if(dir, name, &|| false)
}
pub fn add_offline_if(dir: &Path, name: &str, cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    let name = name.trim();
    if !valid_offline_name(name) {
        return Err("starling.account.error.name_error".into());
    }
    let _lock = account_lock();
    if cancelled() {
        return Err("starling.account.error.cancelled".into());
    }
    let mut list = Accounts::load(dir);
    if !list.offline.iter().any(|n| n == name) {
        list.offline.push(name.into());
    }
    list.offline_selected = Some(name.into());
    list.selected = None;
    list.save(dir)
}
pub fn select_profile(dir: &Path, key: &str) -> Result<(), String> {
    select_profile_if(dir, key, &|| false)
}
pub fn select_profile_if(
    dir: &Path,
    key: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), String> {
    let _lock = account_lock();
    if cancelled() {
        return Err("starling.account.error.cancelled".into());
    }
    let mut list = Accounts::load(dir);
    let player = list
        .players()
        .into_iter()
        .find(|p| p.key() == key)
        .ok_or("starling.account.error.missing")?;
    match player {
        Player::Account(a) => {
            list.selected = Some(a.key());
            list.offline_selected = None;
        }
        Player::Offline(n) => {
            list.selected = None;
            list.offline_selected = Some(n);
        }
    }
    list.save(dir)
}
pub fn remove_profile(store: &dyn SecretStore, dir: &Path, key: &str) -> Result<(), String> {
    remove_profile_if(store, dir, key, &|| false)
}
pub fn remove_profile_if(
    store: &dyn SecretStore,
    dir: &Path,
    key: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), String> {
    let _lock = account_lock();
    let mut list = Accounts::load(dir);
    if cancelled() {
        return Err("starling.account.error.cancelled".into());
    }
    if let Some(name) = key.strip_prefix("offline:") {
        list.offline.retain(|n| n != name);
        if list.offline_selected.as_deref() == Some(name) {
            list.offline_selected = None;
        }
    } else {
        list.remove(store, key)?;
    }
    list.save(dir)
}
pub fn keep_ely(
    store: &dyn SecretStore,
    dir: &Path,
    username: &str,
    password: &str,
    otp: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<Account, String> {
    let (account, secrets) = ely::authenticate(username, password, otp)?;
    let _lock = account_lock();
    if cancelled() {
        return Err("starling.account.error.sign_in_cancelled".into());
    }
    let mut list = Accounts::load(dir);
    list.add(store, account.clone(), &secrets)?;
    list.save(dir)?;
    Ok(account)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "starling-profiles-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn entry(provider: Provider) -> (Account, Secrets) {
        (
            Account {
                provider,
                uuid: "0123456789abcdef0123456789abcdef".into(),
                name: "Player".into(),
                skin: None,
                cape: None,
            },
            Secrets {
                provider,
                refresh_token: "private-refresh".into(),
                minecraft_token: format!("private-{provider:?}"),
                minecraft_expires_at: now() + 7200,
                xuid: None,
            },
        )
    }
    #[test]
    fn legacy_microsoft_accounts_and_secrets_survive_offline_switch_and_restart() {
        let dir = Scratch::new();
        let store = MemoryStore::default();
        std::fs::write(dir.0.join("accounts.json"),r#"{"accounts":[{"uuid":"0123456789abcdef0123456789abcdef","name":"Player"}],"selected":"0123456789abcdef0123456789abcdef"}"#).unwrap();
        let mut list = Accounts::load(&dir.0);
        assert_eq!(list.selected().unwrap().provider, Provider::Microsoft);
        let (a, s) = entry(Provider::Microsoft);
        list.add(&store, a.clone(), &s).unwrap();
        list.save(&dir.0).unwrap();
        add_offline(&dir.0, "LocalPlayer").unwrap();
        assert_eq!(
            Accounts::load(&dir.0).player(),
            Some(Player::Offline("LocalPlayer".into()))
        );
        assert!(
            session(&Endpoints::live(), &store, &dir.0)
                .unwrap()
                .is_none()
        );
        assert!(store.get(&a.key()).unwrap().is_some());
        select_profile(&dir.0, &a.key()).unwrap();
        let (active, secret) = session(&Endpoints::live(), &store, &dir.0)
            .unwrap()
            .unwrap();
        assert_eq!(active, a);
        assert_eq!(secret.access_token(), "private-Microsoft");
        let metadata = std::fs::read_to_string(dir.0.join("accounts.json")).unwrap();
        assert!(!metadata.contains("private"));
    }
    #[test]
    fn identical_uuids_in_two_providers_never_share_credentials_or_removal() {
        let dir = Scratch::new();
        let store = MemoryStore::default();
        let mut list = Accounts::default();
        let (ms, msec) = entry(Provider::Microsoft);
        let (ely, esec) = entry(Provider::Ely);
        list.add(&store, ms.clone(), &msec).unwrap();
        list.add(&store, ely.clone(), &esec).unwrap();
        list.save(&dir.0).unwrap();
        assert_eq!(list.accounts.len(), 2);
        assert_ne!(
            store.get(&ms.key()).unwrap(),
            store.get(&ely.key()).unwrap()
        );
        assert_eq!(
            session(&Endpoints::live(), &store, &dir.0)
                .unwrap()
                .unwrap()
                .1
                .access_token(),
            "private-Ely"
        );
        remove_profile(&store, &dir.0, &ely.key()).unwrap();
        let list = Accounts::load(&dir.0);
        assert_eq!(list.accounts, vec![ms.clone()]);
        assert!(store.get(&ms.key()).unwrap().is_some());
        assert!(store.get(&ely.key()).unwrap().is_none());
        let mut list = list;
        assert!(list.add(&store, ms, &esec).is_err());
    }
    #[test]
    fn a_cancelled_selection_waiting_for_storage_cannot_overwrite_a_later_choice() {
        let dir = Scratch::new();
        add_offline(&dir.0, "First").unwrap();
        add_offline(&dir.0, "Second").unwrap();
        let lock = account_lock();
        let path = dir.0.clone();
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let worker = std::thread::spawn(move || {
            select_profile_if(&path, "offline:First", &|| {
                worker_cancel.load(std::sync::atomic::Ordering::Relaxed)
            })
        });
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        drop(lock);
        assert!(worker.join().unwrap().is_err());
        assert_eq!(Accounts::load(&dir.0).player().unwrap().name(), "Second");
    }
    #[test]
    fn invalid_names_and_unknown_profile_keys_leave_selection_unchanged() {
        let dir = Scratch::new();
        add_offline(&dir.0, "Valid_Name").unwrap();
        for n in ["", "../bad", "two words", "очень", "12345678901234567"] {
            assert!(add_offline(&dir.0, n).is_err());
        }
        assert!(select_profile(&dir.0, "missing").is_err());
        assert_eq!(
            Accounts::load(&dir.0).player().unwrap().name(),
            "Valid_Name"
        );
    }
    #[test]
    fn an_unsigned_target_does_not_replace_the_active_offline_profile() {
        let dir = Scratch::new();
        let store = MemoryStore::default();
        add_offline(&dir.0, "LocalPlayer").unwrap();
        let (a, _) = entry(Provider::Ely);
        let mut list = Accounts::load(&dir.0);
        list.accounts.push(a.clone());
        list.save(&dir.0).unwrap();
        assert!(activate_profile(&Endpoints::live(), &store, &dir.0, &a.key(), &|| false).is_err());
        assert_eq!(
            Accounts::load(&dir.0).player().unwrap().name(),
            "LocalPlayer"
        );
    }
}
