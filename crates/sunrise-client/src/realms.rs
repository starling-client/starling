//! Minecraft Realms screens: `RealmsMainScreen` with its `RealmsHeader`, the join-code and
//! pending-invite screens, the owner's open/close, `RealmsPopups.infoPopupScreen` for the
//! questions, `GetServerDetailsTask` under `RealmsLongRunningMcoTaskScreen` for Play, and
//! `RealmsResetWorldScreen` with its template, local-world and upload screens.
//! Layout from the vanilla screens' behaviour and the game's own sprites.
//!
//! Realms is used only with the signed-in player's own Microsoft account: the session the
//! launcher hands over after its owner check (`auth::current`). Without one (offline play, an
//! expired token) the screen says so and makes no request. Every request runs on a worker
//! thread; the title polls the results once a frame (`poll`).
pub mod availability;
pub mod client;
pub mod download;
mod draw_notice;
mod pack;
mod page;
mod picture;
mod preset_world;
mod settings;
pub mod upload;
mod upload_screen;

pub use pack::{RealmsPack, connect_address, pack_id, take_pending_pack};

/// `CommonLinks.EXTEND_REALMS_LINK`.
const EXTEND_REALMS: &str = "https://aka.ms/ExtendJavaRealms";

/// `CommonLinks.extendRealms`: the extend page with the subscription and profile, and the
/// `ref` only for the expired-Realm paths (the subscription tab passes NONE).
pub(super) fn extend_link(server: &RealmServer, profile: u128, expired: bool) -> String {
    match &server.remote_subscription_id {
        None => EXTEND_REALMS.to_owned(),
        Some(sub) => {
            let base = format!("{EXTEND_REALMS}?subscriptionId={sub}&profileId={profile:032x}");
            if expired {
                let reference = if server.expired_trial {
                    "expiredTrial"
                } else {
                    "expiredRealm"
                };
                format!("{base}&ref={reference}")
            } else {
                base
            }
        }
    }
}

use page::age_presentation;
use page::backup_changes;
use page::backup_info_key;
use page::backup_info_value;
use page::centered;
use page::clamp_text;
use page::closest_expiration;
use page::days_left_text;
pub use page::human_readable;
use page::loading_dots;
use page::main_footer_h;
use page::pdp_entries;
use page::pdp_row_narration;
use page::realms_logo;
use page::reset_content_y;
use page::reset_tile;
use page::short_date_text;
use page::start_date_text;
use page::subscription_label;
use page::task_stack_top;
use page::wrap_lines;
use page::{DIFFICULTIES, DIFFICULTY_DEFAULT, EXPIRATIONS, GAME_MODE_DEFAULT, GAME_MODES};
use page::{percent_label, transfer_bar_width, transfer_percentage, upload_limit_size};

use picture::{face_from_skin, realm_name_valid, resample_icon, slot_name, template_none_lines};
pub use settings::{ConfigureTab, SettingsDraft};
use settings::{
    SETTINGS_CELL_PADDING, SETTINGS_LABEL_HEIGHT, SETTINGS_LABEL_SPACING, SETTINGS_WIDGET_HEIGHT,
    can_play, changed_distance, configure_screen, preference_label, region_rows,
    settings_column_top, settings_rows, sort_servers, tab_geometry,
};

use crate::hud::{HudAtlas, HudBatch, rgba};
use crate::text::Text;
use client::{
    Backup, InviteCode, PendingInviteDto, PlayerInfo, RealmAddress, RealmServer, RealmSetting,
    RealmSlot, RealmTierConfiguration, RealmTierRange, RealmWorldOptions, RealmsClient,
    RealmsError, RealmsNotification, RegionDataDto, Subscription, WorldDownload, WorldTemplate,
    WorldTemplateList,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};
use sunrise_client_render::auth::{self, AuthSession};

const HEADER_HEIGHT: f32 = 44.0;
const CONTENT_WIDTH: f32 = 308.0;
/// `CommonLinks.REALMS_TERMS`: the Realms terms page the terms screen's link opens.
const REALMS_TERMS_URL: &str = "https://aka.ms/MinecraftRealmsTerms";
/// `RealmsPdpScreen`'s list: the row width and the text's width beside the picture.
const PDP_ROW_WIDTH: f32 = 290.0;
const PDP_TEXT_WIDTH: f32 = 182.0;

const ROW_WIDTH: f32 = 300.0;
const ROW_HEIGHT: f32 = 36.0;
/// `RealmsSelectWorldTemplateScreen`'s rows (`ObjectSelectionList`'s 46).
const TEMPLATE_ROW: f32 = 46.0;
/// `RealmsConfigureWorldScreen`'s `MenuTabBar` height (`gui/tab_header_background`).
const TAB_HEIGHT: f32 = 24.0;
/// `hud::rgba` packing (ABGR): 160 grey, vanilla's red and `RealmsPopups.COLOR_INFO`.
const GREY: u32 = 0xFFA0_A0A0;
/// `-65536` — vanilla's invalid-input and warning red.
const RED: u32 = 0xFF00_00FF;
/// `RealmsPopups.COLOR_INFO` (8226750).
const INFO: u32 = 0xFFBE_877D;

