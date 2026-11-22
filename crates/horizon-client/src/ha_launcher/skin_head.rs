//! Real account heads: the 8x8 skin faces taken from the player's own skin.
//!
//! The launcher draws the account head before the game has loaded a skin, so
//! the name goes to Mojang's public profile APIs on a background thread and the
//! decoded faces are kept for the session. A name with no Mojang profile, or one
//! wearing the default skin, takes its skin on Ely.by, where Starling's offline
//! and Ely.by players keep theirs and the game shows it (Skin Customization's
//! Ely.by Skins); with neither, the drawn Steve head.

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// The three 8x8 faces of a head, one `0xRRGGBB` pixel each, plus the second
/// skin layer over the face (`0` where the skin has no hat).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HeadSkin {
    pub front: [[u32; 8]; 8],
    pub top: [[u32; 8]; 8],
    pub side: [[u32; 8]; 8],
    pub hat: [[u32; 8]; 8],
}

static CACHE: OnceLock<Mutex<HashMap<String, Option<HeadSkin>>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<String, Option<HeadSkin>>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Put an already decoded head in the session cache, so the launcher can draw
/// the account's real skin on its first frame instead of waiting for a lookup.
pub fn remember(player: &str, skin: HeadSkin) {
    let name = player.trim();
    if name.is_empty() || name.len() > 16 || name.contains(char::is_whitespace) {
        return;
    }
    if let Ok(mut map) = cache().lock() {
        map.insert(name.to_owned(), Some(skin));
    }
}

/// The real head for `player`: `None` while the request is in flight and for
/// names with no Mojang profile (offline mode).
pub fn head(player: &str) -> Option<HeadSkin> {
    let name = player.trim();
    if name.is_empty() || name.len() > 16 || name.contains(char::is_whitespace) {
        return None;
    }
    let mut map = cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(found) = map.get(name) {
        return *found;
    }
    // The game already prepared the selected account's skin (wardrobe, cloud
    // or local file) into this launch's appearance handoff. That is the skin
    // the game will show, so it wins over a public-profile lookup.
    if let Some(skin) = prepared_skin() {
        map.insert(name.to_owned(), Some(skin));
        return Some(skin);
    }
    // One attempt per name and session: a miss is cached as a miss.
    map.insert(name.to_owned(), None);
    let wanted = name.to_owned();
    std::thread::spawn(move || {
        let found = fetch(&wanted);
        if let Ok(mut map) = cache().lock() {
            map.insert(wanted, found);
        }
    });
    None
}

/// The skin this launch will actually use, already normalized to 64x64 RGBA.
/// The launcher's process has no game appearance loaded: the wardrobe remembers its heads.
fn prepared_skin() -> Option<HeadSkin> {
    None
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(6))
        .timeout(Duration::from_secs(15))
        .build()
}

fn fetch(name: &str) -> Option<HeadSkin> {
    let agent = agent();
    let url = mojang_skin_url(&agent, name).or_else(|| ely_skin_url(&agent, name))?;
    let mut png = Vec::new();
    agent
        .get(&url)
        .call()
        .ok()?
        .into_reader()
        .read_to_end(&mut png)
        .ok()?;
    let (w, h, rgba) = crate::ha_launcher::client::hud::decode_bytes(&png).ok()?;
    from_rgba(w, h, &rgba)
}

/// `name`'s skin on Ely.by (`GET skinsystem.ely.by/textures/{name}`, 204 for a name without
/// one), on https, from Ely.by's own hosts only.
fn ely_skin_url(agent: &ureq::Agent, name: &str) -> Option<String> {
    let answer: serde_json::Value = serde_json::from_str(
        &agent
            .get(&format!("https://skinsystem.ely.by/textures/{name}"))
            .call()
            .ok()?
            .into_string()
            .ok()?,
    )
    .ok()?;
    ely_url(answer.get("SKIN")?.get("url")?.as_str()?)
}

/// An Ely.by skin URL on https, or None for another host.
fn ely_url(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split('/').next()?;
    matches!(host, "ely.by" | "skinsystem.ely.by").then(|| format!("https://{rest}"))
}

/// The skin URL of `name`'s Mojang profile; None without a profile or with the default skin.
fn mojang_skin_url(agent: &ureq::Agent, name: &str) -> Option<String> {
    let profile: serde_json::Value = serde_json::from_str(
        &agent
            .get(&format!(
                "https://api.mojang.com/users/profiles/minecraft/{name}"
            ))
            .call()
            .ok()?
            .into_string()
            .ok()?,
    )
    .ok()?;
    let id = profile.get("id")?.as_str()?;
    let session: serde_json::Value = serde_json::from_str(
        &agent
            .get(&format!(
                "https://sessionserver.mojang.com/session/minecraft/profile/{id}"
            ))
            .call()
            .ok()?
            .into_string()
            .ok()?,
    )
    .ok()?;
    let encoded = session
        .get("properties")?
        .as_array()?
        .iter()
        .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("textures"))?
        .get("value")?
        .as_str()?;
    let payload: serde_json::Value =
        serde_json::from_slice(&crate::ha_launcher::client::skins::base64_decode(encoded)?).ok()?;
    let url = payload.get("textures")?.get("SKIN")?.get("url")?.as_str()?;
    (url.starts_with("https://textures.minecraft.net/")
        || url.starts_with("http://textures.minecraft.net/"))
    .then(|| url.to_owned())
}

fn pixel(w: u32, h: u32, rgba: &[u8], x: u32, y: u32) -> Option<(u32, u8)> {
    if x >= w || y >= h {
        return None;
    }
    let i = ((y * w + x) * 4) as usize;
    let rgb = (u32::from(rgba[i]) << 16) | (u32::from(rgba[i + 1]) << 8) | u32::from(rgba[i + 2]);
    Some((rgb, rgba[i + 3]))
}

