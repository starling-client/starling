use super::{HudAtlas, PackedImage, Result, SMALLEST_SHRUNK_SIDE, TextureFlags, halved};
use std::collections::HashMap;

/// Average the source footprint around each destination texel. Its edges line up with
/// the original sprite, including odd sizes. Weight colors by alpha so invisible texels
/// don't darken a visible outline or icon.
fn half_ui((name, w, h, pixels): &PackedImage) -> PackedImage {
    let (width, height) = (w.div_ceil(2), h.div_ceil(2));
    let mut reduced = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        let (top, bottom) = (y * h, (y + 1) * h);
        for x in 0..width {
            let (left, right) = (x * w, (x + 1) * w);
            let mut sum = [0_u64; 4];
            for sy in top / height..bottom.div_ceil(height) {
                let cover_y = bottom.min((sy + 1) * height) - top.max(sy * height);
                for sx in left / width..right.div_ceil(width) {
                    let cover_x = right.min((sx + 1) * width) - left.max(sx * width);
                    let index = ((sy * w + sx) * 4) as usize;
                    let alpha =
                        u64::from(cover_x) * u64::from(cover_y) * u64::from(pixels[index + 3]);
                    for channel in 0..3 {
                        sum[channel] += alpha * u64::from(pixels[index + channel]);
                    }
                    sum[3] += alpha;
                }
            }
            let total = u64::from(*w) * u64::from(*h);
            let [r, g, b, alpha] = sum;
            let color = [r, g, b].map(|c| ((c + alpha / 2) / alpha.max(1)) as u8);
            reduced.extend([
                color[0],
                color[1],
                color[2],
                ((alpha + total / 2) / total) as u8,
            ]);
        }
    }
    (name.clone(), width, height, reduced)
}