const REALMS_INVITE_CODE: crate::clipboard::FieldId = ("realms.invite_code", 0);
const REALMS_PLAYER_NAME: crate::clipboard::FieldId = ("realms.player_name", 0);
const REALMS_UPLOAD_SEARCH: crate::clipboard::FieldId = ("realms.upload_search", 0);
const REALMS_SLOT_NAME: crate::clipboard::FieldId = ("realms.slot_name", 0);
const REALMS_CREATE_NAME: crate::clipboard::FieldId = ("realms.create_name", 0);
const REALMS_CREATE_DESC: crate::clipboard::FieldId = ("realms.create_desc", 0);
const REALMS_SETTINGS_NAME: crate::clipboard::FieldId = ("realms.settings_name", 0);
const REALMS_SETTINGS_DESC: crate::clipboard::FieldId = ("realms.settings_desc", 0);

/// `CommonLinks.BUY_REALMS`.
const BUY_REALMS: &str = "https://aka.ms/BuyJavaRealms";
/// `CommonLinks.REALMS_CONTENT_CREATION`.
const REALMS_CONTENT_CREATION: &str = "https://aka.ms/MinecraftRealmsContentCreator";

include!("realms/tasks.rs");

/// The account Realms may use: the launcher's Microsoft session (it hands one over only for
/// an account that owns the game) while its token is good.
fn session() -> Option<Arc<AuthSession>> {
    auth::current().filter(|s| !s.expired())
}

fn client() -> Option<RealmsClient> {
    session().map(|s| RealmsClient::production(&s))
}

/// `WorldSelectionList.filterAccepts`: the level name or its folder contains the filter,
/// case-insensitively (`String.toLowerCase(Locale.ROOT)`).
fn upload_matches(world: &crate::title::WorldEntry, query: &str) -> bool {
    let query = query.to_lowercase();
    query.is_empty()
        || world.name.to_lowercase().contains(&query)
        || world.folder.to_lowercase().contains(&query)
}

/// `RealmsTextureManager.getTexture`: the base64 PNG decoded and resampled to the 64x64
/// `sunrise/world-iconN` slot (nearest texel, like `skins::resample_64`).
fn template_image(encoded: &str) -> Option<Vec<u8>> {
    let bytes = crate::skins::base64_decode(encoded)?;
    let (width, height, rgba) = crate::hud::decode_image_bytes(&bytes).ok()?;
    resample_icon(width, height, &rgba)
}

/// A cheap identity for a base64 picture, so two images of the same length never share
/// a cache entry.
fn encoded_key(encoded: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    encoded.hash(&mut hasher);
    hasher.finish()
}

/// `RealmsWorldSlotButton.State`'s picture, in `extractContents`' order: a minigame's
/// image (or `presets/isles.png` when it has none), the empty frame, a template's own
/// image, or the slot's title panorama.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotPicture<'a> {
    Image(&'a str),
    Preset,
    Empty,
    Panorama(&'static str),
}

/// `RealmsWorldSlotButton.State`: `image` is the minigame's `minigameImage` for slot 4
/// and the slot's `templateImage` otherwise (only when its `templateId` is not -1).
fn slot_picture(slot: i32, empty: bool, image: Option<&str>) -> SlotPicture<'_> {
    if slot == 4 {
        return image.map_or(SlotPicture::Preset, SlotPicture::Image);
    }
    if empty {
        return SlotPicture::Empty;
    }
    if let Some(image) = image {
        return SlotPicture::Image(image);
    }
    match slot {
        1 => SlotPicture::Panorama("panorama_0"),
        2 => SlotPicture::Panorama("panorama_2"),
        3 => SlotPicture::Panorama("panorama_3"),
        _ => SlotPicture::Empty,
    }
}