/// Decode a skin PNG into its head faces. Used by the wardrobe, whose selected
/// image is already on disk and needs no profile lookup.
pub fn from_png(bytes: &[u8]) -> Option<HeadSkin> {
    let (w, h, rgba) = crate::ha_launcher::client::hud::decode_bytes(bytes).ok()?;
    from_rgba(w, h, &rgba)
}

pub fn from_rgba(w: u32, h: u32, rgba: &[u8]) -> Option<HeadSkin> {
    // 64x32 is the legacy layout, which has no hat layer; modern skins are
    // 64x64. Anything smaller is not a Minecraft skin.
    if w < 64 || h < 32 || rgba.len() < (w as usize * h as usize * 4) {
        return None;
    }
    let mut skin = HeadSkin {
        front: [[0; 8]; 8],
        top: [[0; 8]; 8],
        side: [[0; 8]; 8],
        hat: [[0; 8]; 8],
    };
    for row in 0..8u32 {
        for col in 0..8u32 {
            skin.front[row as usize][col as usize] = pixel(w, h, rgba, 8 + col, 8 + row)?.0;
            skin.top[row as usize][col as usize] = pixel(w, h, rgba, 8 + col, row)?.0;
            skin.side[row as usize][col as usize] = pixel(w, h, rgba, col, 8 + row)?.0;
            if h >= 64 {
                let (rgb, alpha) = pixel(w, h, rgba, 40 + col, 8 + row)?;
                if alpha > 0 {
                    skin.hat[row as usize][col as usize] = (u32::from(alpha) << 24) | rgb;
                }
            }
        }
    }
    Some(skin)
}

/// Horizon Android's theme drew the account's head (`Ui::avatar_head_2d`); Starling's copy of the
/// theme (`android_control_theme`) left it out with the launcher.
pub trait AvatarHead {
    fn avatar_head_2d(&mut self, x: f32, y: f32, size: f32, player_name: &str);
}

impl AvatarHead for crate::ha_launcher::client::ui_theme::Ui<'_> {
    fn avatar_head_2d(&mut self, x: f32, y: f32, size: f32, player_name: &str) {
        use crate::ha_launcher::client::ui_theme::{active_theme, color};
        let theme = active_theme();
        let radius = size * 0.30;
        self.rounded(x, y, size, size, radius, theme.surface_dim);
        let inset = (size * 0.07).max(1.0);
        let face = (size - inset * 2.0).max(8.0);
        let pw = face / 8.0;
        if let Some(skin) = head(player_name) {
            for row in 0..8 {
                for col in 0..8 {
                    let px = x + inset + col as f32 * pw;
                    let py = y + inset + row as f32 * pw;
                    self.fill(
                        px,
                        py,
                        pw + 0.4,
                        pw + 0.4,
                        color::hex(skin.front[row][col], 255),
                    );
                    let hat = skin.hat[row][col];
                    if hat >> 24 != 0 {
                        self.fill(
                            px,
                            py,
                            pw + 0.4,
                            pw + 0.4,
                            color::hex(hat & 0xFFFFFF, (hat >> 24) as u8),
                        );
                    }
                }
            }
        } else {
            const HAIR: u32 = 0x3B2A1B;
            const SKIN: u32 = 0xC68E5E;
            const EYE: u32 = 0xF4F4F4;
            const IRIS: u32 = 0x4E6E9E;
            const NOSE: u32 = 0xA87548;
            const MOUTH: u32 = 0x6B4A32;
            const BEARD: u32 = 0x2E2117;
            const FACE: [[u32; 8]; 8] = [
                [HAIR, HAIR, HAIR, HAIR, HAIR, HAIR, HAIR, HAIR],
                [HAIR, HAIR, HAIR, HAIR, HAIR, HAIR, HAIR, HAIR],
                [HAIR, HAIR, HAIR, HAIR, HAIR, HAIR, HAIR, HAIR],
                [HAIR, SKIN, SKIN, SKIN, SKIN, SKIN, SKIN, HAIR],
                [SKIN, EYE, IRIS, SKIN, SKIN, IRIS, EYE, SKIN],
                [SKIN, SKIN, SKIN, NOSE, NOSE, SKIN, SKIN, SKIN],
                [SKIN, MOUTH, MOUTH, MOUTH, MOUTH, MOUTH, MOUTH, SKIN],
                [SKIN, BEARD, BEARD, BEARD, BEARD, BEARD, BEARD, SKIN],
            ];
            for (row, pixels) in FACE.iter().enumerate() {
                for (col, tone) in pixels.iter().enumerate() {
                    self.fill(
                        x + inset + col as f32 * pw,
                        y + inset + row as f32 * pw,
                        pw + 0.4,
                        pw + 0.4,
                        color::hex(*tone, 255),
                    );
                }
            }
        }
        self.border(x, y, size, size, radius, theme.border_strong);
    }
}

#[cfg(test)]
mod tests {
    use super::ely_url;

    #[test]
    fn ely_skins_come_on_https_from_ely_hosts_only() {
        assert_eq!(
            ely_url("http://ely.by/storage/skins/a.png").as_deref(),
            Some("https://ely.by/storage/skins/a.png")
        );
        assert_eq!(
            ely_url("https://skinsystem.ely.by/x.png").as_deref(),
            Some("https://skinsystem.ely.by/x.png")
        );
        assert_eq!(ely_url("https://example.com/ely.by/a.png"), None);
        assert_eq!(ely_url("https://ely.by.example.com/a.png"), None);
        assert_eq!(ely_url("https://token@ely.by/a.png"), None);
        assert_eq!(ely_url("ftp://ely.by/a.png"), None);
    }
}
