use super::*;

/// The textures the enabled packs' Custom Item Textures rules draw, beyond those already packed:
/// item rules' textures and their JSON models', armour and elytra rules' textures, under their
/// `cit_assets` ids, with their flags and whether they animate. An animated strip keeps its
/// first frame; a still texture is packed whole. None without `optifine/cit` rules.
pub(super) fn cit_sprites(
    root: &ResourceSource,
    packed: &[PackedImage],
) -> Vec<(PackedImage, Option<TextureFlags>, bool)> {
    let have: HashSet<&str> = packed.iter().map(|i| i.0.as_str()).collect();
    let rules = crate::cit::load_rules(root);
    let mut ids = crate::cit_assets::item_sprites(&rules);
    if !rules.is_empty() {
        let models = crate::cit_assets::load_models(root, &rules);
        ids.extend(crate::cit_assets::model_sprites(&models));
        ids.extend(crate::cit_armor::armor_sprites(&rules));
    }
    let ids: Vec<String> = ids
        .into_iter()
        // Vanilla's sprites are packed without their namespace, as `sprite_name` looks them up;
        // Sunrise's own (`sunrise/white`, the run-time slots) are added after, and stay its own.
        .filter(|id| {
            let name = id.strip_prefix("minecraft:").unwrap_or(id);
            !have.contains(name) && !name.starts_with("sunrise/")
        })
        .collect();
    // No more than the atlas could ever hold, by their headers, so a pack naming thousands of
    // big textures is cut short before they are decoded, not after. Each is charged what it
    // can be packed as (a strip's frame, or a still image whole), 1024x512 at most since
    // `cit_sprite` refuses more, with a gutter;
    // the glyph rows and the shelves' waste (a tenth) are kept aside.
    let side = u64::from(MAX_ATLAS_SIZE);
    let used: u64 = packed
        .iter()
        .map(|i| u64::from(i.1 + 1) * u64::from(i.2 + 1))
        .sum();
    let mut left = (side * side * 9 / 10).saturating_sub(side * u64::from(GLYPH_ROWS) + used);
    let before = ids.len();
    let ids: Vec<String> = ids
        .into_iter()
        .filter(|id| {
            let path = crate::atlas_sources::texture_resource(id, "textures");
            let Some((w, h)) = png_size(root, &path) else {
                return true;
            };
            let (w, h) = (u64::from(w), u64::from(h));
            if w * h > 1024 * 512 * 64 {
                // Refused by `cit_sprite` without decoding: costs nothing.
                return true;
            }
            // A strip's frame is at most its width squared; a still image is all of it.
            let frame = (w * h).min(w * w).min(1024 * 512) + w + h + 1;
            match left.checked_sub(frame) {
                Some(rest) => {
                    left = rest;
                    true
                }
                None => false,
            }
        })
        .collect();
    if ids.len() < before {
        log::warn!(
            "left out {} custom item textures: more than the atlas holds",
            before - ids.len()
        );
    }
    // Decoded on the threads `pack` decodes the rest on: a big pack names thousands.
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let chunk = ids.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let workers: Vec<_> = ids
            .chunks(chunk)
            .map(|ids| scope.spawn(|| ids.iter().filter_map(|id| cit_sprite(root, id)).collect()))
            .collect();
        workers
            .into_iter()
            .flat_map(|w| {
                w.join().unwrap_or_else(|_| {
                    log::warn!("a custom item texture decoder panicked; its textures are left out");
                    Vec::new()
                })
            })
            .collect()
    })
}
/// One of `cit_sprites`, or `None` (with a warning) when it does not load or is too big.
pub(super) fn cit_sprite(
    root: &ResourceSource,
    id: &str,
) -> Option<(PackedImage, Option<TextureFlags>, bool)> {
    let name = id.strip_prefix("minecraft:").unwrap_or(id);
    let path = crate::atlas_sources::texture_resource(id, "textures");
    // The limit `pack` keeps every other texture to, for a frame; and from the header first,
    // so an image bigger than 64 such frames is never decoded.
    let fits = |w: u32, h: u32| w <= 1024 && h <= 512;
    if let Some((w, h)) = png_size(root, &path)
        && u64::from(w) * u64::from(h) > 1024 * 512 * 64
    {
        log::warn!("skipped custom item texture {}: {w}x{h}", path);
        return None;
    }
    let image = match decode_resource(root, &path) {
        Ok(image) => image,
        Err(e) => {
            log::warn!("skipped custom item texture {}: {e}", path);
            return None;
        }
    };
    let mcmeta = root.read_to_string(&format!("{path}.mcmeta")).ok();
    let animation = animation_of(mcmeta.as_deref());
    // Only an animation is cut to its first frame: a still CIT texture is drawn whole, and
    // armour layers are twice as wide as they are tall.
    let (w, h, pixels) = match &animation {
        Some(animation) => match first_frame(image, Some(animation)) {
            Ok(image) => image,
            Err(e) => {
                log::warn!("skipped custom item texture {}: {e}", path);
                return None;
            }
        },
        None => image,
    };
    if !fits(w, h) {
        log::warn!("skipped custom item texture {}: {w}x{h}", path);
        return None;
    }
    let flags = mcmeta.as_deref().and_then(TextureFlags::from_mcmeta);
    Some(((name.to_owned(), w, h, pixels), flags, animation.is_some()))
}