/// Upload a decoded 64x64 picture into a `sunrise/world-iconN` atlas slot (the slots the
/// title world list uses) and name its sprite. `None` when the atlas has no such slot.
fn icon_sprite(batch: &mut HudBatch, atlas: &HudAtlas, slot: usize, rgba: &[u8]) -> Option<String> {
    let sprite = format!("sunrise/world-icon{}", slot % crate::hud::WORLD_ICON_SLOTS);
    let rect = atlas.sprites.get(&sprite)?;
    batch.patches.push(crate::font::Patch {
        x: (rect.u0 * atlas.width as f32).round() as u32,
        y: (rect.v0 * atlas.height as f32).round() as u32,
        width: 64,
        height: 64,
        rgba: rgba.to_vec(),
    });
    Some(sprite)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LayoutState {
    Loading,
    NoRealms,
    List,
}

#[derive(Clone, Debug)]
pub enum FetchState {
    /// No Microsoft session: nothing is asked of the service.
    Unauthenticated,
    Loading(Job<Result<Vec<RealmServer>, RealmsError>>),
    Loaded(Vec<RealmServer>),
    Error(RealmsError),
}

/// A `RealmsPopups.infoPopupScreen`: Continue or Cancel.
#[derive(Clone, Debug)]
pub enum Popup {
    /// `RealmsMainScreen.leaveClicked`.
    Leave(RealmServer),
    /// `GetServerDetailsTask.resourcePackDownloadConfirmationScreen`.
    ResourcePack { pack: RealmsPack },
    /// `RealmsSubscriptionTab`'s delete question (only offered once the Realm has expired).
    Delete(RealmServer),
    /// `RealmsPlayersTab.PlayerEntry.uninvite`'s confirm (`RealmsConfirmScreen`).
    Uninvite {
        server: RealmServer,
        player: PlayerInfo,
    },
    /// `RealmsWorldsTab.switchToFullSlot`'s question, then `RealmsClient.switchSlot`.
    SwitchSlot { server: RealmServer, slot: i32 },
    /// `RealmsWorldsTab.switchToEmptySlot`'s question — the same text as the full slot's —
    /// then `RealmsResetWorldScreen.forEmptySlot`.
    SwitchEmpty { server: RealmServer, slot: i32 },
    /// `RealmsBackupScreen.Entry.restoreClicked`: the warning popup over the backup list,
    /// then `RealmsClient.restoreWorld`.
    RestoreBackup {
        server: RealmServer,
        slot_id: i32,
        backup: Backup,
        /// The list as it was, so the popup can draw it behind (the background screen).
        list: Vec<Backup>,
        selected: Option<usize>,
    },
    /// `RealmsBackupScreen.downloadClicked`: the info popup over the backup list, then
    /// `DownloadTask`.
    DownloadLatest {
        server: RealmServer,
        slot_id: i32,
        /// The list as it was, so the popup can draw it behind (the background screen).
        list: Vec<Backup>,
        selected: Option<usize>,
    },
    /// `checkDownloadSize`'s `warningAcknowledgePopupScreen`: the world is over 5 GiB, OK
    /// starts the download anyway (Escape leaves the download screen idle, as vanilla's
    /// `checked` flag does).
    DownloadOversized {
        server: RealmServer,
        slot_id: i32,
        download: WorldDownload,
        world_name: String,
        /// The download screen under the popup, so its dots keep blinking.
        started: Instant,
    },
    /// `RealmsSettingsTab`'s close question (`RealmsPopups.customPopupScreen` over the
    /// configuration screen): Continue saves and closes, Cancel returns to the tab.
    CloseWorld {
        server: RealmServer,
        settings: SettingsDraft,
        detail: Option<Result<RealmServer, RealmsError>>,
        tier: Option<Result<RealmTierConfiguration, RealmsError>>,
    },
    /// `ConfirmLinkScreen.confirmLink`: a website the screen would open, asked first. The
    /// parent is the screen the answer returns to (`ConfirmLinkScreen`'s `parentScreen`).
    OpenLink {
        url: String,
        parent: Box<SubScreen>,
        /// Where a yes goes; the popup itself when its URL button asked.
        open_parent: Option<Box<SubScreen>>,
    },
    /// `RealmsNotification.InfoPopup.buildScreen`: a `PopupScreen` over the list.
    Notice {
        uuid: String,
        title: String,
        message: String,
        image: Option<String>,
        url: Option<(String, String)>,
    },
}

/// `RealmsResetWorldScreen`'s entry points (`forResetSlot`, `forEmptySlot`, `forNewRealm`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetKind {
    /// `forResetSlot`: the world is replaced after a warning; the active slot's.
    Reset,
    /// `forEmptySlot`: the slot has no world yet.
    Empty,
    /// `forNewRealm`: the Realm itself does not exist yet; the reset creates it first
    /// (`RealmCreationTask`).
    NewRealm,
}

impl ResetKind {
    /// The screen's title (`RealmsResetWorldScreen`'s constants).
    fn title(self) -> &'static str {
        match self {
            Self::Reset => "mco.reset.world.title",
            Self::Empty => "mco.configure.world.switch.slot",
            Self::NewRealm => "mco.selectServer.create",
        }
    }
    /// The subtitle, and whether it is the red warning.
    fn subtitle(self) -> (&'static str, bool) {
        match self {
            Self::Reset => ("mco.reset.world.warning", true),
            Self::Empty => ("mco.configure.world.switch.slot.subtitle", false),
            Self::NewRealm => ("mco.selectServer.create.subtitle", false),
        }
    }
    /// `RESET_WORLD_RESET_TASK_TITLE` and `CREATE_WORLD_RESET_TASK_TITLE`.
    fn task_title(self) -> &'static str {
        match self {
            Self::Reset => "mco.reset.world.resetting.screen.title",
            Self::Empty | Self::NewRealm => "mco.create.world.reset.title",
        }
    }
}

/// The four template groups of `RealmsResetWorldScreen`'s grid, in its order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateGroup {
    Normal,
    Adventure,
    Experience,
    Inspiration,
}

/// `RealmsCreateWorldFlow.createWorld`'s target: the reset screen's Generate New World
/// tile asks the title for the Create New World screen, whose settings upload into
/// `slot`.
#[derive(Clone, Debug)]
pub struct CreateWorldRequest {
    pub server: RealmServer,
    pub slot: i32,
    pub kind: ResetKind,
    /// `RealmCreationTask`'s name and description, for `ResetKind::NewRealm`.
    pub creation: Option<(String, String)>,
}

pub use preset_world::CreatedWorld;

impl TemplateGroup {
    const ALL: [Self; 4] = [
        Self::Normal,
        Self::Adventure,
        Self::Experience,
        Self::Inspiration,
    ];

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|g| *g == self)
            .expect("a group is in ALL")
    }
    /// `RealmsServer.WorldType`'s name: what `fetchWorldTemplates` puts in the path.
    fn world_type(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Adventure => "ADVENTUREMAP",
            Self::Experience => "EXPERIENCE",
            Self::Inspiration => "INSPIRATION",
        }
    }
    fn title(self) -> &'static str {
        match self {
            Self::Normal => "mco.reset.world.template",
            Self::Adventure => "mco.reset.world.adventure",
            Self::Experience => "mco.reset.world.experience",
            Self::Inspiration => "mco.reset.world.inspiration",
        }
    }
}

