//! Vanilla item-atlas coordinates for GUI glint; the HUD's unrelated packing cannot supply them.
use crate::{atlas_sources, hud::Rect};
use std::collections::HashMap;
use sunrise_assets::animation::AnimationMetadata;
use sunrise_assets::resource_source::ResourceSource;

type Entry = (String, u32, u32);

/// `SpriteLoader.Preparations` for one atlas: stitched regions kept separate from missing fallback.
#[derive(Clone, Debug, Default)]
pub(super) struct GlintAtlas {
    regions: HashMap<String, Rect>,
    missing: Option<Rect>,
}

impl GlintAtlas {
    /// `SpriteLoader.Preparations.getSprite`: only sprites that actually stitched.
    pub(super) fn get_stitched(&self, name: &str) -> Option<Rect> {
        self.regions.get(name).copied()
    }

    /// `SpriteLoader.Preparations.missing`: this atlas's `missingno` sprite.
    pub(super) fn missing(&self) -> Option<Rect> {
        self.missing
    }

    /// `TextureAtlas.getSprite` / `SpriteGetter`: stitched sprite, else this atlas's `missingno`.
    pub(super) fn get_or_missing(&self, name: &str) -> Option<Rect> {
        self.get_stitched(name).or(self.missing)
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.regions.len()
    }
}

/// `SpriteLoader.stitch` for any atlas (`items`, `blocks`, `shield_patterns`) with vanilla mipmap
/// clamping and anisotropic padding (`(1 << mip_level) << aniso_shift`).
pub(super) fn load_atlas(
    root: &ResourceSource,
    atlas_id: &str,
    max_mip: u32,
    aniso_shift: u32,
    maximum: u32,
) -> crate::Result<GlintAtlas> {
    let entries = collect_entries(root, atlas_id);
    let mip_level = mip_level_for(&entries, max_mip);
    let padding = (1u32 << mip_level) << aniso_shift;
    finish_atlas(atlas_id, &entries, mip_level, padding, maximum)
}

fn collect_entries(root: &ResourceSource, atlas_id: &str) -> Vec<Entry> {
    let listing = atlas_sources::listing(root);
    let listed = listing.sprites(atlas_id);
    let decode = |name: &str, directory: &str| {
        crate::hud::decode_bytes(
            &root
                .read(&atlas_sources::texture_resource(name, directory))
                .ok()?,
        )
        .ok()
    };
    let mut entries = vec![("minecraft:missingno".into(), 16, 16)];
    for (id, origin) in listed {
        let dimensions = match &origin {
            atlas_sources::SpriteOrigin::File(texture) => (|| {
                let (w, h, _) = decode(texture, "textures")?;
                let path = format!(
                    "{}.mcmeta",
                    atlas_sources::texture_resource(texture, "textures")
                );
                let metadata = match root.read(&path) {
                    Ok(bytes) => Some(crate::packs::first_json::<serde_json::Value>(&bytes).ok()?),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
                    Err(_) => return None,
                };
                if let Some(metadata) = &metadata {
                    metadata.as_object()?;
                    if let Some(texture) = metadata.get("texture") {
                        texture_metadata(texture)?;
                    }
                }
                let size = match metadata.as_ref().and_then(|v| v.get("animation")) {
                    Some(animation) => AnimationMetadata::from_json(&animation.to_string())
                        .ok()?
                        .frame_size(w as i32, h as i32)
                        .map(|x| x as u32),
                    None => [w, h],
                };
                (size[0] > 0
                    && size[1] > 0
                    && w.is_multiple_of(size[0])
                    && h.is_multiple_of(size[1]))
                .then_some((size[0], size[1]))
            })(),
            // Generated sprites have empty resource metadata, so their whole image is one frame.
            _ => atlas_sources::generate(origin, &decode).map(|(w, h, _)| (w, h)),
        };
        if let Some((w, h)) = dimensions {
            entries.push((id.clone(), w, h));
        }
    }
    entries
}

fn finish_atlas(
    atlas_id: &str,
    entries: &[Entry],
    mip_level: u32,
    padding: u32,
    maximum: u32,
) -> crate::Result<GlintAtlas> {
    let regions = stitch(entries, mip_level, padding, maximum).ok_or_else(|| {
        std::io::Error::other(format!(
            "{atlas_id} atlas does not fit texture limit {maximum} with padding {padding}"
        ))
    })?;
    let missing = regions.get("missingno").copied();
    Ok(GlintAtlas { regions, missing })
}

/// `SpriteLoader.stitch`: reduce `max_mip` to the smallest sprite side and lowest power-of-two bit.
fn mip_level_for(entries: &[Entry], max_mip: u32) -> u32 {
    let mut min_texel = i32::MAX as u32;
    let mut lowest_bit = 1_i32.wrapping_shl(max_mip);
    for &(_, w, h) in entries {
        min_texel = min_texel.min(w.min(h));
        let low_w = (w as i32) & (w as i32).wrapping_neg();
        let low_h = (h as i32) & (h as i32).wrapping_neg();
        lowest_bit = lowest_bit.min(low_w.min(low_h));
    }
    let min_size = (min_texel as i32).min(lowest_bit);
    if min_size <= 0 {
        0
    } else {
        (min_size as u32).ilog2().min(max_mip)
    }
}

