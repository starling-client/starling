//! `Screen`'s presentation helpers: dates, ages, list rows, wrapped text and layout.

use super::LayoutState;
use super::client::Backup;
use crate::hud::{HudAtlas, HudBatch};
use std::time::{SystemTime, UNIX_EPOCH};
use sunrise_core::{java_number::format_fixed, jmath};

/// `RealmsSlotOptionsScreen.DIFFICULTIES` and `GAME_MODES`, in menu order (the index
/// is what the service stores).
pub(super) const DIFFICULTY_DEFAULT: usize = 2;
pub(super) const GAME_MODE_DEFAULT: usize = 0;
pub(super) const DIFFICULTIES: [&str; 4] = [
    "options.difficulty.peaceful",
    "options.difficulty.easy",
    "options.difficulty.normal",
    "options.difficulty.hard",
];
pub(super) const GAME_MODES: [&str; 3] = [
    "selectWorld.gameMode.survival",
    "selectWorld.gameMode.creative",
    "selectWorld.gameMode.adventure",
];

/// `InviteCodeExpiration`: the seven choices, the last one never expiring.
pub(super) const EXPIRATIONS: [(&str, Option<i64>); 7] = [
    (
        "mco.configure.world.invite_codes.expiration.30_minutes",
        Some(30 * 60),
    ),
    (
        "mco.configure.world.invite_codes.expiration.1_hour",
        Some(60 * 60),
    ),
    (
        "mco.configure.world.invite_codes.expiration.6_hours",
        Some(6 * 60 * 60),
    ),
    (
        "mco.configure.world.invite_codes.expiration.12_hours",
        Some(12 * 60 * 60),
    ),
    (
        "mco.configure.world.invite_codes.expiration.1_day",
        Some(24 * 60 * 60),
    ),
    (
        "mco.configure.world.invite_codes.expiration.7_days",
        Some(7 * 24 * 60 * 60),
    ),
    ("mco.configure.world.invite_codes.expiration.never", None),
];

/// `InviteCodeExpiration.closestTo`: the choice nearest the time left on the code.
pub(super) fn closest_expiration(expiration_millis: i64) -> usize {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let left = (expiration_millis - now).max(0) / 1000;
    let mut best = EXPIRATIONS.len() - 1;
    let mut best_delta = i64::MAX;
    for (i, (_, seconds)) in EXPIRATIONS.iter().enumerate() {
        let Some(seconds) = seconds else { continue };
        let delta = (left - seconds).abs();
        if delta < best_delta {
            best_delta = delta;
            best = i;
        }
    }
    best
}

/// `RealmsSubscriptionTab.daysLeftPresentation`: what the days-left line says, in the
/// caller's language; empty when nothing is left to say.
pub(super) fn days_left_text(days_left: i32, expired: bool) -> String {
    let t = crate::lang::get;
    if days_left < 0 && expired {
        return t("mco.configure.world.subscription.expired");
    }
    if days_left <= 1 {
        return t("mco.configure.world.subscription.less_than_a_day");
    }
    let (months, days) = (days_left / 30, days_left % 30);
    match (months > 0, days > 0) {
        (true, true) => crate::lang::tr(
            "mco.configure.world.subscription.remaining.months.days",
            &[&months, &days],
        ),
        (true, false) => crate::lang::tr(
            "mco.configure.world.subscription.remaining.months",
            &[&months],
        ),
        (false, true) => {
            crate::lang::tr("mco.configure.world.subscription.remaining.days", &[&days])
        }
        (false, false) => String::new(),
    }
}

/// `RealmsUtil.convertToAgePresentationFromInstant`: "now", seconds, minutes, hours or days
/// ago, from an epoch-millisecond stamp. The clock is the wall clock, not the animation
/// clock `hud_overlays::now_ms` (that one counts from the process start).
pub(super) fn age_presentation(epoch_millis: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let diff = now - epoch_millis;
    if diff < 0 {
        return crate::lang::get("mco.time.now");
    }
    let seconds = diff / 1000;
    let key = if seconds < 60 {
        ("mco.time.secondsAgo", seconds)
    } else if seconds < 3600 {
        ("mco.time.minutesAgo", seconds / 60)
    } else if seconds < 86400 {
        ("mco.time.hoursAgo", seconds / 3600)
    } else {
        ("mco.time.daysAgo", seconds / 86400)
    };
    crate::lang::tr(key.0, &[&key.1 as &dyn std::fmt::Display])
}