/// `RealmsSelectWorldTemplateScreen`'s list: one page fetched on a worker, the number of
/// the page after it, and whether it was the last (the list shows each page as it lands).
type TemplatePages = Job<Result<(Vec<WorldTemplate>, i32, bool), RealmsError>>;

#[derive(Clone, Debug)]
pub enum SubScreen {
    Main,
    JoinCode {
        code: String,
        error: Option<String>,
        job: Option<Job<Result<(), RealmsError>>>,
    },
    PendingInvites {
        invites: Option<Result<Vec<PendingInviteDto>, RealmsError>>,
        job: Option<Job<Result<Vec<PendingInviteDto>, RealmsError>>>,
        /// The list's scroll (`ObjectSelectionList`): the port drew what fit before.
        scroll: crate::ui::Scroll,
        /// The clicked row (`ContainerObjectSelectionList`'s selection).
        selected: Option<usize>,
    },
    /// `RealmsBackupScreen`: the Realm's automatic backups.
    Backups {
        server: RealmServer,
        slot_id: i32,
        backups: Option<Result<Vec<Backup>, RealmsError>>,
        list_job: Option<Job<Result<Vec<Backup>, RealmsError>>>,
        /// `RestoreTask` while the restore runs.
        restore_job: Option<RestoreTask>,
        /// `DownloadTask` while it asks the service for the latest world's links.
        download_task: Option<DownloadTask>,
        /// The clicked row (`ContainerObjectSelectionList`'s selection).
        selected: Option<usize>,
        /// The list's scroll (`ContainerObjectSelectionList`).
        scroll: crate::ui::Scroll,
    },
    /// `RealmsDownloadLatestWorldScreen`: the latest world's archive coming down into the
    /// saves folder, with vanilla's bar, percentage, speed and Cancel.
    DownloadLatest {
        server: RealmServer,
        slot_id: i32,
        /// `WorldDownload`: where the world and its resource pack come from.
        download: WorldDownload,
        /// `downloadName`: the name the unpacked world takes (before `FileDownload`'s
        /// clean-up).
        world_name: String,
        /// `checkDownloadSize`'s HEAD: the link's `Content-Length`, run once.
        size_job: Option<Job<u64>>,
        progress: Arc<download::DownloadProgress>,
        job: Option<Job<Result<(), download::DownloadError>>>,
        error: Option<download::DownloadError>,
        /// The worker answered (done or failed): Cancel gives way to Done.
        done: bool,
        /// When the screen opened: the dots' blink (`tickCount / 10` at 20 ticks a second).
        started: Instant,
        /// `previousWrittenBytes`/`previousTimeSnapshot`/`bytesPersSecond`: the written
        /// bytes, when they were sampled, and the last rate, sampled every 20 ticks.
        speed: std::cell::Cell<(u64, Instant, u64)>,
    },
    /// `RealmsBackupInfoScreen`: one backup's changed metadata.
    BackupInfo {
        server: RealmServer,
        slot_id: i32,
        backup: Backup,
        /// The list it came from, so Back returns to it as it was (`lastScreen`).
        list: Vec<Backup>,
        selected: Option<usize>,
        list_scroll: crate::ui::Scroll,
        scroll: crate::ui::Scroll,
    },
    Configure {
        server: RealmServer,
        tab: ConfigureTab,
        /// `RealmsClient.subscriptionFor` on a worker, then its answer (the subscription tab).
        subscription: Option<Result<Subscription, RealmsError>>,
        subscription_job: Option<Job<Result<Subscription, RealmsError>>>,
        /// `RealmsClient.getOwnRealm` on a worker, then its answer (the slots and players).
        detail: Option<Result<RealmServer, RealmsError>>,
        detail_job: Option<Job<Result<RealmServer, RealmsError>>>,
        /// `RealmsClient.deleteRealm` on a worker, from the subscription tab's question.
        deleting: Option<Job<Result<(), RealmsError>>>,
        /// `RealmsClient.op`/`deop`/`uninvite` on a worker, from the players tab.
        member_job: Option<Job<Result<(), RealmsError>>>,
        /// The settings tab's editable fields, once the tab has been opened.
        settings: Option<SettingsDraft>,
        /// The players tab's list scroll (`InvitedObjectSelectionList`).
        players_scroll: crate::ui::Scroll,
        /// `RealmsClient.getRealmTierConfiguration`, then its answer: the settings tab's
        /// distance sliders (`tierConfigurationFetched` keeps it to one fetch).
        tier: Option<Result<RealmTierConfiguration, RealmsError>>,
        tier_job: Option<Job<Result<RealmTierConfiguration, RealmsError>>>,
        /// `RealmsClient.preferredRegions` for the region preference screen.
        regions: Option<Result<Vec<RegionDataDto>, RealmsError>>,
        regions_job: Option<Job<Result<Vec<RegionDataDto>, RealmsError>>>,
        /// `RealmsClient.updateConfiguration` on a worker.
        saving: Option<Job<Result<(), RealmsError>>>,
        error: Option<String>,
        /// `RealmsClient.open`/`close` on a worker.
        job: Option<Job<Result<(), RealmsError>>>,
    },
    /// `RealmsSlotOptionsScreen`: the active slot's name and world options.
    SlotOptions {
        server: RealmServer,
        slot: RealmSlot,
        name: String,
        difficulty: usize,
        game_mode: usize,
        force_game_mode: bool,
        spawn_protection: i32,
        /// `RealmsClient.updateSlot` on a worker.
        job: Option<Job<Result<(), RealmsError>>>,
        error: Option<String>,
    },
    /// `RealmsPreferredRegionSelectionScreen`: the region preference list.
    RegionPreference {
        server: RealmServer,
        /// The settings tab's draft, which Done writes the choice back to.
        settings: SettingsDraft,
        regions: Vec<RegionDataDto>,
        /// The highlighted row: `(preference, region)`.
        selected: Option<(i32, Option<String>)>,
        /// The list's scroll (`ObjectSelectionList`): the rows can outgrow the screen.
        scroll: crate::ui::Scroll,
        /// The settings tab's data, carried back so its save can run at once.
        detail: Option<Result<RealmServer, RealmsError>>,
        tier: Option<Result<RealmTierConfiguration, RealmsError>>,
    },
    /// `RealmsInviteCodesScreen`: the Realm's shareable codes.
    InviteCodes {
        server: RealmServer,
        codes: Option<Result<Vec<InviteCode>, RealmsError>>,
        list_job: Option<Job<Result<Vec<InviteCode>, RealmsError>>>,
        /// `createInviteCode` on a worker.
        job: Option<Job<Result<(), RealmsError>>>,
        error: Option<String>,
        /// The highlighted row (`InviteCodeSelectionList`).
        selected: Option<usize>,
    },
    /// `RealmsEditInviteCodeScreen`: one code's active flag and expiration.
    EditCode {
        server: RealmServer,
        code: InviteCode,
        active: bool,
        /// `EXPIRATIONS`' index.
        expiration: usize,
        job: Option<Job<Result<(), RealmsError>>>,
        error: Option<String>,
    },
    /// `RealmsInviteScreen`: a name to invite to the Realm.
    Invite {
        server: RealmServer,
        /// The invited players as the tab had them (the duplicate check).
        players: Vec<PlayerInfo>,
        name: String,
        /// `RealmsClient.invite` on a worker, then the players.
        job: Option<Job<Result<Vec<PlayerInfo>, RealmsError>>>,
        error: Option<String>,
    },
    /// `RealmsResetWorldScreen`: the 3x2 tiles that replace the slot's world.
    ResetWorld {
        server: RealmServer,
        slot: i32,
        kind: ResetKind,
        /// `RealmCreationTask`'s name and description, for `ResetKind::NewRealm`.
        creation: Option<(String, String)>,
        /// `init`'s fetcher: page 1 of each group's templates, in `TemplateGroup::ALL`'s order.
        templates: Option<[Result<WorldTemplateList, RealmsError>; 4]>,
        job: Option<Job<[Result<WorldTemplateList, RealmsError>; 4]>>,
        /// The template reset (`ResettingTemplateWorldTask`) while it runs, then its answer.
        reset: Option<RunningTask>,
        error: Option<String>,
    },
    /// `RealmsSelectWorldTemplateScreen`: a world type's templates, a page at a time.
    TemplateList {
        server: RealmServer,
        slot: i32,
        kind: ResetKind,
        /// `RealmCreationTask`'s name and description, for `ResetKind::NewRealm`.
        creation: Option<(String, String)>,
        /// `None` for the minigame list, which switches the Realm instead of resetting it.
        group: Option<TemplateGroup>,
        templates: Vec<WorldTemplate>,
        selected: Option<usize>,
        scroll: crate::ui::Scroll,
        job: Option<TemplatePages>,
        /// `SwitchMinigameTask` on a worker, from the minigame list's Select.
        select: Option<RunningTask>,
        /// `Entry.websiteButton`/`trailerButton` as drawn: (row, rect, trailer) — the
        /// click needs the row's version width, which only the draw measures.
        icons: std::cell::RefCell<Vec<(usize, [f32; 4], bool)>>,
        error: Option<String>,
    },
    /// `RealmsCreateRealmScreen`: the name and description of a Realm the service made
    /// but has not been set up yet (`RealmsMainScreen.ServerEntry.createUnitializedRealm`).
    CreateRealm {
        server: RealmServer,
        name: String,
        description: String,
        /// The field with the caret: the name (false) or the description (true).
        on_description: bool,
        error: Option<String>,
    },
    /// `RealmsSelectFileToUploadScreen`: the local worlds an upload can pick.
    UploadSelect {
        server: RealmServer,
        slot: i32,
        kind: ResetKind,
        /// `RealmCreationTask`'s name and description, for `ResetKind::NewRealm`.
        creation: Option<(String, String)>,
        worlds: Vec<crate::title::WorldEntry>,
        selected: Option<usize>,
        scroll: crate::ui::Scroll,
        /// The search box's text (`WorldSelectionList.updateFilter`).
        query: String,
    },
    /// `RealmsUploadScreen`: the pack and the body on a worker, its numbers and its answer.
    Uploading {
        server: RealmServer,
        slot: i32,
        world: crate::title::WorldEntry,
        progress: Arc<upload::UploadProgress>,
        job: Option<Job<Result<(), upload::UploadError>>>,
        error: Option<upload::UploadError>,
        /// The worker answered (done or failed): Cancel gives way to Done.
        done: bool,
        /// When the screen opened: the dots' blink (`tickCount / 10` at 20 ticks a second).
        started: Instant,
    },
    /// `RealmsCreateWorldFlow.createWorld`: the temporary world the Create New World
    /// screen described is packed and uploaded, under `AlertScreen`
    /// (`mco.create.world.reset.title`) with Cancel.
    CreateWorldUpload {
        request: CreateWorldRequest,
        progress: Arc<upload::UploadProgress>,
        job: Option<Job<Result<(), upload::UploadError>>>,
    },
    /// `RealmsPdpScreen`: the product page behind Add Realm, its picture rows and scroll.
    AddRealm {
        scroll: crate::ui::Scroll,
        /// The list's drawn height, measured in [`Screen::draw`] and read by the wheel.
        content: std::cell::Cell<f32>,
        /// The rows as last drawn, in scroll order: the click needs the heights the
        /// wrapped text gave them.
        rows: std::cell::RefCell<Vec<[f32; 4]>>,
        /// `RealmsClient.trialAvailable` (`RealmsDataFetcher.trialAvailabilityTask`): the
        /// header's trial note is shown only when this says yes.
        trial: bool,
        trial_job: Option<Job<Result<bool, RealmsError>>>,
    },
    Popup(Popup),
    /// `RealmsLongRunningMcoTaskScreen` running `GetServerDetailsTask`: the join request is on
    /// a worker (`joining`); an error stays on screen with a Back button.
    Connecting {
        server: RealmServer,
        error: Option<String>,
        /// `GetServerDetailsTask.lastScreen`: where a 6002 answer returns
        /// (`RealmsTermsScreen`'s Disagree and Escape).
        last: Box<SubScreen>,
    },
    /// `RealmsTermsScreen`: a new account must agree to the Realms terms before its first
    /// join (the service answers 6002).
    Terms {
        server: RealmServer,
        last: Box<SubScreen>,
        /// `RealmsClient.agreeToTos` on a worker; the join runs again when it succeeds.
        agree: Option<Job<Result<(), RealmsError>>>,
    },
    /// `RealmsGenericErrorScreen`: a service failure replaces the screen; OK and Escape
    /// return to `next` (`nextScreen`).
    Error {
        /// `mco.errorMessage.realmsService.realmsError` with the code, or the title itself.
        title: String,
        /// The error's message, drawn in the -2142128 blue.
        detail: String,
        next: Box<SubScreen>,
    },
}