/// `Stitcher.smallestFittingMinTexel`: round `input` up to a multiple of `1 << mip_level`.
fn smallest_fitting_min_texel(input: u32, mip_level: u32) -> u32 {
    ((input >> mip_level) + u32::from((input & ((1u32 << mip_level) - 1)) != 0)) << mip_level
}

/// `TextureMetadataSection.CODEC`: invalid fields reject the sprite, even without mipmaps.
fn texture_metadata(section: &serde_json::Value) -> Option<()> {
    let fields = section.as_object()?;
    for (name, value) in fields.iter().filter(|(_, value)| !value.is_null()) {
        let valid = match name.as_str() {
            "blur" | "clamp" => value.is_boolean(),
            "mipmap_strategy" => matches!(
                value.as_str(),
                Some("auto" | "mean" | "cutout" | "strict_cutout" | "dark_cutout")
            ),
            "alpha_cutoff_bias" => value.is_number(),
            _ => true,
        };
        if !valid {
            return None;
        }
    }
    Some(())
}

#[derive(Default)]
struct Region {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    entry: Option<usize>,
    children: Vec<Region>,
}
impl Region {
    fn new(x: u32, y: u32, w: u32, h: u32) -> Self {
        Self {
            x,
            y,
            w,
            h,
            ..Self::default()
        }
    }
    /// `Stitcher.Region.add`: first fitting child, with vanilla's split order and no rotation.
    fn add(&mut self, index: usize, w: u32, h: u32) -> bool {
        if self.entry.is_some() || w > self.w || h > self.h {
            return false;
        }
        if w == self.w && h == self.h {
            self.entry = Some(index);
            return true;
        }
        if self.children.is_empty() {
            let (dw, dh) = (self.w - w, self.h - h);
            self.children.push(Self::new(self.x, self.y, w, h));
            if dw > 0 && dh > 0 {
                if self.h.max(dw) >= self.w.max(dh) {
                    self.children.push(Self::new(self.x, self.y + h, w, dh));
                    self.children
                        .push(Self::new(self.x + w, self.y, dw, self.h));
                } else {
                    self.children.push(Self::new(self.x + w, self.y, dw, h));
                    self.children
                        .push(Self::new(self.x, self.y + h, self.w, dh));
                }
            } else if dw == 0 {
                self.children.push(Self::new(self.x, self.y + h, w, dh));
            } else {
                self.children.push(Self::new(self.x + w, self.y, dw, h));
            }
        }
        self.children.iter_mut().any(|r| r.add(index, w, h))
    }
    fn gather(
        &self,
        entries: &[Entry],
        size: [u32; 2],
        padding: u32,
        out: &mut HashMap<String, Rect>,
    ) {
        if let Some(i) = self.entry {
            let (name, w, h) = &entries[i];
            let (x, y) = (self.x + padding, self.y + padding);
            out.insert(
                super::sprite_name(name).to_owned(),
                Rect {
                    u0: x as f32 / size[0] as f32,
                    v0: y as f32 / size[1] as f32,
                    u1: (x + w) as f32 / size[0] as f32,
                    v1: (y + h) as f32 / size[1] as f32,
                    w: *w,
                    h: *h,
                },
            );
        }
        for child in &self.children {
            child.gather(entries, size, padding, out);
        }
    }
}

/// `Stitcher.stitch/expand` at `mip_level` with `padding` texels on each side.
fn stitch(
    entries: &[Entry],
    mip_level: u32,
    padding: u32,
    maximum: u32,
) -> Option<HashMap<String, Rect>> {
    let mut order: Vec<_> = entries
        .iter()
        .enumerate()
        .map(|(i, (name, w, h))| {
            let (ns, path) = name.split_once(':').unwrap_or(("minecraft", name));
            (
                i,
                smallest_fitting_min_texel(w + padding * 2, mip_level),
                smallest_fitting_min_texel(h + padding * 2, mip_level),
                path,
                ns,
            )
        })
        .collect();
    order.sort_by(|a, b| {
        b.2.cmp(&a.2)
            .then(b.1.cmp(&a.1))
            .then(a.3.cmp(b.3))
            .then(a.4.cmp(b.4))
    });
    let mut regions: Vec<Region> = Vec::new();
    let (mut width, mut height) = (0u32, 0u32);
    let pow2 = |x: u32| if x == 0 { 0 } else { x.next_power_of_two() };
    for (index, w, h, _, _) in order {
        if regions.iter_mut().any(|r| r.add(index, w, h)) {
            continue;
        }
        let (next_w, next_h) = (pow2(width.checked_add(w)?), pow2(height.checked_add(h)?));
        let (can_w, can_h) = (next_w <= maximum, next_h <= maximum);
        if !can_w && !can_h {
            return None;
        }
        let (grow_w, grow_h) = (
            can_w && pow2(width) != next_w,
            can_h && pow2(height) != next_h,
        );
        let horizontal = if grow_w != grow_h {
            grow_w
        } else {
            can_w && pow2(width) <= pow2(height)
        };
        let mut region = if horizontal {
            if height == 0 {
                height = next_h;
            }
            let r = Region::new(width, 0, next_w - width, height);
            width = next_w;
            r
        } else {
            let r = Region::new(0, height, width, next_h - height);
            height = next_h;
            r
        };
        // Pinned expand ignores add's result for a sprite wider than the device limit.
        region.add(index, w, h);
        regions.push(region);
    }
    let mut out = HashMap::new();
    for r in regions {
        r.gather(entries, [width, height], padding, &mut out);
    }
    Some(out)
}
