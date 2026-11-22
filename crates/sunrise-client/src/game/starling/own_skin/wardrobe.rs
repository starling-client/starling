//! Android's in-game character customizer.
use super::{Game, Page, Worn};
use crate::ha_launcher::{
    account::{Provider, session},
    documents,
    skins::{
        self, cloud,
        library::{Kind, Library},
    },
};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use sunrise_client::{
    hud::HudBatch,
    options_ui::AccountCape,
    own_skin::{CapeChoice, Cloud, SkinChoice, Upload},
};

const LOCAL_CAPE: &str = "capes/starling-local";
const ACCOUNT_CAPE: &str = "capes/starling-account";
struct AccountCapes {
    rows: Vec<AccountCape>,
    equipped_image: Option<Vec<u8>>,
    activated: bool,
}

#[derive(Default)]
pub(super) struct Wardrobe {
    open: bool,
    picking: bool,
    profile: Option<Option<String>>,
    cape: CapeChoice,
    cape_image: Option<Vec<u8>>,
    uploaded: bool,
    account_worker: Option<(Option<String>, Receiver<Result<AccountCapes, String>>)>,
    account_image: Option<Vec<u8>>,
    account_uploaded: bool,
    account_cape_url: Option<String>,
    account_error: Option<String>,
    pending_upload: Option<SkinChoice>,
    saving_skin: bool,
}