/// The join request on its worker, and its abort flag (`LongRunningTask.aborted`).
type Joining = (Job<Result<RealmAddress, RealmsError>>, Arc<AtomicBool>);

#[derive(Clone, Debug)]
pub struct Screen {
    pub state: FetchState,
    pub subscreen: SubScreen,
    pub selected: Option<usize>,
    pub joining: Option<Joining>,
    /// Leaving a Realm (`RealmsMainScreen.leaveServer`): the list reloads when it's done.
    leaving: Option<Job<Result<(), RealmsError>>>,
    pub last_click: Option<(usize, SystemTime)>,
    /// The game's `saves` folder: `RealmsSelectFileToUploadScreen` lists it
    /// (`minecraft.gameDirectory`'s `saves`).
    saves: Option<PathBuf>,
    /// `RealmsNewsManager`: the page the header's news button opens, and whether it has
    /// not been read yet (vanilla keeps the read link in `realms_persistence.json`).
    news_link: Option<String>,
    news_job: Option<Job<Result<Option<String>, RealmsError>>>,
    news_unread: bool,
    /// `RealmsHeader`'s pending-invite count for the invite button's badge
    /// (`numberOfPendingInvites`).
    pending_invites: u32,
    pending_invites_job: Option<Job<Result<Vec<PendingInviteDto>, RealmsError>>>,
    /// `RealmsHeader.createLayout`'s join button width, measured in the draw (text plus 16,
    /// at most half of the 308 content the logo leaves).
    join_width: std::cell::Cell<f32>,
    /// The product page's selected row (`ObjectSelectionList.getSelected`).
    pub(crate) pdp_row: Option<usize>,
    /// The last click took a product-page row (`ObjectSelectionList.mouseClicked`): the
    /// keyboard focus moves to the list.
    list_clicked: bool,
    /// `ConfirmLinkScreen`'s three buttons as last drawn (`Choice::new` lays them out from
    /// the message's width).
    link_buttons: std::cell::Cell<[(f32, f32); 3]>,
    /// `RealmsTermsScreen`'s terms link as last drawn: its hit box follows the text's
    /// rendered width (`extractRenderState`'s `onLink`).
    terms_link: std::cell::Cell<[f32; 4]>,
    /// `RealmsMainScreen.notifications`: the service's messages; the first `visitUrl`
    /// becomes the list's top entry (`NotificationMessageEntry`).
    notifications: Vec<RealmsNotification>,
    notifications_job: Option<Job<Result<Vec<RealmsNotification>, RealmsError>>>,
    /// `handledSeenNotifications`: the ones `notifications/seen` answered for.
    handled_seen: Vec<String>,
    /// `notifications/dismiss` on a worker.
    dismiss_job: Option<Job<Result<(), RealmsError>>>,
    /// `NotificationMessageEntry`'s open button, then its dismiss cross, as last drawn; and
    /// the entry's height. The click needs the wrapped text's height.
    notice_rects: std::cell::Cell<[[f32; 4]; 2]>,
    notice_height: std::cell::Cell<f32>,
    /// `PopupScreen`'s two buttons as last drawn: OK, then the URL button.
    notice_buttons: std::cell::Cell<[[f32; 4]; 2]>,
    /// `PopupScreen`'s Continue and Cancel as last drawn (`buildButtonRow`'s geometry,
    /// which the message's wrapped height decides).
    popup_buttons: std::cell::Cell<[[f32; 4]; 2]>,
    /// The main list's scroll (`ObjectSelectionList`): the port drew what fit before.
    main_scroll: crate::ui::Scroll,
    /// The subscription tab's Extend and Delete button tops, as the draw laid the
    /// column out (`RealmsSubscriptionTab`'s centred stack).
    subscription_buttons: std::cell::Cell<(f32, f32)>,
    /// The players tab's per-row buttons as last drawn: the rect and its id (the op
    /// toggle's width follows its label, which only the draw measures).
    players_button_rects: std::cell::RefCell<Vec<([f32; 4], u8)>>,
    /// `RealmsBackupScreen.Entry`'s Changes/Restore buttons as last drawn.
    backup_row_buttons: std::cell::RefCell<Vec<([f32; 4], u8)>>,
    /// The settings tab's `ScrollableLayout`: the column is taller than the tab area on
    /// short windows (vanilla scrolls it).
    settings_scroll: crate::ui::Scroll,
    /// The settings tab's held distance slider (`AbstractSliderButton.dragging`): 0 for
    /// render distance, 1 for simulation distance.
    settings_slider: Option<usize>,
    /// The other held sliders: 1 the slot options' spawn protection, 2 the edit-code
    /// expiration.
    other_slider: Option<u8>,
    /// A button press that has to wait for the settings tab's save (`Play`, the open/close
    /// toggle): `RealmsConfigureWorldScreen` saves an active tab before either runs.
    after_save: Option<AfterSave>,
    /// `RealmsMainScreen.redirectToPdpIfNoRealms`: opened from the title screen, the first
    /// empty server list opens the product page.
    redirect_to_pdp: bool,
    /// `mco.template.select.none`'s link segment as last drawn: the empty template list's
    /// link to the content-creation page.
    template_link_rect: std::cell::Cell<[f32; 4]>,
    /// `RealmsMainScreen.trialAvailable`: the Add Realm button reads "free trial" and the
    /// product page offers "Try for free".
    trial: bool,
    trial_job: Option<Job<Result<bool, RealmsError>>>,
    /// `RealmsDataFetcher.liveStatsTask`: each Realm's online players (`getLiveStats`).
    live_players: std::collections::HashMap<i64, Vec<String>>,
    live_job: Option<Job<Result<std::collections::HashMap<i64, Vec<String>>, RealmsError>>>,
    /// `RealmsDataFetcher`'s periods: the server list every 60 s, pending invites every
    /// 10 s, news and notifications every 300 s.
    last_list_refresh: Instant,
    last_pending_refresh: Instant,
    last_news_refresh: Instant,
    /// The region list answered with the region button's press waiting: open the
    /// preference screen then (`RealmsConfigureWorldScreen.init` fetches it up front).
    region_screen_requested: bool,
    /// `RealmsTextureManager`'s world-template pictures, decoded to the 64x64
    /// `sunrise/world-iconN` slots, by `(template id, image length)`.
    template_images: std::cell::RefCell<std::collections::HashMap<String, Option<Vec<u8>>>>,
    /// `RealmsSelectFileToUploadScreen`'s world icons: `icon.png` decoded, cached by the
    /// file's modified time (the same cache the title world list uses).
    world_icons: std::cell::RefCell<
        std::collections::HashMap<std::path::PathBuf, (SystemTime, Option<Vec<u8>>)>,
    >,
    /// `RealmsUtil.extractPlayerFace`: the profile resolver and skin downloader the
    /// title screen otherwise has none of (`Game` keeps its own for the tab list).
    skin_workers: SkinWorkers,
    /// The 64x64 face of each downloaded skin, by its skin location.
    skin_faces: std::cell::RefCell<std::collections::HashMap<String, Vec<u8>>>,
}