/// `RealmsBackupScreen.Entry.populateChangeList`: each backup's metadata keys that differ
/// from the older backup (or are missing there); the last backup has none. An `uploaded`
/// key's value is the backup's short date.
pub(super) fn backup_changes(backups: &[Backup]) -> Vec<std::collections::HashMap<String, String>> {
    let mut out = Vec::with_capacity(backups.len());
    for (index, backup) in backups.iter().enumerate() {
        let mut changes = std::collections::HashMap::new();
        if index + 1 < backups.len() {
            let older = &backups[index + 1];
            for (key, value) in &backup.metadata {
                if let Some(old) = older.metadata.get(key)
                    && !key.contains("uploaded")
                    && old == value
                {
                    continue;
                }
                if key.contains("uploaded") {
                    changes.insert(key.clone(), short_date_text(backup.last_modified));
                } else {
                    changes.insert(key.clone(), value.clone());
                }
            }
        }
        out.push(changes);
    }
    out
}

/// `RealmsBackupInfoScreen.BackupInfoListEntry.translateKey`.
pub(super) fn backup_info_key(key: &str) -> String {
    let t = crate::lang::get;
    let candidate = match key {
        "template_name" => "mco.backup.entry.templateName",
        "game_difficulty" => "mco.backup.entry.gameDifficulty",
        "name" => "mco.backup.entry.name",
        "game_server_version" => "mco.backup.entry.gameServerVersion",
        "uploaded" => "mco.backup.entry.uploaded",
        "enabled_packs" => "mco.backup.entry.enabledPack",
        "description" => "mco.backup.entry.description",
        "game_mode" => "mco.backup.entry.gameMode",
        "seed" => "mco.backup.entry.seed",
        "world_type" => "mco.backup.entry.worldType",
        _ => "mco.backup.entry.undefined",
    };
    t(candidate)
}

/// `RealmsBackupInfoScreen.checkForSpecificMetadata`: game modes and difficulties by the
/// index the service stored, world types by their name; everything else literal.
pub(super) fn backup_info_value(key: &str, value: &str) -> String {
    let t = crate::lang::get;
    let lower = key.to_lowercase();
    if lower.contains("game") && lower.contains("mode") {
        return value
            .parse::<usize>()
            .ok()
            .and_then(|index| GAME_MODES.get(index))
            .map(|key| t(key))
            .unwrap_or_else(|| t("mco.backup.unknown"));
    }
    if lower.contains("game") && lower.contains("difficulty") {
        return value
            .parse::<usize>()
            .ok()
            .and_then(|index| DIFFICULTIES.get(index))
            .map(|key| t(key))
            .unwrap_or_else(|| t("mco.backup.unknown"));
    }
    if key == "world_type" {
        let name = match value.to_uppercase().as_str() {
            "NORMAL" => "normal",
            "MINIGAME" => "minigame",
            "ADVENTUREMAP" => "adventureMap",
            "EXPERIENCE" => "experience",
            "INSPIRATION" => "inspiration",
            _ => return t("mco.backup.entry.worldType.unknown"),
        };
        return t(&format!("mco.backup.entry.worldType.{name}"));
    }
    value.to_owned()
}

/// `RealmsSubscriptionTab.localPresentation`: the start date in the player's own local
/// time. The process has no locale to format with, so the shape is fixed (`%Y-%m-%d`)
/// where vanilla asks for the locale's medium date.
/// `Util.localizedDateFormatter(FormatStyle.MEDIUM)` — `DateTimeFormatter
/// .ofLocalizedDateTime`: each locale's medium date-time shape. The port has no
/// locale database, so the language's shape comes from this table (zh's medium
/// date-time is `2026年11月6日 12:16:04`), with an ISO fallback.
pub(super) fn start_date_text(epoch_millis: i64) -> String {
    let Some(time) = chrono::DateTime::from_timestamp_millis(epoch_millis) else {
        return String::new();
    };
    let time = time.with_timezone(&chrono::Local);
    let format = match crate::lang::code().as_str() {
        "zh_cn" | "zh_tw" | "lzh" => "%Y年%-m月%-d日 %H:%M:%S",
        "ja_jp" => "%Y/%-m/%-d %H:%M:%S",
        _ => "%Y-%m-%d %H:%M:%S",
    };
    time.format(format).to_string()
}

