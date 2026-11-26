//! Starling's part of atlas_pack.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl HudAtlas {
    /// Pack against a skyline so space below shorter neighbours stays usable.
    /// Pixel UI must not be reduced merely because tall shelves leave gaps.
    #[cfg(any(target_os = "android", test))]
    pub(super) fn mobile_layout(
        images: &[PackedImage],
        maximum: u32,
    ) -> Result<(u32, Vec<(u32, u32)>, u32)> {
        let width = maximum.min(4096);
        // [x, occupied height, segment width]. Segments cover the complete width.
        let mut skyline = vec![[0u32, 0, width]];
        let mut placements = Vec::with_capacity(images.len());
        let mut bottom = 0;
        for (_, w, h, _) in images {
            let (rw, rh) = (w + 1, h + 1);
            let mut best: Option<(u32, u32, usize, u32)> = None;
            for (i, node) in skyline.iter().enumerate() {
                let x = node[0];
                if x + rw > width {
                    continue;
                }
                let (mut left, mut y, mut j) = (rw, 0, i);
                while left > 0 {
                    y = y.max(skyline[j][1]);
                    left = left.saturating_sub(skyline[j][2]);
                    j += 1;
                }
                let candidate = (y + rh, x, i, y);
                if best.is_none_or(|previous| candidate < previous) {
                    best = Some(candidate);
                }
            }
            let Some((top, x, index, y)) = best else {
                return Err("A GUI texture exceeds the mobile atlas width".into());
            };
            if top + GLYPH_ROWS > maximum {
                return Err(format!(
                    "GUI atlas needs {} rows; adapter permits {maximum}",
                    top + GLYPH_ROWS
                )
                .into());
            }
            placements.push((x, y));
            bottom = bottom.max(top);
            let end = x + rw;
            skyline.insert(index, [x, top, rw]);
            let j = index + 1;
            while j < skyline.len() && skyline[j][0] < end {
                let covered = end - skyline[j][0];
                if covered < skyline[j][2] {
                    skyline[j][0] += covered;
                    skyline[j][2] -= covered;
                    break;
                }
                skyline.remove(j);
            }
            let mut j = 0;
            while j + 1 < skyline.len() {
                if skyline[j][1] == skyline[j + 1][1] {
                    skyline[j][2] += skyline[j + 1][2];
                    skyline.remove(j + 1);
                } else {
                    j += 1;
                }
            }
        }
        Ok((width, placements, bottom))
    }

    /// Fit static sprites to a small adapter without shrinking pixel icons, glyphs or dynamic
    /// map/skin/GUI slots. Terrain uses separate texture arrays and is unchanged.
    #[cfg(any(target_os = "android", test))]
    pub(super) fn fit_mobile_atlas(
        images: &mut [PackedImage],
        gui_sizes: &mut HashMap<String, (u32, u32)>,
        flags: &HashMap<String, TextureFlags>,
        limit: u32,
    ) -> Result<()> {
        loop {
            let mut dimensions: Vec<PackedImage> = images
                .iter()
                .map(|(name, w, h, _)| {
                    let border = if flags.get(name).is_some_and(|f| f.blur) {
                        2
                    } else {
                        0
                    };
                    (name.clone(), w + border, h + border, Vec::new())
                })
                .collect();
            dimensions.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)));
            match Self::layout(&dimensions, limit) {
                Ok(_) => return Ok(()),
                Err(error) => {
                    // The interface's own textures wait until the rest is as small as it goes:
                    // a phone whose GPU takes 4096 and whose default resources need 4108 rows
                    // (the PowerVR GE8322) then fits them, instead of losing its packs.
                    let mut changed = false;
                    for interface in [false, true] {
                        for (name, w, h, rgba) in images.iter_mut() {
                            // Never the glyphs, and never the slots written at run time
                            // (`slots`, all under `sunrise/`), whose size is their own.
                            if name.contains("font/")
                                || name.starts_with("sunrise/")
                                || (*w).max(*h) <= SMALLEST_SHRUNK_SIDE
                                || rgba.len() < (*w as usize * *h as usize * 4)
                            {
                                continue;
                            }
                            if (name.starts_with("gui/") || name.starts_with("ui/")) != interface {
                                continue;
                            }
                            // Keep original GUI units and normalized UVs. Runtime patches
                            // retain their full-size slots, so no coordinate remapping occurs.
                            gui_sizes.entry(name.clone()).or_insert((*w, *h));
                            let (old_w, old_h) = (*w, *h);
                            *w = (*w / 2).max(1);
                            *h = (*h / 2).max(1);
                            let mut smaller = Vec::with_capacity((*w * *h * 4) as usize);
                            for y in 0..*h {
                                for x in 0..*w {
                                    let at =
                                        ((y * old_h / *h * old_w + x * old_w / *w) * 4) as usize;
                                    smaller.extend_from_slice(&rgba[at..at + 4]);
                                }
                            }
                            *rgba = smaller;
                            changed = true;
                        }
                        if changed {
                            log::info!(
                                "Reduced {} atlas sprites to fit the adapter's {limit}px texture limit",
                                if interface { "interface" } else { "non-UI" }
                            );
                            break;
                        }
                    }
                    if !changed {
                        return Err(error);
                    }
                }
            }
        }
    }

    /// `layout_leaving_out`, but when the images still don't fit (a small GPU: the PowerVR GE8320
    /// takes 4096x4096 and the default resources need 4096x5120), with the largest at half size,
    /// then the largest of what is left, until they do. A sprite's `Rect` keeps its size in
    /// `full_sizes` and draws the same, with fewer texels. What is written or read by the texel
    /// keeps its size: the run-time slots, the fonts, and the sprites with a wrapped border.
    pub(super) fn layout_within(
        images: &mut Vec<PackedImage>,
        animated: &mut Vec<String>,
        extras: &HashSet<String>,
        bordered: &HashSet<String>,
        max: u32,
        full_sizes: &mut HashMap<String, (u32, u32)>,
    ) -> Result<(u32, Vec<(u32, u32)>, u32)> {
        let shrinks = |name: &str| {
            !name.starts_with("sunrise/")
                && !name.contains("font/")
                && name != MISSING_SPRITE
                && !bordered.contains(name)
        };
        loop {
            let error = match Self::layout_leaving_out(images, animated, extras, max) {
                Ok(layout) => return Ok(layout),
                Err(error) => error,
            };
            let largest = |gui: bool| {
                images
                    .iter()
                    .filter(|i| shrinks(&i.0) && i.0.contains("gui/") == gui)
                    .map(|i| i.1.max(i.2))
                    .max()
                    .unwrap_or(0)
            };
            // The interface last, once the rest is down to this: the logo shows it the most.
            let world = largest(false);
            let world_first = world > INTERFACE_WAITS_FOR_SIDE;
            let side = if world_first {
                world
            } else {
                world.max(largest(true))
            };
            if side < SMALLEST_SHRUNK_SIDE {
                return Err(error);
            }
            for image in images.iter_mut() {
                if shrinks(&image.0)
                    && image.1.max(image.2) == side
                    && !(world_first && image.0.contains("gui/"))
                {
                    full_sizes
                        .entry(image.0.clone())
                        .or_insert((image.1, image.2));
                    *image = halved(image);
                }
            }
            images.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)));
            log::warn!("the atlas does not fit {max}x{max}: {side} px textures at half size");
        }
    }
}