/// The title screen's own skin resolution workers (§`RealmsUtil.extractPlayerFace`).
/// Shared through `Arc` because `Screen` is cloned; `Debug` is a placeholder because the
/// resolver and downloader hold threads and files, not printable state.
#[derive(Clone)]
struct SkinWorkers {
    profiles: std::sync::Arc<std::sync::Mutex<crate::skins::Resolver>>,
    downloads: std::sync::Arc<std::sync::Mutex<crate::skins::Downloader>>,
}

impl std::fmt::Debug for SkinWorkers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkinWorkers").finish_non_exhaustive()
    }
}

/// `RealmsSettingsTab.save`'s callers: the settings are written before the action runs.
#[derive(Clone, Debug)]
enum AfterSave {
    /// `RealmsConfigureWorldScreen`'s Play: `onClose()` saves, then joins.
    Play(RealmServer),
    /// The open/close toggle: save, then `openTheWorld(false)`/`closeTheWorld()`.
    Close(i64),
    Open(i64),
}

pub enum Outcome {
    None,
    Back,
    Connect(String),
    OpenUri(String),
    /// `KeyboardHandler.setClipboard` for the title's own clipboard.
    Copy(String),
    /// `RealmsResetWorldScreen`'s Generate New World tile: `RealmsCreateWorldFlow`
    /// opens Create World and uploads the world it describes.
    CreateWorld(CreateWorldRequest),
}