/// `RealmsEditInviteCodeScreen.DATE_FORMAT`: `Util.localizedDateFormatter(SHORT)`. The
/// port has no locale database, so each language's short shape is spelled out with an
/// ISO fallback (vanilla itself formats per locale).
pub(super) fn short_date_text(epoch_millis: i64) -> String {
    let Some(time) = chrono::DateTime::from_timestamp_millis(epoch_millis) else {
        return String::new();
    };
    let time = time.with_timezone(&chrono::Local);
    let format = match crate::lang::code().as_str() {
        "zh_cn" | "zh_tw" | "lzh" | "ja_jp" => "%Y/%-m/%-d %H:%M",
        "en_us" => "%-m/%-d/%y %-I:%M %p",
        _ => "%Y-%m-%d %H:%M",
    };
    time.format(format).to_string()
}

/// One 200-wide column line of the subscription tab.
pub(super) fn subscription_label(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    x: f32,
    y: f32,
    text: &str,
    color: u32,
) {
    batch.text(atlas, text, x, y, color, true);
}

/// `RealmsPdpScreen`'s list: the trial header (no picture) and the three features, each
/// with its 80x45 picture and the text beside it. The header's subtitle
/// (`mco.pdp.trial.description`, with `icon/trial_available`) is shown only when the service
/// says a trial is available (`trialAvailabilityTask`).
pub(super) fn pdp_entries() -> [(&'static str, &'static str, &'static [&'static str]); 4] {
    [
        ("", "mco.pdp.trial.title", &["mco.pdp.trial.description"]),
        (
            "gui/realms/friends",
            "mco.pdp.friends.title",
            &["mco.pdp.friends.description"],
        ),
        (
            "gui/realms/minigames",
            "mco.pdp.minigames.title",
            &["mco.pdp.minigames.description"],
        ),
        (
            "gui/realms/private_server",
            "mco.pdp.private.title",
            &[
                "mco.pdp.private.backups",
                "mco.pdp.private.easyToManage",
                "mco.pdp.private.worldSlots",
                "mco.pdp.private.secure",
            ],
        ),
    ]
}

/// `RealmsPdpScreen.AbstractPdpEntry.getNarration`'s lines: the title and its descriptions,
/// a line each (`CommonComponents.joinLines`), the trial's only with a trial available.
pub(super) fn pdp_row_lines(row: usize, trial: bool) -> Vec<String> {
    let (_, title, descriptions) = pdp_entries()[row];
    let mut lines = vec![crate::lang::get(title)];
    // `PdpList`'s header carries the trial description only with a trial available.
    if title != "mco.pdp.trial.title" || trial {
        lines.extend(descriptions.iter().map(|key| crate::lang::get(key)));
    }
    lines
}

/// `RealmsPdpScreen.AbstractPdpEntry.getNarration`: `narrator.select` of those lines.
pub(super) fn pdp_row_narration(row: usize, trial: bool) -> String {
    crate::screen_narrator::translate("narrator.select", &[&pdp_row_lines(row, trial).join("\n")])
}

pub(super) fn wrap_lines(text: &str, atlas: &HudAtlas, width: f32) -> Vec<String> {
    crate::multiplayer::wrap(text, width, atlas)
}

pub(super) fn centered(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    text: &str,
    cx: f32,
    y: f32,
    color: u32,
) {
    let w = HudBatch::text_width(atlas, text);
    batch.text(atlas, text, cx - (w / 2.0).floor(), y, color, true);
}

/// `RealmsResetWorldScreen.init`'s grid: three 60-wide tiles with 16 padding a side, and a
/// 16-tall spacer between the two rows (`FrameButton` is 60 wide and 69 tall: the button
/// plus its 9-tall label). The contents frame starts at `min(63, h - 187)`.
pub(super) fn reset_content_y(h: f32) -> f32 {
    63.0_f32.min(h - 187.0)
}

