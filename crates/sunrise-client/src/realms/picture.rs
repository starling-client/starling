//! Slot and template names, and the pictures Realms uploads into the atlas.

use super::client::RealmWorldOptions;

/// `RealmsSettingsTab.isRealmNameValid`: a name that trims to something and has no
/// leading or trailing whitespace.
pub(super) fn realm_name_valid(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty() && name.len() == trimmed.len()
}

/// `RealmsWorldOptions.getSlotName`: the player's own name, else the empty marker, else
/// "World N" by the slot's number.
pub(super) fn slot_name(options: &RealmWorldOptions, slot: i32) -> String {
    if !options.slot_name.trim().is_empty() {
        return options.slot_name.clone();
    }
    if options.empty {
        return crate::lang::get("mco.configure.world.slot.empty");
    }
    crate::lang::tr("mco.configure.world.slot", &[&slot])
}

/// `TextRenderingUtils.decompose(I18n.get("mco.template.select.none", "%link"), link)`:
/// the translated message's lines, the one holding the `%s` argument split into
/// `before`/`link`/`after`; every other line is whole (`link` is None).
pub(super) fn template_none_lines(text: &str, link: &str) -> Vec<(String, Option<String>, String)> {
    text.replacen("%s", "%link", 1)
        .split('\n')
        .map(|line| match line.split_once("%link") {
            Some((before, after)) => (before.to_owned(), Some(link.to_owned()), after.to_owned()),
            None => (line.to_owned(), None, String::new()),
        })
        .collect()
}

pub(super) fn resample_icon(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return None;
    }
    let mut out = vec![0u8; 64 * 64 * 4];
    for y in 0..64 {
        for x in 0..64 {
            let (sx, sy) = (x * w / 64, y * h / 64);
            let source = (sy * w + sx) * 4;
            let target = (y * 64 + x) * 4;
            out[target..target + 4].copy_from_slice(&rgba[source..source + 4]);
        }
    }
    Some(out)
}

/// `PlayerFaceExtractor`/`RealmsUtil.extractPlayerFace`: the head's front 8x8 texels at
/// (8, 8) with the hat layer (40, 8) over them, scaled to the 64x64 slot.
pub(super) fn face_from_skin(skin: &[u8]) -> Option<Vec<u8>> {
    if skin.len() < 64 * 64 * 4 {
        return None;
    }
    let mut out = vec![0u8; 64 * 64 * 4];
    for y in 0..8 {
        for x in 0..8 {
            let base = ((8 + y) * 64 + 8 + x) * 4;
            let hat = ((8 + y) * 64 + 40 + x) * 4;
            let mut texel = [skin[base], skin[base + 1], skin[base + 2], skin[base + 3]];
            let hat_alpha = skin[hat + 3] as u16;
            if hat_alpha > 0 {
                for channel in 0..3 {
                    texel[channel] = ((skin[hat + channel] as u16 * hat_alpha
                        + texel[channel] as u16 * (255 - hat_alpha))
                        / 255) as u8;
                }
                texel[3] = 255;
            }
            for dy in 0..8 {
                for dx in 0..8 {
                    let target = ((y * 8 + dy) * 64 + x * 8 + dx) * 4;
                    out[target..target + 4].copy_from_slice(&texel);
                }
            }
        }
    }
    Some(out)
}
