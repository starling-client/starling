//! Starling's part of control.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// `sunrise-ctl account ...`: Starling's Microsoft account (`accounts`). The status never holds
/// a token: the name, the device code the player types at the link, the game files' progress.
pub(super) fn account_control(words: &[&str]) -> Result<Option<Value>> {
    use sunrise_client::accounts::{self, Files, State};
    if words[0] != "account" {
        return Ok(None);
    }
    match words.get(1).copied() {
        Some("sign-in") => accounts::sign_in(),
        Some("cancel") => accounts::cancel(),
        Some("sign-out") => accounts::sign_out()?,
        Some("files") => accounts::download_files(),
        Some("stop-files") => accounts::cancel_download(),
        Some("status") => {}
        _ => return Err("account status|sign-in|cancel|sign-out|files|stop-files".into()),
    }
    let mut out = match accounts::state() {
        State::Offline => json!({ "state": "offline" }),
        State::OfflineProfile(name) => json!({ "state": "offline", "name": name }),
        State::Resuming => json!({ "state": "resuming" }),
        State::Starting => json!({ "state": "starting" }),
        State::Progress(stage) => json!({ "state": "progress", "stage": stage.key() }),
        State::Code { code, link } => json!({ "state": "code", "code": code, "link": link }),
        State::SignedIn(account) => {
            json!({ "state": "signed-in", "name": account.name, "uuid": account.uuid })
        }
        State::Failed(reason) => json!({ "state": "failed", "reason": reason }),
    };
    out["in_game"] = accounts::in_game().into();
    out["online"] = sunrise_client::auth::current().is_some().into();
    out["files"] = match accounts::files() {
        Files::Missing => json!({ "state": "missing" }),
        Files::Checking => json!({ "state": "checking" }),
        Files::Downloading(p) => json!({
            "state": "downloading",
            "files": p.files,
            "files_total": p.files_total,
            "bytes": p.bytes,
            "bytes_total": p.bytes_total,
        }),
        Files::Ready => json!({ "state": "ready", "root": accounts::files_root() }),
        Files::Failed(reason) => json!({ "state": "failed", "reason": reason }),
    };
    Ok(Some(out))
}
