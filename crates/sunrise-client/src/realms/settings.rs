//! `RealmsConfigureWorldScreen`'s settings draft, its tab bar, its rows and the helpers
//! around them.

use super::SubScreen;
use super::client;
use super::client::{RealmServer, RealmTierRange, RegionDataDto, RegionSelectionPreferenceDto};

pub(super) const SETTINGS_CELL_PADDING: f32 = 8.0;
pub(super) const SETTINGS_LABEL_HEIGHT: f32 = 9.0;
pub(super) const SETTINGS_LABEL_SPACING: f32 = 4.0;
pub(super) const SETTINGS_WIDGET_HEIGHT: f32 = 20.0;

/// A plain `labeledElement` cell: (9 + 4 + 20) + padding top and bottom.
pub(super) const SETTINGS_ROW_HEIGHT: f32 = SETTINGS_LABEL_HEIGHT
    + SETTINGS_LABEL_SPACING
    + SETTINGS_WIDGET_HEIGHT
    + 2.0 * SETTINGS_CELL_PADDING;
/// The region cell: (label 9 + spacing 4 + line 9 + button 20) + padding.
pub(super) const SETTINGS_REGION_ROW_HEIGHT: f32 = SETTINGS_LABEL_HEIGHT
    + SETTINGS_LABEL_SPACING
    + SETTINGS_LABEL_HEIGHT
    + SETTINGS_WIDGET_HEIGHT
    + 2.0 * SETTINGS_CELL_PADDING;

/// `RealmsSettingsTab`'s fields: the name, the description and the region preference,
/// as the player is editing them.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsDraft {
    pub name: String,
    pub description: String,
    /// `RegionSelectionPreference`'s id: [`client::AUTOMATIC_PLAYER`],
    /// [`client::AUTOMATIC_OWNER`] or [`client::MANUAL`].
    pub preference: i32,
    /// The `RealmsRegion`'s `nameId`, while `preference` is `MANUAL`.
    pub region: Option<String>,
    /// The field with the caret: the name (false) or the description (true).
    pub on_description: bool,
    /// The picked distances, once the tier configuration arrived
    /// (`RealmsSettingsTab`'s two `OptionInstance`s; none while it hasn't).
    pub render_distance: Option<i32>,
    pub simulation_distance: Option<i32>,
}

impl SettingsDraft {
    /// The draft the tab opens with (`RealmsSettingsTab.updateData`): a Realm without a
    /// preference has `RegionSelectionPreferenceDto.DEFAULT`, automatic by owner.
    pub(super) fn of(server: &RealmServer) -> Self {
        let (preference, region) = server.region_selection_preference.as_ref().map_or(
            (client::AUTOMATIC_OWNER, None),
            |p| {
                (
                    p.region_selection_preference,
                    p.preferred_region.clone().filter(|r| !r.is_empty()),
                )
            },
        );
        Self {
            name: server.name.clone(),
            description: server.motd.clone(),
            preference,
            region,
            on_description: false,
            render_distance: None,
            simulation_distance: None,
        }
    }
    /// The preference as `RegionSelectionPreferenceDto`: the region only for `MANUAL`.
    pub(super) fn preference_dto(&self) -> RegionSelectionPreferenceDto {
        RegionSelectionPreferenceDto {
            region_selection_preference: self.preference,
            preferred_region: self
                .region
                .clone()
                .filter(|_| self.preference == client::MANUAL),
        }
    }
}

/// `RealmsConfigureWorldScreen`'s tabs, in `MenuTabBar` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigureTab {
    Worlds,
    Players,
    Subscription,
    Settings,
}

impl ConfigureTab {
    /// The tabs in `MenuTabBar` order, which is also their index.
    pub(super) const ALL: [Self; 4] = [
        Self::Worlds,
        Self::Players,
        Self::Subscription,
        Self::Settings,
    ];
    /// `RealmsWorldsTab.TITLE` and the other tabs' own titles.
    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Worlds => "mco.configure.worlds.title",
            Self::Players => "mco.configure.world.players.title",
            Self::Subscription => "mco.configure.world.subscription.tab",
            Self::Settings => "mco.configure.world.settings.title",
        }
    }
    /// `RealmsConfigureWorldScreen.init`'s `setTabActiveState(3, !serverData.expired)`: an
    /// expired Realm's settings tab is off (with the expired tooltip).
    pub(super) fn enabled(self, expired: bool) -> bool {
        self != Self::Settings || !expired
    }
}

/// `RealmsSettingsTab.save`'s `updatedRenderDistance`/`updatedSimulationDistance`: a distance
/// is sent only when the player changed it and the tier answer lets the service configure it.
pub(super) fn changed_distance(picked: Option<i32>, range: Option<&RealmTierRange>) -> Option<i32> {
    picked.filter(|picked| range.is_some_and(|range| *picked != range.current_or_default()))
}

/// `RealmsServer.shouldPlayButtonBeActive`: open and unexpired, and this client can play
/// it — the compatibility says compatible or an upgrade, or the player owns the Realm.
pub(super) fn can_play(server: &RealmServer, me: Option<u128>) -> bool {
    server.is_open()
        && (server.compatibility == "COMPATIBLE"
            || server.compatibility == "NEEDS_UPGRADE"
            || me.is_some_and(|me| server.owned_by(me)))
}