impl Default for Screen {
    fn default() -> Self {
        Self::new()
    }
}

/// `GetServerDetailsTask.fetchServerAddress`: up to 40 tries, waiting `Retry-After` between
/// them; an abort stops it.
fn fetch_address(
    client: &RealmsClient,
    realm_id: i64,
    aborted: &AtomicBool,
) -> Result<RealmAddress, RealmsError> {
    for _ in 0..40 {
        if aborted.load(Ordering::Relaxed) {
            return Err(RealmsError::Timeout);
        }
        match client.join_world(realm_id) {
            Err(RealmsError::Retry(seconds)) => {
                let until = Instant::now() + Duration::from_secs(seconds.min(60));
                while Instant::now() < until && !aborted.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(100));
                }
            }
            other => return other,
        }
    }
    Err(RealmsError::Timeout)
}

/// `RealmsCreateWorldFlow.createWorld`'s worker: `createTemporaryWorldFolder`, then
/// `RealmCreationTask` → `SwitchSlotTask` → `packAndUpload`, as an upload's worker does.
fn upload_created_world(
    client: &RealmsClient,
    server: &RealmServer,
    slot: i32,
    creation: Option<&(String, String)>,
    world: &CreatedWorld,
    progress: &upload::UploadProgress,
) -> Result<(), upload::UploadError> {
    let dir = create_temporary_world(world).map_err(upload::UploadError::Failed)?;
    let result = (|| {
        if let Some((name, description)) = creation {
            client
                .initialize_realm(server.id, name, description)
                .map_err(|e| upload::UploadError::Failed(e.text()))?;
        }
        if slot != server.active_slot {
            client
                .switch_slot(server.id, slot)
                .map_err(|e| upload::UploadError::Failed(e.text()))?;
        }
        // `RealmsWorldOptions.createFromSettings`: the world's own settings, the game's
        // version as the world version.
        let options = RealmWorldOptions {
            spawn_protection: 0,
            force_game_mode: false,
            difficulty: i32::from(world.level.difficulty_id()),
            game_mode: if world.level.creative { 1 } else { 0 },
            slot_name: world.level.name.clone(),
            version: crate::save::VERSION_NAME.into(),
            compatibility: "UNVERIFIABLE".into(),
            template_id: -1,
            template_image: None,
            empty: false,
        };
        let realm_slot = RealmSlot {
            slot_id: slot,
            options,
            settings: vec![RealmSetting {
                name: "hardcore".into(),
                value: world.level.hardcore.to_string(),
            }],
        };
        upload::upload_world(client, server.id, &realm_slot, &dir, progress)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// `RealmsCreateWorldFlow.createTemporaryWorldFolder`: level.dat as
/// `WorldData.createTag(null)` writes it (no `singleplayer_uuid`), the world generation
/// and game rules, and the Create World screen's data packs moved into the world. No
/// chunks: the Realm generates them when the world first opens.
fn create_temporary_world(world: &CreatedWorld) -> std::result::Result<PathBuf, String> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "sunrise-realms-world-upload-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let result = (|| {
        let mut level = world.level.clone();
        level.data_packs = Some(world.data_packs.clone());
        level.write_upload(&dir).map_err(|e| e.to_string())?;
        crate::save::write_world_gen_settings_with(
            &dir,
            level.seed,
            world.world_gen,
            world.flat_settings.clone(),
            world.single_biome.as_deref(),
            world.dimensions.clone(),
        )
        .map_err(|e| e.to_string())?;
        if let Some(pack_dir) = &world.pack_dir {
            // `Files.move(tempDataPackDir, worldFolder.resolve("datapacks"))`; the
            // Create World screen's temporary folder is gone afterwards.
            crate::datapack::install(pack_dir, &dir, &world.data_packs.0)?;
            let _ = std::fs::remove_dir_all(pack_dir);
        }
        Ok(())
    })();
    match result {
        Ok(()) => Ok(dir),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            Err(e)
        }
    }
}