impl HudAtlas {
    /// Fit static sprites to a small adapter. Decorative pictures give up detail first,
    /// then world textures, interface sprites and oversized button art.
    pub(super) fn fit_mobile_atlas(
        images: &mut [PackedImage],
        full_sizes: &mut HashMap<String, (u32, u32)>,
        gui_sizes: &HashMap<String, (u32, u32)>,
        flags: &HashMap<String, TextureFlags>,
        limit: u32,
    ) -> Result<()> {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        enum Priority {
            Decoration,
            World,
            Interface,
            Controls,
        }

        let shrink_rank = |(name, w, h, rgba): &PackedImage| {
            let path = name.split_once(':').map_or(name.as_str(), |(_, path)| path);
            let side = (*w).max(*h);
            // The two standard skin layouts use at most 64×64 texels. The slots for
            // runtime patches keep their own pixel coordinates.
            if path.contains("font/")
                || name.starts_with("horizon/")
                || (path.starts_with("entity/player/") && side <= 64)
                || side <= SMALLEST_SHRUNK_SIDE
                || rgba.len() < (*w as usize * *h as usize * 4)
            {
                return None;
            }
            let button_background =
                path.strip_prefix("gui/sprites/widget/")
                    .is_some_and(|widget| {
                        widget.starts_with("button") || widget.starts_with("realms_button")
                    });
            let touch = path.starts_with("gui/bedrock/")
                || path.starts_with("gui/sprites/touch/")
                || path.starts_with("android-controls/");
            // Only remove a level of detail if the button still has at least its GUI size.
            // Smaller pack replacements and standard touch sheets are already pixel art.
            let (gui_w, gui_h) = gui_sizes.get(name).copied().unwrap_or((*w, *h));
            if (button_background && (w.div_ceil(2) < gui_w || h.div_ceil(2) < gui_h))
                || (touch && side <= 256)
            {
                return None;
            }
            let decoration = path.starts_with("gui/advancements/backgrounds/")
                || matches!(
                    path,
                    "gui/title/mojangstudios"
                        | "gui/title/minceraft"
                        | "gui/realms/adventure"
                        | "gui/realms/experience"
                        | "gui/realms/inspiration"
                        | "gui/realms/new_world"
                        | "gui/realms/survival_spawn"
                        | "gui/realms/upload"
                );
            let priority = if button_background || touch {
                Priority::Controls
            } else if decoration {
                if side <= 128 {
                    return None;
                }
                Priority::Decoration
            } else if path.starts_with("gui/") || path.starts_with("ui/") {
                Priority::Interface
            } else {
                Priority::World
            };
            Some((priority, std::cmp::Reverse(side)))
        };
        let pack = |dimensions: &[PackedImage]| {
            let mut sorted = dimensions.to_vec();
            sorted.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)));
            if limit <= 4096 {
                Self::mobile_layout(&sorted, limit)
            } else {
                Self::layout(&sorted, limit)
            }
        };
        let borders: Vec<_> = images
            .iter()
            .map(|(name, ..)| {
                if flags.get(name).is_some_and(|f| f.blur) {
                    2
                } else {
                    0
                }
            })
            .collect();
        let mut dimensions: Vec<PackedImage> = images
            .iter()
            .zip(&borders)
            .map(|((_, w, h, _), border)| (String::new(), w + border, h + border, Vec::new()))
            .collect();
        let Err(mut error) = pack(&dimensions) else {
            return Ok(());
        };
        loop {
            // Repack after reducing one size group. Small sprites can keep all their
            // texels when making room in a few large sprites is enough.
            let Some(rank @ (priority, std::cmp::Reverse(side))) =
                images.iter().filter_map(shrink_rank).min()
            else {
                return Err(error);
            };
            let mut group: Vec<_> = images
                .iter()
                .enumerate()
                .filter_map(|(index, image)| (shrink_rank(image) == Some(rank)).then_some(index))
                .collect();
            group.sort_by_key(|&index| {
                let (_, w, h, _) = &images[index];
                std::cmp::Reverse(u64::from(*w) * u64::from(*h))
            });
            let set_trial = |dimensions: &mut [PackedImage], count| {
                for (index, &sprite) in group.iter().enumerate() {
                    let (_, w, h, _) = &images[sprite];
                    let divisor = if index < count { 2 } else { 1 };
                    dimensions[sprite].1 = w.div_ceil(divisor) + borders[sprite];
                    dimensions[sprite].2 = h.div_ceil(divisor) + borders[sprite];
                }
            };
            let mut count = group.len();
            set_trial(&mut dimensions, count);
            let remaining = pack(&dimensions).err();
            if remaining.is_none() {
                // Keep a known fit while trying smaller prefixes of this size group.
                let mut failed = 0;
                while count - failed > 1 {
                    let middle = failed + (count - failed) / 2;
                    set_trial(&mut dimensions, middle);
                    if pack(&dimensions).is_ok() {
                        count = middle;
                    } else {
                        failed = middle;
                    }
                }
            }
            for &index in &group[..count] {
                let image = &mut images[index];
                full_sizes
                    .entry(image.0.clone())
                    .or_insert((image.1, image.2));
                *image = match priority {
                    Priority::World => halved(image),
                    _ => half_ui(image),
                };
            }
            let category = match priority {
                Priority::Decoration => "decorative",
                Priority::World => "non-UI",
                Priority::Interface => "interface",
                Priority::Controls => "controls and button backgrounds",
            };
            log::info!(
                "Reduced {count} {category} atlas sprites with a {side}px side to fit the adapter's {limit}px texture limit"
            );
            let Some(next_error) = remaining else {
                return Ok(());
            };
            error = next_error;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_filter_preserves_mirrored_edges_and_transparent_color() {
        for side in [6, 7] {
            let mut pixels = Vec::new();
            for y in 0..side {
                for x in 0..side {
                    let pixel = if x == 0 || y == 0 || x + 1 == side || y + 1 == side {
                        [0, 0, 0, 0]
                    } else {
                        [81, 143, 207, 255]
                    };
                    pixels.extend(pixel);
                }
            }
            let (_, width, height, filtered) = half_ui(&("button".into(), side, side, pixels));
            let at = |x, y| {
                let index = ((y * width + x) * 4) as usize;
                &filtered[index..index + 4]
            };
            for y in 0..height {
                for x in 0..width {
                    assert_eq!(at(x, y), at(width - 1 - x, y));
                    assert_eq!(at(x, y), at(x, height - 1 - y));
                    assert_eq!(&at(x, y)[..3], &[81, 143, 207]);
                }
            }
            assert_eq!(at(1, 1), &[81, 143, 207, 255]);
            assert!(at(0, 0)[3] < 128);
        }
    }

    #[test]
    fn small_atlas_keeps_buttons_and_control_sprites_pixel_exact() {
        let art = |name: &str, w, h| {
            let mut pixels = vec![255; w * h * 4];
            for (i, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let (x, y) = (i % w, i / w);
                if x == 0 || y == 0 || x + 1 == w || y + 1 == h {
                    *pixel = [0, 0, 0, 255];
                } else if x == 1 || y == 1 {
                    *pixel = [128, 128, 128, 255];
                }
            }
            (name.to_owned(), w as u32, h as u32, pixels)
        };
        let original = [
            art("gui/sprites/widget/button", 200, 20),
            art("minecraft:gui/sprites/widget/button_highlighted", 70, 70),
            art("gui/bedrock/touchgui", 128, 128),
            art("starling:gui/sprites/touch/attack_pressed", 64, 64),
            art("starling:android-controls/settings", 64, 64),
        ];
        let mut images = original.to_vec();
        images.push(art("entity/mob", 400, 240));
        images.push(art("gui/realms/adventure", 256, 256));
        let mut full_sizes = HashMap::new();
        HudAtlas::fit_mobile_atlas(
            &mut images,
            &mut full_sizes,
            &HashMap::new(),
            &HashMap::new(),
            768,
        )
        .unwrap();

        assert_eq!(&images[..original.len()], &original);
        assert_eq!((images[5].1, images[5].2), (400, 240));
        assert_eq!((images[6].1, images[6].2), (128, 128));
        assert_eq!(
            full_sizes,
            HashMap::from([(images[6].0.clone(), (256, 256))])
        );
    }

    #[test]
    fn hd_button_can_lose_texels_above_its_gui_size() {
        let button = "gui/sprites/widget/button".to_owned();
        let mut images = [
            ("horizon/ui-text".into(), 600, 180, Vec::new()),
            (button.clone(), 400, 50, vec![255; 400 * 50 * 4]),
        ];
        let gui_sizes = HashMap::from([(button, (200, 20))]);
        HudAtlas::fit_mobile_atlas(
            &mut images,
            &mut HashMap::new(),
            &gui_sizes,
            &HashMap::new(),
            720,
        )
        .unwrap();
        assert_eq!((images[1].1, images[1].2), (200, 25));
    }

    #[test]
    fn small_atlas_reduces_only_the_sprites_needed_from_the_last_size_group() {
        let mut images: Vec<PackedImage> = (0..4)
            .map(|i| (format!("entity/{i}"), 200, 200, vec![255; 200 * 200 * 4]))
            .collect();
        let mut full_sizes = HashMap::new();
        HudAtlas::fit_mobile_atlas(
            &mut images,
            &mut full_sizes,
            &HashMap::new(),
            &HashMap::new(),
            768,
        )
        .unwrap();

        assert_eq!(
            images
                .iter()
                .filter(|image| image.1 == 200 && image.2 == 200)
                .count(),
            3
        );
        let reduced = images
            .iter()
            .find(|image| image.1 == 100 && image.2 == 100)
            .unwrap();
        assert_eq!(full_sizes, HashMap::from([(reduced.0.clone(), (200, 200))]));
    }
}