/// `RealmsServer.McoServerComparator` through `RealmsServerList.updateServersList`:
/// uninitialized Realms first, then expired trials, then the player's own Realm, then
/// unexpired before expired, open before closed, and finally the id. (Snapshot realms are
/// not modelled: 26.3 release never lists them.)
pub(super) fn sort_servers(servers: &mut [RealmServer], owner: Option<&str>) {
    servers.sort_by(|a, b| {
        (b.state == "UNINITIALIZED")
            .cmp(&(a.state == "UNINITIALIZED"))
            .then(b.expired_trial.cmp(&a.expired_trial))
            .then((b.owner.as_deref() == owner).cmp(&(a.owner.as_deref() == owner)))
            .then(a.expired.cmp(&b.expired))
            .then((b.state == "OPEN").cmp(&(a.state == "OPEN")))
            .then(a.id.cmp(&b.id))
    });
}

/// `RealmsSettingsTab.getTranslatableFromPreference`: a manual choice shows its region,
/// the automatic ones their own name.
pub(super) fn preference_label(preference: i32, region: Option<&str>) -> String {
    let t = crate::lang::get;
    match (preference, region) {
        (client::MANUAL, Some(region)) => {
            client::region_translation_key(region).map_or_else(|| region.to_owned(), t)
        }
        (client::AUTOMATIC_PLAYER, _) => {
            t("realms.configuration.region_preference.automatic_player")
        }
        _ => t("realms.configuration.region_preference.automatic_owner"),
    }
}

/// `RealmsPreferredRegionSelectionScreen.RegionSelectionList`'s rows: automatic by player,
/// automatic by owner, then every region as a manual choice.
pub(super) fn region_rows(regions: &[RegionDataDto]) -> Vec<(i32, Option<String>)> {
    [
        (client::AUTOMATIC_PLAYER, None),
        (client::AUTOMATIC_OWNER, None),
    ]
    .into_iter()
    .chain(
        regions
            .iter()
            .map(|r| (client::MANUAL, Some(r.region.clone()))),
    )
    .collect()
}

/// The tab bar's geometry over `MenuTabBar.builder(this, width)`: the bar is at most 400
/// wide, centred, and each of the four tabs is an eighth of its inner width rounded up to an
/// even number (`Java`'s integer division before the rounding).
pub(super) fn tab_geometry(w: f32) -> (f32, f32) {
    let width = w.min(400.0) - 28.0;
    let x = (((w - width) / 2.0).floor() / 2.0).ceil() * 2.0;
    let tab = ((width / 4.0).floor() / 2.0).ceil() * 2.0;
    (x, tab)
}

/// A plain `labeledElement` cell: (9 + 4 + 20) + padding top and bottom.
/// `RealmsSettingsTab`'s row tops relative to the column's own top — name, description,
/// render distance, simulation distance, region, toggle — and the column's height. The
/// two distance rows exist only once the tier answer is there (`createDistanceOption`).
pub(super) fn settings_rows(with_tier: bool) -> ([f32; 6], f32) {
    let mut rows = [0.0; 6];
    rows[1] = SETTINGS_ROW_HEIGHT;
    if with_tier {
        rows[2] = 2.0 * SETTINGS_ROW_HEIGHT;
        rows[3] = 3.0 * SETTINGS_ROW_HEIGHT;
        rows[4] = 4.0 * SETTINGS_ROW_HEIGHT;
        rows[5] = rows[4] + SETTINGS_REGION_ROW_HEIGHT;
    } else {
        rows[4] = 2.0 * SETTINGS_ROW_HEIGHT;
        rows[5] = rows[4] + SETTINGS_REGION_ROW_HEIGHT;
    }
    (
        rows,
        rows[5] + SETTINGS_WIDGET_HEIGHT + 2.0 * SETTINGS_CELL_PADDING,
    )
}

/// `GridLayoutTab.doLayout`'s `FrameLayout.alignInRectangle(..., 0.5F, 1/6F)` inside the
/// tab area below the 24-tall `MenuTabBar` and above the 33-tall footer. Java's `int`
/// cast truncates toward zero, which matters on short windows.
pub(super) fn settings_column_top(h: f32, content_height: f32) -> f32 {
    24.0 + ((h - 24.0 - 33.0 - content_height) / 6.0).trunc()
}

/// A configuration screen with no data yet, for an error screen's `nextScreen`
/// (`RealmsConfigureWorldScreen.createErrorScreen` returns the list or the tab it came
/// from); the fetch starters run when it is restored.
pub(super) fn configure_screen(server: RealmServer, tab: ConfigureTab) -> SubScreen {
    SubScreen::Configure {
        server,
        tab,
        subscription: None,
        subscription_job: None,
        detail: None,
        detail_job: None,
        deleting: None,
        member_job: None,
        settings: None,
        players_scroll: crate::ui::Scroll::default(),
        tier: None,
        tier_job: None,
        regions: None,
        regions_job: None,
        saving: None,
        error: None,
        job: None,
    }
}