impl Game<'_> {
    pub(in crate::game) fn draw_wardrobe_preview(&mut self, batch: &mut HudBatch) {
        let Some([left, top, right, bottom]) = self
            .ui
            .options
            .wardrobe_preview_rect(batch.width, batch.height)
        else {
            return;
        };
        let in_world = self.title.is_none();
        let mut mob = if in_world {
            let Some(mob) = self.player_model.clone() else {
                return;
            };
            mob
        } else {
            let worn = self.own_worn();
            if self.own_skin.title.is_none() {
                self.read_title_player();
            }
            let Some(player) = &self.own_skin.title else {
                return;
            };
            let mut mob = player.mob.clone();
            (mob.skin, mob.slim) = match worn {
                Worn::Skin(skin, slim) => (Some(skin), slim),
                Worn::Default | Worn::Waiting => (None, false),
            };
            mob
        };
        if in_world {
            if let Some((skin, slim)) = self.own_skin_sprite() {
                mob.skin = Some(skin);
                mob.slim = slim;
            }
        }
        self.dress_wardrobe_preview(&mut mob);
        if mob.extra.hidden_model_parts & 1 == 0 {
            match self.ui.options.pending_account_cape() {
                Some(Some(cape)) => {
                    let url = cape.url.clone();
                    if let Some(texture) = self.cape_texture(url) {
                        mob.cape = Some(texture);
                    }
                }
                Some(None) => mob.cape = None,
                None => {}
            }
        }
        let data = if self.res.rest_ready() {
            self.res.mob_data()
        } else if let Some(data) = &self.own_skin.title_data {
            data
        } else {
            return;
        };
        let cx = (left + right) * 0.5;
        let cy = (top + bottom) * 0.5;
        let size = ((right - left) * 0.68).min((bottom - top) * 0.43);
        self.mob_preview(
            &mob,
            data,
            batch,
            [left + 2.0, top + 2.0, right - 2.0, bottom - 2.0],
            size,
            0.0625,
            (cx, cy),
            self.ui.options.wardrobe_rotation(),
            false,
        );
    }

    pub(super) fn show_wardrobe(&mut self) {
        let open = self.ui.options.page() == Page::Skin;
        if open {
            let changed = self.ensure_wardrobe_profile();
            if changed || !self.own_skin.wardrobe.open {
                self.refresh_wardrobe();
            }
        }
        self.own_skin.wardrobe.open = open;
        if open && let Some(err) = self.own_skin.wardrobe.account_error.take() {
            self.ui.options.skin_warning(err);
        }
        if self.own_skin.wardrobe.account_worker.is_some() {
            self.ensure_wardrobe_profile();
        }
        if self.own_skin.wardrobe.saving_skin {
            let result = match &self.ui.options.skin_cloud {
                Cloud::Microsoft(Upload::Sending) => None,
                Cloud::Microsoft(Upload::Sent) => Some(Ok(())),
                Cloud::Microsoft(Upload::Failed(error)) => Some(Err(error.clone())),
                _ => Some(Err("starling.skin.error.signed_out".into())),
            };
            if let Some(result) = result {
                self.own_skin.wardrobe.saving_skin = false;
                self.finish_wardrobe_save(result);
                return;
            }
        }
        if let Some(result) =
            self.own_skin
                .wardrobe
                .account_worker
                .as_ref()
                .and_then(|(_, worker)| match worker.try_recv() {
                    Ok(result) => Some(result),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        Some(Err("Could not load account capes".into()))
                    }
                    Err(mpsc::TryRecvError::Empty) => None,
                })
        {
            let owner = self
                .own_skin
                .wardrobe
                .account_worker
                .take()
                .and_then(|(owner, _)| owner);
            self.ui.options.wardrobe_account_busy = false;
            self.ui.options.layout_changed();
            if owner.is_none() || owner != crate::accounts::player_key() {
                return;
            }
            match result {
                Ok(capes) => {
                    if capes.activated
                        && let Some(owner) = owner.as_deref()
                    {
                        let url = capes
                            .rows
                            .iter()
                            .find(|cape| cape.active)
                            .map(|cape| cape.url.clone());
                        crate::accounts::reflect_cape(owner, url.clone());
                        self.own_skin.wardrobe.account_cape_url = url;
                    }
                    self.ui.options.wardrobe_account_capes = Some(capes.rows);
                    self.own_skin.wardrobe.account_image = capes.equipped_image;
                    self.own_skin.wardrobe.account_uploaded = false;
                    if capes.activated {
                        self.choose_own_cape(CapeChoice::Account);
                        self.ui.options.wardrobe_cape_saved();
                        if let Some(choice) = self.own_skin.wardrobe.pending_upload.take() {
                            self.start_wardrobe_skin_save(choice);
                        } else {
                            self.finish_wardrobe_save(Ok(()));
                        }
                    }
                }
                Err(err) => {
                    log::warn!("Wardrobe: {err}");
                    if self.ui.options.wardrobe_saving() {
                        self.own_skin.wardrobe.pending_upload = None;
                        self.finish_wardrobe_save(Err(err));
                    } else {
                        self.own_skin.wardrobe.account_error = Some(err);
                    }
                }
            }
        }
    }

    pub(in crate::game) fn save_wardrobe(
        &mut self,
        cape: Option<Option<String>>,
        skin: Option<SkinChoice>,
    ) {
        if cape.is_some() {
            match self.account_capes(cape) {
                Ok(()) => self.own_skin.wardrobe.pending_upload = skin,
                Err(error) => self.finish_wardrobe_save(Err(error)),
            }
        } else if let Some(choice) = skin {
            self.start_wardrobe_skin_save(choice);
        }
    }

    fn start_wardrobe_skin_save(&mut self, choice: SkinChoice) {
        match self.upload_own_skin(choice) {
            Ok(()) => self.own_skin.wardrobe.saving_skin = true,
            Err(error) => self.finish_wardrobe_save(Err(error)),
        }
    }

    fn finish_wardrobe_save(&mut self, result: Result<(), String>) {
        let events = self
            .ui
            .options
            .finish_wardrobe_save(&mut self.settings, result);
        if let Err(error) = self.handle_option_events(events) {
            log::warn!("Wardrobe: {error}");
        }
    }

    pub(in crate::game) fn account_capes(
        &mut self,
        change: Option<Option<String>>,
    ) -> Result<(), String> {
        self.ensure_wardrobe_profile();
        if self.own_skin.wardrobe.account_worker.is_some() {
            return Err("starling.skin.error.busy".into());
        }
        let key = crate::accounts::player_key();
        if key.is_none() {
            return Err("starling.skin.error.signed_out".into());
        }
        match (&change, &self.ui.options.wardrobe_account_capes) {
            (None, None) => {}
            (Some(Some(id)), Some(rows))
                if rows.iter().any(|cape| cape.id == *id && !cape.active) => {}
            (Some(None), Some(rows)) if rows.iter().any(|cape| cape.active) => {}
            _ => return Err("starling.wardrobe.capes_changed".into()),
        }
        let root = Self::wardrobe_root();
        let (send, receive) = mpsc::channel();
        self.own_skin.wardrobe.account_worker = Some((key.clone(), receive));
        self.ui.options.wardrobe_account_busy = true;
        self.ui.options.layout_changed();
        std::thread::spawn(move || {
            let result = (|| {
                let (account, secrets) = session::selected_session("")?
                    .ok_or("Sign in to Microsoft to manage account capes.")?;
                if account.provider != Provider::Microsoft || Some(account.key()) != key {
                    return Err("The playing Microsoft account changed.".into());
                }
                if let Some(id) = &change {
                    cloud::activate_cape(&account, &secrets, id.as_deref())?;
                }
                let mut snapshot = cloud::fetch_account_capes(&secrets, &root)?;
                if let Some(id) = &change {
                    for delay in [250, 500] {
                        if snapshot
                            .0
                            .iter()
                            .find(|cape| cape.active)
                            .map(|cape| &cape.id)
                            == id.as_ref()
                        {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(delay));
                        snapshot = cloud::fetch_account_capes(&secrets, &root)?;
                    }
                    if snapshot
                        .0
                        .iter()
                        .find(|cape| cape.active)
                        .map(|cape| &cape.id)
                        != id.as_ref()
                    {
                        return Err("starling.wardrobe.capes_changed".into());
                    }
                }
                let (capes, image) = snapshot;
                if change.is_some() {
                    let url = capes
                        .iter()
                        .find(|cape| cape.active)
                        .map(|cape| cape.url.clone());
                    starling_account::skins::remember_cape(
                        &crate::accounts::dir(),
                        &account.key(),
                        url,
                    )?;
                }
                let equipped_image = image.as_deref().map(Self::decode_cape).transpose()?;
                let rows = capes
                    .into_iter()
                    .map(|cape| AccountCape {
                        id: cape.id,
                        name: cape.title,
                        url: cape.url,
                        active: cape.active,
                    })
                    .collect();
                Ok(AccountCapes {
                    rows,
                    equipped_image,
                    activated: change.is_some(),
                })
            })();
            let _ = send.send(result);
        });
        Ok(())
    }

    fn refresh_wardrobe(&mut self) {
        let root = Self::wardrobe_root();
        let result = Library::load(&root).and_then(|library| {
            let skin_dir = Self::skins_dir();
            let mut titles = std::collections::HashMap::new();
            let mut capes = Vec::new();
            for entry in library.entries {
                let source = Library::image(&root, &entry.id)?;
                if !source.is_file() {
                    continue;
                }
                match entry.kind {
                    Kind::Skin => {
                        let name = format!("wardrobe-{}.png", entry.id);
                        let path = skin_dir.join(&name);
                        if !path.is_file() {
                            std::fs::create_dir_all(&skin_dir).map_err(|e| e.to_string())?;
                            let part = path.with_extension("png.part");
                            std::fs::copy(source, &part).map_err(|e| e.to_string())?;
                            std::fs::rename(part, path).map_err(|e| e.to_string())?;
                        }
                        titles.insert(name, entry.title);
                    }
                    Kind::Cape => capes.push((entry.id, entry.title)),
                }
            }
            capes.sort_by_cached_key(|(_, title)| title.to_lowercase());
            self.ui.options.wardrobe_skin_titles = titles;
            self.ui.options.wardrobe_capes = capes;
            Ok(())
        });
        self.ui.options.refresh_skin_page();
        self.ui.options.wardrobe_cape = self.own_skin.wardrobe.cape.clone();
        if let Err(error) = result {
            self.ui.options.skin_warning(error);
        }
    }

    pub(in crate::game) fn pick_wardrobe_png(&mut self, kind: Kind) {
        match documents::begin_appearance(Self::wardrobe_root(), kind) {
            Ok(()) => self.own_skin.wardrobe.picking = true,
            Err(error) => self.ui.options.skin_warning(error),
        }
    }

    pub(in crate::game) fn poll_wardrobe_import(&mut self) {
        if self.own_skin.wardrobe.picking && !documents::busy() {
            self.own_skin.wardrobe.picking = false;
            let notice = documents::poll();
            self.refresh_wardrobe();
            if let Some(notice) = notice
                && (notice.text.starts_with("Import failed")
                    || notice.text == "Android document picker is unavailable")
            {
                self.ui.options.skin_warning(notice.text);
            }
        }
    }

    fn ensure_wardrobe_profile(&mut self) -> bool {
        let key = crate::accounts::player_key();
        if self.own_skin.wardrobe.profile.as_ref() == Some(&key) {
            return false;
        }
        if let Some((owner, _)) = &mut self.own_skin.wardrobe.account_worker {
            *owner = None;
        }
        self.own_skin.wardrobe.account_image = None;
        self.own_skin.wardrobe.account_uploaded = false;
        self.own_skin.wardrobe.account_cape_url = match crate::accounts::profiles().player() {
            Some(starling_account::Player::Account(account)) => account.cape,
            _ => None,
        };
        self.own_skin.wardrobe.account_error = None;
        self.own_skin.wardrobe.pending_upload = None;
        self.own_skin.wardrobe.saving_skin = false;
        self.ui.options.wardrobe_account_capes = None;
        self.ui.options.discard_wardrobe_changes();
        self.ui.options.wardrobe_account_busy = self.own_skin.wardrobe.account_worker.is_some();
        let choice = CapeChoice::load(&Self::player_dir(), key.as_deref()).unwrap_or_default();
        let image = Self::cape_image(&choice).ok().flatten();
        self.set_wardrobe_cape(key, choice, image);
        true
    }

    fn set_wardrobe_cape(
        &mut self,
        key: Option<String>,
        choice: CapeChoice,
        image: Option<Vec<u8>>,
    ) {
        let wardrobe = &mut self.own_skin.wardrobe;
        wardrobe.profile = Some(key);
        wardrobe.cape = choice.clone();
        wardrobe.cape_image = image;
        wardrobe.uploaded = false;
        self.ui.options.wardrobe_cape = choice;
    }

    pub(in crate::game) fn choose_own_cape(&mut self, choice: CapeChoice) {
        let key = crate::accounts::player_key();
        let result = Self::cape_image(&choice).and_then(|image| {
            choice
                .save(&Self::player_dir(), key.as_deref())
                .map_err(|e| e.to_string())?;
            Ok(image)
        });
        match result {
            Ok(image) => self.set_wardrobe_cape(key, choice, image),
            Err(error) => self.ui.options.skin_warning(error),
        }
    }

    pub(super) fn dress_wardrobe_preview(&mut self, mob: &mut sunrise_client::mobs::Mob) {
        mob.extra.hidden_model_parts = !self.settings.file.model_customisation() & 0x7F;
        let server_cape = mob
            .cape
            .take()
            .filter(|cape| cape != LOCAL_CAPE && cape != ACCOUNT_CAPE);
        if mob.extra.hidden_model_parts & 1 == 0 {
            mob.cape = self.wardrobe_cape(server_cape);
        }
    }

    pub(in crate::game) fn wardrobe_cape(&mut self, account: Option<String>) -> Option<String> {
        self.ensure_wardrobe_profile();
        match &self.own_skin.wardrobe.cape {
            CapeChoice::Account if self.ui.options.wardrobe_account_capes.is_some() => {
                if !self.own_skin.wardrobe.account_uploaded || !self.show_skin(ACCOUNT_CAPE) {
                    let image = self.own_skin.wardrobe.account_image.clone()?;
                    self.store_skin(ACCOUNT_CAPE.into(), image);
                    self.own_skin.wardrobe.account_uploaded = true;
                }
                self.show_skin(ACCOUNT_CAPE).then(|| ACCOUNT_CAPE.into())
            }
            CapeChoice::Account => account.or_else(|| self.account_cape_preview()),
            CapeChoice::Hidden => None,
            CapeChoice::Local(_) => {
                if !self.own_skin.wardrobe.uploaded || !self.show_skin(LOCAL_CAPE) {
                    let image = self.own_skin.wardrobe.cape_image.clone()?;
                    self.store_skin(LOCAL_CAPE.into(), image);
                    self.own_skin.wardrobe.uploaded = true;
                }
                self.show_skin(LOCAL_CAPE).then(|| LOCAL_CAPE.into())
            }
        }
    }

    fn account_cape_preview(&mut self) -> Option<String> {
        let url = self.own_skin.wardrobe.account_cape_url.clone()?;
        self.cape_texture(url)
    }

    fn cape_texture(&mut self, url: String) -> Option<String> {
        use sunrise_client::skins::{Download, Kind, Textures};
        let downloaded = self.skin_downloads.borrow_mut().state(Kind::Cape, &url);
        if downloaded != Download::Ready {
            return None;
        }
        self.store_ready_skins();
        let location = sunrise_client::skins::cape(&Textures {
            cape: Some(url),
            ..Default::default()
        })?;
        self.again_skin(&location).then_some(location)
    }

    pub(in crate::game) fn wardrobe_root() -> PathBuf {
        Self::player_dir().join("game-wardrobe")
    }

    fn cape_image(choice: &CapeChoice) -> Result<Option<Vec<u8>>, String> {
        let CapeChoice::Local(id) = choice else {
            return Ok(None);
        };
        let path = Library::image(&Self::wardrobe_root(), id)?;
        Self::decode_cape(&path).map(Some)
    }

    fn decode_cape(path: &std::path::Path) -> Result<Vec<u8>, String> {
        let image = skins::read_cape(&skins::read_file(path)?)?;
        sunrise_client::skins::process_cape(image.width(), image.height(), image.as_raw())
            .ok_or_else(|| "Could not read this cape".into())
    }
}