pub(super) fn reset_tile(cx: f32, top: f32, i: usize) -> (f32, f32) {
    let column = (i % 3) as f32;
    let row = (i / 3) as f32;
    (cx - 122.0 + column * 92.0, top + row * 85.0)
}

/// `Unit.humanReadable`: binary units without carrying a rounded 1024.0 to
/// the next unit. Also used by the server resource-pack download toast.
pub fn human_readable(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    byte_size(bytes, unit_exponent(bytes), 1)
}

/// `Unit.getLargest` has no units above GB, and the size limit uses a decimal
/// place only for GB (normal transfer rates always use one).
pub(super) fn upload_limit_size(bytes: u64) -> String {
    let exp = unit_exponent(bytes).min(3);
    byte_size(bytes, exp, usize::from(exp == 3))
}

fn unit_exponent(bytes: u64) -> u32 {
    if bytes < 1024 {
        0
    } else {
        (jmath::log(bytes as f64) / jmath::log(1024.0)) as u32
    }
}

fn byte_size(bytes: u64, exp: u32, decimals: usize) -> String {
    let unit = ["B", "KB", "MB", "GB", "TB", "PB", "EB"][exp as usize];
    let value = bytes as f64 / ((1u64 << (10 * exp)) as f64);
    format!("{} {unit}", format_fixed(value, decimals))
}

pub(super) fn transfer_percentage(written: u64, total: u64) -> f64 {
    let ratio = written as f64 / total as f64;
    // Java's Math.min retains NaN for an empty transfer (0 / 0).
    if ratio > 1.0 { 1.0 } else { ratio }
}

pub(super) fn transfer_bar_width(percentage: f64) -> f32 {
    jmath::round(200.0 * percentage) as f32
}

pub(super) fn percent_label(percentage: f64) -> String {
    format_fixed(percentage * 100.0, 1)
}

pub(super) fn loading_dots() -> &'static str {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    ["O o o", "o O o", "o o O", "o O o"][(millis / 300 % 4) as usize]
}

/// `Entry.extractClampedString`: the string cut to `max` pixels — vanilla cuts with
/// `plainSubstrByWidth` into the space `width("... ")` leaves and appends the dots.
pub(super) fn clamp_text(atlas: &HudAtlas, text: &str, max: f32) -> String {
    if max <= 0.0 || HudBatch::text_width(atlas, text) <= max {
        return text.to_owned();
    }
    let room = max - HudBatch::text_width(atlas, "... ");
    let mut cut = String::new();
    for ch in text.chars() {
        let mut candidate = cut.clone();
        candidate.push(ch);
        if HudBatch::text_width(atlas, &candidate) > room {
            break;
        }
        cut = candidate;
    }
    format!("{cut}...")
}

/// `RealmsScreen.realmsLogo`: `ImageWidget.texture(128, 34, LOGO_LOCATION, 128, 64)` —
/// the top half of `textures/gui/title/realms.png` at 128×34.
pub(super) fn realms_logo(batch: &mut HudBatch, atlas: &HudAtlas, cx: f32, y: f32) {
    batch.sprite_region(
        atlas,
        "gui/title/realms",
        [cx - 64.0, y, 128.0, 34.0],
        128.0,
        64.0,
    );
}

/// `RealmsMainScreen.updateLayout`'s footer height: the two-row grid (44) plus its 22
/// of padding when the list shows, the single row's 20 otherwise. `FrameLayout` centres
/// the grid, so the rows sit 11 into the area - LIST at h-55/h-31, otherwise h-31.
pub(super) fn main_footer_h(state: LayoutState) -> f32 {
    if state == LayoutState::List {
        66.0
    } else {
        42.0
    }
}

/// `RealmsLongRunningMcoTaskScreen`'s centered stack (`FrameLayout.centerInRectangle`):
/// the logo 34 tall, the 28-tall `LoadingDotsWidget` with 10 above and 30 below
/// (`paddingTop(10).paddingBottom(30)`), then the 150×20 Cancel button — 122 in all.
pub(super) fn task_stack_top((_, h): (f32, f32)) -> f32 {
    ((h - 122.0) / 2.0).floor()
}