/// `RealmsSelectWorldTemplateScreen.fetchTemplates`: one page at a time; the list shows it
/// as it arrives and the screen chains the next page until the last or an empty one.
fn fetch_template_page(
    client: &RealmsClient,
    world_type: &str,
    page: i32,
    page_size: i32,
) -> Result<(Vec<WorldTemplate>, i32, bool), RealmsError> {
    let answer = client.fetch_world_templates(page, page_size, world_type)?;
    let empty = answer.templates.is_empty();
    Ok((
        answer.templates,
        page + 1,
        empty || last_template_page(answer.page, answer.size, answer.total),
    ))
}

/// `WorldTemplatePaginatedList.isLastPage`.
fn last_template_page(page: i32, size: i32, total: i32) -> bool {
    page > 0 && size > 0 && total > 0 && page * size >= total
}

/// The row `y` falls on in a list that starts at `top` and has scrolled `offset` pixels.
fn row_at(
    x: f32,
    y: f32,
    w: f32,
    top: f32,
    bottom: f32,
    height: f32,
    offset: f32,
) -> Option<usize> {
    let rx = ((w - ROW_WIDTH) / 2.0).floor();
    if y < top || y >= bottom || x < rx || x > rx + ROW_WIDTH {
        return None;
    }
    let row = (y - top - 2.0 + offset) / height;
    (row >= 0.0).then(|| row.floor() as usize)
}

include!("realms/screen.rs");

#[cfg(test)]
mod tests;
